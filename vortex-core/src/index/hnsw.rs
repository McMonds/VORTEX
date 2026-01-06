use crate::index::VectorIndex;
use crate::index::simd;
use super::quantization;
use log::info;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};

#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    distance: f32,
    node_id: usize,
}

impl Eq for Candidate {}

// Wrapper for Min-Heap (BinaryHeap pops largest, so Reverse comparisons)
#[derive(PartialEq, Eq)]
struct MinCandidate(Candidate);
impl PartialOrd for MinCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        other.0.distance.partial_cmp(&self.0.distance)
    }
}
impl Ord for MinCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

// Wrapper for Max-Heap
#[derive(PartialEq, Eq)]
struct MaxCandidate(Candidate);
impl PartialOrd for MaxCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.distance.partial_cmp(&other.0.distance)
    }
}
impl Ord for MaxCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

pub struct HnswIndex {
    dimension: usize,
    max_elements: usize,
    
    // HNSW Parameters
    m: usize,
    m0: usize,
    ef_construction: usize,
    max_layers: usize,

    // The "Giant Arena" (Rule 3 Optimization)
    arena: RwLock<Vec<f32>>,
    
    // The "Quantized Arena" (Target 2 Integration)
    quantized_arena: RwLock<Vec<u8>>,
    magnitudes: RwLock<Vec<f32>>,

    // Reverse Map: Logical Index -> External ID
    external_ids: RwLock<Vec<u64>>,

    // The "Flat Link Arena"
    link_arena: RwLock<Vec<u32>>,
    
    // Maps User ID -> Logical Index
    map: RwLock<HashMap<u64, usize>>,

    // Entry Point for the HNSW Graph
    entry_point: AtomicU32,
    max_layer_active: AtomicU32,

    // Visited Versioning
    visited_tags: RwLock<Vec<u32>>,
    global_search_id: AtomicU32,

    // High-speed distance kernel
    metric_kernel: simd::SimdFunc,
}

impl HnswIndex {
    pub fn new(dimension: usize, max_elements: usize) -> Self {
        info!("Initializing Multi-Layer HNSW Index (Dim: {}, Max: {})", dimension, max_elements);
        
        let m = 16;
        let m0 = 32;
        let ef_construction = 128;
        let max_layers = 16; 
        
        let link_stride = m0 + (max_layers - 1) * m;
        
        Self { 
            dimension, 
            max_elements,
            m,
            m0,
            ef_construction,
            max_layers,
            arena: RwLock::new(Vec::with_capacity(max_elements * dimension)),
            quantized_arena: RwLock::new(Vec::with_capacity(max_elements * dimension)),
            magnitudes: RwLock::new(Vec::with_capacity(max_elements)),
            external_ids: RwLock::new(Vec::with_capacity(max_elements)),
            link_arena: RwLock::new(vec![u32::MAX; max_elements * link_stride]),
            map: RwLock::new(HashMap::with_capacity(max_elements)),
            entry_point: AtomicU32::new(u32::MAX),
            max_layer_active: AtomicU32::new(0),
            visited_tags: RwLock::new(vec![0; max_elements]),
            global_search_id: AtomicU32::new(1),
            metric_kernel: simd::get_vector_kernel(),
        }
    }

    #[inline(always)]
    fn link_stride(&self) -> usize {
        self.m0 + (self.max_layers - 1) * self.m
    }

    #[inline(always)]
    fn link_offset(&self, node_id: usize, level: usize) -> usize {
        let base = node_id * self.link_stride();
        if level == 0 { base } else { base + self.m0 + (level - 1) * self.m }
    }

    fn get_neighbors<'a>(&self, link_arena: &'a [u32], node_id: usize, level: usize) -> &'a [u32] {
        let offset = self.link_offset(node_id, level);
        let max_links = if level == 0 { self.m0 } else { self.m };
        let slice = &link_arena[offset..offset + max_links];
        let mut count = 0;
        while count < max_links && slice[count] != u32::MAX { count += 1; }
        &slice[..count]
    }

    fn add_neighbor(&self, link_arena: &mut [u32], node_id: usize, level: usize, neighbor_id: u32) {
        let offset = self.link_offset(node_id, level);
        let max_links = if level == 0 { self.m0 } else { self.m };
        let slice = &mut link_arena[offset..offset + max_links];
        for i in 0..max_links {
            if slice[i] == u32::MAX { slice[i] = neighbor_id; return; }
            if slice[i] == neighbor_id { return; }
        }
    }

    fn next_search_version(&self) -> u32 {
        self.global_search_id.fetch_add(1, AtomicOrdering::Relaxed)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_layer_f32(
        &self,
        query: &[f32],
        ep: usize,
        ef: usize,
        level: usize,
        arena: &[f32],
        link_arena: &[u32],
        visited: &mut [u32],
        search_id: u32,
    ) -> Vec<Candidate> {
        let mut candidates = BinaryHeap::new();
        let mut results = BinaryHeap::new();
        let dist = unsafe { (self.metric_kernel)(query.as_ptr(), arena.as_ptr().add(ep * self.dimension), self.dimension) };
        let entry = Candidate { node_id: ep, distance: dist };
        candidates.push(MinCandidate(entry.clone()));
        results.push(MaxCandidate(entry));
        visited[ep] = search_id;
        while let Some(MinCandidate(top)) = candidates.pop() {
            if top.distance > results.peek().unwrap().0.distance && results.len() >= ef { break; }
            let neighbors = self.get_neighbors(link_arena, top.node_id, level);
            for &nid in neighbors {
                if visited[nid as usize] == search_id { continue; }
                visited[nid as usize] = search_id;
                let d = unsafe { (self.metric_kernel)(query.as_ptr(), arena.as_ptr().add(nid as usize * self.dimension), self.dimension) };
                if results.len() < ef || d < results.peek().unwrap().0.distance {
                    let c = Candidate { node_id: nid as usize, distance: d };
                    candidates.push(MinCandidate(c.clone()));
                    results.push(MaxCandidate(c));
                    if results.len() > ef { results.pop(); }
                }
            }
        }
        let mut res_vec: Vec<Candidate> = results.into_iter().map(|mc| mc.0).collect();
        res_vec.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        res_vec
    }

    #[allow(clippy::too_many_arguments)]
    fn search_layer_u8(
        &self,
        q_i8: &[i8],
        ep: usize,
        ef: usize,
        level: usize,
        q_arena: &[u8],
        link_arena: &[u32],
        visited: &mut [u32],
        search_id: u32,
    ) -> Vec<Candidate> {
        let mut candidates = BinaryHeap::new();
        let mut results = BinaryHeap::new();
        let dist = unsafe { simd::dot_product_u8_avx2(q_i8.as_ptr(), q_arena.as_ptr().add(ep * self.dimension), self.dimension) } as f32;
        let entry = Candidate { node_id: ep, distance: dist };
        candidates.push(MinCandidate(entry.clone()));
        results.push(MaxCandidate(entry));
        visited[ep] = search_id;
        while let Some(MinCandidate(top)) = candidates.pop() {
            if top.distance > results.peek().unwrap().0.distance && results.len() >= ef { break; }
            let neighbors = self.get_neighbors(link_arena, top.node_id, level);
            for (i, &nid) in neighbors.iter().enumerate() {
                if let Some(&next_id) = neighbors.get(i + 1) {
                    #[cfg(target_arch = "x86_64")]
                    unsafe { _mm_prefetch(q_arena.as_ptr().add(next_id as usize * self.dimension) as *const i8, _MM_HINT_T0); }
                }
                if visited[nid as usize] == search_id { continue; }
                visited[nid as usize] = search_id;
                let d = unsafe { simd::dot_product_u8_avx2(q_i8.as_ptr(), q_arena.as_ptr().add(nid as usize * self.dimension), self.dimension) } as f32;
                if results.len() < ef || d < results.peek().unwrap().0.distance {
                    let c = Candidate { node_id: nid as usize, distance: d };
                    candidates.push(MinCandidate(c.clone()));
                    results.push(MaxCandidate(c));
                    if results.len() > ef { results.pop(); }
                }
            }
        }
        let mut res_vec: Vec<Candidate> = results.into_iter().map(|mc| mc.0).collect();
        res_vec.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        res_vec
    }

    fn random_level(&self) -> usize {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let mut level = 0;
        while rng.gen::<f32>() < 0.5 && level < self.max_layers - 1 { level += 1; }
        level
    }

    fn prune_connections(&self, link_arena: &mut [u32], node_id: usize, level: usize, arena: &[f32]) {
        let offset = self.link_offset(node_id, level);
        let max_links = if level == 0 { self.m0 } else { self.m };
        let slice = &mut link_arena[offset..offset + max_links];
        let mut neighbors = Vec::with_capacity(max_links);
        for &nid in slice.iter() {
            if nid == u32::MAX { break; }
            let d = unsafe { (self.metric_kernel)(arena.as_ptr().add(node_id * self.dimension), arena.as_ptr().add(nid as usize * self.dimension), self.dimension) };
            neighbors.push((nid, d));
        }
        if neighbors.len() <= max_links { return; }
        neighbors.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        for i in 0..max_links { slice[i] = neighbors[i].0; }
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&mut self, id: u64, vector: &[f32]) {
        assert_eq!(vector.len(), self.dimension, "Dimension Mismatch");
        let mut arena = self.arena.write().unwrap();
        let mut map = self.map.write().unwrap();
        let mut external_ids = self.external_ids.write().unwrap();
        if map.len() >= self.max_elements { return; }
        let logical_idx = arena.len() / self.dimension;
        arena.extend_from_slice(vector);
        let (q_vec, mag) = quantization::ScalarQuantizer::quantize_vector(vector);
        self.quantized_arena.write().unwrap().extend_from_slice(&q_vec);
        self.magnitudes.write().unwrap().push(mag);
        map.insert(id, logical_idx);
        external_ids.push(id);
        let mut link_arena = self.link_arena.write().unwrap();
        let mut visited_tags = self.visited_tags.write().unwrap();
        let ep = self.entry_point.load(AtomicOrdering::Relaxed);
        let max_l = self.max_layer_active.load(AtomicOrdering::Relaxed) as usize;
        let node_level = self.random_level();
        if ep == u32::MAX {
            self.entry_point.store(logical_idx as u32, AtomicOrdering::Relaxed);
            self.max_layer_active.store(node_level as u32, AtomicOrdering::Relaxed);
            return;
        }
        let mut curr_obj = ep as usize;
        let search_id = self.next_search_version();
        if node_level < max_l {
            for level in (node_level + 1..=max_l).rev() {
                let candidates = self.search_layer_f32(vector, curr_obj, 1, level, &arena, &link_arena, &mut visited_tags, search_id);
                if let Some(c) = candidates.get(0) { curr_obj = c.node_id; }
            }
        }
        let start_layer = std::cmp::min(node_level, max_l);
        for level in (0..=start_layer).rev() {
            let candidates = self.search_layer_f32(vector, curr_obj, self.ef_construction, level, &arena, &link_arena, &mut visited_tags, search_id);
            let max_neighbors = if level == 0 { self.m0 } else { self.m };
            for c in candidates.iter().take(max_neighbors) {
                self.add_neighbor(&mut link_arena, logical_idx, level, c.node_id as u32);
                self.add_neighbor(&mut link_arena, c.node_id, level, logical_idx as u32);
                self.prune_connections(&mut link_arena, c.node_id, level, &arena);
            }
            if let Some(top) = candidates.get(0) { curr_obj = top.node_id; }
        }
        if node_level > max_l {
            self.entry_point.store(logical_idx as u32, AtomicOrdering::Relaxed);
            self.max_layer_active.store(node_level as u32, AtomicOrdering::Relaxed);
        }
    }

    fn search(&self, query: &[f32], top_k: usize) -> Vec<(u64, f32)> {
        let arena = self.arena.read().unwrap();
        let link_arena = self.link_arena.read().unwrap();
        let external_ids = self.external_ids.read().unwrap();
        let q_arena = self.quantized_arena.read().unwrap();
        let (q_i8, _) = quantization::ScalarQuantizer::quantize_query(query);
        let ep = self.entry_point.load(AtomicOrdering::Relaxed);
        let max_l = self.max_layer_active.load(AtomicOrdering::Relaxed) as usize;
        if ep == u32::MAX || arena.is_empty() { return Vec::new(); }
        let mut visited_tags = self.visited_tags.write().unwrap();
        let search_id = self.next_search_version();
        let mut curr_obj = ep as usize;
        for level in (1..=max_l).rev() {
            let candidates = self.search_layer_u8(&q_i8, curr_obj, 1, level, &q_arena, &link_arena, &mut visited_tags, search_id);
            if let Some(c) = candidates.get(0) { curr_obj = c.node_id; }
        }
        let ef_search = top_k.max(self.ef_construction);
        let coarse_candidates = self.search_layer_u8(&q_i8, curr_obj, ef_search, 0, &q_arena, &link_arena, &mut visited_tags, search_id);
        let mut refined: Vec<(u64, f32)> = coarse_candidates.into_iter()
            .map(|c| {
                let nid = c.node_id;
                let d = unsafe { (self.metric_kernel)(query.as_ptr(), arena.as_ptr().add(nid * self.dimension), self.dimension) };
                (external_ids[nid], d)
            }).collect();
        refined.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        refined.truncate(top_k);
        refined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_hnsw_precision_recall() {
        let mut index = HnswIndex::new(3, 100);
        index.insert(0, &[1.0, 0.0, 0.0]);
        index.insert(1, &[0.0, 1.0, 0.0]);
        index.insert(2, &[0.0, 0.0, 1.0]);
        let results = index.search(&[0.1, 0.9, 0.1], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }
}

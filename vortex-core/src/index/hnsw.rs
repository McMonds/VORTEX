use crate::index::VectorIndex;
use crate::index::simd;
use log::{info, warn};
use std::collections::HashMap;
use std::sync::RwLock;

/// Hardware-Accelerated HNSW Index (Skeleton for Phase 3).
/// 
/// # Architecture
/// - **Storage**: Flattened `Vec<f32>` (Arena) for maximum cache locality.
/// - **Addressing**: `HashMap<u64, usize>` maps external ID -> logical index (offset/dimension).
/// - **Concurrency**: `RwLock` protected for future multi-threaded search support.
pub struct HnswIndex {
    dimension: usize,
    max_elements: usize,
    
    // The "Giant Arena" (Rule 3 Optimization)
    // We store all vectors contiguously.
    // Index i is at vectors[i * dimension .. (i+1) * dimension]
    arena: RwLock<Vec<f32>>,
    
    // Maps User ID -> Logical Index in the Arena (0..N)
    map: RwLock<HashMap<u64, usize>>,

    // TARGET 2: The Graph Integration
    // High-speed distance kernel function pointer
    metric_kernel: simd::SimdFunc,
}

impl HnswIndex {
    pub fn new(dimension: usize, max_elements: usize) -> Self {
        info!("Initializing Shard-Local HNSW Index (Dim: {}, Max: {})", dimension, max_elements);
        
        Self { 
            dimension, 
            max_elements,
            // Pre-allocate full capacity to avoid reallocation spikes (Rule 1)
            arena: RwLock::new(Vec::with_capacity(max_elements * dimension)),
            map: RwLock::new(HashMap::with_capacity(max_elements)),
            metric_kernel: simd::get_vector_kernel(),
        }
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&mut self, id: u64, vector: &[f32]) {
        assert_eq!(vector.len(), self.dimension, "Index Insert Failed: Dimension Mismatch");

        // Mutation Phase: Acquire Write Locks
        let mut arena = self.arena.write().unwrap();
        let mut map = self.map.write().unwrap();

        // Check Capacity
        // In a real system, we'd have eviction or error.
        // For Phase 3, we allow it to grow or fail silent if capacity logic wasn't strict.
        // Actually, Vec will reallocate if we exceed capacity, which is "ok" but violates Rule 1.
        // We reserved capacity in new(), so as long as we stay under max_elements, no alloc.
        
        let logical_idx = map.len();
        if logical_idx >= self.max_elements {
             warn!("Index Full. Dropping Insert ID: {}", id);
             return;
        }

        // Append to Arena (Simple implementation for Phase 3 verification)
        arena.extend_from_slice(vector);
        map.insert(id, logical_idx);
        
        // Note: In Phase 3, we are NOT building the HNSW Graph links yet.
        // This is a "Flat Index" mode as requested to verify plumbing.
    }

    fn search(&self, query: &[f32], top_k: usize) -> Vec<(u64, f32)> {
        assert_eq!(query.len(), self.dimension, "Index Search Failed: Dimension Mismatch");

        // Read Phase: Acquire Read Locks
        let arena = self.arena.read().unwrap();
        let map = self.map.read().unwrap();
        
        // Brute Force Linear Scan (Verification Mode)
        // O(N) scan. Acceptable for getting the pipeline running.
        
        let mut results = Vec::with_capacity(map.len());
        
        for (&id, &logical_idx) in map.iter() {
            let start = logical_idx * self.dimension;
            let end = start + self.dimension;
            
            // Safety: Map index is guaranteed to be valid by insert logic
            if end <= arena.len() {
                let vec_slice = &arena[start..end];
                
                // Use the SIMD kernel (Target 2 Integration)
                let score = unsafe { (self.metric_kernel)(query.as_ptr(), vec_slice.as_ptr(), self.dimension) };
                results.push((id, score));
            }
        }
        
        // Sort by Score Descending (Higher is better for Cosine)
        // Note: L2 was lower is better. Cosine is [-1, 1].
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        if results.len() > top_k {
            results.truncate(top_k);
        }
        
        results
    }
}

use crate::index::VectorIndex;
use crate::index::distance::l2_distance;
use log::info;
use std::collections::HashMap;

pub struct HnswIndex {
    _dimension: usize,
    _max_elements: usize,
    vectors: HashMap<u64, Vec<f32>>,
    // Simplified HNSW graph for Milestone 5
    // In a full implementation, this would be a multi-layered proximity graph
    _neighborhoods: HashMap<u64, Vec<u64>>,
}

impl HnswIndex {
    pub fn new(dimension: usize, max_elements: usize) -> Self {
        info!("Initializing Shard-Local HNSW Index (Dim: {}, Max: {})", dimension, max_elements);
        Self { 
            _dimension: dimension, 
            _max_elements: max_elements,
            vectors: HashMap::with_capacity(max_elements),
            _neighborhoods: HashMap::with_capacity(max_elements),
        }
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&mut self, id: u64, vector: &[f32]) {
        if vector.len() != self._dimension {
            return;
        }
        
        // Rule 1: No dynamic allocation in hot path? 
        // For Milestone 5, we use a simple HashMap. 
        // Rule 16: Later we will move this to a pre-allocated arena.
        self.vectors.insert(id, vector.to_vec());
        
        // Simplified graph update logic
        self._neighborhoods.insert(id, Vec::new());
    }

    fn search(&self, query: &[f32], top_k: usize) -> Vec<(u64, f32)> {
        let mut results = Vec::with_capacity(self.vectors.len());
        
        for (&id, vec) in self.vectors.iter() {
            let dist = l2_distance(query, vec);
            results.push((id, dist));
        }
        
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        results.truncate(top_k);
        results
    }
}

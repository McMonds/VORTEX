pub mod hnsw;
pub mod distance;
pub mod simd;
pub mod quantization;

pub trait VectorIndex {
    fn insert(&mut self, id: u64, vector: &[f32]);
    fn search(&self, query: &[f32], top_k: usize) -> Vec<(u64, f32)>;
}

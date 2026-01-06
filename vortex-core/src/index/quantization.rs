/// Scalar Quantizer (Phase 5B: Optimized)
/// 
/// This module implements L2-normalization and linear mapping to u8/i8.
pub struct ScalarQuantizer;

impl ScalarQuantizer {
    /// L2 Normalization helper.
    fn l2_norm(vector: &[f32]) -> f32 {
        let sum: f32 = vector.iter().map(|&x| x * x).sum();
        sum.sqrt()
    }

    /// Database Path: Quantize to u8 (0..255).
    /// Returns (Quantized Vector, Original Magnitude).
    pub fn quantize_vector(vector: &[f32]) -> (Vec<u8>, f32) {
        let magnitude = Self::l2_norm(vector);
        let scale = if magnitude > 0.0 { 1.0 / magnitude } else { 1.0 };
        
        let quantized = vector.iter()
            .map(|&x| {
                let normalized = x * scale;
                // Map [-1.0, 1.0] to [0, 255]
                let mapped = (normalized + 1.0) * 127.5;
                mapped.clamp(0.0, 255.0) as u8
            })
            .collect();
            
        (quantized, magnitude)
    }

    /// Query Path: Quantize to i8 (-128..127).
    /// Returns (Quantized Query, Original Magnitude).
    pub fn quantize_query(query: &[f32]) -> (Vec<i8>, f32) {
        let magnitude = Self::l2_norm(query);
        let scale = if magnitude > 0.0 { 1.0 / magnitude } else { 1.0 };
        
        let quantized = query.iter()
            .map(|&x| {
                let normalized = x * scale;
                // Map [-1.0, 1.0] to [-128, 127]
                let mapped = normalized * 127.0;
                mapped.clamp(-128.0, 127.0) as i8
            })
            .collect();
            
        (quantized, magnitude)
    }
}

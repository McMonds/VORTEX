#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// L2 Square Distance (Euclidean)
/// Automatically selects best SIMD implementation based on CPU features.
pub fn l2_distance(v1: &[f32], v2: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { l2_distance_avx2(v1, v2) };
        }
    }
    
    // Fallback: Scalar implementation
    l2_distance_scalar(v1, v2)
}

fn l2_distance_scalar(v1: &[f32], v2: &[f32]) -> f32 {
    v1.iter().zip(v2.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
}

#[cfg(target_arch = "x86_64")]
unsafe fn l2_distance_avx2(v1: &[f32], v2: &[f32]) -> f32 {
    let mut sum_vec = _mm256_setzero_ps();
    let n = v1.len();
    let n_simd = n - (n % 8);
    
    for i in (0..n_simd).step_by(8) {
        let x = _mm256_loadu_ps(v1.as_ptr().add(i));
        let y = _mm256_loadu_ps(v2.as_ptr().add(i));
        let diff = _mm256_sub_ps(x, y);
        let sq = _mm256_mul_ps(diff, diff);
        sum_vec = _mm256_add_ps(sum_vec, sq);
    }
    
    let mut results = [0.0f32; 8];
    _mm256_storeu_ps(results.as_mut_ptr(), sum_vec);
    
    let mut sum = results.iter().sum::<f32>();
    
    // Tail
    for i in n_simd..n {
        sum += (v1[i] - v2[i]) * (v1[i] - v2[i]);
    }
    
    sum
}

/// Cosine Similarity (DotProduct / (NormA * NormB))
/// Range: [-1.0, 1.0]
#[inline]
pub fn cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
    // Scalar implementation for baseline verification.
    // In production, we would use AVX2 FMA (Fused Multiply Add).
    let dot_product: f32 = v1.iter().zip(v2.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = v2.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm_a * norm_b)
}

use std::arch::x86_64::*;

/// Function pointer signature for high-speed distance computation.
pub type SimdFunc = unsafe fn(*const f32, *const f32, usize) -> f32;

/// Signature for distance functions specifically (e.g. Euclidean).
pub type DistanceFunc = unsafe fn(*const f32, *const f32, usize) -> f32;

/// The Reference Implementation.
/// Safe loop fallback for "potato hardware."
/// Returns: Negative Dot Product (Distance Proxy: Lower is Better)
pub unsafe fn scalar_dot(a: *const f32, b: *const f32, n: usize) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..n {
        sum += (*a.add(i)) * (*b.add(i));
    }
    -sum
}

/// Scalar Integer Dot Product Fallback.
pub fn scalar_dot_u8(q: *const i8, v: *const u8, n: usize) -> i32 {
    let mut sum: i32 = 0;
    for i in 0..n {
        unsafe {
            sum += (*v.add(i) as i16 * *q.add(i) as i16) as i32;
        }
    }
    -sum
}

/// The AVX2 Intrinsic Kernel.
/// Uses 256-bit YMM registers and Fused Multiply-Add (FMA).
#[target_feature(enable = "avx2", enable = "fma")]
pub unsafe fn avx2_dot(a: *const f32, b: *const f32, n: usize) -> f32 {
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut acc2 = _mm256_setzero_ps();
    let mut acc3 = _mm256_setzero_ps();

    let mut i = 0;
    // Loop i from 0 to n in steps of 32 (4 accumulators x 8 floats)
    while i + 32 <= n {
        let va0 = _mm256_loadu_ps(a.add(i));
        let vb0 = _mm256_loadu_ps(b.add(i));
        acc0 = _mm256_fmadd_ps(va0, vb0, acc0);

        let va1 = _mm256_loadu_ps(a.add(i + 8));
        let vb1 = _mm256_loadu_ps(b.add(i + 8));
        acc1 = _mm256_fmadd_ps(va1, vb1, acc1);

        let va2 = _mm256_loadu_ps(a.add(i + 16));
        let vb2 = _mm256_loadu_ps(b.add(i + 16));
        acc2 = _mm256_fmadd_ps(va2, vb2, acc2);

        let va3 = _mm256_loadu_ps(a.add(i + 24));
        let vb3 = _mm256_loadu_ps(b.add(i + 24));
        acc3 = _mm256_fmadd_ps(va3, vb3, acc3);

        i += 32;
    }

    // Handle remaining blocks of 8 (Optimization)
    while i + 8 <= n {
        let va = _mm256_loadu_ps(a.add(i));
        let vb = _mm256_loadu_ps(b.add(i));
        acc0 = _mm256_fmadd_ps(va, vb, acc0);
        i += 8;
    }

    // Reduction: Sum the 4 accumulators into 1
    acc0 = _mm256_add_ps(acc0, acc1);
    acc2 = _mm256_add_ps(acc2, acc3);
    acc0 = _mm256_add_ps(acc0, acc2);

    // Horizontal Sum reduction to single f32
    let upper = _mm256_extractf128_ps(acc0, 1);
    let lower = _mm256_castps256_ps128(acc0);
    let sum128 = _mm_add_ps(upper, lower);
    let sum_h = _mm_hadd_ps(sum128, sum128);
    let final_vector = _mm_hadd_ps(sum_h, sum_h);

    let mut result = _mm_cvtss_f32(final_vector);

    // Handle the tail (n % 8) using a scalar loop
    while i < n {
        result += (*a.add(i)) * (*b.add(i));
        i += 1;
    }

    -result
}

/// The AVX2 Integer Dot Product Kernel (Coarse Optimization).
/// Input: Query (i8), Database Vector (u8).
/// Returns: Negative Dot Product (Distance Proxy).
/// 
/// SAFETY: Includes i32 widening cascade to prevent maddubs saturation.
#[target_feature(enable = "avx2")]
pub unsafe fn dot_product_u8_avx2(q: *const i8, v: *const u8, n: usize) -> i32 {
    let mut sum_i32 = _mm256_setzero_si256();
    let ones = _mm256_set1_epi16(1);
    let mut i = 0;

    // Process 32 bytes at a time
    while i + 32 <= n {
        let v_vec = _mm256_loadu_si256(v.add(i) as *const _);
        let q_vec = _mm256_loadu_si256(q.add(i) as *const _);
        
        // u8 * i8 -> i16 (Sums adjacent pairs)
        let prod_i16 = _mm256_maddubs_epi16(v_vec, q_vec);
        // i16 * 1 + i16 * 1 -> i32 (Widening Cascade)
        let prod_i32 = _mm256_madd_epi16(prod_i16, ones);
        // Accumulate i32 lanes
        sum_i32 = _mm256_add_epi32(sum_i32, prod_i32);
        
        i += 32;
    }

    // Horizontal sum of the 8 i32 lanes
    let upper = _mm256_extracti128_si256(sum_i32, 1);
    let lower = _mm256_castsi256_si128(sum_i32);
    let sum128 = _mm_add_epi32(upper, lower);
    let hsum = _mm_hadd_epi32(sum128, sum128);
    let final_vector = _mm_hadd_epi32(hsum, hsum);
    
    let mut result = _mm_cvtsi128_si32(final_vector);

    // Handle remaining elements (Scalar Fallback)
    while i < n {
        result += (*v.add(i) as i16 * *q.add(i) as i16) as i32;
        i += 1;
    }

    -result
}

/// Returns the optimal vector kernel.
pub fn get_vector_kernel() -> SimdFunc {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return avx2_dot;
        }
    }
    scalar_dot_product_wrapper
}

unsafe fn scalar_dot_product_wrapper(a: *const f32, b: *const f32, n: usize) -> f32 {
    scalar_dot(a, b, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dot_product_equivalence() {
        let n = 100;
        let a = vec![1.1f32; n];
        let b = vec![2.2f32; n];

        unsafe {
            let ref_res = scalar_dot(a.as_ptr(), b.as_ptr(), n);
            let kernel_res = get_vector_kernel()(a.as_ptr(), b.as_ptr(), n);
            
            let diff = (ref_res - kernel_res).abs();
            // FMA and sum order differences cause small mismatches (1e-3 is safe)
            assert!(diff < 1e-3, "SIMD ({}) and Scalar ({}) mismatch by {}", kernel_res, ref_res, diff);
            assert!(ref_res < 0.0, "Scalar dot product should be negative");
            assert!(kernel_res < 0.0, "SIMD dot product should be negative");
        }
    }

    #[test]
    fn test_dot_product_u8_equivalence() {
        let n = 256;
        // Realistic range: Database ~127 (normalized), Query ~ [-30, 30]
        let v: Vec<u8> = (0..n).map(|i| (128 + (i % 32)) as u8).collect();
        let q: Vec<i8> = (0..n).map(|i| ((i % 64) as i16 - 32) as i8).collect();
        
        let ref_res = scalar_dot_u8(q.as_ptr(), v.as_ptr(), n);
        
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            if is_x86_feature_detected!("avx2") {
                let ker_res = unsafe { dot_product_u8_avx2(q.as_ptr(), v.as_ptr(), n) };
                assert_eq!(ref_res, ker_res, "Integer SIMD ({}) and Scalar ({}) mismatch", ker_res, ref_res);
            }
        }
    }
}

use std::time::Instant;
use vortex_core::index::simd;

fn main() {
    let n = 128; // Standard dimension
    let iterations = 10_000_000;
    
    // 1. f32 Benchmark
    let v1_f32 = vec![1.2f32; n];
    let v2_f32 = vec![0.8f32; n];
    
    println!("Benchmarking f32 Dot Product ({} iterations)...", iterations);
    let start_f32 = Instant::now();
    let mut sum_f32 = 0.0;
    for _ in 0..iterations {
        unsafe {
            sum_f32 += simd::scalar_dot(v1_f32.as_ptr(), v2_f32.as_ptr(), n);
        }
    }
    let duration_f32 = start_f32.elapsed();
    println!("Scalar f32: {:?} (Dummy sum: {})", duration_f32, sum_f32);

    println!("Benchmarking AVX2 f32 Dot Product...");
    let start_avx2_f32 = Instant::now();
    let mut sum_avx2_f32 = 0.0;
    for _ in 0..iterations {
        unsafe {
            sum_avx2_f32 += simd::avx2_dot(v1_f32.as_ptr(), v2_f32.as_ptr(), n);
        }
    }
    let duration_avx2_f32 = start_avx2_f32.elapsed();
    println!("AVX2 f32: {:?} (Dummy sum: {})", duration_avx2_f32, sum_avx2_f32);

    // 2. u8 Benchmark
    let v_u8 = vec![127u8; n];
    let q_i8 = vec![64i8; n];
    
    println!("\nBenchmarking u8 Dot Product ({} iterations)...", iterations);
    let start_u8 = Instant::now();
    let mut sum_u8 = 0;
    for _ in 0..iterations {
        sum_u8 += simd::scalar_dot_u8(q_i8.as_ptr(), v_u8.as_ptr(), n);
    }
    let duration_u8 = start_u8.elapsed();
    println!("Scalar u8: {:?} (Dummy sum: {})", duration_u8, sum_u8);

    println!("Benchmarking AVX2 u8 Dot Product (Quantized Engine Hot Path)...");
    let start_avx2_u8 = Instant::now();
    let mut sum_avx2_u8 = 0;
    for _ in 0..iterations {
        unsafe {
            sum_avx2_u8 += simd::dot_product_u8_avx2(q_i8.as_ptr(), v_u8.as_ptr(), n);
        }
    }
    let duration_avx2_u8 = start_avx2_u8.elapsed();
    println!("AVX2 u8 (Widening): {:?} (Dummy sum: {})", duration_avx2_u8, sum_avx2_u8);
    
    println!("\nSummary Speedup (AVX2 u8 vs Scalar f32): {:.2}x", 
        duration_f32.as_secs_f64() / duration_avx2_u8.as_secs_f64());
}

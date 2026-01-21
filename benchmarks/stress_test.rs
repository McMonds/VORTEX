use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use std::time::{Instant, Duration};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Barrier;
use vortex_rpc::{RequestHeader, VBP_MAGIC, OP_UPSERT};
// use rand::Rng;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 32)]
    concurrency: usize,

    #[arg(short, long, default_value_t = 80000)]
    requests: usize,

    #[arg(short, long, default_value_t = 9000)]
    port: u16,

    #[arg(short, long, default_value = "upsert")]
    mode: String, // upsert, search, mixed
}

const OP_SEARCH: u8 = 5;

const DIMENSION: usize = 128;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();
    
    let (concurrency, reqs_per_task) = if args.mode == "mixed" {
        (16, args.requests) // 8 Writers + 8 Readers
    } else {
        (args.concurrency, args.requests)
    };
    
    let total_requests = concurrency * reqs_per_task;

    println!("--- VORTEX SATURATION BENCHMARK ---");
    println!("Mode:         {}", args.mode);
    println!("Concurrency:  {} Tasks", concurrency);
    println!("Reqs per Task: {}", reqs_per_task);
    println!("Total Reqs:    {}", total_requests);
    println!("Target Port:   {}", args.port);
    println!("-----------------------------------\n");
    
    let barrier = Arc::new(Barrier::new(concurrency));
    let global_acks = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    
    let addr = format!("127.0.0.1:{}", args.port);
    let global_start = Instant::now();

    // Minimal Modular Telemetry (Phase 12/13)
    let monitor = Arc::new(vortex_core::telemetry_beacon::BenchmarkGuard::new(
        &format!("STRESS_{}", args.mode),
        total_requests as u64,
        global_acks.clone()
    ));
    let stats_ref = monitor.stats.clone();
    
    for task_id in 0..concurrency {
        let b = barrier.clone();
        let acks_ref = global_acks.clone();
        let addr_clone = addr.clone();
        let mode_clone = args.mode.clone();
        let stats_task = stats_ref.clone();
        
        let handle = tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(reqs_per_task);
            let mut stream = None;
            for _attempt in 0..50 {
                if let Ok(s) = TcpStream::connect(&addr_clone).await {
                    stream = Some(s);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            
            let stream = match stream {
                Some(s) => s,
                None => return vec![],
            };
            stream.set_nodelay(true).unwrap();
            let (mut reader, mut writer) = stream.into_split();
            
            b.wait().await;
            
            // Determine if this task is a Reader or Writer in mixed mode
            let is_search = if mode_clone == "mixed" {
                task_id >= 8
            } else {
                mode_clone == "search"
            };

            let writer_handle = tokio::spawn(async move {
                for i in 0..reqs_per_task {
                    let id = (task_id * reqs_per_task + i) as u64;
                    let opcode = if is_search { OP_SEARCH } else { OP_UPSERT };
                    let payload_len = if is_search { DIMENSION * 4 } else { 8 + (DIMENSION * 4) };
                    
                    let mut packet = vec![0u8; 16 + payload_len];
                    let header = RequestHeader {
                        magic: VBP_MAGIC, version: 1, opcode,
                        payload_len: payload_len as u32, request_id: id,
                    };

                    unsafe {
                        std::ptr::copy_nonoverlapping(&header as *const _ as *const u8, packet.as_mut_ptr(), 16);
                    }
                    
                    if !is_search {
                        packet[16..24].copy_from_slice(&id.to_le_bytes());
                    }
                    
                    if writer.write_all(&packet).await.is_err() { break; }
                }
                let _ = writer.flush().await;
            });

            let mut acks_received = 0;
            let mut buffer = [0u8; 16]; 
            while acks_received < reqs_per_task {
                let start = Instant::now();
                match tokio::time::timeout(Duration::from_secs(10), reader.read_exact(&mut buffer)).await {
                    Ok(Ok(_)) => {
                        let lat = start.elapsed();
                        latencies.push(lat);
                        stats_task.record(lat);
                        acks_received += 1;
                        let total = acks_ref.fetch_add(1, Ordering::Relaxed) + 1;
                        if total % 10000 == 0 {
                            println!("[PROGRESS] {:>6} / {} ACKs received...", total, total_requests);
                        }
                    },
                    _ => break,
                }
            }
            let _ = writer_handle.await;
            latencies
        });
        handles.push(handle);
    }
    
    let mut all_latencies = Vec::new();
    for h in handles {
        if let Ok(mut task_lats) = h.await {
            all_latencies.append(&mut task_lats);
        }
    }
    
    let total_time = global_start.elapsed();
    let actual_acks = global_acks.load(Ordering::Relaxed);
    let throughput = actual_acks as f64 / total_time.as_secs_f64();
    
    // Statistics
    all_latencies.sort();
    let count = all_latencies.len();
    let avg = if count > 0 { all_latencies.iter().sum::<Duration>() / count as u32 } else { Duration::from_secs(0) };
    let p50 = if count > 0 { all_latencies[count / 2] } else { Duration::from_secs(0) };
    let p99 = if count > 0 { all_latencies[(count as f64 * 0.99) as usize] } else { Duration::from_secs(0) };
    let max = if count > 0 { all_latencies[count - 1] } else { Duration::from_secs(0) };

    println!("\n==================================================");
    println!("          VORTEX BENCHMARK RECEIPT               ");
    println!("==================================================");
    println!(" [ BLOCK 1: TEST CONFIGURATION ]");
    println!(" Targets:      {} requests", total_requests);
    println!(" Concurrency:  {} pipelines", concurrency);
    println!(" Mode:         {}", args.mode);
    println!("--------------------------------------------------");
    println!(" [ BLOCK 2: EXECUTION INTEGRITY ]");
    let status = if actual_acks == total_requests { "PASS" } else { "FAIL" };
    println!(" Status:       {}", status);
    println!(" ACKs:         {}/{}", actual_acks, total_requests);
    println!(" Drops:        {}", total_requests - actual_acks);
    println!("--------------------------------------------------");
    println!(" [ BLOCK 3: PERFORMANCE METRICS ]");
    println!(" Wall Clock:   {:.2?}", total_time);
    println!(" Throughput:   {:.2} ops/sec", throughput);
    println!("--------------------------------------------------");
    println!(" [ BLOCK 4: STATISTICAL LATENCY ]");
    println!(" Average:      {:.2?}", avg);
    println!(" P50 (Median): {:.2?}", p50);
    println!(" P99 (Tail):   {:.2?}", p99);
    println!(" Max/Jitter:   {:.2?}", max);
    println!("==================================================\n");
    
    // Final Report Beacon (Phase 12)
    vortex_core::telemetry_beacon::send_vortex_beacon(&vortex_core::telemetry_beacon::BeaconReport {
        name: format!("STRESS_{}", args.mode),
        acks: actual_acks as u64,
        drops: (total_requests - actual_acks) as u64,
        target: total_requests as u64,
        p50_us: p50.as_micros() as u64,
        p99_us: p99.as_micros() as u64,
        throughput,
    });

    Ok(())
}

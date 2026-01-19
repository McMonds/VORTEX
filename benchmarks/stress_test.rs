use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use std::time::{Instant, Duration};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Barrier;
use vortex_rpc::{RequestHeader, VBP_MAGIC, OP_UPSERT};
use rand::Rng;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 16)]
    concurrency: usize,

    #[arg(short, long, default_value_t = 5000)]
    requests: usize,

    #[arg(short, long, default_value_t = 9000)]
    port: u16,
}

const DIMENSION: usize = 128;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();
    
    let total_requests = args.concurrency * args.requests;

    println!("--- VORTEX SATURATION BENCHMARK ---");
    println!("Concurrency:  {} Tasks", args.concurrency);
    println!("Reqs per Task: {}", args.requests);
    println!("Total Reqs:    {}", total_requests);
    println!("Target Port:   {}", args.port);
    println!("-----------------------------------");
    
    let barrier = Arc::new(Barrier::new(args.concurrency));
    let global_acks = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    
    let addr = format!("127.0.0.1:{}", args.port);
    let global_start = Instant::now();
    
    for task_id in 0..args.concurrency {
        let b = barrier.clone();
        let acks_ref = global_acks.clone();
        let addr_clone = addr.clone();
        let reqs_per_task = args.requests;
        
        let handle = tokio::spawn(async move {
            let stream = match TcpStream::connect(&addr_clone).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[Task {}] Connection Failed to {}: {}", task_id, addr_clone, e);
                    return Duration::from_secs(0);
                }
            };
            stream.set_nodelay(true).expect("Failed to set NODELAY");
            
            let (mut reader, mut writer) = stream.into_split();
            
            // Wait for all tasks to be ready
            b.wait().await;
            let task_start = Instant::now();
            
            // WRITER TASK
            let writer_handle = tokio::spawn(async move {
                for i in 0..reqs_per_task {
                    let id = (task_id * reqs_per_task + i) as u64;
                    let logical_payload_len = 8 + (DIMENSION * 4);
                    let mut packet = vec![0u8; 16 + logical_payload_len];
                    
                    {
                        let mut rng = rand::thread_rng();
                        let vector: Vec<f32> = (0..DIMENSION).map(|_| rng.gen::<f32>()).collect();
                        
                        let header = RequestHeader {
                            magic: VBP_MAGIC,
                            version: 1,
                            opcode: OP_UPSERT,
                            payload_len: logical_payload_len as u32,
                            request_id: id,
                        };

                        let header_bytes = unsafe {
                            std::slice::from_raw_parts(
                                &header as *const RequestHeader as *const u8,
                                16
                            )
                        };

                        packet[0..16].copy_from_slice(header_bytes);
                        packet[16..24].copy_from_slice(&id.to_le_bytes());
                        for (idx, val) in vector.iter().enumerate() {
                            packet[24 + idx * 4 .. 28 + idx * 4].copy_from_slice(&val.to_le_bytes());
                        }
                    } 

                    if let Err(_) = writer.write_all(&packet).await {
                        break;
                    }
                }
                let _ = writer.flush().await;
            });

            // READER TASK
            let mut acks_received = 0;
            let mut buffer = [0u8; 16]; 
            while acks_received < reqs_per_task {
                match tokio::time::timeout(Duration::from_secs(30), reader.read_exact(&mut buffer)).await {
                    Ok(Ok(_)) => {
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
            task_start.elapsed()
        });
        handles.push(handle);
    }
    
    let mut total_latency = Duration::from_secs(0);
    let mut successful_tasks = 0;
    for h in handles {
        if let Ok(task_duration) = h.await {
            if task_duration.as_secs_f64() > 0.0 {
                total_latency += task_duration;
                successful_tasks += 1;
            }
        }
    }
    
    let total_time = global_start.elapsed();
    let actual_acks = global_acks.load(Ordering::Relaxed);
    let throughput = actual_acks as f64 / total_time.as_secs_f64();
    
    println!("\n--- FINAL RESULTS ---");
    println!("Total ACKs:     {}", actual_acks);
    println!("Wall Clock:     {:?}", total_time);
    println!("Throughput:     {:.2} ops/sec", throughput);
    if successful_tasks > 0 {
        println!("Avg Latency:    {:?}", total_latency / successful_tasks as u32);
    }
    
    Ok(())
}

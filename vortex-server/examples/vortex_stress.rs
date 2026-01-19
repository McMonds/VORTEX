use std::io::{Write, Read};
use std::net::TcpStream;
use std::thread;
use std::sync::Arc;
use std::time::{Instant, Duration};
use std::sync::atomic::{AtomicUsize, Ordering};
use rand::Rng;
use vortex_rpc::{RequestHeader, VBP_MAGIC, OP_UPSERT, OP_SEARCH};

/// VORTEX Performance Benchmarking Tool
/// Goal: Saturate the 4-Shard Reactor cluster with 1M Vectors.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let num_vectors = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let concurrency = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let dimension = 128;

    println!("--- VORTEX STRESS TEST ---");
    println!("Target: {} vectors, Concurrency: {}", num_vectors, concurrency);

    let vectors_per_thread = num_vectors / concurrency;
    let completed = Arc::new(AtomicUsize::new(0));
    let start_time = Instant::now();

    let mut handles = Vec::new();

    for t_id in 0..concurrency {
        let completed = Arc::clone(&completed);
        let handle = thread::spawn(move || {
            let mut stream = TcpStream::connect("127.0.0.1:8080").expect("Failed to connect");
            stream.set_nodelay(true).unwrap();
            let mut rng = rand::thread_rng();
            
            for i in 0..vectors_per_thread {
                let id = (t_id * vectors_per_thread + i) as u64;
                let vector: Vec<f32> = (0..dimension).map(|_| rng.gen::<f32>()).collect();
                
                // Pack VBP Upsert with LOGICAL length
                let logical_payload_len = 8 + (dimension * 4);
                let mut payload = Vec::with_capacity(logical_payload_len);
                payload.extend_from_slice(&id.to_le_bytes());
                for &val in &vector {
                    payload.extend_from_slice(&val.to_le_bytes());
                }

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
                        std::mem::size_of::<RequestHeader>()
                    )
                };

                // PADDING: Align physical transmission to 4096 bytes for O_DIRECT
                let mut packet = Vec::with_capacity(4096);
                packet.extend_from_slice(header_bytes);
                packet.extend_from_slice(&payload);
                let padding = 4096 - packet.len();
                packet.extend(std::iter::repeat(0u8).take(padding));

                stream.write_all(&packet).unwrap();
                stream.flush().unwrap();
                
                // Wait for ACK
                let mut ack = [0u8; 16];
                stream.read_exact(&mut ack).unwrap();
                
                completed.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }

    // Progress Monitor
    let completed_mon = Arc::clone(&completed);
    let monitor_handle = thread::spawn(move || {
        while completed_mon.load(Ordering::Relaxed) < num_vectors {
            thread::sleep(Duration::from_secs(1));
            let c = completed_mon.load(Ordering::Relaxed);
            let elapsed = start_time.elapsed().as_secs_f64();
            println!("Progress: {}/{} vectors ({:.2} upserts/sec)", c, num_vectors, c as f64 / elapsed);
            if c >= num_vectors { break; }
        }
    });

    for h in handles {
        h.join().unwrap();
    }
    // Final wait for monitor
    thread::sleep(Duration::from_millis(500));

    let total_duration = start_time.elapsed();
    println!("\nIngestion Complete: {} vectors in {:?}", num_vectors, total_duration);
    println!("Average Throughput: {:.2} upserts/sec", num_vectors as f64 / total_duration.as_secs_f64());

    // Final Search Probe (Multiple Trials for Latency Distribution)
    println!("\nExecuting 100 Search Probes for Latency Analysis...");
    let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap();
    stream.set_nodelay(true).unwrap();
    
    let mut latencies = Vec::new();
    for i in 0..100 {
        let search_header = RequestHeader {
            magic: VBP_MAGIC,
            version: 1,
            opcode: OP_SEARCH,
            payload_len: 0,
            request_id: 1000 + i,
        };
        let h_bytes = unsafe {
            std::slice::from_raw_parts(
                &search_header as *const RequestHeader as *const u8,
                std::mem::size_of::<RequestHeader>()
            )
        };
        
        let search_start = Instant::now();
        stream.write_all(h_bytes).unwrap();
        stream.flush().unwrap();
        
        let mut res = [0u8; 16];
        stream.read_exact(&mut res).unwrap();
        latencies.push(search_start.elapsed());
    }
    
    latencies.sort();
    let p50 = latencies[50];
    let p95 = latencies[95];
    let p99 = latencies[99];
    
    println!("Search Latency p50: {:?}", p50);
    println!("Search Latency p95: {:?}", p95);
    println!("Search Latency p99: {:?}", p99);
}

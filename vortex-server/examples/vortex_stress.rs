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
            let mut rng = rand::thread_rng();
            
            for i in 0..vectors_per_thread {
                let id = (t_id * vectors_per_thread + i) as u64;
                let vector: Vec<f32> = (0..dimension).map(|_| rng.gen::<f32>()).collect();
                
                // Pack VBP Upsert
                let mut payload = Vec::with_capacity(8 + (dimension * 4));
                payload.extend_from_slice(&id.to_le_bytes());
                for &val in &vector {
                    payload.extend_from_slice(&val.to_le_bytes());
                }

                let header = RequestHeader {
                    magic: VBP_MAGIC,
                    version: 1,
                    opcode: OP_UPSERT,
                    payload_len: payload.len() as u32,
                    request_id: id,
                };

                let header_bytes = unsafe {
                    std::slice::from_raw_parts(
                        &header as *const RequestHeader as *const u8,
                        std::mem::size_of::<RequestHeader>()
                    )
                };

                stream.write_all(header_bytes).unwrap();
                stream.write_all(&payload).unwrap();
                
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
    thread::spawn(move || {
        while completed_mon.load(Ordering::Relaxed) < num_vectors {
            thread::sleep(Duration::from_secs(1));
            let c = completed_mon.load(Ordering::Relaxed);
            let elapsed = start_time.elapsed().as_secs_f64();
            println!("Progress: {}/{} vectors ({:.2} upserts/sec)", c, num_vectors, c as f64 / elapsed);
        }
    });

    for h in handles {
        h.join().unwrap();
    }

    let total_duration = start_time.elapsed();
    println!("\nIngestion Complete: {} vectors in {:?}", num_vectors, total_duration);
    println!("Average Throughput: {:.2} upserts/sec", num_vectors as f64 / total_duration.as_secs_f64());

    // Final Search Probe
    println!("\nExecuting Search Probes...");
    let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap();
    let search_header = RequestHeader {
        magic: VBP_MAGIC,
        version: 1,
        opcode: OP_SEARCH,
        payload_len: 0,
        request_id: 9999,
    };
    let h_bytes = unsafe {
        std::slice::from_raw_parts(
            &search_header as *const RequestHeader as *const u8,
            std::mem::size_of::<RequestHeader>()
        )
    };
    
    let search_start = Instant::now();
    stream.write_all(h_bytes).unwrap();
    let mut res = [0u8; 16];
    stream.read_exact(&mut res).unwrap();
    println!("Search Latency: {:?}", search_start.elapsed());
    println!("Search ACK Header: {:?}", res);
}

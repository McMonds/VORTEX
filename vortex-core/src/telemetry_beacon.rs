use std::net::TcpStream;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::thread;

pub struct BeaconReport {
    pub name: String,
    pub acks: u64,
    pub drops: u64,
    pub target: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub throughput: f64,
}

/// A lock-free latency histogram for real-time telemetry (Rule 3 Optimization).
/// Buckets: [1us, 10us, 50us, 100us, 200us, 500us, 1ms, 5ms, 10ms, 50ms, 100ms, 500ms+]
pub struct LiveHistogram {
    buckets: [AtomicU64; 12],
}

impl LiveHistogram {
    pub fn new() -> Self {
        const ZERO: AtomicU64 = AtomicU64::new(0);
        Self { buckets: [ZERO; 12] }
    }

    pub fn record(&self, elapsed: Duration) {
        let us = elapsed.as_micros() as u64;
        let idx = if us < 1 { 0 }
            else if us < 10 { 1 }
            else if us < 50 { 2 }
            else if us < 100 { 3 }
            else if us < 200 { 4 }
            else if us < 500 { 5 }
            else if us < 1000 { 6 }
            else if us < 5000 { 7 }
            else if us < 10000 { 8 }
            else if us < 50000 { 9 }
            else if us < 100000 { 10 }
            else { 11 };
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
    }

    pub fn calculate_percentile(&self, p: f64) -> u64 {
        let total: u64 = self.buckets.iter().map(|b| b.load(Ordering::Relaxed)).sum();
        if total == 0 { return 0; }
        let target = (total as f64 * p) as u64;
        let mut count = 0;
        let thresholds = [1, 10, 50, 100, 200, 500, 1000, 5000, 10000, 50000, 100000, 500000];
        for (i, b) in self.buckets.iter().enumerate() {
            count += b.load(Ordering::Relaxed);
            if count >= target { return thresholds[i]; }
        }
        500000
    }
}

pub struct BenchmarkGuard {
    _handle: thread::JoinHandle<()>,
    pub stats: Arc<LiveHistogram>,
}

impl BenchmarkGuard {
    pub fn new(name: &str, target: u64, acks: Arc<AtomicUsize>) -> Self {
        let name = name.to_string();
        let stats = Arc::new(LiveHistogram::new());
        let stats_clone = stats.clone();
        
        let _handle = thread::spawn(move || {
            let start = Instant::now();
            loop {
                thread::sleep(Duration::from_secs(1));
                let a = acks.load(Ordering::Relaxed);
                let t = start.elapsed().as_secs_f64();
                let throughput = if t > 0.1 { a as f64 / t } else { 0.0 };
                
                let p50 = stats_clone.calculate_percentile(0.50);
                let p99 = stats_clone.calculate_percentile(0.99);
                
                send_vortex_beacon(&BeaconReport {
                    name: name.clone(),
                    acks: a as u64,
                    drops: 0, 
                    target,
                    p50_us: p50,
                    p99_us: p99,
                    throughput,
                });
                
                if a as u64 >= target { break; }
            }
        });
        Self { _handle, stats }
    }
}

pub fn send_vortex_beacon(report: &BeaconReport) {
    if let Ok(mut stream) = TcpStream::connect("127.0.0.1:2329") {
        let json = format!(
            "{{\"name\":\"{}\",\"acks\":{},\"drops\":{},\"target\":{},\"p50\":{},\"p99\":{},\"throughput\":{:.2}}}",
            report.name, report.acks, report.drops, report.target, report.p50_us, report.p99_us, report.throughput
        );
        let _ = stream.write_all(json.as_bytes());
    }
}

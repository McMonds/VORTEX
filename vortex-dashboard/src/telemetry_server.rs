use std::net::TcpListener;
use std::io::Read;
use std::sync::mpsc;
use std::thread;
use log::{error, info};

pub struct WorkerReport {
    pub name: String,
    pub acks: u64,
    pub drops: u64,
    pub target: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub throughput: f64,
}

pub struct TelemetryServer {
    tx: mpsc::Sender<crate::DashboardEvent>,
}

impl TelemetryServer {
    pub fn new(tx: mpsc::Sender<crate::DashboardEvent>) -> Self {
        Self { tx }
    }

    pub fn start(self) {
        thread::spawn(move || {
            let listener = match TcpListener::bind("127.0.0.1:2329") {
                Ok(l) => l,
                Err(e) => {
                    error!("TelemetryServer failed to bind to 2329: {}", e);
                    return;
                }
            };

            info!("TelemetryServer listening on 127.0.0.1:2329");

            for stream in listener.incoming() {
                match stream {
                    Ok(mut s) => {
                        let mut buffer = String::new();
                        if s.read_to_string(&mut buffer).is_ok() {
                            if let Some(report) = self.parse_report(&buffer) {
                                let _ = self.tx.send(crate::DashboardEvent::WorkerUpdate(report));
                            }
                        }
                    }
                    Err(e) => error!("TelemetryServer accept error: {}", e),
                }
            }
        });
    }

    fn parse_report(&self, buffer: &str) -> Option<WorkerReport> {
        // Manual parsing to avoid Serde overhead if possible, or just use simple regex/string splits
        // Since we are sending a very specific JSON format from telemetry_beacon.rs:
        // {"name":"{}","acks":{},"drops":{},"target":{},"p50":{},"p99":{},"throughput":{:.2}}
        
        // Let's use simple string searching for speed and zero-dependency
        let find_val = |key: &str| -> Option<&str> {
            let pattern = format!("\"{}\":", key);
            let start = buffer.find(&pattern)? + pattern.len();
            let end = buffer[start..].find(',')
                .or_else(|| buffer[start..].find('}'))?;
            Some(buffer[start..start + end].trim_matches('"'))
        };

        Some(WorkerReport {
            name: find_val("name")?.to_string(),
            acks: find_val("acks")?.parse().ok()?,
            drops: find_val("drops")?.parse().ok()?,
            target: find_val("target")?.parse().ok()?,
            p50_us: find_val("p50")?.parse().ok()?,
            p99_us: find_val("p99")?.parse().ok()?,
            throughput: find_val("throughput")?.parse().ok()?,
        })
    }
}

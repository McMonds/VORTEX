use std::io::{BufRead, BufReader};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::collections::VecDeque;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use regex::Regex;

mod config;
mod metrics;
mod tui;
mod lifecycle;
mod telemetry_server;

use config::Args;
use metrics::{SystemSampler, MetricsSnapshot};
use telemetry_server::{TelemetryServer, WorkerReport};


// =================================================================================
// ACTOR MESSAGES (MPSC)
// =================================================================================
pub enum DashboardEvent {
    // From System Thread
    HardwareUpdate(MetricsSnapshot),
    ServerOffline,
    
    // From Log Parser Thread (Pre-Aggregated)
    LogTick {
        requests: u64,
        flushes_full: u64,
        flushes_eot: u64,
        backpressure_events: usize,
        bytes_written: u64,
        search: Option<SearchStats>,
        health: Option<HealthStats>,
    },
    
    // From Telemetry Beacon (Benchmarks)
    WorkerUpdate(WorkerReport),
    
    // From Input Thread
    Input(KeyCode),
    Resize,
}

struct AppState {
    // History Buckets
    metrics_history: VecDeque<MetricsSnapshot>,
    
    // Throughput State
    total_requests: u64,
    total_acks: u64,
    start_time: Option<Instant>, 
    
    // Status
    server_online: bool,
    is_release: bool,
    
    // Viewport
    throughput_instant: f64,
    last_log_tick: Option<LogTickSummary>,
    
    // Foreman Sub-Layer
    search_stats: Option<SearchStats>,
    health_stats: Option<HealthStats>,
    
    // High Water Marks
    peak_throughput: f64,
    peak_rss_mb: f64,
    
    // Worker Telemetry
    worker_stats: Option<WorkerReport>,
    last_worker_update: Option<Instant>,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct SearchStats {
    pub ops: u64,
    pub time_us: u64,
    pub dist_calcs: u64,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct HealthStats {
    pub ingress_ms: u64,
    pub flush_ms: u64,
}

#[derive(Clone, Copy, Default)]
struct LogTickSummary {
    pub flushes_full: u64,
    pub flushes_eot: u64,
    pub bytes: u64,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    
    let mut lifecycle = lifecycle::LifecycleManager::new();
    
    if args.clean {
        lifecycle.cleanup_zombies();
        let _ = lifecycle.clean_data_dir(&args.dir);
    }

    lifecycle.spawn_server(&args).expect("Failed to start vortex-server.");
    
    let server_pid = lifecycle.server_process.as_ref().map(|c| c.id()).expect("Missing Server PID");
    let server_log_stream = lifecycle.server_stderr.take().expect("Failed to capture server stderr");

    // Defect 13: Signal Handling (Kill Child)
    let pid_clone = server_pid;
    ctrlc::set_handler(move || {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        // Kill the child
        let _ = Command::new("kill").arg("-9").arg(pid_clone.to_string()).output();
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");

    // Channels
    let (tx, rx) = mpsc::channel();
    
    // --- 1. System Sampler Thread ---
    let tx_sys = tx.clone();
    let port_copy = args.port;
    thread::spawn(move || {
        let mut sampler = SystemSampler::new(Some(server_pid), port_copy);
        loop {
            // Defect 14: Master Clock alignment is hard across threads without shared passing.
            // But Sampler creates standard frames via `capture()`.
            match sampler.capture() {
                Ok(snapshot) => {
                    let _ = tx_sys.send(DashboardEvent::HardwareUpdate(snapshot));
                }
                Err(_) => {
                    // Defect 5: Silent Death
                    let _ = tx_sys.send(DashboardEvent::ServerOffline);
                }
            }
            thread::sleep(Duration::from_millis(100)); // 10Hz
        }
    });
    
    // --- 2. Log Parser Thread ---
    // Reads from Child Stderr (where env_logger writes)
    let tx_log = tx.clone();
    thread::spawn(move || {
        let mut reader = BufReader::new(server_log_stream);
        let mut line_buf = Vec::with_capacity(1024);
        
        let mut tick_reqs = 0;
        let mut tick_full = 0;
        let mut tick_eot = 0;
        let mut tick_bp = 0;
        let mut tick_bytes = 0;
        
        // Regex for Foreman Pulses
        let pulse_re = Regex::new(r"PULSE Shard \d+ \| \[Search\] ops=(\d+) time=(\d+)us dist=(\d+) \| \[Health\] ingress=(\d+)ms flush=(\d+)ms").unwrap();
        
        // Time-based aggregation (100ms)
        let mut last_send = Instant::now();
        
        loop {
            line_buf.clear();
            if let Err(_) = reader.read_until(b'\n', &mut line_buf) { break; }
            if line_buf.is_empty() { break; }

            // Use from_utf8 to avoid allocations (Borrowing from buf)
            let line = match std::str::from_utf8(&line_buf) {
                Ok(s) => s,
                Err(_) => continue,
            };
            
            // Pulse Parsing
            if let Some(caps) = pulse_re.captures(line) {
                let s_ops: u64 = caps[1].parse().unwrap_or(0);
                let s_time: u64 = caps[2].parse().unwrap_or(0);
                let s_dist: u64 = caps[3].parse().unwrap_or(0);
                let h_ingress: u64 = caps[4].parse().unwrap_or(0);
                let h_flush: u64 = caps[5].parse().unwrap_or(0);
                
                let _ = tx_log.send(DashboardEvent::LogTick {
                    requests: tick_reqs,
                    flushes_full: tick_full,
                    flushes_eot: tick_eot,
                    backpressure_events: tick_bp,
                    bytes_written: tick_bytes,
                    search: Some(SearchStats { ops: s_ops, time_us: s_time, dist_calcs: s_dist }),
                    health: Some(HealthStats { ingress_ms: h_ingress, flush_ms: h_flush }),
                });
                
                // Reset aggregators after pulse (Pulses are 1Hz, we send on pulse)
                tick_reqs = 0; tick_full = 0; tick_eot = 0; tick_bp = 0; tick_bytes = 0;
                last_send = Instant::now();
                continue;
            }
             
            // Simple Parsing
            if line.contains("Flushing batch") {
                if line.contains("Batch Full") { tick_full += 1; }
                else if line.contains("End-of-Tick") { tick_eot += 1; }
                
                if let Some(start) = line.find('(') {
                     if let Some(end) = line[start..].find(" requests") {
                         let num_str = &line[start+1 .. start+end];
                         if let Ok(n) = num_str.parse::<u64>() {
                             tick_reqs += n;
                         }
                     }
                }
            }
            else if line.contains("BACKPRESSURE") {
                if let Some(pos) = line.find("Aggregator: ") {
                    let sub = &line[pos + 12 ..];
                    if let Some(space) = sub.find(' ') {
                        if let Ok(n) = sub[..space].parse::<usize>() {
                            tick_bp += n;
                        }
                    }
                }
            }

            // If we don't get pulses (e.g. debug mode or idle), still send 2Hz updates
            if last_send.elapsed() >= Duration::from_millis(500) {
                 let _ = tx_log.send(DashboardEvent::LogTick {
                     requests: tick_reqs,
                     flushes_full: tick_full,
                     flushes_eot: tick_eot,
                     backpressure_events: tick_bp,
                     bytes_written: tick_bytes,
                     search: None,
                     health: None,
                 });
                 // Reset
                 tick_reqs = 0;
                 tick_full = 0;
                 tick_eot = 0;
                 tick_bp = 0;
                 tick_bytes = 0;
                 last_send = Instant::now();
            }
        }
    });
    
    // --- 3. Telemetry Server Thread (Benchmark Beacons) ---
    let tx_telemetry = tx.clone();
    let telemetry_server = TelemetryServer::new(tx_telemetry);
    telemetry_server.start();

    // --- 4. Input Thread ---
    let tx_input = tx.clone();
    thread::spawn(move || {
        loop {
            if event::poll(Duration::from_millis(100)).unwrap() {
                if let CEvent::Key(key) = event::read().unwrap() {
                    let _ = tx_input.send(DashboardEvent::Input(key.code));
                } else if let CEvent::Resize(_, _) = event::read().unwrap() {
                     let _ = tx_input.send(DashboardEvent::Resize);
                }
            }
        }
    });

    // --- 4. Main Event Loop (UI) ---
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    let mut app = AppState {
        metrics_history: VecDeque::new(),
        total_requests: 0,
        total_acks: 0,
        start_time: None,
        server_online: true,
        is_release: !cfg!(debug_assertions),
        throughput_instant: 0.0,
        last_log_tick: None,
        search_stats: None,
        health_stats: None,
        peak_throughput: 0.0,
        peak_rss_mb: 0.0,
        worker_stats: None,
        last_worker_update: None,
    };

    'main_loop: loop {
        // Draw
        terminal.draw(|f| {
             // Defect 20: Bounds Check
             let size = f.size();
             if size.width < 80 || size.height < 20 {
                 // warning
                 return; 
             }
             
             tui::draw_ui(f, &app);
        })?;

        // Handle Messages (Non-blocking drain)
        for _ in 0..100 { 
            match rx.try_recv() {
                Ok(DashboardEvent::HardwareUpdate(snapshot)) => {
                    if snapshot.rss_mem_mb > app.peak_rss_mb {
                        app.peak_rss_mb = snapshot.rss_mem_mb;
                    }
                    app.metrics_history.push_back(snapshot);
                    if app.metrics_history.len() > 60 { app.metrics_history.pop_front(); }
                }
                Ok(DashboardEvent::WorkerUpdate(report)) => {
                    app.worker_stats = Some(report);
                    app.last_worker_update = Some(Instant::now());
                }
                Ok(DashboardEvent::LogTick { requests, flushes_full, flushes_eot, backpressure_events: _, bytes_written, search, health }) => {
                    if requests > 0 && app.start_time.is_none() { app.start_time = Some(Instant::now()); }
                    
                    app.total_requests += requests;
                    app.total_acks += requests;
                    
                    if let Some(s) = search { app.search_stats = Some(s); }
                    if let Some(h) = health { app.health_stats = Some(h); }

                    let summary = LogTickSummary {
                        flushes_full,
                        flushes_eot,
                        bytes: bytes_written,
                    };
                    app.last_log_tick = Some(summary);
                    
                    if let Some(s) = search {
                        app.throughput_instant = s.ops as f64;
                    } else {
                        app.throughput_instant = requests as f64 * 2.0; // 500ms fallback
                    }
                    
                    if app.throughput_instant > app.peak_throughput {
                        app.peak_throughput = app.throughput_instant;
                    }
                }
                Ok(DashboardEvent::ServerOffline) => {
                    app.server_online = false;
                }
                Ok(DashboardEvent::Input(KeyCode::Char('q'))) => {
                    break 'main_loop;
                }
                Ok(DashboardEvent::Resize) => { terminal.autoresize()?; }
                Ok(_) => {},
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break 'main_loop,
            }
        }
        
        thread::sleep(Duration::from_millis(50));
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

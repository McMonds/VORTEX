use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, BufRead, BufReader},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use regex::Regex;

mod lifecycle;
mod metrics;
mod tui;

use lifecycle::LifecycleManager;
use metrics::MetricsState;
use tui::TuiAgent;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long, default_value_t = 2)]
    shards: usize,

    #[arg(short, long, default_value_t = 1000000)]
    capacity: usize,

    #[arg(short, long, default_value_t = 9000)]
    port: u16,

    #[arg(long)]
    clean: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // 1. Setup Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 2. Initialize State
    let mut state_init = MetricsState::new();
    state_init.shard_pulses = vec![Instant::now(); args.shards];
    let state = Arc::new(Mutex::new(state_init));
    let tui_agent = TuiAgent::new();

    
    // 4. Operational Checks
    let mut lifecycle = LifecycleManager::new();
    lifecycle.cleanup_zombies();
    if args.clean {
        let _ = lifecycle.clean_data_dir("./data");
    }
    let _ = MetricsState::check_perf_permissions();

    // 4. Start VORTEX Server
    lifecycle.spawn_server(args.shards, args.capacity, args.port)?;
    let server_pid = lifecycle.server_process.as_ref().unwrap().id();
    let server_stderr = lifecycle.server_stderr.take().expect("Failed to link server stderr");
    
    // 5. Spawn Log Parser Thread
    let state_clone = state.clone();
    let batch_accum = Arc::new(Mutex::new((0.0f64, 0.0f64))); // (count, size)
    let batch_accum_log = batch_accum.clone();
    
    thread::spawn(move || {
        let reader = BufReader::new(server_stderr);
        // Regex 1: Group Commit (Bytes, Requests, Reason)
        let re_batch = Regex::new(r"Shard (\d+) Group Commit -> Flushing batch of (\d+) bytes \((\d+) requests\) \(([^)]+)\)").unwrap();
        // Regex 2: Backpressure Aggregator
        let re_backpressure = Regex::new(r"Shard (\d+) BACKPRESSURE Aggregator: (\d+) stalls").unwrap();
        // Regex 3: Heartbeat
        let re_heartbeat = Regex::new(r"Shard (\d+) Heartbeat").unwrap();
        
        for line in reader.lines() {
            if let Ok(l) = line {
                let mut s = state_clone.lock().unwrap();
                
                if let Some(caps) = re_batch.captures(&l) {
                    let shard: usize = caps[1].parse().unwrap_or(0);
                    let bytes: f64 = caps[2].parse().unwrap_or(0.0);
                    let reqs: f64 = caps[3].parse().unwrap_or(0.0);
                    let reason = &caps[4];
                    
                    if shard < s.shard_pulses.len() {
                        s.shard_pulses[shard] = Instant::now();
                    }
                    
                    s.total_acks += reqs as usize;
                    if reason == "Batch Full" {
                        s.full_flushes += 1;
                    } else if reason == "End-of-Tick" {
                        s.eot_flushes += 1;
                    }

                    let mut accum = batch_accum_log.lock().unwrap();
                    accum.0 += reqs;
                    accum.1 += bytes;
                } else if let Some(caps) = re_backpressure.captures(&l) {
                    let shard: usize = caps[1].parse().unwrap_or(0);
                    let count: usize = caps[2].parse().unwrap_or(0);
                    if shard < s.shard_pulses.len() {
                        s.shard_pulses[shard] = Instant::now();
                    }
                    s.backpressure_events += count;
                } else if let Some(caps) = re_heartbeat.captures(&l) {
                    let shard: usize = caps[1].parse().unwrap_or(0);
                    if shard < s.shard_pulses.len() {
                        s.shard_pulses[shard] = Instant::now();
                    }
                }
            }
        }
    });

    // 6. Wait for Readiness
    lifecycle.wait_for_readiness(args.port, Duration::from_secs(30))?;
    // Note: Local stress_test spawning removed. Run manually in another terminal.

    // 7. Main TUI & Sampling Loop
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(100);

    loop {
        terminal.draw(|f| {
            let s = state.lock().unwrap();
            tui_agent.draw(f, &s);
        })?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code {
                    break;
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            let mut s = state.lock().unwrap();
            let (reqs, bytes) = {
                let mut accum = batch_accum.lock().unwrap();
                let res = *accum;
                *accum = (0.0, 0.0);
                res
            };
            
            let inst_throughput = reqs / 0.1;
            if inst_throughput > s.peak_throughput {
                s.peak_throughput = inst_throughput;
            }

            let _ = s.sample(server_pid, inst_throughput, bytes / 1024.0 / 0.1);
            last_tick = Instant::now();
        }
    }

    // 8. Restore Terminal & Cleanup
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    lifecycle.kill_all();

    Ok(())
}

use vortex_io::platform::topology::SystemTopology;
use vortex_io::platform::affinity::pin_thread_to_core;
use vortex_io::platform::lock_memory_pages;
use log::{info, warn};
use clap::Parser;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context, Result};

/// VORTEX: The Kernel-Bypass Vector Database
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port for VBP Ingress (TCP)
    #[arg(short, long, default_value_t = 9000)]
    port: u16,

    /// Directory for WAL and Storage
    #[arg(short, long, default_value = "./data")]
    dir: String,
}

fn main() -> Result<()> {
    // 0. Initialize Logger
    env_logger::init();
    let args = Args::parse();
    
    info!("Starting VORTEX Server v{}", env!("CARGO_PKG_VERSION"));
    info!("Configuration: Port={}, StorageDir={}", args.port, args.dir);

    // 1. Lock Memory (Standard Rule 4) - MUST BE FIRST
    // Rule I: Unwrap allowed at startup
    info!("Phase 1: locking memory pages...");
    lock_memory_pages();

    // 2. Interrogate Hardware
    info!("Phase 2: hardware topology detection...");
    let topology = SystemTopology::new();
    let core_ids = topology.physical_cores();
    let num_shards = core_ids.len();
    
    // Development Mode Guard
    if num_shards < 4 {
        warn!("============================================================");
        warn!("WARNING: Running in Constrained Environment ({} cores).", num_shards);
        warn!("VORTEX is optimized for high-core-count servers.");
        warn!("Expect sub-optimal performance due to lack of isolation.");
        warn!("============================================================");
        // Wait so user sees the warning (UX)
        std::thread::sleep(Duration::from_secs(2));
    }

    info!("VORTEX detected {} physical cores: {:?}. Initializing Clustered Architecture...", num_shards, core_ids);

    // 3. Pin Main Thread to Core 0 (Standard Rule 7/13)
    info!("Phase 3: pinning control thread to core 0...");
    pin_thread_to_core(0);

    // 4. Initialize Milestone 6 Shard Proxy (The Brain)
    info!("Phase 4: initializing Shard Proxy...");
    let proxy = Arc::new(vortex_core::proxy::ShardProxy::new(num_shards));
    
    // 5. Setup Graceful Shutdown (Signal Handler)
    info!("Phase 5: registering signal handlers...");
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let p = proxy.clone();

    ctrlc::set_handler(move || {
        info!("Received Shutdown Signal (SIGINT/SIGTERM)!");
        r.store(false, Ordering::SeqCst);
        p.shutdown();
        info!("Refusing new connections. Waiting for reactor convergence...");
        // In a real system, we'd wait for a condvar or channel. 
        // For now, we rely on the main loop to exit or the OS to kill us after cleanup.
        std::process::exit(0);
    }).context("Error setting Ctrl-C handler")?;

    // 6. Spawn Shards
    info!("Phase 6: spawning {} Shard Reactors (pinned to cores 0-{})...", num_shards, num_shards - 1);
    proxy.spawn_shards(args.port);

    info!("VORTEX Cluster ready and optimized for hardware.");
    
    // 7. Supervision Loop
    while running.load(Ordering::SeqCst) {
        // Heartbeat / Telemetry could go here.
        // We park to save Core 0 cycles.
        // The Signal Handler will exit the process, so this loop is just a "keep-alive".
        std::thread::park_timeout(Duration::from_secs(1));
    }

    info!("VORTEX Shutdown Complete.");
    Ok(())
}

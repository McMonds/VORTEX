use vortex_io::platform::topology::SystemTopology;
use vortex_io::platform::affinity::pin_thread_to_core;
use vortex_io::platform::lock_memory_pages;
use vortex_core::reactor::ShardReactor;
use log::info;

fn main() {
    env_logger::init();
    info!("Starting VORTEX Server...");

    // 1. Interrogate Hardware
    let topology = SystemTopology::new();
    topology.print_summary();

    // 2. Lock Memory (Standard Rule 4)
    info!("Locking memory pages...");
    lock_memory_pages();

    // 3. Pin Main Thread to Core 0 (Standard Rule 7/13)
    info!("Pinning control thread to core 0...");
    pin_thread_to_core(0);

    // 4. Initialize Milestone 3 Reactor (The Heart)
    info!("Initializing Shard Reactor 0...");
    let mut reactor = ShardReactor::new(0, 256);
    
    info!("Binding VBP Listener to 0.0.0.0:9000...");
    reactor.listen("0.0.0.0:9000").expect("Failed to bind port 9000");

    info!("VORTEX Shard 0 Active. Waiting for VBP packets...");
    loop {
        reactor.run_tick();
    }
}

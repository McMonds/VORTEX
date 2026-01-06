use vortex_io::platform::topology::SystemTopology;
use vortex_io::platform::affinity::pin_thread_to_core;
use vortex_io::platform::lock_memory_pages;
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

    let core_ids = topology.physical_cores();
    let num_shards = core_ids.len();
    info!("VORTEX detected {} physical cores: {:?}. Initializing Clustered Architecture...", num_shards, core_ids);

    // 4. Initialize Milestone 6 Shard Proxy (The Brain)
    info!("Initializing Shard Proxy...");
    let proxy = vortex_core::proxy::ShardProxy::new(num_shards);
    
    info!("Spawning {} Shard Reactors (pinned to cores 0-{})...", num_shards, num_shards - 1);
    proxy.spawn_shards(9000);

    info!("VORTEX Cluster ready and optimized for hardware.");
    loop {
        std::thread::park();
    }
}

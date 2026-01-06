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

    // 4. Adaptive Storage Check (Rule 16)
    let sector_size = SystemTopology::get_sector_size("/dev/sda");
    info!("Adaptive Storage: Detected sector size for /dev/sda: {} bytes", sector_size);

    info!("VORTEX Platform Skeleton Initialized.");
}

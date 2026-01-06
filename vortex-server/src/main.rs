use vortex_io::platform::topology::SystemTopology;
use vortex_io::platform::affinity::pin_thread_to_core;
use vortex_io::platform::lock_memory_pages;
use vortex_io::memory::BufferPool;
use vortex_rpc::{VbpHeader, VBP_MAGIC, VBP_VERSION, Command};
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

    // 4. Memory Sovereignty (Milestone 2 / Rule 1)
    info!("Initializing Memory Sovereignty Pool...");
    let mut pool = BufferPool::new(1024, 4096); // 1024 pages of 4KB
    
    if let Some(lease) = pool.lease() {
        info!("Successfully leased buffer at index {}", lease.index);
        let page = pool.get_page_mut(lease.index);
        info!("Buffer address: {:p}", page.as_ptr());
        
        // Demonstration of VBP header in pre-allocated memory
        let header = VbpHeader {
            magic: VBP_MAGIC,
            version: VBP_VERSION,
            command_code: Command::Ping as u8,
            correlation_id: 1234,
            payload_len: 0,
            flags: 0,
        };
        info!("VBP Header magic check: 0x{:x}", header.magic);
        
        pool.release(lease);
        info!("Lease released back to pool.");
    }

    // 5. Adaptive Storage Check (Rule 16)
    let sector_size = SystemTopology::get_sector_size("/dev/sda");
    info!("Adaptive Storage: Detected sector size for /dev/sda: {} bytes", sector_size);

    info!("VORTEX Platform Milestone 2 Initialized.");
}

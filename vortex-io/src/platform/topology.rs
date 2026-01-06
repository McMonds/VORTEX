use hwloc2::{Topology, ObjectType};
use log::info;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use nix::libc::ioctl;

pub struct SystemTopology {
    topology: Topology,
}

impl SystemTopology {
    pub fn new() -> Self {
        let topology = Topology::new().expect("Failed to initialize hwloc topology");
        Self { topology }
    }

    pub fn physical_cores(&self) -> Vec<usize> {
        let mut cores = Vec::new();
        let core_objects = self.topology.objects_with_type(&ObjectType::Core).expect("Failed to get cores");
        
        for core in core_objects {
            // Get the physical index (os_index)
            cores.push(core.os_index() as usize);
        }
        cores
    }

    pub fn numa_nodes(&self) -> usize {
        match self.topology.depth_for_type(&ObjectType::NUMANode) {
            Ok(depth) => {
                self.topology.objects_at_depth(depth).len()
            },
            Err(_) => 1, // Fallback for hardware with no distinct NUMA topology
        }
    }

    pub fn print_summary(&self) {
        info!("Hardware Topology Detected:");
        info!("  Physical Cores: {}", self.physical_cores().len());
        info!("  NUMA Nodes: {}", self.numa_nodes());
    }

    pub fn get_sector_size(path: &str) -> usize {
        // nix doesn't have a direct wrapper for BLKSSZGET in all versions
        // We use the raw libc ioctl constant
        const BLKSSZGET: u64 = 0x1268; // 0x12 is 'V', 104 is BLKSSZGET
        
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return 4096, // Default fallback
        };

        let mut size: i32 = 0;
        unsafe {
            if ioctl(file.as_raw_fd(), BLKSSZGET, &mut size) == 0 {
                size as usize
            } else {
                4096 // Fallback
            }
        }
    }
}

use log::{warn, info};

/// Hardware Topology Detector.
/// Identifies physical cores to enable accurate Shard-per-Core placement.
pub struct SystemTopology {
    physical_cores: Vec<usize>,
    available_ram: u64,
}

impl SystemTopology {
    /// Detects the system's physical core and memory configuration.
    pub fn new() -> Self {
        let count = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        let total_pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) } as u64;
        let av_pages = unsafe { libc::sysconf(libc::_SC_AVPHYS_PAGES) } as u64;

        let total_ram = total_pages * page_size;
        let available_ram = av_pages * page_size;
        
        let num_cores = if count <= 0 {
            warn!("Failed to detect core count via libc. Fallback to 1.");
            1
        } else {
            count as usize
        };

        // Milestone 1 simplification: Assume cores 0 to N-1 are valid physical cores.
        // In production, we'd use `hwloc` to filter out HyperThreads (SMT).
        let physical_cores: Vec<usize> = (0..num_cores).collect();
        
        info!("Topology Discovery: {} cores, {:.2} GB RAM total ({:.2} GB available).", 
            num_cores, 
            total_ram as f64 / 1e9, 
            available_ram as f64 / 1e9
        );

        if available_ram < 2_000_000_000 {
            warn!("DANGER: Low memory environment detected (< 2GB available). Adaptive scaling required.");
        }

        Self { physical_cores, available_ram }
    }

    /// Returns the IDs of available physical cores.
    pub fn physical_cores(&self) -> &[usize] {
        &self.physical_cores
    }

    /// Returns the available RAM in bytes.
    pub fn available_ram(&self) -> u64 {
        self.available_ram
    }

    /// Higher-level heuristic: Is this a "Potato" or mobile environment?
    /// VORTEX is a high-performance engine; anything under 8 cores or 16GB RAM 
    /// is treated as "Constrained" for adaptive scaling.
    pub fn is_constrained(&self) -> bool {
        self.physical_cores.len() < 8 || self.available_ram < 16_000_000_000
    }
}

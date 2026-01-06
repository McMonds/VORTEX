use log::{warn, info};

/// Hardware Topology Detector.
/// Identifies physical cores to enable accurate Shard-per-Core placement.
pub struct SystemTopology {
    physical_cores: Vec<usize>,
}

impl SystemTopology {
    /// Detects the system's physical core configuration.
    ///
    /// # Logic
    /// Attempts to query `sysconf(_SC_NPROCESSORS_ONLN)` to get the count.
    /// In a real NUMA-aware implementation, we would parse `/sys/devices/system/cpu`.
    /// For Milestone 1/Hardening, we assume a flat topology of 0..N-1 cores.
    pub fn new() -> Self {
        let count = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
        
        let num_cores = if count <= 0 {
            warn!("Failed to detect core count via libc. Fallback to 1.");
            1
        } else {
            count as usize
        };

        // Milestone 1 simplification: Assume cores 0 to N-1 are valid physical cores.
        // In production, we'd use `hwloc` to filter out HyperThreads (SMT).
        let physical_cores: Vec<usize> = (0..num_cores).collect();
        
        info!("Topology Discovery: Found {} logical processors.", num_cores);

        Self { physical_cores }
    }

    /// Returns the IDs of available physical cores.
    pub fn physical_cores(&self) -> &[usize] {
        &self.physical_cores
    }
}

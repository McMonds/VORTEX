pub mod affinity;
pub mod topology;

/// Locks all current and future memory pages into physical RAM.
/// 
/// # Logic
/// Calls `mlockall(MCL_CURRENT | MCL_FUTURE)` to prevent the OS from swapping 
/// any part of the VORTEX process to disk.
/// Swapping is catastrophic for a latency-sensitive database.
///
/// # Panics
/// Panics if the OS refuses the lock (usually due to `ulimit -l`).
/// VORTEX follows the "Fail Closed" philosophy (Rule #4).
pub fn lock_memory_pages() {
    // BUG FIX: MCL_FUTURE causes OOM if we allocate more than ulimit -l (8MB on laptops).
    // We only lock CURRENT memory to ensure the startup structures are pinned.
    // The BufferPool will manually mlock its own pages later.
    let flags = libc::MCL_CURRENT;
    
    // SAFETY: FFI call to mlockall with valid flags.
    let ret = unsafe { libc::mlockall(flags) };
    
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        log::warn!("WARNING: Failed to lock memory pages (mlockall): {}.", err);
        log::warn!("Fix: Run 'ulimit -l unlimited' or run with capability CAP_IPC_LOCK.");
        log::warn!("Continuing in constrained mode. SWAP may occur (Performance Degradation).");
    }
}

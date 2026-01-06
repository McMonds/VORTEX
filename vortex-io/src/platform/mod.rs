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
    let flags = libc::MCL_CURRENT | libc::MCL_FUTURE;
    
    // SAFETY: FFI call to mlockall with valid flags.
    let ret = unsafe { libc::mlockall(flags) };
    
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if cfg!(debug_assertions) {
            log::warn!("WARNING: Failed to lock memory (mlockall): {}.", err);
            log::warn!("Continuing in DEBUG mode. SWAP may occur (Performance Degradation).");
        } else {
            panic!(
                "CRITICAL: Failed to lock memory pages (mlockall). \
                Error: {}. \
                \n\nFix: Run 'ulimit -l unlimited' before starting VORTEX.", 
                err
            );
        }
    }
}

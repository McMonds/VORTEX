use std::mem;
use log::{info, warn};

/// Pins the current thread to a specific physical CPU core.
///
/// # Logic
/// Uses `libc::sched_setaffinity` to restrict the OS scheduler for this thread 
/// to a single bit in the CPU mask. This prevents the OS from migrating the 
/// Shard Reactor to other cores, preserving L1/L2 cache locality.
///
/// # Safety
/// This function performs an FFI call to `sched_setaffinity`. 
/// It relies on `libc::cpu_set_t` layout being correct for the target OS.
///
/// # Errors
/// Logs a warning if pinning fails (e.g., core index out of bounds). 
/// It does NOT panic, allowing the thread to run "floating" if affinity is impossible.
pub fn pin_thread_to_core(core_id: usize) {
    let mut cpu_set: libc::cpu_set_t = unsafe { mem::zeroed() };
    
    // Manual implementation of CPU_SET to avoid C-macro dependency issues
    // cpu_set_t is typically an array of bits.
    // In Rust libc, it's often a struct wrapping an array.
    unsafe {
        libc::CPU_ZERO(&mut cpu_set);
        libc::CPU_SET(core_id, &mut cpu_set);
    }

    let pid = 0; // 0 means current thread (technically process/TID in Linux)
    
    // SAFETY: 
    // - `pid` 0 refers to current thread.
    // - `cpu_set` is stack-allocated and valid.
    // - `sizeof(cpu_set_t)` is correct.
    let ret = unsafe {
        libc::sched_setaffinity(pid, mem::size_of::<libc::cpu_set_t>(), &cpu_set)
    };

    if ret != 0 {
        let err = std::io::Error::last_os_error();
        warn!("Failed to pin thread to core {}. Error: {} (Running floating)", core_id, err);
        return;
    }

    info!("Thread successfully pinned to Physical Core {}", core_id);
}

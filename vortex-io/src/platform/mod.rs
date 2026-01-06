pub mod topology;
pub mod affinity;

pub fn lock_memory_pages() {
    use nix::sys::mman::{mlockall, MlockAllFlags};
    if let Err(e) = mlockall(MlockAllFlags::MCL_CURRENT | MlockAllFlags::MCL_FUTURE) {
        log::warn!("ADAPTIVE WARNING: Failed to lock memory pages (mlockall): {}. System will continue in 'Jitter-Prone Mode'.", e);
    } else {
        log::info!("Memory pages successfully locked in RAM.");
    }
}

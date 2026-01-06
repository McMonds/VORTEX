use core_affinity::CoreId;
use log::info;

pub fn pin_thread_to_core(core_id: usize) -> bool {
    let core = CoreId { id: core_id };
    if core_affinity::set_for_current(core) {
        info!("Thread successfully pinned to physical core {}", core_id);
        true
    } else {
        panic!("CRITICAL: Failed to pin thread to core {}. Violates Implementation Standard Rule 4/13.", core_id);
    }
}

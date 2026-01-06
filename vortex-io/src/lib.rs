//! Vortex I/O: The Hardware Abstraction Layer (HAL).
//! 
//! This crate provides direct access to hardware resources, bypassing the OS where possible.
//! It implements the "Mechanical Sympathy" philosophy of VORTEX.

pub mod platform;
pub mod ring;
pub mod memory;
pub mod net;
pub mod storage;

// Re-exports for easier access by vortex-core
pub use ring::RingDriver as VortexRing;
pub use memory::BufferPool;
pub use net::VortexListener;
pub use platform::lock_memory_pages as lock_all_memory;
pub use platform::affinity;

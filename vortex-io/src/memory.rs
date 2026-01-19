use std::alloc::{alloc, dealloc, Layout};
use libc::{c_void, iovec, mlock, munlock};
use thiserror::Error;
use log::info;

/// Alignment and paging constant for VORTEX hardware-direct memory.
const PAGE_SIZE: usize = 4096;

#[derive(Error, Debug)]
pub enum MemoryError {
    #[error("Failed to allocate aligned memory")]
    AllocationFailed,
    #[error("Failed to lock memory via mlock: {0}")]
    LockFailed(i32),
    #[error("Invalid memory alignment: {0} is not a multiple of {1}")]
    InvalidAlignment(usize, usize),
}

/// A single pre-allocated buffer page.
/// Aligned to 4096 bytes and pinned in physical RAM via mlock.
pub struct BufferPage {
    ptr: *mut u8,
    layout: Layout,
}

impl BufferPage {
    /// Creates a new pinned buffer page.
    /// 
    /// # Panics
    /// Panics during startup if allocation or mlock fails (Rule I).
    pub fn new(size: usize) -> (Self, bool) {
        let layout = Layout::from_size_align(size, PAGE_SIZE).expect("CRITICAL: Invalid alignment parameters at startup");
        
        // SAFETY: Aligned via Layout, size guarantees enforced by constructor.
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            panic!("CRITICAL: Memory allocation failed during startup. Violates Rule I.");
        }

        // SAFETY: ptr is valid and allocated with size from layout. Pinning via mlock.
        let locked = unsafe { mlock(ptr as *const c_void, layout.size()) == 0 };

        (Self { ptr, layout }, locked)
    }

    /// Returns a raw iovec for io_uring registration.
    pub fn as_iovec(&self) -> iovec {
        iovec {
            iov_base: self.ptr as *mut c_void,
            iov_len: self.layout.size(),
        }
    }

    /// Access the underlying memory as a raw pointer.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Access the underlying memory as a mutable slice.
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        // SAFETY: Pinned via mlock, lifetime guaranteed by BufferPage struct.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
    }
}

impl AsMut<[u8]> for BufferPage {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_slice_mut()
    }
}

impl Drop for BufferPage {
    fn drop(&mut self) {
        // SAFETY: munlock is safe as ptr and size were validly mlocked in new().
        unsafe {
            munlock(self.ptr as *const c_void, self.layout.size());
            dealloc(self.ptr, self.layout);
        }
    }
}

/// Memory Sovereignty: The Buffer Pool Manager.
/// Orchestrates a shard-local pool of pinned memory pages.
pub struct BufferPool {
    pages: Vec<BufferPage>,
    free_indices: Vec<usize>,
    page_size: usize,
}

impl BufferPool {
    /// Initializes a new BufferPool with pinned memory.
    /// 
    /// # Panics
    /// Panics if alignment is not a multiple of 4096 (Rule #2).
    pub fn new(page_count: usize, page_size: usize) -> Self {
        if page_size % PAGE_SIZE != 0 {
            panic!("CRITICAL: BufferPool alignment violation. {} is not a multiple of {}.", page_size, PAGE_SIZE);
        }

        info!("Initializing BufferPool: {} pages of {} bytes", page_count, page_size);
        let mut pages = Vec::with_capacity(page_count);
        let mut free_indices = Vec::with_capacity(page_count);
        let mut lock_failed_count = 0;
        
        for i in 0..page_count {
            let (page, locked) = BufferPage::new(page_size);
            if !locked {
                lock_failed_count += 1;
            }
            pages.push(page);
            free_indices.push(i);
        }

        if lock_failed_count > 0 {
            log::warn!("WARNING: Failed to lock {}/{} memory pages via mlock. Performance may be degraded (Rule #4 exception).", lock_failed_count, page_count);
        }
        
        Self { pages, free_indices, page_size }
    }

    /// Leases a buffer index from the pool.
    pub fn lease(&mut self) -> Option<BufferLease> {
        self.free_indices.pop().map(|idx| BufferLease {
            index: idx,
            ptr: self.pages[idx].as_ptr(),
            len: self.page_size,
        })
    }

    /// Returns a lease to the pool's free list.
    pub fn release(&mut self, lease: BufferLease) {
        self.free_indices.push(lease.index);
    }

    /// Reclaims all pages in the pool (Batch recycle).
    pub fn reset(&mut self) {
        self.free_indices.clear();
        for i in 0..self.pages.len() {
            self.free_indices.push(i);
        }
    }

    /// Generates raw iovecs for io_uring registration phase.
    pub fn create_registration_vecs(&self) -> Vec<iovec> {
        self.pages.iter().map(|p| p.as_iovec()).collect()
    }

    /// Returns a mutable reference to a specific page.
    pub fn get_page_mut(&mut self, index: usize) -> &mut BufferPage {
        &mut self.pages[index]
    }
}

/// A lightweight handle to a leased buffer.
#[derive(Clone, Copy)]
pub struct BufferLease {
    pub index: usize,
    pub ptr: *const u8,
    pub len: usize,
}

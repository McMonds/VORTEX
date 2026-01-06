use std::alloc::{alloc, dealloc, Layout};
use log::info;

/// A single pre-allocated buffer page.
/// Aligned to 4096 bytes (Page aligned) and padded to 64-byte cache lines.
pub struct BufferPage {
    ptr: *mut u8,
    layout: Layout,
}

impl BufferPage {
    pub fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 4096).expect("Invalid alignment");
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            panic!("CRITICAL: Memory allocation failed during startup. Violates Rule 1.");
        }
        Self { ptr, layout }
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }
}

impl Drop for BufferPage {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

/// Memory Sovereignty: The Buffer Pool.
/// Pre-allocates all buffers at startup.
pub struct BufferPool {
    pages: Vec<BufferPage>,
    free_indices: Vec<usize>,
}

impl BufferPool {
    pub fn new(page_count: usize, page_size: usize) -> Self {
        info!("Initializing BufferPool: {} pages of {} bytes", page_count, page_size);
        let mut pages = Vec::with_capacity(page_count);
        let mut free_indices = Vec::with_capacity(page_count);
        
        for i in 0..page_count {
            pages.push(BufferPage::new(page_size));
            free_indices.push(i);
        }
        
        Self { pages, free_indices }
    }

    pub fn lease(&mut self) -> Option<BufferLease> {
        self.free_indices.pop().map(|idx| BufferLease {
            index: idx,
            // Safety: We manage the lifetime via the pool and lease.
            // This is a simplified version; in a real reactor, this would be more complex.
        })
    }

    pub fn release(&mut self, lease: BufferLease) {
        self.free_indices.push(lease.index);
    }

    pub fn get_page_mut(&mut self, index: usize) -> &mut BufferPage {
        &mut self.pages[index]
    }
}

#[derive(Clone, Copy)]
pub struct BufferLease {
    pub index: usize,
}

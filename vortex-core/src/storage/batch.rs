use vortex_io::memory::BufferPage;
use std::ptr;

/// High-Performance WAL Batch Accumulator (Mechanical Sympathy BP)
/// 
/// # Purpose
/// Aggregates multiple small vector updates into a single 16KB hardware sector
/// to bypass the physical IOPS limit of synchronous disk writes.
pub struct BatchAccumulator {
    buffer: BufferPage,
    cursor: usize,
    pub tags: Vec<u64>,
    capacity: usize,
}

impl BatchAccumulator {
    pub fn new() -> Self {
        let capacity = 262144; // 256KB (64 Pages)
        let (buffer, _) = BufferPage::new(capacity);
        Self {
            buffer,
            cursor: 0,
            tags: Vec::with_capacity(32),
            capacity,
        }
    }

    /// Appends data to the batch. Returns Err(()) if capacity is exceeded.
    pub fn try_add(&mut self, data: &[u8], tag: u64) -> Result<(), ()> {
        if self.cursor + data.len() > self.capacity {
            return Err(());
        }

        // SAFETY: Bounds checked above. buffer is mlocked and aligned.
        unsafe {
            let dst = self.buffer.as_slice_mut().as_mut_ptr().add(self.cursor);
            ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
        }

        self.cursor += data.len();
        self.tags.push(tag);
        Ok(())
    }

    /// Returns true if there is pending data in the buffer.
    pub fn is_dirty(&self) -> bool {
        self.cursor > 0
    }

    /// Preparces the buffer for O_DIRECT flush.
    /// Returns: (Pointer, Sector-Aligned Length)
    /// 
    /// # Safety
    /// Zeroes the tail to next 4KB boundary to satisfy mechanical sympathy.
    pub fn prepare_flush(&mut self) -> (*const u8, usize) {
        if self.cursor == 0 {
            return (ptr::null(), 0);
        }

        // 1. Sector Alignment (Rule #9 Scaling)
        let aligned_len = (self.cursor + 4095) & !4095;
        
        // 2. Zero-Masking stale data (Rule #10 Security)
        if aligned_len > self.cursor {
            let slice = self.buffer.as_slice_mut();
            unsafe {
                ptr::write_bytes(slice.as_mut_ptr().add(self.cursor), 0, aligned_len - self.cursor);
            }
        }

        let ptr = self.buffer.as_ptr();
        let len = aligned_len;
        
        // Reset cursor for next usage (if reused) or tracking
        self.cursor = 0;
        
        (ptr, len)
    }

    pub fn take_tags(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.tags)
    }

    pub fn has_tags(&self) -> bool {
        !self.tags.is_empty()
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
        self.tags.clear();
    }
}

//! Sector-aligned heap buffer for direct (unbuffered) I/O.
//!
//! O_DIRECT / FILE_FLAG_NO_BUFFERING require the user buffer to be aligned
//! to the logical sector size (4096 bytes is universally safe). `std::vec`
//! cannot guarantee alignment, so we allocate raw memory ourselves.

use std::slice;

pub struct AlignedBuf {
    ptr: *mut u8,
    cap: usize,
    layout: std::alloc::Layout,
}

// The buffer is owned memory, never shared across threads without a
// `&mut` borrow, so Send/Sync are sound here.
unsafe impl Send for AlignedBuf {}
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    pub fn new(len: usize) -> Self {
        Self::with_align(len, 4096)
    }

    pub fn with_align(len: usize, align: usize) -> Self {
        let len = len.max(1);
        let layout = std::alloc::Layout::from_size_align(len, align)
            .expect("AlignedBuf: bad layout");
        // SAFETY: layout has non-zero size and valid alignment.
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Self { ptr, cap: len, layout }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: ptr is valid for `cap` bytes, exclusively borrowed here.
        unsafe { slice::from_raw_parts_mut(self.ptr, self.cap) }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr is valid for `cap` bytes.
        unsafe { slice::from_raw_parts(self.ptr, self.cap) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.cap
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: ptr was allocated with this exact layout and is valid.
        unsafe { std::alloc::dealloc(self.ptr, self.layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_and_contents() {
        let mut b = AlignedBuf::with_align(1 << 20, 4096);
        assert_eq!(b.as_slice().len(), 1 << 20);
        let addr = b.as_mut_slice().as_ptr() as usize;
        assert_eq!(addr % 4096, 0);
        assert!(b.as_slice().iter().all(|&x| x == 0));
        b.as_mut_slice()[77] = 9;
        assert_eq!(b.as_slice()[77], 9);
    }
}

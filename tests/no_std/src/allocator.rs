use core::alloc::{GlobalAlloc, Layout};

struct Allocator;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[cfg(windows)]
unsafe extern "C" {
    fn _aligned_malloc(size: usize, alignment: usize) -> *mut u8;
    fn _aligned_free(ptr: *mut u8);
}

// SAFETY: Both backends honor Layout alignment and pair allocations with the
// matching native free function. They report allocation failure with null.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(windows)]
        // SAFETY: Layout supplies a nonzero size and power-of-two alignment.
        unsafe {
            _aligned_malloc(layout.size(), layout.align())
        }
        #[cfg(target_os = "linux")]
        {
            let mut ptr = core::ptr::null_mut();
            let alignment = layout.align().max(core::mem::size_of::<*mut u8>());
            // SAFETY: alignment is a power of two and a multiple of pointer size.
            let result = unsafe { libc::posix_memalign(&mut ptr, alignment, layout.size()) };
            if result == 0 {
                ptr.cast()
            } else {
                core::ptr::null_mut()
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
        #[cfg(windows)]
        // SAFETY: ptr came from _aligned_malloc and has not been freed.
        unsafe {
            _aligned_free(ptr);
        }
        #[cfg(target_os = "linux")]
        // SAFETY: ptr came from posix_memalign and has not been freed.
        unsafe {
            libc::free(ptr.cast());
        }
    }
}

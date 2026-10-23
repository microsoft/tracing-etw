use core::{ffi::c_void, mem::MaybeUninit};

// Keys are process-lifetime: deleting one would strand other threads' values.
pub(super) struct Key(libc::pthread_key_t);

impl Key {
    pub(super) fn new<T: 'static>() -> Self {
        let mut key = MaybeUninit::uninit();
        // SAFETY: key points to writable storage, and the callback matches T.
        let result = unsafe { libc::pthread_key_create(key.as_mut_ptr(), Some(destroy::<T>)) };
        assert_eq!(result, 0, "pthread_key_create failed: {result}");
        // SAFETY: Successful creation initialized the key.
        Self(unsafe { key.assume_init() })
    }

    pub(super) fn get(&self) -> *mut c_void {
        // SAFETY: The key is allocated once and is never deleted.
        unsafe { libc::pthread_getspecific(self.0) }
    }

    pub(super) fn set(&self, value: *mut c_void) {
        // SAFETY: The key remains valid; value is a live allocation, null, or
        // the initialization/destruction sentinel.
        let result = unsafe { libc::pthread_setspecific(self.0, value) };
        assert_eq!(result, 0, "pthread_setspecific failed: {result}");
    }
}

unsafe extern "C" fn destroy<T: 'static>(value: *mut c_void) {
    // SAFETY: pthread passes the allocation originally stored in this typed key.
    unsafe { super::native::destroy::<T>(value) }
}

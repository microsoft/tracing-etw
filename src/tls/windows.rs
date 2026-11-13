use core::ffi::c_void;
use windows_sys::Win32::{
    Foundation::GetLastError,
    System::Threading::{FLS_OUT_OF_INDEXES, FlsAlloc, FlsGetValue, FlsSetValue},
};

// Keys are process-lifetime: freeing one would invalidate other threads' slots.
//
// Each key registers a cleanup callback in this module. Keys are never freed,
// so this backend must not be used from a DLL that can be unloaded with
// FreeLibrary while threads or fibers that logged are still running: Windows
// would later invoke the callback in unmapped code.
//
// FLS calls may overwrite the calling thread's Win32 last-error code (e.g.
// FlsGetValue on a fiber without FLS data). Do not save/restore it around
// these calls: it is a significant cost on every logging call for no value.
pub(super) struct Key(u32);

impl Key {
    pub(super) fn new<T: 'static>() -> Self {
        // SAFETY: The callback matches the type stored by LocalKey<T>.
        let key = unsafe { FlsAlloc(Some(destroy::<T>)) };
        assert!(key != FLS_OUT_OF_INDEXES, "FlsAlloc failed: {}", unsafe {
            GetLastError()
        });
        Self(key)
    }

    pub(super) fn get(&self) -> *mut c_void {
        // SAFETY: The key is allocated once and is never freed. Null means no
        // value, including a new fiber without an allocated FLS data block
        // (for which Windows also sets ERROR_INVALID_PARAMETER).
        unsafe { FlsGetValue(self.0) }
    }

    pub(super) fn set(&self, value: *mut c_void) {
        // SAFETY: The key remains valid; value is a live allocation, null, or
        // the initialization/destruction sentinel.
        assert!(
            unsafe { FlsSetValue(self.0, value) } != 0,
            "FlsSetValue failed: {}",
            unsafe { GetLastError() }
        );
    }

    pub(super) fn is_current(&self, value: *mut c_void) -> bool {
        // DeleteFiber may run cleanup for another fiber. Do not overwrite a
        // different live value belonging to the calling fiber.
        let current = self.get();
        current.is_null() || current == value
    }
}

unsafe extern "system" fn destroy<T: 'static>(value: *const c_void) {
    // SAFETY: FLS passes the allocation originally stored in this typed key.
    unsafe { super::native::destroy::<T>(value.cast_mut()) }
}

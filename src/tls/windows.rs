use core::{
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
extern crate alloc;
use alloc::boxed::Box;
use windows_sys::Win32::{
    Foundation::GetLastError,
    System::Threading::{TLS_OUT_OF_INDEXES, TlsAlloc, TlsGetValue, TlsSetValue},
};

// TLS slots are cheaper to read than FLS slots, but have no destructor
// callback. A PE TLS callback runs registered destructors at thread exit
// instead, as std does. Values are per thread, shared by a thread's fibers.

// Keys are process-lifetime: freeing one would invalidate other threads' slots.
pub(super) struct Key(u32);

// Leaked, immutable registration of a key's destructor, linked at creation.
struct Registration {
    index: u32,
    destroy: unsafe fn(*mut c_void),
    next: *const Registration,
}

static REGISTRATIONS: AtomicPtr<Registration> = AtomicPtr::new(ptr::null_mut());

impl Key {
    pub(super) fn new<T: 'static>() -> Self {
        // SAFETY: TlsAlloc has no preconditions.
        let index = unsafe { TlsAlloc() };
        assert!(index != TLS_OUT_OF_INDEXES, "TlsAlloc failed: {}", unsafe {
            GetLastError()
        });
        let registration = Box::into_raw(Box::new(Registration {
            index,
            destroy: super::native::destroy::<T>,
            next: ptr::null(),
        }));
        let mut head = REGISTRATIONS.load(Ordering::Relaxed);
        loop {
            // SAFETY: The registration is unpublished and exclusively owned.
            unsafe { (*registration).next = head };
            match REGISTRATIONS.compare_exchange_weak(
                head,
                registration,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => head = current,
            }
        }
        enable_thread_exit_callback();
        Self(index)
    }

    #[inline]
    pub(super) fn get(&self) -> *mut c_void {
        // SAFETY: The index is allocated once and is never freed. Unset slots
        // are null on every thread.
        unsafe { TlsGetValue(self.0) }
    }

    pub(super) fn set(&self, value: *mut c_void) {
        // SAFETY: The index remains valid; value is a live allocation, null, or
        // the initialization/destruction sentinel.
        assert!(
            unsafe { TlsSetValue(self.0, value) } != 0,
            "TlsSetValue failed: {}",
            unsafe { GetLastError() }
        );
    }
}

const DLL_PROCESS_DETACH: u32 = 0;
const DLL_THREAD_DETACH: u32 = 3;
// Matches the minimum PTHREAD_DESTRUCTOR_ITERATIONS: destructors may access,
// and so reinitialize, other keys.
const DESTRUCTOR_ITERATIONS: usize = 4;

// The loader calls entries in .CRT$XL* between the CRT's .CRT$XLA and
// .CRT$XLZ markers, for each thread detach and for process detach.
#[unsafe(link_section = ".CRT$XLB")]
static THREAD_EXIT_CALLBACK: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) =
    on_thread_exit;

unsafe extern "C" {
    // The CRT's TLS directory, which points the loader at the callbacks.
    static _tls_used: u8;
}

fn enable_thread_exit_callback() {
    // The linker discards the CRT's TLS directory and this callback unless
    // they are referenced, so read both.
    // SAFETY: Both are valid statics that are read but never written.
    unsafe {
        ptr::read_volatile(&raw const _tls_used);
        ptr::read_volatile(&raw const THREAD_EXIT_CALLBACK);
    }
}

unsafe extern "system" fn on_thread_exit(_: *mut c_void, reason: u32, _: *mut c_void) {
    if reason == DLL_THREAD_DETACH || reason == DLL_PROCESS_DETACH {
        run_destructors();
    }
}

// Like pthread keys: clear each slot before destroying its value, and repeat
// while destructors initialize new values. The non-unwinding loader callback
// aborts if a value's destructor panics.
fn run_destructors() {
    for _ in 0..DESTRUCTOR_ITERATIONS {
        let mut destroyed = false;
        let mut registration = REGISTRATIONS.load(Ordering::Acquire);
        // SAFETY: Registrations are leaked and immutable after Release publication.
        while let Some(current) = unsafe { registration.as_ref() } {
            // SAFETY: Registered indexes are never freed.
            let value = unsafe { TlsGetValue(current.index) };
            if !value.is_null() {
                // SAFETY: As above; this thread's slot no longer owns the value.
                unsafe { TlsSetValue(current.index, ptr::null_mut()) };
                // SAFETY: The value was stored by LocalKey<T> for this key's T,
                // and ownership transfers to its destructor.
                unsafe { (current.destroy)(value) };
                destroyed = true;
            }
            registration = current.next.cast_mut();
        }
        if !destroyed {
            return;
        }
    }
}

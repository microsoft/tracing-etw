extern crate alloc;
use alloc::boxed::Box;
use core::{
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use super::Key;
use crate::sync::LazyLock;

#[cfg(test)]
use core::cell::RefCell;

#[cfg(test)]
std::thread_local! {
    static ACCESSES: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn access_count() -> usize {
    ACCESSES.with(core::cell::Cell::get)
}

#[inline]
fn read_slot(key: &Key) -> *mut c_void {
    #[cfg(test)]
    let _ = ACCESSES.try_with(|accesses| accesses.set(accesses.get() + 1));
    key.get()
}

// Value contains a key reference, so its allocation cannot use this unaligned address.
const BUSY: *mut c_void = ptr::without_provenance_mut(1);

pub(crate) struct LocalKey<T: 'static> {
    key: LazyLock<Key>,
    published: AtomicPtr<Key>,
    initialize: fn() -> T,
}

struct Value<T: 'static> {
    key: &'static Key,
    value: T,
}

struct ResetSlot<'a>(&'a Key);

impl Drop for ResetSlot<'_> {
    fn drop(&mut self) {
        self.0.set(ptr::null_mut());
    }
}

impl<T> LocalKey<T> {
    pub(crate) const fn new(initialize: fn() -> T) -> Self {
        Self {
            key: LazyLock::new(Key::new::<T>),
            published: AtomicPtr::new(ptr::null_mut()),
            initialize,
        }
    }

    fn key(&'static self) -> &'static Key {
        let key = self.published.load(Ordering::Acquire);
        if !key.is_null() {
            // SAFETY: Release publication follows initialization. The key is
            // immutable, and &'static self keeps its backing storage stationary.
            return unsafe { &*key };
        }
        let key: &'static Key = &self.key;
        self.published
            .store(ptr::from_ref(key).cast_mut(), Ordering::Release);
        key
    }

    pub(crate) fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        let key = self.key();
        let mut value = read_slot(key);
        assert!(
            value != BUSY,
            "Thread-local value accessed during initialization or destruction"
        );
        if value.is_null() {
            key.set(BUSY);
            let reset = ResetSlot(key);
            let initialized = Box::new(Value {
                key,
                value: (self.initialize)(),
            });
            key.set(ptr::from_ref(&*initialized).cast_mut().cast());
            value = Box::into_raw(initialized).cast();
            core::mem::forget(reset);
        }
        // SAFETY: Only this thread can access its slot. The allocation
        // survives until its native destructor, and the callback cannot retain
        // a reference beyond this call.
        f(unsafe { &(*value.cast::<Value<T>>()).value })
    }

    pub(crate) fn try_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Option<R> {
        let key = self.published.load(Ordering::Acquire);
        if key.is_null() {
            return None;
        }
        // SAFETY: Publication keeps this immutable key alive for the process.
        let value = read_slot(unsafe { &*key });
        if value.is_null() || value == BUSY {
            return None;
        }
        // SAFETY: As in with(), this thread owns the live allocation and
        // the callback cannot retain a borrow beyond this call.
        Some(f(unsafe { &(*value.cast::<Value<T>>()).value }))
    }
}

#[cfg(test)]
impl<T> LocalKey<RefCell<T>> {
    pub(crate) fn with_borrow<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        self.with(|value| f(&value.borrow()))
    }

    pub(crate) fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        self.with(|value| f(&mut value.borrow_mut()))
    }
}

// SAFETY: Called only by a native key's destructor with its original allocation.
// The non-unwinding native callback aborts if a value's destructor panics.
pub(super) unsafe fn destroy<T: 'static>(data: *mut c_void) {
    if data.is_null() || data == BUSY {
        return;
    }
    // SAFETY: This key was created with the destructor specialized for T, and
    // the native runtime transfers ownership of the slot to this callback.
    let value = unsafe { Box::from_raw(data.cast::<Value<T>>()) };
    let key = value.key;
    // The native runtime cleared this thread's slot before this call. Mark it
    // busy so the value's destructor cannot reinitialize it mid-destruction.
    key.set(BUSY);
    drop(value);
    key.set(ptr::null_mut());
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicUsize;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn optional_access_does_not_initialize_or_reenter_a_busy_slot() {
        static LOCAL: LocalKey<usize> = LocalKey::new(|| {
            assert!(LOCAL.try_with(|_| ()).is_none());
            7
        });
        std::thread::spawn(|| {
            assert!(LOCAL.try_with(|_| ()).is_none());
            LOCAL.with(|value| assert_eq!(*value, 7));
            assert_eq!(LOCAL.try_with(|value| *value), Some(7));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn published_key_bypasses_the_blocking_initializer() {
        static BACKING: LazyLock<Key> = LazyLock::new(Key::new::<usize>);
        static LOCAL: LocalKey<usize> = LocalKey {
            key: LazyLock::new(|| panic!("Published key must bypass lazy initialization")),
            published: AtomicPtr::new(ptr::null_mut()),
            initialize: || 42,
        };
        let key: &'static Key = &BACKING;
        LOCAL
            .published
            .store(ptr::from_ref(key).cast_mut(), Ordering::Release);
        LOCAL.with(|value| assert_eq!(*value, 42));
        LOCAL.with(|value| assert_eq!(*value, 42));
    }

    #[test]
    fn failed_key_initialization_remains_poisoned() {
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        static LOCAL: LocalKey<usize> = LocalKey {
            key: LazyLock::new(|| {
                ATTEMPTS.fetch_add(1, Ordering::SeqCst);
                panic!("Simulated native key initialization failure");
            }),
            published: AtomicPtr::new(ptr::null_mut()),
            initialize: || 42,
        };
        for _ in 0..2 {
            assert!(catch_unwind(AssertUnwindSafe(|| LOCAL.with(|_| {}))).is_err());
        }
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
    }
}

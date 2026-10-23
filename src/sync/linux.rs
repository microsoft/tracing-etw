extern crate alloc;

use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

pub(crate) struct RwLock<T> {
    raw: AtomicPtr<libc::pthread_rwlock_t>,
    value: UnsafeCell<T>,
}

// SAFETY: Moving the owner transfers T without relocating the native allocation.
unsafe impl<T: Send> Send for RwLock<T> {}
// SAFETY: Pthread locks exclude writers and permit only shared access by readers.
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

fn check(result: libc::c_int, operation: &str) {
    assert_eq!(result, 0, "{operation} failed");
}

// The caller must own an initialized, unlocked allocation with no other users.
unsafe fn destroy_raw(raw: *mut libc::pthread_rwlock_t) {
    // SAFETY: The caller guarantees exclusive ownership of an unlocked lock.
    check(
        unsafe { libc::pthread_rwlock_destroy(raw) },
        "pthread_rwlock_destroy",
    );
    // SAFETY: The pointer came from Box<MaybeUninit<pthread_rwlock_t>>.
    unsafe {
        drop(Box::from_raw(
            raw.cast::<MaybeUninit<libc::pthread_rwlock_t>>(),
        ))
    };
}

impl<T> RwLock<T> {
    pub(crate) fn get_mut(&mut self) -> Option<&mut T> {
        Some(self.value.get_mut())
    }

    pub(crate) const fn new(value: T) -> Self {
        Self {
            raw: AtomicPtr::new(ptr::null_mut()),
            value: UnsafeCell::new(value),
        }
    }

    fn raw(&self) -> *mut libc::pthread_rwlock_t {
        let raw = self.raw.load(Ordering::Acquire);
        if !raw.is_null() {
            return raw;
        }

        // Initialize at the final heap address: pthread locks cannot be moved.
        let mut storage = Box::new(MaybeUninit::<libc::pthread_rwlock_t>::uninit());
        // SAFETY: The allocation is writable, aligned, and not yet initialized.
        check(
            unsafe { libc::pthread_rwlock_init(storage.as_mut_ptr(), ptr::null()) },
            "pthread_rwlock_init",
        );
        let candidate = Box::into_raw(storage).cast::<libc::pthread_rwlock_t>();
        match self.raw.compare_exchange(
            ptr::null_mut(),
            candidate,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => candidate,
            Err(existing) => {
                // SAFETY: This unpublished candidate has never been acquired.
                unsafe { destroy_raw(candidate) };
                existing
            }
        }
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<'_, T> {
        // SAFETY: raw() returns an initialized lock kept alive by this borrow.
        check(
            unsafe { libc::pthread_rwlock_rdlock(self.raw()) },
            "pthread_rwlock_rdlock",
        );
        RwLockReadGuard {
            lock: self,
            _thread_bound: PhantomData,
        }
    }

    pub(crate) fn write(&self) -> RwLockWriteGuard<'_, T> {
        // SAFETY: raw() returns an initialized lock kept alive by this borrow.
        check(
            unsafe { libc::pthread_rwlock_wrlock(self.raw()) },
            "pthread_rwlock_wrlock",
        );
        RwLockWriteGuard {
            lock: self,
            _thread_bound: PhantomData,
        }
    }
}

impl<T> Drop for RwLock<T> {
    fn drop(&mut self) {
        let raw = *self.raw.get_mut();
        if raw.is_null() {
            return;
        }
        // SAFETY: Ownership is exclusive, but a forgotten guard can leave it held.
        let result = unsafe { libc::pthread_rwlock_trywrlock(raw) };
        if result == libc::EBUSY || result == libc::EDEADLK {
            // Leak a forgotten guard's allocation rather than destroy a held lock.
            return;
        }
        check(result, "pthread_rwlock_trywrlock");
        // SAFETY: This thread just acquired exclusive access.
        check(
            unsafe { libc::pthread_rwlock_unlock(raw) },
            "pthread_rwlock_unlock",
        );
        // SAFETY: The lock is unlocked, with no guards or waiting borrowers.
        unsafe { destroy_raw(raw) };
    }
}

pub(crate) struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
    // Pthread locks must be released on the acquiring thread.
    _thread_bound: PhantomData<*mut ()>,
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: This guard holds shared access until it is dropped.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: This thread holds shared access to this initialized lock.
        check(
            unsafe { libc::pthread_rwlock_unlock(self.lock.raw.load(Ordering::Relaxed)) },
            "pthread_rwlock_unlock",
        );
    }
}

pub(crate) struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
    _thread_bound: PhantomData<*mut ()>,
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: This guard holds exclusive access until it is dropped.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: This guard is the only writer and is exclusively borrowed.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: This thread holds exclusive access to this initialized lock.
        check(
            unsafe { libc::pthread_rwlock_unlock(self.lock.raw.load(Ordering::Relaxed)) },
            "pthread_rwlock_unlock",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_the_owner_keeps_the_initialized_native_lock_at_the_same_address() {
        let lock = RwLock::new(7);
        *lock.write() = 42;
        let raw = lock.raw();
        let relocated = Box::new(lock);
        assert_eq!(relocated.raw(), raw);
        assert_eq!(*relocated.read(), 42);
        let relocated = *relocated;
        assert_eq!(relocated.raw(), raw);
        *relocated.write() += 1;
        assert_eq!(*relocated.read(), 43);
    }
}

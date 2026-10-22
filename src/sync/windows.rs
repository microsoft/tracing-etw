use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};
use windows_sys::Win32::System::Threading::{
    AcquireSRWLockExclusive, AcquireSRWLockShared, ReleaseSRWLockExclusive, ReleaseSRWLockShared,
    SRWLOCK, SRWLOCK_INIT,
};

pub(crate) struct RwLock<T> {
    raw: UnsafeCell<SRWLOCK>,
    value: UnsafeCell<T>,
}

// SAFETY: Moving ownership transfers T; borrows prevent moving an in-use lock.
unsafe impl<T: Send> Send for RwLock<T> {}
// SAFETY: SRW locks exclude writers and permit only shared access by readers.
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            raw: UnsafeCell::new(SRWLOCK_INIT),
            value: UnsafeCell::new(value),
        }
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<'_, T> {
        // SAFETY: The lock is initialized and its borrow keeps its address stable.
        unsafe { AcquireSRWLockShared(self.raw.get()) };
        RwLockReadGuard {
            lock: self,
            _thread_bound: PhantomData,
        }
    }

    pub(crate) fn write(&self) -> RwLockWriteGuard<'_, T> {
        // SAFETY: The lock is initialized and its borrow keeps its address stable.
        unsafe { AcquireSRWLockExclusive(self.raw.get()) };
        RwLockWriteGuard {
            lock: self,
            _thread_bound: PhantomData,
        }
    }
}

pub(crate) struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
    // SRW locks must be released on the acquiring thread.
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
        // SAFETY: This thread owns shared access and releases it exactly once.
        unsafe { ReleaseSRWLockShared(self.lock.raw.get()) };
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
        // SAFETY: This thread owns exclusive access and releases it exactly once.
        unsafe { ReleaseSRWLockExclusive(self.lock.raw.get()) };
    }
}

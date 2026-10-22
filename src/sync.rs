//! Blocking synchronization using the standard library or native platform locks.

#[cfg(all(windows, not(feature = "std")))]
mod windows;
#[cfg(all(windows, not(feature = "std")))]
pub(crate) use windows::RwLock;

#[cfg(all(target_os = "linux", not(feature = "std")))]
mod linux;
#[cfg(all(target_os = "linux", not(feature = "std")))]
pub(crate) use linux::RwLock;

#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
mod lazy;
#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
pub(crate) use lazy::LazyLock;

#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
pub(crate) use std::sync::LazyLock;

#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
pub(crate) struct RwLock<T>(std::sync::RwLock<T>);

#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
impl<T> RwLock<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(std::sync::RwLock::new(value))
    }

    pub(crate) fn read(&self) -> std::sync::RwLockReadGuard<'_, T> {
        self.0.read().expect("RwLock poisoned")
    }

    pub(crate) fn write(&self) -> std::sync::RwLockWriteGuard<'_, T> {
        self.0.write().expect("RwLock poisoned")
    }
}

#[cfg(all(test, not(feature = "std"), any(windows, target_os = "linux")))]
mod tests;

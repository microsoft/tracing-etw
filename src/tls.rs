//! Thread-local values with native thread-exit cleanup in feature-off builds.

#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
use std::thread::LocalKey;
#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
pub(crate) use std::thread_local;

#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
mod native;
#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
pub(crate) use native::LocalKey;

pub(crate) fn try_with<T: 'static, R>(
    key: &'static LocalKey<T>,
    f: impl FnOnce(&T) -> R,
) -> Option<R> {
    #[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
    {
        key.try_with(f).ok()
    }
    #[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
    {
        key.try_with(f)
    }
}

#[cfg(all(not(feature = "std"), windows))]
mod windows;
#[cfg(all(not(feature = "std"), windows))]
use windows::Key;

#[cfg(all(not(feature = "std"), target_os = "linux"))]
mod linux;
#[cfg(all(not(feature = "std"), target_os = "linux"))]
use linux::Key;

#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
macro_rules! native_thread_local {
    ($(static $name:ident: $ty:ty = $initialize:expr;)+) => {
        $(static $name: $crate::tls::LocalKey<$ty> =
            $crate::tls::LocalKey::new(|| $initialize);)+
    };
}

#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
pub(crate) use native_thread_local as thread_local;

#[cfg(all(test, not(feature = "std"), any(windows, target_os = "linux")))]
mod tests;

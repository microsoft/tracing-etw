//! Thread-local values with native thread-exit cleanup in feature-off builds.

#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
pub(crate) use std::thread_local;

#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
mod native;
#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
pub(crate) use native::LocalKey;

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

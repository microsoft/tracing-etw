#[cfg(target_os = "windows")]
#[doc(hidden)]
pub mod etw_cs;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub use etw_cs::CommonSchemaProvider as Provider;

#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod user_events_cs;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use user_events_cs::CommonSchemaProvider as Provider;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
#[path = "../noop.rs"]
pub mod noop;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub use noop::Provider;

impl crate::native::EventMode for Provider {
    type Provider = Provider;
}

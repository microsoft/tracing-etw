#[cfg(target_os = "windows")]
#[doc(hidden)]
pub mod etw;
#[cfg(target_os = "windows")]
pub(crate) use etw::_start__etw_kw;
#[cfg(target_os = "windows")]
pub(crate) use etw::_stop__etw_kw;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub use etw::Provider;
#[cfg(target_os = "windows")]
pub(crate) use etw::ProviderGroupType;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub mod noop;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub use noop::Provider;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) use noop::ProviderGroupType;

#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod user_events;
#[cfg(target_os = "linux")]
pub(crate) use user_events::_start__etw_kw;
#[cfg(target_os = "linux")]
pub(crate) use user_events::_stop__etw_kw;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use user_events::Provider;
#[cfg(target_os = "linux")]
pub(crate) use user_events::ProviderGroupType;

#[cfg(target_os = "linux")]
pub(crate) use eventheader::Guid as native_guid;
#[cfg(not(target_os = "linux"))]
pub(crate) use tracelogging_dynamic::Guid as native_guid;

use crate::error::EtwError;
use core::pin::Pin;

extern crate alloc;
use alloc::sync::Arc;

#[doc(hidden)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct GuidWrapper(u128);

impl From<&native_guid> for GuidWrapper {
    fn from(value: &native_guid) -> Self {
        Self(value.to_u128())
    }
}

impl From<u128> for GuidWrapper {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<&GuidWrapper> for GuidWrapper {
    fn from(value: &GuidWrapper) -> Self {
        Self(value.0)
    }
}

impl From<GuidWrapper> for native_guid {
    fn from(value: GuidWrapper) -> Self {
        native_guid::from_u128(&value.0)
    }
}

impl From<&GuidWrapper> for u128 {
    fn from(value: &GuidWrapper) -> Self {
        value.0
    }
}

impl AsRef<u128> for GuidWrapper {
    fn as_ref(&self) -> &u128 {
        &self.0
    }
}

pub const fn to_hex_utf8_bytes(val: u64) -> [u8; 16] {
    const HEX_DIGITS: &[u8] = b"0123456789abcdef";
    [
        HEX_DIGITS[((val >> 60) & 0xf) as usize],
        HEX_DIGITS[((val >> 56) & 0xf) as usize],
        HEX_DIGITS[((val >> 52) & 0xf) as usize],
        HEX_DIGITS[((val >> 48) & 0xf) as usize],
        HEX_DIGITS[((val >> 44) & 0xf) as usize],
        HEX_DIGITS[((val >> 40) & 0xf) as usize],
        HEX_DIGITS[((val >> 36) & 0xf) as usize],
        HEX_DIGITS[((val >> 32) & 0xf) as usize],
        HEX_DIGITS[((val >> 28) & 0xf) as usize],
        HEX_DIGITS[((val >> 24) & 0xf) as usize],
        HEX_DIGITS[((val >> 20) & 0xf) as usize],
        HEX_DIGITS[((val >> 16) & 0xf) as usize],
        HEX_DIGITS[((val >> 12) & 0xf) as usize],
        HEX_DIGITS[((val >> 8) & 0xf) as usize],
        HEX_DIGITS[((val >> 4) & 0xf) as usize],
        HEX_DIGITS[((val >> 0) & 0xf) as usize],
    ]
}

impl GuidWrapper {
    pub fn from_name(name: &str) -> Self {
        Self(native_guid::from_name(name).to_u128())
    }

    pub fn to_u128(&self) -> u128 {
        self.0
    }
}

#[doc(hidden)]
mod private {
    #[doc(hidden)]
    pub trait Sealed {}
    pub struct SealedStruct;
}

#[doc(hidden)]
pub trait OutputMode: private::Sealed {}

#[doc(hidden)]
pub struct NormalOutput(private::SealedStruct);
impl private::Sealed for NormalOutput {}
impl OutputMode for NormalOutput {}

#[doc(hidden)]
pub struct CommonSchemaOutput(private::SealedStruct);
impl private::Sealed for CommonSchemaOutput {}
impl OutputMode for CommonSchemaOutput {}

#[doc(hidden)]
pub trait ProviderTraits {
    fn new<G>(
        provider_name: &str,
        provider_id: &G,
        provider_group: &Option<ProviderGroupType>,
        _default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<GuidWrapper>;

    fn supports_enable_callback() -> bool;

    fn is_valid_provider(provider_name: &str) -> Result<(), EtwError>;

    fn is_valid_group(provider_name: &str, value: &ProviderGroupType) -> Result<(), EtwError>;

    fn enabled(&self, level: &tracing_core::Level, keyword: u64) -> bool;
}

#[doc(hidden)]
pub trait EventWriter<OutMode: OutputMode> {
    fn span_start(
        self: Pin<&Self>,
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    );

    fn span_stop(
        self: Pin<&Self>,
        start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    );

    #[allow(clippy::too_many_arguments)]
    fn write_record(
        self: Pin<&Self>,
        timestamp: std::time::SystemTime,
        current_span: u64,
        parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
    );
}

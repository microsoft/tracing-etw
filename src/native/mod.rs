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
pub use etw::ProviderGroupId;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub mod noop;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub use noop::Provider;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub use noop::ProviderGroupId;

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
pub use user_events::ProviderGroupId;

#[cfg(target_os = "linux")]
pub(crate) use eventheader::Guid as native_guid;
#[cfg(not(target_os = "linux"))]
pub(crate) use tracelogging_dynamic::Guid as native_guid;

pub(crate) mod time;

use crate::error::EtwError;
use crate::layer::common::SpanStrings;
use core::pin::Pin;

extern crate alloc;
use alloc::sync::Arc;

#[cfg(windows)]
type LocalEventBuilder = tracelogging_dynamic::EventBuilder;
#[cfg(target_os = "linux")]
type LocalEventBuilder = eventheader_dynamic::EventBuilder;
#[doc(hidden)]
pub struct EventContext {
    #[cfg(any(windows, target_os = "linux"))]
    builder: core::cell::OnceCell<core::cell::RefCell<LocalEventBuilder>>,
}

impl EventContext {
    pub(crate) const fn new() -> Self {
        Self {
            #[cfg(any(windows, target_os = "linux"))]
            builder: core::cell::OnceCell::new(),
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    pub(crate) fn with_builder<R>(
        &self,
        f: impl FnOnce(core::cell::RefMut<'_, LocalEventBuilder>) -> R,
    ) -> R {
        f(self
            .builder
            .get_or_init(|| core::cell::RefCell::new(LocalEventBuilder::new()))
            .borrow_mut())
    }
}

pub(crate) struct ContextWriter<'a, W> {
    writer: Pin<&'a W>,
    context: &'a EventContext,
}

impl<'a, W> ContextWriter<'a, W> {
    pub(crate) fn new(writer: Pin<&'a W>, context: &'a EventContext) -> Self {
        Self { writer, context }
    }
}

impl<W: EventWriter<Mode>, Mode: OutputMode> EventWriter<Mode> for ContextWriter<'_, W> {
    fn span_start(
        self: Pin<&Self>,
        data: crate::layer::common::SpanRef<'_>,
        keyword: u64,
        event_tag: u32,
    ) {
        self.writer
            .span_start_in_context(data, keyword, event_tag, self.context);
    }

    fn span_stop(
        self: Pin<&Self>,
        times: (time::CivilTime, time::CivilTime),
        data: crate::layer::common::SpanRef<'_>,
        keyword: u64,
        event_tag: u32,
    ) {
        self.writer
            .span_stop_in_context(times, data, keyword, event_tag, self.context);
    }

    fn write_record(
        self: Pin<&Self>,
        timestamp: time::CivilTime,
        current_span: u64,
        parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
        span_strings: Option<SpanStrings>,
    ) {
        self.writer.write_record_in_context(
            timestamp,
            current_span,
            parent_span,
            event_name,
            level,
            keyword,
            event_tag,
            event,
            span_strings,
            self.context,
        );
    }
}

#[cfg(any(windows, target_os = "linux"))]
macro_rules! delegate_event_writer {
    ($mode:ty, $emit_start:literal) => {
        fn span_start(
            self: core::pin::Pin<&Self>,
            data: crate::layer::common::SpanRef<'_>,
            keyword: u64,
            event_tag: u32,
        ) {
            if $emit_start {
                crate::layer::core_subscriber::with_event_context(|context| {
                    <Self as crate::native::EventWriter<$mode>>::span_start_in_context(
                        self, data, keyword, event_tag, context,
                    )
                });
            }
        }
        fn span_stop(
            self: core::pin::Pin<&Self>,
            times: (
                crate::native::time::CivilTime,
                crate::native::time::CivilTime,
            ),
            data: crate::layer::common::SpanRef<'_>,
            keyword: u64,
            event_tag: u32,
        ) {
            crate::layer::core_subscriber::with_event_context(|context| {
                <Self as crate::native::EventWriter<$mode>>::span_stop_in_context(
                    self, times, data, keyword, event_tag, context,
                )
            });
        }
        fn write_record(
            self: core::pin::Pin<&Self>,
            timestamp: crate::native::time::CivilTime,
            current_span: u64,
            parent_span: u64,
            event_name: &str,
            level: &tracing_core::Level,
            keyword: u64,
            event_tag: u32,
            event: &tracing::Event<'_>,
            span_strings: Option<crate::layer::common::SpanStrings>,
        ) {
            crate::layer::core_subscriber::with_event_context(|context| {
                <Self as crate::native::EventWriter<$mode>>::write_record_in_context(
                    self,
                    timestamp,
                    current_span,
                    parent_span,
                    event_name,
                    level,
                    keyword,
                    event_tag,
                    event,
                    span_strings,
                    context,
                )
            });
        }
    };
}
#[cfg(any(windows, target_os = "linux"))]
pub(crate) use delegate_event_writer;

/// A platform-independent GUID type.
///
/// Supports conversion from `u128` and platform native GUID types
/// like `tracelogging::Guid`.
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
    pub(crate) fn from_name(name: &str) -> Self {
        Self(native_guid::from_name(name).to_u128())
    }

    /// Gets the `u128` representation
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

/// The format of events emitted by an [`EtwLayer`](crate::EtwLayer).
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait OutputMode: private::Sealed {}

/// An [`OutputMode`] representing the standard ETW or user_events format.
pub struct NormalOutput(private::SealedStruct);
impl private::Sealed for NormalOutput {}
impl OutputMode for NormalOutput {}

/// An [`OutputMode`] representing the Common Schema 4.0 format.
pub struct CommonSchemaOutput(private::SealedStruct);
impl private::Sealed for CommonSchemaOutput {}
impl OutputMode for CommonSchemaOutput {}

#[doc(hidden)]
pub trait ProviderTraits {
    fn new<G>(
        provider_name: &str,
        provider_id: &G,
        provider_group: &Option<ProviderGroupId>,
        _default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<GuidWrapper>;

    fn supports_enable_callback() -> bool;

    fn is_valid_provider(provider_name: &str) -> Result<(), EtwError>;

    fn is_valid_group(provider_name: &str, value: &ProviderGroupId) -> Result<(), EtwError>;

    fn enabled(&self, level: &tracing_core::Level, keyword: u64) -> bool;
}

#[doc(hidden)]
pub trait EventWriter<OutMode: OutputMode> {
    fn span_start_in_context(
        self: Pin<&Self>,
        data: crate::layer::common::SpanRef<'_>,
        keyword: u64,
        event_tag: u32,
        _context: &EventContext,
    ) {
        self.span_start(data, keyword, event_tag);
    }

    fn span_stop_in_context(
        self: Pin<&Self>,
        times: (time::CivilTime, time::CivilTime),
        data: crate::layer::common::SpanRef<'_>,
        keyword: u64,
        event_tag: u32,
        _context: &EventContext,
    ) {
        self.span_stop(times, data, keyword, event_tag);
    }

    #[allow(clippy::too_many_arguments)]
    fn write_record_in_context(
        self: Pin<&Self>,
        timestamp: time::CivilTime,
        current_span: u64,
        parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
        span_strings: Option<SpanStrings>,
        _context: &EventContext,
    ) {
        self.write_record(
            timestamp,
            current_span,
            parent_span,
            event_name,
            level,
            keyword,
            event_tag,
            event,
            span_strings,
        );
    }

    fn span_start(
        self: Pin<&Self>,
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    );

    fn span_stop(
        self: Pin<&Self>,
        start_stop_times: (time::CivilTime, time::CivilTime),
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    );

    #[allow(clippy::too_many_arguments)]
    fn write_record(
        self: Pin<&Self>,
        timestamp: time::CivilTime,
        current_span: u64,
        parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
        span_strings: Option<SpanStrings>,
    );
}

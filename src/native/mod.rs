#[cfg(target_os = "windows")]
#[doc(hidden)]
pub mod etw;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub use etw::Provider;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub(crate) use etw::_start__etw_kw;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub(crate) use etw::_stop__etw_kw;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub mod noop;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[doc(hidden)]
pub use noop::Provider;

#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod user_events;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use user_events::Provider;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub(crate) use user_events::_start__etw_kw;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub(crate) use user_events::_stop__etw_kw;

#[cfg(feature = "common_schema")]
pub(crate) mod common_schema;

#[doc(hidden)]
pub struct GuidWrapper(u128);

impl From<&tracelogging::Guid> for GuidWrapper {
    fn from(value: &tracelogging::Guid) -> Self {
        Self(value.to_u128())
    }
}

impl From<&eventheader::Guid> for GuidWrapper {
    fn from(value: &eventheader::Guid) -> Self {
        Self(value.to_u128())
    }
}

impl From<GuidWrapper> for tracelogging::Guid {
    fn from(value: GuidWrapper) -> Self {
        tracelogging::Guid::from_u128(&value.0)
    }
}

impl From<GuidWrapper> for eventheader::Guid {
    fn from(value: GuidWrapper) -> Self {
        eventheader::Guid::from_u128(&value.0)
    }
}

#[derive(Clone)]
#[doc(hidden)]
pub enum ProviderGroup {
    Unset,
    #[allow(dead_code)]
    Windows(tracelogging::Guid),
    #[allow(dead_code)]
    Linux(std::borrow::Cow<'static, str>),
}

#[doc(hidden)]
pub trait EventWriter {
    fn new<G>(
        provider_name: &str,
        provider_id: &G,
        provider_group: &ProviderGroup,
        _default_keyword: u64,
    ) -> std::pin::Pin<std::sync::Arc<Self>>
    where
        for<'a> &'a G: Into<GuidWrapper>;

    fn enabled(&self, level: u8, keyword: u64) -> bool;

    fn supports_enable_callback() -> bool;

    #[allow(clippy::too_many_arguments)]
    fn span_start<'a, 'b, R>(
        self: std::pin::Pin<&Self>,
        span: &'b tracing_subscriber::registry::SpanRef<'a, R>,
        timestamp: std::time::SystemTime,
        activity_id: &[u8; 16],
        related_activity_id: &[u8; 16],
        fields: &'b [crate::values::FieldValueIndex],
        level: u8,
        keyword: u64,
        event_tag: u32,
    ) where
        R: tracing_subscriber::registry::LookupSpan<'a>;

    #[allow(clippy::too_many_arguments)]
    fn span_stop<'a, 'b, R>(
        self: std::pin::Pin<&Self>,
        span: &'b tracing_subscriber::registry::SpanRef<'a, R>,
        start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        activity_id: &[u8; 16],
        related_activity_id: &[u8; 16],
        fields: &'b [crate::values::FieldValueIndex],
        level: u8,
        keyword: u64,
        event_tag: u32,
    ) where
        R: tracing_subscriber::registry::LookupSpan<'a>;

    #[allow(clippy::too_many_arguments)]
    fn write_record(
        self: std::pin::Pin<&Self>,
        timestamp: std::time::SystemTime,
        current_span: u64,
        parent_span: u64,
        event_name: &str,
        level: u8,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
    );
}

#[doc(hidden)]
pub trait EventMode {
    type Provider;
}

#[doc(hidden)]
impl EventMode for Provider {
    type Provider = Provider;
}

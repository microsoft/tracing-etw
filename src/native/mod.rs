#[cfg(target_os = "windows")]
#[doc(hidden)]
pub mod etw;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub use etw::Provider;
#[cfg(target_os = "windows")]
pub(crate) use etw::_start__etw_kw;
#[cfg(target_os = "windows")]
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
pub(crate) use user_events::_start__etw_kw;
#[cfg(target_os = "linux")]
pub(crate) use user_events::_stop__etw_kw;

#[cfg(feature = "common_schema")]
pub(crate) mod common_schema;

#[cfg(not(target_os = "linux"))]
pub(crate) use tracelogging_dynamic::Guid as native_guid;
#[cfg(target_os = "linux")]
pub(crate) use eventheader::Guid as native_guid;

#[cfg(not(target_os = "linux"))]
pub(crate) use tracelogging_dynamic::Level as native_level;
#[cfg(target_os = "linux")]
pub(crate) use eventheader::Level as native_level;

#[doc(hidden)]
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

impl GuidWrapper {
    pub fn from_name(name: &str) -> Self {
        Self(native_guid::from_name(name).to_u128())
    }
}

#[doc(hidden)]
pub trait ProviderTypes {
    type Provider;
    type ProviderGroupType;

    fn supports_enable_callback() -> bool;

    fn assert_valid(value: &Self::ProviderGroupType);

    // The compiler can't see through a 'type' within a trait to tell
    // that it trivially matches a constraint unless we lower the constraint
    // checking into the impl, done here through constraint on the return type.
    #[cfg(target_os = "linux")]
    fn get_provider_group(value: &Self::ProviderGroupType) -> impl Into<String>;
}

#[doc(hidden)]
pub trait EventWriter<Mode: ProviderTypes> {
    fn new<G>(
        provider_name: &str,
        provider_id: &G,
        provider_group: &Option<Mode::ProviderGroupType>,
        _default_keyword: u64,
    ) -> std::pin::Pin<std::sync::Arc<Self>>
    where
        for<'a> &'a G: Into<GuidWrapper>;

    fn enabled(&self, level: u8, keyword: u64) -> bool;

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

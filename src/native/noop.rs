use core::{marker::PhantomData, pin::Pin};
extern crate alloc;
use alloc::sync::Arc;

use crate::error::EtwError;

use super::{OutputMode, time};

/// The ID of a provider group, as passed to
/// [`LayerBuilder::with_provider_group`](crate::LayerBuilder::with_provider_group).
///
/// This type is platform-specific: a GUID on Windows, a string on Linux,
/// and a placeholder on other platforms, which do not support provider groups.
pub type ProviderGroupId = PhantomData<char>;

#[doc(hidden)]
pub struct Provider<Mode: OutputMode> {
    _m: PhantomData<Mode>,
}

impl<Mode: OutputMode> crate::native::ProviderTraits for Provider<Mode> {
    // The no-op provider is never enabled, so callsites can cache `Interest::never`.
    #[inline(always)]
    fn supports_enable_callback() -> bool {
        true
    }

    fn is_valid_provider(_provider_name: &str) -> Result<(), EtwError> {
        Ok(())
    }

    fn is_valid_group(_provider_name: &str, _value: &ProviderGroupId) -> Result<(), EtwError> {
        Ok(())
    }

    #[inline(always)]
    fn enabled(&self, _level: &tracing_core::Level, _keyword: u64) -> bool {
        false
    }

    fn new<G>(
        _provider_name: &str,
        _provider_id: &G,
        _provider_group: &Option<ProviderGroupId>,
        _default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<crate::native::GuidWrapper>,
    {
        Arc::pin(Self { _m: PhantomData })
    }
}

impl<OutMode: OutputMode> crate::native::EventWriter<OutMode> for Provider<OutMode> {
    fn span_start(
        self: Pin<&Self>,
        _data: crate::layer::common::SpanRef,
        _keyword: u64,
        _event_tag: u32,
    ) {
    }

    fn span_stop(
        self: Pin<&Self>,
        _start_stop_times: (time::CivilTime, time::CivilTime),
        _data: crate::layer::common::SpanRef,
        _keyword: u64,
        _event_tag: u32,
    ) {
    }

    fn write_record(
        self: Pin<&Self>,
        _timestamp: time::CivilTime,
        _current_span: u64,
        _parent_span: u64,
        _event_name: &str,
        _level: &tracing_core::Level,
        _keyword: u64,
        _event_tag: u32,
        _event: &tracing::Event<'_>,
        _span_strings: Option<crate::layer::common::SpanStrings>,
    ) {
    }
}

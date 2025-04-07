use std::{marker::PhantomData, pin::Pin, sync::Arc, time::SystemTime};

use crate::error::EtwError;

use super::OutputMode;

pub(crate) type ProviderGroupType = PhantomData<char>;

#[doc(hidden)]
pub struct Provider<Mode: OutputMode> {
    _m: PhantomData<Mode>,
}

impl<Mode: OutputMode> crate::native::ProviderTraits for Provider<Mode> {
    #[inline(always)]
    fn supports_enable_callback() -> bool {
        false
    }

    fn is_valid_provider(_provider_name: &str) -> Result<(), EtwError> {
        Ok(())
    }

    fn is_valid_group(_provider_name: &str, _value: &ProviderGroupType) -> Result<(), EtwError> {
        Ok(())
    }

    #[inline(always)]
    fn enabled(&self, _level: &tracing_core::Level, _keyword: u64) -> bool {
        false
    }

    fn new<G>(
        _provider_name: &str,
        _provider_id: &G,
        _provider_group: &Option<ProviderGroupType>,
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
        _start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        _data: crate::layer::common::SpanRef,
        _keyword: u64,
        _event_tag: u32,
    ) {
    }

    fn write_record(
        self: Pin<&Self>,
        _timestamp: SystemTime,
        _current_span: u64,
        _parent_span: u64,
        _event_name: &str,
        _level: &tracing_core::Level,
        _keyword: u64,
        _event_tag: u32,
        _event: &tracing::Event<'_>,
    ) {
    }
}

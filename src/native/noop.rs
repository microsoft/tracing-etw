use std::{pin::Pin, sync::Arc, time::SystemTime};

use tracing_subscriber::registry::{LookupSpan, SpanRef};

use crate::error::EtwError;

#[doc(hidden)]
pub struct Provider;

impl crate::native::ProviderTypes for Provider {
    type Provider = Self;
    type ProviderGroupType = std::marker::PhantomData<u8>;

    #[inline(always)]
    fn supports_enable_callback() -> bool {
        false
    }

    fn is_valid(_value: &Self::ProviderGroupType) -> Result<(), EtwError> {
        Ok(())
    }
}

impl crate::native::EventWriter<Provider> for Provider {
    fn new<G>(
        _provider_name: &str,
        _provider_id: &G,
        _provider_group: &Option<<Self as crate::native::ProviderTypes>::ProviderGroupType>,
        _default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<crate::native::GuidWrapper>,
    {
        Arc::pin(Self)
    }

    #[inline(always)]
    fn enabled(&self, _level: &tracing_core::Level, _keyword: u64) -> bool {
        false
    }

    fn span_start<'a, 'b, R>(
        self: Pin<&Self>,
        _span: &'b SpanRef<'a, R>,
        _timestamp: SystemTime,
        _activity_id: &[u8; 16],
        _related_activity_id: &[u8; 16],
        _fields: &'b [crate::values::span_values::FieldValueIndex],
        _level: &tracing_core::Level,
        _keyword: u64,
        _event_tag: u32,
    ) where
        R: LookupSpan<'a>,
    {
    }

    fn span_stop<'a, 'b, R>(
        self: Pin<&Self>,
        _span: &'b SpanRef<'a, R>,
        _start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        _activity_id: &[u8; 16],
        _related_activity_id: &[u8; 16],
        _fields: &'b [crate::values::span_values::FieldValueIndex],
        _level: &tracing_core::Level,
        _keyword: u64,
        _event_tag: u32,
    ) where
        R: LookupSpan<'a>,
    {
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

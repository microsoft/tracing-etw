use core::sync::atomic::AtomicU64;

use crate::{layer::common, native::ProviderTraits, statics::get_event_metadata};

use super::*;

static CURRENT_SPAN_ID: AtomicU64 = AtomicU64::new(1);

impl<OutMode: OutputMode + 'static> tracing::Subscriber for _EtwTracingSubscriber<OutMode>
where
    crate::native::Provider<OutMode>: crate::native::EventWriter<OutMode>,
{
    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing_core::Interest {
        let etw_meta = get_event_metadata(&metadata.callsite());
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.default_keyword
        };

        if crate::native::Provider::<OutMode>::supports_enable_callback() {
            if self.provider.enabled(metadata.level(), keyword) {
                tracing::subscriber::Interest::always()
            } else {
                tracing::subscriber::Interest::never()
            }
        } else {
            // Returning "sometimes" means the enabled function will be called every time an event or span is created from the callsite.
            // This will let us perform a global "is enabled" check each time.
            tracing::subscriber::Interest::sometimes()
        }
    }

    // Only called if register_callsite returned Interest::sometimes
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        self.is_enabled(&metadata.callsite(), metadata.level())
    }

    fn new_span(&self, attrs: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        // SAFETY: The current ID starts at 1 and only ever goes up
        let id = unsafe {
            tracing::span::Id::from_non_zero_u64(core::num::NonZero::new_unchecked(
                CURRENT_SPAN_ID.fetch_add(1, core::sync::atomic::Ordering::AcqRel),
            ))
        };

        common::create_span_data_for_new_span(attrs, &id);

        id
    }

    fn clone_span(&self, id: &tracing_core::span::Id) -> tracing_core::span::Id {
        common::addref_span(id);
        id.clone()
    }

    fn try_close(&self, id: tracing_core::span::Id) -> bool {
        common::release_span(&id)
    }

    fn record(&self, id: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        common::update_span_values(id, values)
    }

    fn record_follows_from(
        &self,
        _span: &tracing_core::span::Id,
        _follows: &tracing_core::span::Id,
    ) {
        // Do nothing
    }

    fn event(&self, event: &tracing::Event<'_>) {
        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.default_keyword, 0)
        };

        common::write_event(self.provider.as_ref(), event, name, keyword, tag)
    }

    fn enter(&self, id: &tracing_core::span::Id) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::enter_span(id, self.provider.as_ref(), self.default_keyword, 0)
    }

    fn exit(&self, id: &tracing_core::span::Id) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::exit_span(id, self.provider.as_ref(), self.default_keyword, 0);
    }
}

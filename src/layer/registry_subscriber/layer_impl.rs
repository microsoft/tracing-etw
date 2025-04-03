use tracing::Subscriber;
#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing_core::{callsite, span};
use tracing_subscriber::registry::LookupSpan;

use crate::{layer::common, native::OutputMode, statics::*};

use super::EtwLayer;

impl<S, OutMode: OutputMode + 'static> tracing_subscriber::Layer<S> for EtwLayer<S, OutMode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    crate::native::Provider<OutMode>: crate::native::EventWriter<OutMode>,
{
    fn on_register_dispatch(&self, _collector: &tracing::Dispatch) {
        // Late init when the layer is installed as a subscriber
    }

    fn on_layer(&mut self, _subscriber: &mut S) {
        // Late init when the layer is attached to a subscriber
    }

    #[cfg(any(feature = "global_filter", docsrs))]
    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.layer
            .is_enabled(&metadata.callsite(), metadata.level())
    }

    #[cfg(any(feature = "global_filter", docsrs))]
    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.layer
            .is_enabled(&event.metadata().callsite(), event.metadata().level())
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.default_keyword, 0)
        };

        common::write_event(self.provider.as_ref(), event, name, keyword, tag)
    }

    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        common::create_span_data_for_new_span(attrs, id)
    }

    fn on_enter(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::enter_span(id, self.provider.as_ref(), self.default_keyword, 0)
    }

    fn on_exit(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::exit_span(id, self.provider.as_ref(), self.default_keyword, 0)
    }

    fn on_close(&self, id: span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let _ = common::release_span(&id);
    }

    fn on_record(
        &self,
        id: &span::Id,
        values: &span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        common::update_span_values(id, values)
    }
}

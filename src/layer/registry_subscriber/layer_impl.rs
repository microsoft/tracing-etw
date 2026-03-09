use tracing::Subscriber;
#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing_core::{callsite, span};
use tracing_subscriber::registry::LookupSpan;

use crate::{
    layer::{_EtwTracingSubscriber, common, common::SpanStrings},
    native::OutputMode,
    statics::*,
};

fn get_otel_span_strings<S>(span: Option<tracing_subscriber::registry::SpanRef<'_, S>>) -> Option<SpanStrings>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    span.and_then(|span| {
        // Extract OpenTelemetry context if available
        #[cfg(feature = "opentelemetry")]
        {
            crate::otel::get_otel_span_data(&span)
        }
        #[cfg(not(feature = "opentelemetry"))]
        {
            None
        }
    })
}

impl<OutMode: OutputMode + 'static, S> tracing_subscriber::Layer<S>
    for _EtwTracingSubscriber<OutMode, S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    crate::native::Provider<OutMode>: crate::native::EventWriter<OutMode>,
{
    fn on_register_dispatch(&self, _collector: &tracing::Dispatch) {
        // Late init when the layer is installed as a tracing_core::subscriber and becomes a Dispatcher
    }

    fn on_layer(&mut self, _subscriber: &mut S) {
        // Late init when the layer is added to a subscriber
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.default_keyword, 0)
        };

        common::write_event(self.provider.as_ref(), event, name, keyword, tag, get_otel_span_strings(ctx.event_span(event)))
    }

    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // We can't cache the otel spans here because we don't know which layer will see the span first
        common::create_span_data_for_new_span(attrs, id)
    }

    fn on_enter(&self, id: &span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::enter_span(id, self.provider.as_ref(), self.default_keyword, 0, get_otel_span_strings(ctx.span(id)))
    }

    fn on_exit(&self, id: &span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::exit_span(id, self.provider.as_ref(), self.default_keyword, 0, get_otel_span_strings(ctx.span(id)))
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

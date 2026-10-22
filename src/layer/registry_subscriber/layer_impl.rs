use tracing::Subscriber;
#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing_core::{callsite, span};
use tracing_subscriber::registry::LookupSpan;

use crate::{
    layer::{_EtwTracingSubscriber, common, common::SpanStrings},
    native::OutputMode,
    statics::*,
};

impl<OutMode: OutputMode, S> _EtwTracingSubscriber<OutMode, S> {
    fn get_otel_span_strings(&self, id: Option<&span::Id>) -> Option<SpanStrings> {
        #[cfg(feature = "opentelemetry")]
        {
            let dispatch = self.dispatch.get()?.upgrade()?;
            crate::otel::get_otel_span_data(id?, &dispatch)
        }
        #[cfg(not(feature = "opentelemetry"))]
        {
            let _ = id;
            None
        }
    }
}

impl<OutMode: OutputMode + 'static, S> tracing_subscriber::Layer<S>
    for _EtwTracingSubscriber<OutMode, S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    crate::native::Provider<OutMode>: crate::native::EventWriter<OutMode>,
{
    fn on_register_dispatch(&self, _collector: &tracing::Dispatch) {
        #[cfg(feature = "opentelemetry")]
        self.dispatch.get_or_init(|| _collector.downgrade());
    }

    fn on_layer(&mut self, _subscriber: &mut S) {
        // Late init when the layer is added to a subscriber
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.default_keyword, 0)
        };

        let id = ctx.event_span(event).map(|span| span.id());
        common::write_event(
            self.subscriber_id,
            self.provider.as_ref(),
            event,
            name,
            keyword,
            tag,
            id.as_ref(),
            self.get_otel_span_strings(id.as_ref()),
        )
    }

    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // We can't cache the otel spans here because we don't know which layer will see the span first
        let parent = if attrs.is_contextual() {
            ctx.lookup_current().map(|span| span.id())
        } else {
            attrs.parent().cloned()
        };
        common::create_span_data_for_new_span(self.subscriber_id, attrs, id, parent.as_ref())
    }

    fn on_enter(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::enter_span(
            self.subscriber_id,
            id,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            self.get_otel_span_strings(Some(id)),
        )
    }

    fn on_exit(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::exit_span(
            self.subscriber_id,
            id,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            self.get_otel_span_strings(Some(id)),
        )
    }

    fn on_close(&self, id: span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let _ = common::release_span(self.subscriber_id, &id);
    }

    fn on_record(
        &self,
        id: &span::Id,
        values: &span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        common::update_span_values(self.subscriber_id, id, values)
    }
}

#[cfg(all(test, feature = "opentelemetry"))]
mod tests {
    use opentelemetry::{
        Context,
        trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
    };
    use tracing::{Dispatch, callsite::Callsite};
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::{LayerBuilder, NormalOutput};

    fn exercise_context<S: 'static, T: Subscriber + Send + Sync>(
        layer: _EtwTracingSubscriber<NormalOutput, S>,
        build: impl FnOnce(_EtwTracingSubscriber<NormalOutput, S>) -> T,
        valid_parent: bool,
    ) {
        let dispatch = Dispatch::new(build(layer));
        let layer = dispatch
            .downcast_ref::<_EtwTracingSubscriber<NormalOutput, S>>()
            .unwrap();
        let span = tracing::dispatcher::with_default(&dispatch, || {
            tracing::info_span!("OtelContext", value = 1)
        });
        if valid_parent {
            let parent = SpanContext::new(
                TraceId::from(0x1234),
                SpanId::from(0x5678),
                TraceFlags::SAMPLED,
                true,
                TraceState::default(),
            );
            span.set_parent(Context::new().with_remote_span_context(parent))
                .unwrap();
        }
        let id = span.id().unwrap();

        // Span callbacks and explicitly dispatched events must use their own
        // subscriber, not the thread's current default dispatch.
        tracing::dispatcher::with_default(&Dispatch::none(), || {
            let strings = layer.get_otel_span_strings(Some(&id));
            if valid_parent {
                let strings = strings.expect("A valid OpenTelemetry context is available");
                assert_eq!(&strings.trace_id, b"00000000000000000000000000001234");
                assert_eq!(&strings.span_id, b"0000000000005678");
                assert!(strings.parent_span_id.is_none());
            } else {
                assert!(strings.is_none());
            }

            let _entered = span.enter();
            let metadata = tracing::callsite! {
                name: "OtelEvent",
                kind: tracing::metadata::Kind::EVENT,
                target: module_path!(),
                level: tracing::Level::INFO,
                fields: value
            }
            .metadata();
            dispatch.event(&tracing::Event::new_child_of(
                Some(id.clone()),
                metadata,
                &tracing::valueset!(metadata.fields(), value = 1),
            ));
        });
        let weak = layer.dispatch.get().unwrap().clone();
        drop(span);
        drop(dispatch);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn extracts_context_in_either_layer_order() {
        exercise_context(
            LayerBuilder::new("OtelInner").__build_for_test().unwrap(),
            |layer| {
                tracing_subscriber::registry()
                    .with(layer)
                    .with(tracing_opentelemetry::layer())
            },
            true,
        );
        exercise_context(
            LayerBuilder::new("OtelOuter").__build_for_test().unwrap(),
            |layer| {
                tracing_subscriber::registry()
                    .with(tracing_opentelemetry::layer())
                    .with(layer)
            },
            true,
        );
    }

    #[test]
    fn invalid_context_preserves_local_span_fallback() {
        exercise_context(
            LayerBuilder::new("OtelInvalid").__build_for_test().unwrap(),
            |layer| {
                tracing_subscriber::registry()
                    .with(layer)
                    .with(tracing_opentelemetry::layer())
            },
            false,
        );
    }

    #[test]
    fn missing_otel_layer_preserves_local_span_fallback() {
        exercise_context(
            LayerBuilder::new("OtelMissing").__build_for_test().unwrap(),
            |layer| tracing_subscriber::registry().with(layer),
            false,
        );
    }
}

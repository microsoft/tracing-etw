use tracing::Subscriber;
#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing_core::{callsite, span};
use tracing_subscriber::registry::LookupSpan;

use crate::{
    layer::{
        _EtwTracingSubscriber,
        common::{SpanStrings, registry},
    },
    native::OutputMode,
    statics::*,
};

impl<OutMode: OutputMode, S> _EtwTracingSubscriber<OutMode, S> {
    fn get_otel_span_strings(&self, id: Option<&span::Id>) -> Option<SpanStrings> {
        #[cfg(feature = "opentelemetry")]
        {
            // Skip the dispatch upgrade and context lookup when the output
            // mode would discard the result.
            if !crate::native::uses_span_strings::<OutMode>() {
                return None;
            }
            // Layers that were never registered with a dispatcher (installed
            // through reload::Handle::reload, or inside an Arc-shared
            // subscriber) cannot tell which registry owns a span ID, so they
            // use local span IDs rather than risk another registry's context.
            let id = id?;
            let dispatch = self.dispatch.upgrade()?;
            crate::otel::get_otel_span_data(id, &dispatch)
        }
        #[cfg(not(feature = "opentelemetry"))]
        {
            let _ = id;
            None
        }
    }

    // Span stop records also carry the parent's span ID. The child and parent
    // are resolved independently: the OpenTelemetry layer may track a parent
    // but filter out the child. Each falls back to its local span ID, and the
    // trace ID stays empty when the child has no OpenTelemetry context.
    fn get_otel_span_stop_strings(
        &self,
        id: &span::Id,
        parent: impl FnOnce() -> Option<span::Id>,
    ) -> Option<SpanStrings> {
        if !cfg!(feature = "opentelemetry") || !crate::native::uses_span_strings::<OutMode>() {
            return None;
        }
        let child = self.get_otel_span_strings(Some(id));
        let parent = parent().map(|parent| (self.get_otel_span_strings(Some(&parent)), parent));
        if child.is_none() && !matches!(parent, Some((Some(_), _))) {
            // Nothing from OpenTelemetry; the stored local IDs are equivalent.
            return None;
        }
        let mut strings = child.unwrap_or(SpanStrings {
            trace_id: [0; 32],
            span_id: crate::native::to_hex_utf8_bytes(id.into_u64()),
            parent_span_id: None,
        });
        strings.parent_span_id = parent.map(|(otel, parent)| {
            otel.map_or_else(
                || crate::native::to_hex_utf8_bytes(parent.into_u64()),
                |parent| parent.span_id,
            )
        });
        Some(strings)
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
        self.dispatch.register(_collector);
    }

    fn on_layer(&mut self, _subscriber: &mut S) {
        // Span IDs are only unique within one subscriber, so each attachment
        // needs its own storage identity. Clones attached to different
        // registries would otherwise share span data for colliding IDs.
        self.subscriber_id = crate::layer::core_subscriber::next_id();
        *self.attached.get_mut() = true;
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.default_keyword, 0)
        };

        let id = ctx.event_span(event).map(|span| span.id());
        registry::write_event(
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
        // We can't cache the otel spans here because we don't know which layer will see the span first.
        // Use the registry ancestry rather than the entered-span stack: the
        // registry keeps ancestors alive until their children close, so the
        // retained parent ID cannot be recycled early. For filtered layers,
        // parent() also skips ancestors this layer did not enable.
        let parent = ctx
            .span(id)
            .and_then(|span| span.parent())
            .map(|parent| parent.id());
        // Layers installed through reload::Handle::reload never see on_layer.
        // Load first so span creation doesn't keep writing a shared cache line.
        if !self.attached.load(core::sync::atomic::Ordering::Relaxed) {
            self.attached
                .store(true, core::sync::atomic::Ordering::Relaxed);
        }
        registry::create_span_data_for_new_span(self.subscriber_id, attrs, id, parent.as_ref())
    }

    fn on_enter(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        // No output mode writes span strings in span start events, so skip
        // the OpenTelemetry lookup here.
        registry::enter_span(
            self.subscriber_id,
            id,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            None,
        )
    }

    fn on_exit(&self, id: &span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        registry::exit_span(
            self.subscriber_id,
            id,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            self.get_otel_span_stop_strings(id, || {
                ctx.span(id)
                    .and_then(|span| span.parent())
                    .map(|parent| parent.id())
            }),
        )
    }

    // Callbacks for spans created before this layer was attached (for example
    // through tracing_subscriber::reload) are unknown and ignored.
    fn on_close(&self, id: span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let _ = registry::release_span(self.subscriber_id, &id);
    }

    fn on_record(
        &self,
        id: &span::Id,
        values: &span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        registry::update_span_values(self.subscriber_id, id, values)
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
    use crate::{CommonSchemaOutput, LayerBuilder, NormalOutput};

    fn exercise_context<Mode: OutputMode + 'static, S: 'static, T: Subscriber + Send + Sync>(
        layer: _EtwTracingSubscriber<Mode, S>,
        build: impl FnOnce(_EtwTracingSubscriber<Mode, S>) -> T,
        valid_parent: bool,
    ) where
        crate::native::Provider<Mode>: crate::native::EventWriter<Mode>,
    {
        let dispatch = Dispatch::new(build(layer));
        let layer = dispatch
            .downcast_ref::<_EtwTracingSubscriber<Mode, S>>()
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
            if valid_parent && crate::native::uses_span_strings::<Mode>() {
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
        let weak = layer.dispatch.registered()[0].clone();
        drop(span);
        drop(dispatch);
        // Concurrent callsite-interest rebuilds in other tests can briefly
        // upgrade every registered dispatcher, so allow those to finish.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while weak.upgrade().is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "The layer must not keep its dispatcher alive"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn extracts_context_in_either_layer_order() {
        exercise_context(
            LayerBuilder::new_common_schema_events("OtelInner")
                .__build_for_test()
                .unwrap(),
            |layer| {
                tracing_subscriber::registry()
                    .with(layer)
                    .with(tracing_opentelemetry::layer())
            },
            true,
        );
        exercise_context(
            LayerBuilder::new_common_schema_events("OtelOuter")
                .__build_for_test()
                .unwrap(),
            |layer| {
                tracing_subscriber::registry()
                    .with(tracing_opentelemetry::layer())
                    .with(layer)
            },
            true,
        );
    }

    #[test]
    fn span_stop_strings_include_the_parent_span_id() {
        let dispatch = Dispatch::new(
            tracing_subscriber::registry()
                .with(
                    LayerBuilder::new_common_schema_events("OtelParent")
                        .__build_for_test()
                        .unwrap(),
                )
                .with(tracing_opentelemetry::layer()),
        );
        let layer = dispatch
            .downcast_ref::<_EtwTracingSubscriber<CommonSchemaOutput, tracing_subscriber::Registry>>()
            .unwrap();
        tracing::dispatcher::with_default(&dispatch, || {
            let parent = tracing::info_span!("OtelParent");
            let remote = SpanContext::new(
                TraceId::from(0x1234),
                SpanId::from(0x5678),
                TraceFlags::SAMPLED,
                true,
                TraceState::default(),
            );
            parent
                .set_parent(Context::new().with_remote_span_context(remote))
                .unwrap();
            let child = tracing::info_span!(parent: &parent, "OtelChild");
            let parent_id = parent.id().unwrap();
            let child_id = child.id().unwrap();
            // Without a parent valid OpenTelemetry context, use the local ID.
            let local = tracing::info_span!(parent: None, "OtelLocalParent");
            let local_id = local.id().unwrap();

            tracing::dispatcher::with_default(&Dispatch::none(), || {
                let strings = layer
                    .get_otel_span_stop_strings(&child_id, || Some(parent_id.clone()))
                    .expect("A valid OpenTelemetry context is available");
                assert_eq!(&strings.trace_id, b"00000000000000000000000000001234");
                let parent_strings = layer.get_otel_span_strings(Some(&parent_id)).unwrap();
                assert_eq!(strings.parent_span_id, Some(parent_strings.span_id));

                let strings = layer
                    .get_otel_span_stop_strings(&child_id, || Some(local_id.clone()))
                    .unwrap();
                assert_eq!(
                    strings.parent_span_id,
                    Some(crate::native::to_hex_utf8_bytes(local_id.into_u64()))
                );

                let strings = layer
                    .get_otel_span_stop_strings(&child_id, || None)
                    .unwrap();
                assert!(strings.parent_span_id.is_none());
            });
        });
    }

    fn remote_parent() -> Context {
        Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from(0x1234),
            SpanId::from(0x5678),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ))
    }

    #[test]
    fn registered_dispatches_fall_back_to_any_live_dispatch() {
        let dispatches = crate::layer::registry_subscriber::Dispatches::default();
        assert!(dispatches.is_empty() && dispatches.upgrade().is_none());
        let first = Dispatch::new(tracing_subscriber::registry());
        let second = Dispatch::new(tracing_subscriber::registry());
        dispatches.register(&first);
        dispatches.register(&second);
        drop(first);
        let live = dispatches.upgrade().expect("The second dispatch is alive");
        assert!(
            live.downcast_ref::<tracing_subscriber::Registry>()
                .is_some()
        );
        drop((live, second));
        assert!(!dispatches.is_empty());
        // Concurrent callsite-interest rebuilds in other tests can briefly
        // upgrade every registered dispatcher, so allow those to finish.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while dispatches.upgrade().is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "Registered dispatches must not keep subscribers alive"
            );
            std::thread::yield_now();
        }
    }

    // tracing-core's `Arc<S>` subscriber does not forward on_register_dispatch,
    // so the layer cannot tell which registry owns a span ID. Another registry
    // can hold a span with the same ID and callsite; its context must not be
    // used.
    #[test]
    fn unregistered_layer_uses_local_ids_instead_of_another_registrys_context() {
        let subscriber = std::sync::Arc::new(
            tracing_subscriber::registry()
                .with(
                    LayerBuilder::new_common_schema_events("OtelUnregistered")
                        .__build_for_test()
                        .unwrap(),
                )
                .with(tracing_opentelemetry::layer()),
        );
        let owner = Dispatch::new(subscriber.clone());
        let other =
            Dispatch::new(tracing_subscriber::registry().with(tracing_opentelemetry::layer()));
        let layer = subscriber
            .downcast_ref::<_EtwTracingSubscriber<CommonSchemaOutput, tracing_subscriber::Registry>>()
            .unwrap();
        let new_span = || {
            let span = tracing::info_span!("OtelCollision");
            span.set_parent(remote_parent()).unwrap();
            span
        };
        tracing::dispatcher::with_default(&owner, || {
            let ours = new_span();
            let id = ours.id().unwrap();
            tracing::dispatcher::with_default(&other, || {
                let theirs = new_span();
                assert_eq!(theirs.id(), Some(id.clone()));
                assert!(layer.get_otel_span_strings(Some(&id)).is_none());
                assert!(layer.get_otel_span_stop_strings(&id, || None).is_none());
            });
        });
    }

    #[test]
    fn parent_otel_context_is_resolved_when_the_child_is_filtered_out() {
        use tracing_subscriber::filter::filter_fn;

        let dispatch = Dispatch::new(
            tracing_subscriber::registry()
                .with(
                    LayerBuilder::new_common_schema_events("OtelFilteredChild")
                        .__build_for_test()
                        .unwrap(),
                )
                .with(
                    tracing_opentelemetry::layer()
                        .with_filter(filter_fn(|metadata| metadata.name() != "OtelFiltered")),
                ),
        );
        let layer = dispatch
            .downcast_ref::<_EtwTracingSubscriber<CommonSchemaOutput, tracing_subscriber::Registry>>()
            .unwrap();
        tracing::dispatcher::with_default(&dispatch, || {
            let parent = tracing::info_span!("OtelTracked");
            let remote = SpanContext::new(
                TraceId::from(0x1234),
                SpanId::from(0x5678),
                TraceFlags::SAMPLED,
                true,
                TraceState::default(),
            );
            parent
                .set_parent(Context::new().with_remote_span_context(remote))
                .unwrap();
            let child = tracing::info_span!(parent: &parent, "OtelFiltered");
            let parent_id = parent.id().unwrap();
            let child_id = child.id().unwrap();
            let untracked = tracing::info_span!(parent: None, "OtelFiltered");
            let untracked_id = untracked.id().unwrap();

            tracing::dispatcher::with_default(&Dispatch::none(), || {
                assert!(layer.get_otel_span_strings(Some(&child_id)).is_none());
                let parent_strings = layer.get_otel_span_strings(Some(&parent_id)).unwrap();
                let strings = layer
                    .get_otel_span_stop_strings(&child_id, || Some(parent_id.clone()))
                    .expect("The parent has an OpenTelemetry context");
                assert_eq!(strings.trace_id, [0; 32]);
                assert_eq!(
                    strings.span_id,
                    crate::native::to_hex_utf8_bytes(child_id.into_u64())
                );
                assert_eq!(strings.parent_span_id, Some(parent_strings.span_id));

                // Neither span has a context, so the stored local IDs are used.
                assert!(
                    layer
                        .get_otel_span_stop_strings(&child_id, || Some(untracked_id.clone()))
                        .is_none()
                );
            });
        });
    }

    #[test]
    fn normal_output_skips_unused_context_lookups() {
        exercise_context::<NormalOutput, _, _>(
            LayerBuilder::new("OtelNormal").__build_for_test().unwrap(),
            |layer| {
                tracing_subscriber::registry()
                    .with(layer)
                    .with(tracing_opentelemetry::layer())
            },
            true,
        );
    }

    #[test]
    fn invalid_context_preserves_local_span_fallback() {
        exercise_context::<CommonSchemaOutput, _, _>(
            LayerBuilder::new_common_schema_events("OtelInvalid")
                .__build_for_test()
                .unwrap(),
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
        exercise_context::<CommonSchemaOutput, _, _>(
            LayerBuilder::new_common_schema_events("OtelMissing")
                .__build_for_test()
                .unwrap(),
            |layer| tracing_subscriber::registry().with(layer),
            false,
        );
    }
}

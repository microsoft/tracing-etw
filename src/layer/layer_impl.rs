use std::time::SystemTime;

use tracing::Subscriber;
#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing_core::{callsite, span};
use tracing_subscriber::{registry::LookupSpan, Layer};

use crate::{
    native::{EventWriter, ProviderTypes},
    statics::*,
    values::*,
};

use super::*;

struct SpanData {
    fields: Box<[FieldValueIndex]>,
    activity_id: [u8; 16], // // if set, byte 0 is 1 and 64-bit span ID in the lower 8 bytes
    related_activity_id: [u8; 16], // if set, byte 0 is 1 and 64-bit span ID in the lower 8 bytes
    start_time: SystemTime,
}

impl<S, Mode> Layer<S> for EtwLayer<S, Mode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    Mode: ProviderTypes + 'static,
    Mode::Provider: EventWriter<Mode> + 'static,
{
    fn on_register_dispatch(&self, _collector: &tracing::Dispatch) {
        // Late init when the layer is installed as a subscriber
    }

    fn on_layer(&mut self, _subscriber: &mut S) {
        // Late init when the layer is attached to a subscriber
    }

    #[cfg(any(feature = "global_filter", docsrs))]
    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        let etw_meta = get_event_metadata(&metadata.callsite());
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.layer.default_keyword
        };

        if Mode::supports_enable_callback() {
            if self.layer.provider.enabled(metadata.level(), keyword) {
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

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let timestamp = std::time::SystemTime::now();

        let current_span = ctx
            .event_span(event)
            .map(|evt| evt.id())
            .map_or(0, |id| (id.into_u64()));
        let parent_span = ctx
            .event_span(event)
            .map_or(0, |evt| evt.parent().map_or(0, |p| p.id().into_u64()));

        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.layer.default_keyword, 0)
        };

        self.layer.provider.as_ref().write_record(
            timestamp,
            current_span,
            parent_span,
            name,
            event.metadata().level(),
            keyword,
            tag,
            event,
        );
    }

    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let span = if let Some(span) = ctx.span(id) {
            span
        } else {
            return;
        };

        if span.extensions().get::<SpanData>().is_some() {
            return;
        }

        let metadata = span.metadata();

        let parent_span_id = if attrs.is_contextual() {
            attrs.parent().map_or(0, |id| id.into_u64())
        } else {
            0
        };

        let n = metadata.fields().len();

        let mut data = {
            let mut v: Vec<FieldValueIndex> = Vec::with_capacity(n);
            v.resize_with(n, Default::default);

            let mut i = 0;
            for field in metadata.fields().iter() {
                v[i].field = field.name();
                v[i].value = ValueTypes::None;
                v[i].sort_index = i as u8;
                i += 1;
            }

            let mut indexes: [u8; 32] = [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ];

            indexes[0..n].sort_by_key(|idx| v[v[*idx as usize].sort_index as usize].field);

            i = 0;
            for f in &mut v {
                f.sort_index = indexes[i];
                i += 1;
            }

            SpanData {
                fields: v.into_boxed_slice(),
                activity_id: *GLOBAL_ACTIVITY_SEED,
                related_activity_id: *GLOBAL_ACTIVITY_SEED,
                start_time: SystemTime::UNIX_EPOCH,
            }
        };

        let (_, half) = data.activity_id.split_at_mut(8);
        half.copy_from_slice(&id.into_u64().to_le_bytes());

        data.activity_id[0] = 1;
        data.related_activity_id[0] = if parent_span_id != 0 {
            let (_, half) = data.related_activity_id.split_at_mut(8);
            half.copy_from_slice(&parent_span_id.to_le_bytes());
            1
        } else {
            0
        };

        attrs.values().record(&mut ValueVisitor {
            fields: &mut data.fields,
        });

        // This will unfortunately box data. It would be ideal if we could avoid this second heap allocation
        // by packing everything into a single alloc.
        span.extensions_mut().replace(data);
    }

    fn on_enter(&self, id: &span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        // A span was started
        let timestamp = std::time::SystemTime::now();

        let span = if let Some(span) = ctx.span(id) {
            span
        } else {
            return;
        };

        let metadata = span.metadata();

        let mut extensions = span.extensions_mut();
        let data = if let Some(data) = extensions.get_mut::<SpanData>() {
            data
        } else {
            // We got a span that was entered without being new'ed?
            return;
        };

        let etw_meta = get_event_metadata(&metadata.callsite());
        let (keyword, tag) = if let Some(meta) = etw_meta {
            (meta.kw, meta.event_tag)
        } else {
            (self.layer.default_keyword, 0)
        };

        self.layer.provider.as_ref().span_start(
            &span,
            timestamp,
            &data.activity_id,
            &data.related_activity_id,
            &data.fields,
            metadata.level(),
            keyword,
            tag,
        );

        data.start_time = timestamp;
    }

    fn on_exit(&self, id: &span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        // A span was exited
        let stop_timestamp = std::time::SystemTime::now();

        let span = if let Some(span) = ctx.span(id) {
            span
        } else {
            return;
        };

        let metadata = span.metadata();

        let mut extensions = span.extensions_mut();
        let data = if let Some(data) = extensions.get_mut::<SpanData>() {
            data
        } else {
            // We got a span that was entered without being new'ed?
            return;
        };

        let etw_meta = get_event_metadata(&metadata.callsite());
        let (keyword, tag) = if let Some(meta) = etw_meta {
            (meta.kw, meta.event_tag)
        } else {
            (self.layer.default_keyword, 0)
        };

        self.layer.provider.as_ref().span_stop(
            &span,
            (data.start_time, stop_timestamp),
            &data.activity_id,
            &data.related_activity_id,
            &data.fields,
            metadata.level(),
            keyword,
            tag,
        );
    }

    fn on_close(&self, _id: span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // A span was closed
        // Good for knowing when to log a summary event?
    }

    fn on_record(
        &self,
        id: &span::Id,
        values: &span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // Values were added to the given span

        let span = if let Some(span) = ctx.span(id) {
            span
        } else {
            return;
        };

        let mut extensions = span.extensions_mut();
        let data = if let Some(data) = extensions.get_mut::<SpanData>() {
            data
        } else {
            // We got a span that was entered without being new'ed?
            return;
        };

        values.record(&mut ValueVisitor {
            fields: &mut data.fields,
        });
    }
}

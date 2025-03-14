use std::{
    hash::{BuildHasher, Hash, Hasher}, sync::RwLock, time::SystemTime
};

use tracing::Subscriber;
#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing_core::{callsite, span};
use tracing_subscriber::{registry::LookupSpan, Layer};
use hashbrown::HashMap;
use hashers::fnv::FNV1aHasher64;

use crate::{
    native::EventWriter,
    statics::*,
    values::{span_values::*, *},
};

use super::*;

static SPAN_DATA: RwLock<HashMap<tracing::span::Id, SpanData, FNV1aHasher64HasherBuilder>> =
    RwLock::new(HashMap::with_hasher(FNV1aHasher64HasherBuilder::new()));

#[derive(Default)]
struct FNV1aHasher64HasherBuilder {}
impl FNV1aHasher64HasherBuilder {
    const fn new() -> Self {
        Self {}
    }
}

impl BuildHasher for FNV1aHasher64HasherBuilder {
    type Hasher = FNV1aHasher64;
    fn build_hasher(&self) -> Self::Hasher {
        FNV1aHasher64::default()
    }

    fn hash_one<T: Hash>(&self, x: T) -> u64
    where
        Self: Sized,
        Self::Hasher: Hasher,
    {
        let mut hasher = self.build_hasher();
        x.hash(&mut hasher);
        hasher.finish()
    }
}

struct SpanData {
    fields: Box<[FieldValueIndex]>,
    activity_id: [u8; 16], // // if set, byte 0 is 1 and 64-bit span ID in the lower 8 bytes
    related_activity_id: [u8; 16], // if set, byte 0 is 1 and 64-bit span ID in the lower 8 bytes
    start_time: SystemTime,
}

impl<S, OutMode: OutputMode + 'static> Layer<S> for EtwLayer<S, OutMode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    crate::native::Provider<OutMode>: EventWriter<OutMode>,
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

        if crate::native::Provider::<OutMode>::supports_enable_callback() {
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

        attrs.values().record(&mut SpanValueVisitor {
            fields: &mut data.fields,
        });

        // The tracing_subscriber::Registry guarantees that there will only ever be 1 span with a given ID
        // active at any time, but other implementations may not provide the same guarantees.
        // The Subscriber trait allows for this, and says any spans with the same ID can be considered
        // as having identical metadata and attributes (even if they are not actually identical).
        // We can thus just overwrite any potentially existing spans with this ID.
        SPAN_DATA.write().unwrap().insert(span.id(), data);
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

        let mut span_data_guard = SPAN_DATA.write().unwrap();
        let data = if let Some(data) = span_data_guard.get_mut(&span.id()) {
            data
        } else {
            debug_assert!(false, "Enter of unrecognized span");
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

        // TODO:
        //   - In order to mutate this, we currently have to lock the entire hashmap every time a span is entered.
        //     This is not great for performance.
        //   - A span can be entered multiple times in a row without being exited. Storing the start time like this
        //     is insufficient for associating a start and stop event.
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

        let span_data_guard = SPAN_DATA.read().unwrap();
        let data = if let Some(data) = span_data_guard.get(&span.id()) {
            data
        } else {
            debug_assert!(false, "Exit of unrecognized span");
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

    fn on_close(&self, id: span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut span_data_guard = SPAN_DATA.write().unwrap();
        if let None = span_data_guard.remove(&id) {
            debug_assert!(false, "Close of unrecognized span");
        }
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

        let mut span_data_guard = SPAN_DATA.write().unwrap();
        let data = if let Some(data) = span_data_guard.get_mut(&span.id()) {
            data
        } else {
            debug_assert!(false, "Event on unrecognized span");
            return;
        };

        values.record(&mut SpanValueVisitor {
            fields: &mut data.fields,
        });
    }
}

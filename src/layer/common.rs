use core::{
    num::NonZeroU64,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
};
extern crate alloc;
use alloc::{boxed::Box, vec::Vec};
use std::sync::RwLock;

use hashbrown::HashMap;

use crate::{
    native::{EventWriter, OutputMode},
    statics::GLOBAL_ACTIVITY_SEED,
    values::{
        ValueTypes,
        span_values::{FieldValueIndex, SpanValueVisitor},
    },
};

// DOS resistance is not needed in this hash
static SPAN_DATA: RwLock<HashMap<tracing::span::Id, SpanData, ahash::RandomState>> =
    RwLock::new(HashMap::with_hasher(ahash::RandomState::with_seeds(
        17_616_942_133_695_121_499,
        9_565_839_503_509_016_163,
        2_756_528_679_765_226_774,
        228_784_672_216_536_063)));

pub(crate) struct SpanIds {
    pub(crate) span_id: NonZeroU64,
    pub(crate) parent_span_id: Option<NonZeroU64>, // sizeof(Option<NonZeroU64>) == sizeof(u64) is guaranteed by the standard
}

pub(crate) struct SpanStrings {
    pub(crate) trace_id: [u8; 32], // UTF-8 hex bytes, only non-zero if the otel feature is enabled and the span has a valid trace ID
    pub(crate) span_id: [u8; 16], // UTF-8 hex bytes, either the otel span ID if the otel feature is enabled, or the local span ID
    pub(crate) parent_span_id: Option<[u8; 16]>, // UTF-8 hex bytes, only Some if the otel feature is enabled and the span has a valid parent span ID
}

impl SpanStrings {
    pub(crate) fn build(span_data: &SpanData, otel_span_strings: Option<OtelSpanStrings>) -> SpanStrings
    {
        otel_span_strings.unwrap_or(
        SpanStrings {
            trace_id: [0; 32],
            span_id: crate::native::to_hex_utf8_bytes(span_data.ids.span_id.into()),
            parent_span_id: span_data.ids.parent_span_id.map(|pid| crate::native::to_hex_utf8_bytes(pid.into())),
        })
    }

    pub(crate) fn trace_id(&self) -> &[u8; 32] {
        &self.trace_id
    }

    pub(crate) fn span_id(&self) -> &[u8; 16] {
        &self.span_id
    }

    pub(crate) fn parent_span_id(&self) -> Option<&[u8; 16]> {
        self.parent_span_id.as_ref()
    }
}

// Add an alias for "incomplete" strings, just for clarity.
// This also makes it easier to completely exclude otel.rs when the feature is not enabled, without adding conditionals everywhere.
// These strings come from Otel (if available), but do not yet have the missing values filled in from the SpanData.
pub(crate) type OtelSpanStrings = SpanStrings;

// Data created by this crate for a span.
// Exists for the lifetime of the span.
struct SpanData {
    fields: Box<[FieldValueIndex]>,
    ids: SpanIds,
    activity_id: [u8; 16], // if set, byte 0 is 1 and 64-bit span ID in the lower 8 bytes
    related_activity_id: [u8; 16], // if set, byte 0 is 1 and 64-bit parent span ID in the lower 8 bytes
    start_time: std::time::SystemTime,
    name: &'static str,
    level: tracing_core::Level,
    ref_count: AtomicUsize,
}

// Data crated by tracing_core for a span, plus the crate data.
// Exists for the duration of the enter/exit call; short-lived.
#[doc(hidden)]
pub struct SpanRef<'a> {
    data: &'a SpanData,
    span_strings: SpanStrings,
}

impl SpanRef<'_> {
    pub(crate) fn id(&self) -> u64 {
        self.data.ids.span_id.into()
    }

    pub(crate) fn parent(&self) -> Option<u64> {
        self.data.ids.parent_span_id.map(|id| id.into())
    }

    pub(crate) fn name(&self) -> &'static str {
        self.data.name
    }

    pub(crate) fn level(&self) -> tracing_core::Level {
        self.data.level
    }

    pub(crate) fn timestamp(&self) -> std::time::SystemTime {
        self.data.start_time
    }

    // LE bytes rather than a GUID so we don't need a dependency on a GUID type
    // if set, byte 0 is 1 and 64-bit span ID in the lower 8 bytes
    pub(crate) fn activity_id(&self) -> &[u8; 16] {
        &self.data.activity_id
    }

    // LE bytes rather than a GUID so we don't need a dependency on a GUID type
    // if set, byte 0 is 1 and 64-bit parent span ID in the lower 8 bytes
    pub(crate) fn related_activity_id(&self) -> &[u8; 16] {
        &self.data.related_activity_id
    }

    pub(crate) fn fields(&self) -> impl Iterator<Item = &FieldValueIndex> {
        self.data.fields.iter()
    }

    pub(crate) fn field_count(&self) -> usize {
        self.data.fields.len()
    }

    pub(crate) fn span_strings(&self) -> &SpanStrings {
        &self.span_strings
    }
}

pub(crate) fn create_span_data_for_new_span(
    attrs: &tracing::span::Attributes<'_>,
    id: &tracing::span::Id,
) {
    let metadata = attrs.metadata();

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
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
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
            start_time: std::time::SystemTime::UNIX_EPOCH,
            name: metadata.name(),
            ids: SpanIds {
                span_id: id.into_non_zero_u64(),
                parent_span_id: NonZeroU64::new(parent_span_id),
            },
            level: *metadata.level(),
            ref_count: AtomicUsize::new(1),
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
    SPAN_DATA.write().unwrap().insert(id.clone(), data);
}

#[allow(unused)]
pub(crate) fn addref_span(id: &tracing::span::Id) {
    let span_data_guard = SPAN_DATA.read().unwrap();
    let spandata = span_data_guard.get(id);
    if let Some(span) = spandata {
        span.ref_count.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn release_span(id: &tracing::span::Id) -> bool {
    let mut current_refcount = {
        // Check the refcount while allowing others to also interact with thte map
        let span_data_guard = SPAN_DATA.read().unwrap();
        let spandata = span_data_guard.get(id);
        if let Some(span) = spandata {
            span.ref_count.fetch_sub(1, Ordering::Relaxed)
        } else {
            debug_assert!(false, "Close of unrecognized span");
            1 // Magic up a refcount so we skip the next part in release builds
        }
    };

    if current_refcount == 0 {
        // Lock the map and check the refcount again now that nobody else can modify it
        let mut span_data_guard = SPAN_DATA.write().unwrap();
        let spandata = span_data_guard.get(id);
        if let Some(span) = spandata {
            current_refcount = span.ref_count.load(Ordering::Relaxed);
            if current_refcount == 0 {
                let _ = span_data_guard.remove(id);
            }
        }
    }

    current_refcount != 0
}

pub(crate) fn enter_span<OutMode: OutputMode>(
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let timestamp = std::time::SystemTime::now();

    let mut span_data_guard = SPAN_DATA.write().unwrap();
    let data = if let Some(data) = span_data_guard.get_mut(id) {
        data
    } else {
        debug_assert!(false, "Enter of unrecognized span");
        return;
    };

    // TODO:
    //   - In order to mutate this, we currently have to lock the entire hashmap every time a span is entered.
    //     This is not great for performance.
    //   - A span can be entered multiple times in a row without being exited. Storing the start time like this
    //     is insufficient for associating a start and stop event.
    data.start_time = timestamp;

    writer.span_start(
        SpanRef {
            data,
            span_strings: SpanStrings::build(data, otel_span_strings),
        },
        keyword,
        tag,
    );
}

pub(crate) fn exit_span<OutMode: OutputMode>(
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let stop_timestamp = std::time::SystemTime::now();

    let span_data_guard = SPAN_DATA.read().unwrap();
    let data = if let Some(data) = span_data_guard.get(id) {
        data
    } else {
        debug_assert!(false, "Exit of unrecognized span");
        return;
    };

    writer.span_stop(
        (data.start_time, stop_timestamp),
        SpanRef {
            data,
            span_strings: SpanStrings::build(data, otel_span_strings),
        },
        keyword,
        tag,
    );
}

pub(crate) fn update_span_values(id: &tracing::span::Id, values: &tracing::span::Record<'_>) {
    let mut span_data_guard = SPAN_DATA.write().unwrap();
    let data = if let Some(data) = span_data_guard.get_mut(id) {
        data
    } else {
        debug_assert!(false, "Event on unrecognized span");
        return;
    };

    values.record(&mut SpanValueVisitor {
        fields: &mut data.fields,
    });
}

pub(crate) fn write_event<OutMode: OutputMode>(
    writer: Pin<&impl EventWriter<OutMode>>,
    event: &tracing::Event<'_>,
    name: &str,
    keyword: u64,
    tag: u32,
    _otel_span_strings: Option<OtelSpanStrings>,
) {
    let timestamp = std::time::SystemTime::now();

    let current_span = ctx
        .event_span(event)
        .map(|evt| evt.id())
        .map_or(0, |id| (id.into_u64()));
    let parent_span = ctx
        .event_span(event)
        .map_or(0, |evt| evt.parent().map_or(0, |p| p.id().into_u64()));

    writer.write_record(
        timestamp,
        name,
        event.metadata().level(),
        keyword,
        tag,
        event,
        None,
    );
}

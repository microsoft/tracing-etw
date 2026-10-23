use core::{num::NonZeroU64, pin::Pin};
extern crate alloc;
use alloc::{boxed::Box, vec::Vec};

use hashbrown::{HashMap, hash_map::RawEntryMut};

use crate::{
    native::{EventWriter, OutputMode, time::Timestamp},
    statics::GLOBAL_ACTIVITY_SEED,
    sync::RwLock,
    values::{
        ValueTypes,
        span_values::{FieldValueIndex, SpanValueVisitor, update_span_value},
    },
};

type SpanKey = (u64, tracing::span::Id);
type SpanMap = HashMap<SpanKey, SpanData, ahash::RandomState>;
type SpanEntry<'a> = RawEntryMut<'a, SpanKey, SpanData, ahash::RandomState>;

const SPAN_SHARD_COUNT: usize = 32;
// DOS resistance is not needed in this hash.
const SPAN_HASHER: ahash::RandomState = ahash::RandomState::with_seeds(
    17_616_942_133_695_121_499,
    9_565_839_503_509_016_163,
    2_756_528_679_765_226_774,
    228_784_672_216_536_063,
);

#[repr(align(64))]
struct SpanShard {
    spans: RwLock<SpanMap>,
}

static SPAN_DATA: [SpanShard; SPAN_SHARD_COUNT] = [const {
    SpanShard {
        spans: RwLock::new(HashMap::with_hasher(SPAN_HASHER)),
    }
}; SPAN_SHARD_COUNT];

#[derive(Clone, Copy)]
struct SpanLookup<'a> {
    key: &'a SpanKey,
    hash: u64,
}

impl<'a> SpanLookup<'a> {
    #[inline]
    fn new(key: &'a SpanKey) -> Self {
        Self {
            key,
            // Match hashbrown's BuildHasher path, including when rehashing.
            hash: core::hash::BuildHasher::hash_one(&SPAN_HASHER, key),
        }
    }

    #[inline]
    fn shard_index(self) -> usize {
        // Keep the low bucket bits and high control-byte bits independent.
        (self.hash >> 32) as usize & (SPAN_SHARD_COUNT - 1)
    }

    #[inline]
    fn shard(self) -> &'static RwLock<SpanMap> {
        &SPAN_DATA[self.shard_index()].spans
    }

    #[inline]
    fn get(self, spans: &SpanMap) -> Option<&SpanData> {
        spans
            .raw_entry()
            .from_key_hashed_nocheck(self.hash, self.key)
            .map(|(_, data)| data)
    }

    #[inline]
    fn entry(self, spans: &mut SpanMap) -> SpanEntry<'_> {
        spans
            .raw_entry_mut()
            .from_key_hashed_nocheck(self.hash, self.key)
    }

    #[inline]
    fn get_mut(self, spans: &mut SpanMap) -> Option<&mut SpanData> {
        match self.entry(spans) {
            RawEntryMut::Occupied(entry) => Some(entry.into_mut()),
            RawEntryMut::Vacant(_) => None,
        }
    }
}

#[cfg(test)]
fn span_shard_index(key: &SpanKey) -> usize {
    SpanLookup::new(key).shard_index()
}

#[cfg(test)]
fn span_shard(key: &SpanKey) -> &'static RwLock<SpanMap> {
    SpanLookup::new(key).shard()
}

#[derive(Clone, Copy)]
pub(crate) struct SpanIds {
    pub(crate) span_id: NonZeroU64,
    pub(crate) parent_span_id: Option<NonZeroU64>, // sizeof(Option<NonZeroU64>) == sizeof(u64) is guaranteed by the standard
}

#[derive(Clone, Copy)]
pub(crate) struct SpanContext {
    pub(crate) ids: SpanIds,
    pub(crate) metadata: &'static tracing_core::Metadata<'static>,
}

impl SpanContext {
    pub(crate) fn id(self) -> tracing::span::Id {
        tracing::span::Id::from_u64(self.ids.span_id.get())
    }
}

pub(crate) enum EventSpan<'a> {
    Id(&'a tracing::span::Id),
    Cached(SpanIds),
}

#[doc(hidden)]
pub struct SpanStrings {
    pub(crate) trace_id: [u8; 32], // UTF-8 hex bytes, only non-zero if the otel feature is enabled and the span has a valid trace ID
    pub(crate) span_id: [u8; 16], // UTF-8 hex bytes, either the otel span ID if the otel feature is enabled, or the local span ID
    pub(crate) parent_span_id: Option<[u8; 16]>, // UTF-8 hex bytes, only Some if the otel feature is enabled and the span has a valid parent span ID
}

impl SpanStrings {
    fn build(ids: SpanIds, otel_span_strings: Option<OtelSpanStrings>) -> SpanStrings {
        otel_span_strings.unwrap_or_else(|| SpanStrings {
            trace_id: [0; 32],
            span_id: crate::native::to_hex_utf8_bytes(ids.span_id.into()),
            parent_span_id: ids
                .parent_span_id
                .map(|pid| crate::native::to_hex_utf8_bytes(pid.into())),
        })
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
    start_time: Timestamp,
    metadata: &'static tracing_core::Metadata<'static>,
    ref_count: usize,
    retained_parent: Option<tracing::span::Id>,
}

// Data crated by tracing_core for a span, plus the crate data.
// Exists for the duration of the enter/exit call; short-lived.
#[doc(hidden)]
pub struct SpanRef<'a> {
    data: &'a SpanData,
    span_strings: SpanStrings,
}

impl SpanRef<'_> {
    pub(crate) fn name(&self) -> &'static str {
        self.data.metadata.name()
    }

    pub(crate) fn level(&self) -> tracing_core::Level {
        *self.data.metadata.level()
    }

    pub(crate) fn timestamp(&self) -> crate::native::time::CivilTime {
        self.data.start_time.into()
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
    subscriber_id: u64,
    attrs: &tracing::span::Attributes<'_>,
    id: &tracing::span::Id,
    parent: Option<&tracing::span::Id>,
) {
    let metadata = attrs.metadata();

    let parent_span_id = parent.map_or(0, |id| id.into_u64());

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

        let activity_seed = *GLOBAL_ACTIVITY_SEED;
        SpanData {
            fields: v.into_boxed_slice(),
            activity_id: activity_seed,
            related_activity_id: activity_seed,
            start_time: Timestamp::UNIX_EPOCH,
            metadata,
            ids: SpanIds {
                span_id: id.into_non_zero_u64(),
                parent_span_id: NonZeroU64::new(parent_span_id),
            },
            ref_count: 1,
            retained_parent: None,
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
        record: |name, mut value| update_span_value(&mut data.fields, name, &mut value),
    });

    let key = (subscriber_id, id.clone());
    let lookup = SpanLookup::new(&key);
    let index = lookup.shard_index();
    let parent_key = parent.map(|parent| (subscriber_id, parent.clone()));
    let parent = parent_key.as_ref().map(SpanLookup::new);
    let parent_index = parent.map(SpanLookup::shard_index);
    // When two shards are needed, always acquire the lower index first.
    let unused = match parent_index {
        Some(parent_index) if parent_index < index => {
            let mut parents = SPAN_DATA[parent_index].spans.write();
            let mut spans = SPAN_DATA[index].spans.write();
            insert_span_data(&mut spans, Some(&mut parents), lookup, data, parent)
        }
        Some(parent_index) if parent_index > index => {
            let mut spans = SPAN_DATA[index].spans.write();
            let mut parents = SPAN_DATA[parent_index].spans.write();
            insert_span_data(&mut spans, Some(&mut parents), lookup, data, parent)
        }
        _ => {
            let mut spans = SPAN_DATA[index].spans.write();
            insert_span_data(&mut spans, None, lookup, data, parent)
        }
    };
    // Duplicate layers' unused payloads must be dropped outside both locks.
    drop(unused);
}

fn insert_span_data(
    spans: &mut SpanMap,
    parents: Option<&mut SpanMap>,
    lookup: SpanLookup<'_>,
    mut data: SpanData,
    parent: Option<SpanLookup<'_>>,
) -> Option<SpanData> {
    if let Some(existing) = lookup.get_mut(spans) {
        // Multiple exporter layers can observe the same registry span.
        existing.ref_count = existing
            .ref_count
            .checked_add(1)
            .expect("Span reference count overflow");
        return Some(data);
    }
    if let Some(parent) = parent {
        if let Some(parent_data) = parent.get_mut(parents.unwrap_or(&mut *spans)) {
            parent_data.ref_count = parent_data
                .ref_count
                .checked_add(1)
                .expect("Span reference count overflow");
            data.retained_parent = Some(parent.key.1.clone());
        }
    }
    match lookup.entry(spans) {
        RawEntryMut::Vacant(entry) => {
            entry.insert_hashed_nocheck(lookup.hash, lookup.key.clone(), data);
            None
        }
        RawEntryMut::Occupied(_) => {
            unreachable!("Span became occupied while its shard was locked")
        }
    }
}

pub(crate) fn addref_span(subscriber_id: u64, id: &tracing::span::Id) -> SpanContext {
    let key = (subscriber_id, id.clone());
    let lookup = SpanLookup::new(&key);
    let mut spans = lookup.shard().write();
    let span = lookup
        .get_mut(&mut spans)
        .expect("Clone or enter of an unrecognized span");
    retain_span_data(span)
}

fn retain_span_data(span: &mut SpanData) -> SpanContext {
    span.ref_count = span
        .ref_count
        .checked_add(1)
        .expect("Span reference count overflow");
    SpanContext {
        ids: span.ids,
        metadata: span.metadata,
    }
}

fn release_span_entry(
    mut entry: hashbrown::hash_map::RawOccupiedEntryMut<'_, SpanKey, SpanData, ahash::RandomState>,
) -> Option<SpanData> {
    let span = entry.get_mut();
    span.ref_count = span
        .ref_count
        .checked_sub(1)
        .expect("Span reference count underflow");
    (span.ref_count == 0).then(|| entry.remove())
}

pub(crate) fn release_span(subscriber_id: u64, id: &tracing::span::Id) -> bool {
    let mut current = (subscriber_id, id.clone());
    loop {
        let lookup = SpanLookup::new(&current);
        let mut removed = {
            let mut spans = lookup.shard().write();
            let entry = match lookup.entry(&mut spans) {
                RawEntryMut::Occupied(entry) => entry,
                RawEntryMut::Vacant(_) => panic!("Close of an unrecognized span"),
            };
            if let Some(removed) = release_span_entry(entry) {
                removed
            } else {
                return current.1 != *id;
            }
        };
        let parent = removed.retained_parent.take();
        drop(removed);
        // The child's retained reference keeps its parent alive between locks.
        match parent {
            Some(parent) => current.1 = parent,
            None => return true,
        }
    }
}

#[cfg(feature = "std")]
pub(crate) fn enter_span<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    enter_span_impl(
        subscriber_id,
        id,
        writer,
        keyword,
        tag,
        otel_span_strings,
        None::<fn(SpanContext)>,
    );
}

pub(crate) fn enter_span_retained<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
    on_enter: impl FnOnce(SpanContext),
) {
    enter_span_impl(
        subscriber_id,
        id,
        writer,
        keyword,
        tag,
        otel_span_strings,
        Some(on_enter),
    );
}

fn enter_span_impl<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
    on_enter: Option<impl FnOnce(SpanContext)>,
) {
    let timestamp = Timestamp::now();

    let key = (subscriber_id, id.clone());
    let lookup = SpanLookup::new(&key);
    let mut span_data_guard = lookup.shard().write();
    let data = if let Some(data) = lookup.get_mut(&mut span_data_guard) {
        data
    } else {
        if on_enter.is_some() {
            panic!("Clone or enter of an unrecognized span");
        }
        debug_assert!(false, "Enter of unrecognized span");
        return;
    };

    if let Some(on_enter) = on_enter {
        // The caller transfers this reference into pre-reserved TLS storage
        // without allocating or panicking, and releases its TLS borrow.
        on_enter(retain_span_data(data));
    }

    // TODO:
    //   - In order to mutate this, we currently have to lock the span's shard every time a span is entered.
    //     This is not great for performance.
    //   - A span can be entered multiple times in a row without being exited. Storing the start time like this
    //     is insufficient for associating a start and stop event.
    data.start_time = timestamp;

    writer.span_start(
        SpanRef {
            data,
            span_strings: SpanStrings::build(data.ids, otel_span_strings),
        },
        keyword,
        tag,
    );
}

#[cfg(any(feature = "std", not(any(windows, target_os = "linux"))))]
pub(crate) fn exit_span<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let stop_timestamp = Timestamp::now();

    let key = (subscriber_id, id.clone());
    let lookup = SpanLookup::new(&key);
    let span_data_guard = lookup.shard().read();
    let data = if let Some(data) = lookup.get(&span_data_guard) {
        data
    } else {
        debug_assert!(false, "Exit of unrecognized span");
        return;
    };

    write_span_stop(
        data,
        stop_timestamp,
        writer,
        keyword,
        tag,
        otel_span_strings,
    );
}

fn write_span_stop<OutMode: OutputMode>(
    data: &SpanData,
    stop_timestamp: Timestamp,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    writer.span_stop(
        (data.start_time.into(), stop_timestamp.into()),
        SpanRef {
            data,
            span_strings: SpanStrings::build(data.ids, otel_span_strings),
        },
        keyword,
        tag,
    );
}

#[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
pub(crate) fn exit_span_releasing<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
    retained: &mut Option<SpanContext>,
) {
    let stop_timestamp = Timestamp::now();
    let key = (subscriber_id, id.clone());
    let lookup = SpanLookup::new(&key);
    let removed = {
        let mut spans = lookup.shard().write();
        let entry = match lookup.entry(&mut spans) {
            RawEntryMut::Occupied(entry) => entry,
            RawEntryMut::Vacant(_) => panic!("Exit of unrecognized span"),
        };
        let data = entry.get();
        write_span_stop(
            data,
            stop_timestamp,
            writer,
            keyword,
            tag,
            otel_span_strings,
        );
        let removed = release_span_entry(entry);
        *retained = None;
        removed
    };
    if let Some(mut removed) = removed {
        let parent = removed.retained_parent.take();
        drop(removed);
        if let Some(parent) = parent {
            release_span(subscriber_id, &parent);
        }
    }
}

pub(crate) fn update_span_values(
    subscriber_id: u64,
    id: &tracing::span::Id,
    values: &tracing::span::Record<'_>,
) {
    // Most records update one field; keep that update inline without allocating.
    let mut first = None;
    let mut remaining = Vec::new();
    values.record(&mut SpanValueVisitor {
        record: |name, value| {
            if first.is_none() {
                first = Some((name, value));
            } else {
                remaining.push((name, value));
            }
        },
    });
    {
        let key = (subscriber_id, id.clone());
        let lookup = SpanLookup::new(&key);
        let mut span_data_guard = lookup.shard().write();
        let data = if let Some(data) = lookup.get_mut(&mut span_data_guard) {
            data
        } else {
            debug_assert!(false, "Event on unrecognized span");
            return;
        };
        for (name, value) in first.iter_mut().chain(remaining.iter_mut()) {
            update_span_value(&mut data.fields, name, value);
        }
    }
    // Updates now own the displaced values, which are dropped without the lock.
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_event<OutMode: OutputMode>(
    subscriber_id: u64,
    writer: Pin<&impl EventWriter<OutMode>>,
    event: &tracing::Event<'_>,
    name: &str,
    keyword: u64,
    tag: u32,
    span: Option<EventSpan<'_>>,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let timestamp = Timestamp::now();

    let (current_span, ids) = match span {
        None => (0, None),
        Some(EventSpan::Cached(ids)) => (ids.span_id.get(), Some(ids)),
        Some(EventSpan::Id(id)) => {
            let key = (subscriber_id, id.clone());
            let lookup = SpanLookup::new(&key);
            let ids = lookup.get(&lookup.shard().read()).map(|data| data.ids);
            (id.into_u64(), ids)
        }
    };
    let parent_span = ids
        .and_then(|ids| ids.parent_span_id)
        .map_or(0, |id| id.get());
    let span_strings = match ids {
        Some(ids) => Some(SpanStrings::build(ids, otel_span_strings)),
        None => otel_span_strings,
    };

    writer.write_record(
        timestamp.into(),
        current_span,
        parent_span,
        name,
        event.metadata().level(),
        keyword,
        tag,
        event,
        span_strings,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayerBuilder, layer::_EtwTracingSubscriber, native::NormalOutput};
    use std::{sync::mpsc, time::Duration};
    use tracing::{
        Metadata, Subscriber,
        callsite::{Callsite, DefaultCallsite},
        field::FieldSet,
        metadata::Kind,
        span::{Attributes, Id},
    };

    // Lock-holding probes must not create artificial cycles with other probes.
    static LOCK_PROBES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    static CALLSITE: DefaultCallsite = DefaultCallsite::new(&META);
    static META: Metadata<'static> = Metadata::new(
        "standalone span",
        module_path!(),
        tracing::Level::INFO,
        Some(file!()),
        Some(line!()),
        Some(module_path!()),
        FieldSet::new(&[], tracing_core::identify_callsite!(&CALLSITE)),
        Kind::SPAN,
    );

    enum Parent<'a> {
        Root,
        Current,
        Explicit(&'a Id),
    }

    fn new_subscriber() -> _EtwTracingSubscriber<NormalOutput> {
        let builder = LayerBuilder::new("StandaloneSpanTests");
        #[cfg(feature = "std")]
        let subscriber = builder.build_subscriber();
        #[cfg(not(feature = "std"))]
        let subscriber = builder.build();
        subscriber.unwrap()
    }

    fn new_span(subscriber: &impl Subscriber, parent: Parent<'_>) -> Id {
        let values = META.fields().value_set(&[]);
        let attrs = match parent {
            Parent::Root => Attributes::new_root(&META, &values),
            Parent::Current => Attributes::new(&META, &values),
            Parent::Explicit(parent) => Attributes::child_of(parent.clone(), &META, &values),
        };
        subscriber.new_span(&attrs)
    }

    fn contains(subscriber_id: u64, id: &Id) -> bool {
        let key = (subscriber_id, id.clone());
        span_shard(&key).read().contains_key(&key)
    }

    #[test]
    fn shard_growth_preserves_entries_and_bucket_distribution() {
        let subscriber = new_subscriber();
        let subscriber_id = subscriber.subscriber_id;
        let ids: Vec<_> = (1..)
            .map(Id::from_u64)
            .filter(|id| span_shard_index(&(subscriber_id, id.clone())) == SPAN_SHARD_COUNT / 2)
            .take(256)
            .collect();
        let mut buckets = 0u8;
        for id in &ids {
            let key = (subscriber_id, id.clone());
            buckets |= 1 << (SpanLookup::new(&key).hash & 7);
            let values = META.fields().value_set(&[]);
            create_span_data_for_new_span(
                subscriber_id,
                &Attributes::new_root(&META, &values),
                id,
                None,
            );
        }
        // Selecting a shard must not collapse its entries into one map bucket.
        assert!(buckets.count_ones() >= 6);
        for id in &ids {
            // Ordinary map lookup also verifies the precomputed hash after growth.
            assert!(contains(subscriber_id, id));
            assert_eq!(addref_span(subscriber_id, id).id(), *id);
            assert!(!subscriber.try_close(id.clone()));
        }
        for id in ids {
            assert!(subscriber.try_close(id.clone()));
            assert!(!contains(subscriber_id, &id));
        }
    }

    #[test]
    fn full_key_shards_allow_independent_span_operations() {
        let _probe = LOCK_PROBES.lock().unwrap();
        let subscriber = new_subscriber();
        let id = Id::from_u64(1);
        let blocked_key = (subscriber.subscriber_id, id.clone());
        let blocked_index = span_shard_index(&blocked_key);
        let other_id = (2..1024)
            .map(Id::from_u64)
            .find(|id| span_shard_index(&(subscriber.subscriber_id, id.clone())) != blocked_index)
            .unwrap();
        let other_subscriber = (0..128)
            .map(|_| new_subscriber())
            .find(|other| span_shard_index(&(other.subscriber_id, id.clone())) != blocked_index)
            .unwrap();
        let values = META.fields().value_set(&[]);
        create_span_data_for_new_span(
            subscriber.subscriber_id,
            &Attributes::new_root(&META, &values),
            &id,
            None,
        );
        for (other, other_id) in [(&subscriber, other_id), (&other_subscriber, id.clone())] {
            let (sender, receiver) = mpsc::channel();
            let guard = span_shard(&blocked_key).write();
            std::thread::scope(|scope| {
                let worker = scope.spawn(move || {
                    let values = META.fields().value_set(&[]);
                    create_span_data_for_new_span(
                        other.subscriber_id,
                        &Attributes::new_root(&META, &values),
                        &other_id,
                        None,
                    );
                    let cloned = other.clone_span(&other_id);
                    other.record(&other_id, &tracing::span::Record::new(&values));
                    other.enter(&other_id);
                    assert_eq!(other.current_span().id(), Some(&other_id));
                    other.event(&tracing::Event::new_child_of(
                        Some(other_id.clone()),
                        &META,
                        &values,
                    ));
                    other.exit(&other_id);
                    assert!(!other.try_close(cloned));
                    assert!(other.try_close(other_id));
                    sender.send(()).unwrap();
                });
                let result = receiver.recv_timeout(Duration::from_secs(5));
                drop(guard);
                worker.join().unwrap();
                result.unwrap();
            });
        }
        assert!(subscriber.try_close(id));
    }

    #[test]
    fn concurrent_duplicate_creation_retains_parents_once_across_shards() {
        use core::cmp::Ordering;

        let subscriber = new_subscriber();
        let subscriber_id = subscriber.subscriber_id;
        let parent_index = SPAN_SHARD_COUNT / 2;
        let parent = (1..1024)
            .map(Id::from_u64)
            .find(|id| span_shard_index(&(subscriber_id, id.clone())) == parent_index)
            .unwrap();
        let children = [Ordering::Less, Ordering::Equal, Ordering::Greater].map(|ordering| {
            (1..1024)
                .map(Id::from_u64)
                .find(|id| {
                    id != &parent
                        && span_shard_index(&(subscriber_id, id.clone())).cmp(&parent_index)
                            == ordering
                })
                .unwrap()
        });
        let values = META.fields().value_set(&[]);
        create_span_data_for_new_span(
            subscriber_id,
            &Attributes::new_root(&META, &values),
            &parent,
            None,
        );
        let barrier = std::sync::Barrier::new(7);
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for child in &children {
                for _ in 0..2 {
                    let subscriber = &subscriber;
                    let parent = &parent;
                    let barrier = &barrier;
                    handles.push(scope.spawn(move || {
                        let values = META.fields().value_set(&[]);
                        create_span_data_for_new_span(
                            subscriber_id,
                            &Attributes::child_of(parent.clone(), &META, &values),
                            child,
                            Some(parent),
                        );
                        barrier.wait();
                        barrier.wait();
                        subscriber.try_close(child.clone())
                    }));
                }
            }
            barrier.wait();
            let parent_key = (subscriber_id, parent.clone());
            let parent_refs = span_shard(&parent_key)
                .read()
                .get(&parent_key)
                .map(|data| data.ref_count);
            let child_refs = children.each_ref().map(|child| {
                let key = (subscriber_id, child.clone());
                span_shard(&key).read().get(&key).map(|data| data.ref_count)
            });
            let parent_closed = subscriber.try_close(parent.clone());
            barrier.wait();
            let closed = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .filter(|closed| *closed)
                .count();
            assert_eq!(parent_refs, Some(4));
            assert_eq!(child_refs, [Some(2); 3]);
            assert!(!parent_closed);
            assert_eq!(closed, 3);
        });
        assert!(!contains(subscriber_id, &parent));
        for child in &children {
            assert!(!contains(subscriber_id, child));
        }
    }

    #[test]
    fn cached_current_context_and_events_bypass_the_map() {
        let _probe = LOCK_PROBES.lock().unwrap();
        let subscriber = new_subscriber();
        let parent = new_span(&subscriber, Parent::Root);
        let child_metadata = tracing::callsite! {
            name: "cached child",
            kind: Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::DEBUG,
            fields:
        }
        .metadata();
        let child = subscriber.new_span(&Attributes::child_of(
            parent.clone(),
            child_metadata,
            &child_metadata.fields().value_set(&[]),
        ));
        assert!(!subscriber.try_close(parent.clone()));
        std::thread::scope(|scope| {
            let (ready_sender, ready) = mpsc::channel();
            let (start, start_receiver) = mpsc::channel();
            let (finished, result) = mpsc::channel();
            let subscriber = &subscriber;
            let child = &child;
            let reader = scope.spawn(move || {
                subscriber.enter(child);
                ready_sender.send(()).unwrap();
                start_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                let current = subscriber.current_span();
                let correct = current.id() == Some(child)
                    && core::ptr::eq(current.metadata().unwrap(), child_metadata);
                let values = META.fields().value_set(&[]);
                subscriber.event(&tracing::Event::new(&META, &values));
                finished.send(correct).unwrap();
                subscriber.exit(child);
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(!subscriber.try_close(child.clone()));
            let guard = span_shard(&(subscriber.subscriber_id, child.clone())).write();
            start.send(()).unwrap();
            let completed = result.recv_timeout(Duration::from_secs(5));
            drop(guard);
            reader.join().unwrap();
            assert!(completed.unwrap());
        });
        assert!(!contains(subscriber.subscriber_id, &child));
        assert!(!contains(subscriber.subscriber_id, &parent));
    }

    #[test]
    fn formats_recorded_values_outside_the_map_lock() {
        let _probe = LOCK_PROBES.lock().unwrap();
        struct FormattingProbe<'a> {
            start: mpsc::Sender<()>,
            completed: mpsc::Receiver<()>,
            unlocked: core::cell::Cell<bool>,
            subscriber: &'a _EtwTracingSubscriber<NormalOutput>,
            id: &'a Id,
        }
        impl core::fmt::Debug for FormattingProbe<'_> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                self.start.send(()).unwrap();
                self.unlocked
                    .set(self.completed.recv_timeout(Duration::from_secs(5)).is_ok());
                if self.unlocked.get() {
                    let values = META.fields().value_set(&[]);
                    self.subscriber.event(&tracing::Event::new_child_of(
                        Some(self.id.clone()),
                        &META,
                        &values,
                    ));
                }
                formatter.write_str("formatted")
            }
        }

        let subscriber = new_subscriber();
        let metadata = tracing::callsite! {
            name: "recorded fields",
            kind: Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields: value, count, text
        }
        .metadata();
        let id = subscriber.new_span(&Attributes::new_root(
            metadata,
            &tracing::valueset!(
                metadata.fields(),
                value = "before",
                count = 1u64,
                text = "old"
            ),
        ));
        let (start, start_receiver) = mpsc::channel();
        let (completed_sender, completed) = mpsc::channel();
        let probe = FormattingProbe {
            start,
            completed,
            unlocked: core::cell::Cell::new(false),
            subscriber: &subscriber,
            id: &id,
        };
        std::thread::scope(|scope| {
            let subscriber_id = subscriber.subscriber_id;
            let id = &id;
            let reader = scope.spawn(move || {
                start_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                let key = (subscriber_id, id.clone());
                let spans = span_shard(&key).read();
                assert!(spans.contains_key(&key));
                completed_sender.send(()).unwrap();
            });
            subscriber.record(
                id,
                &tracing::span::Record::new(&tracing::valueset!(
                    metadata.fields(),
                    value = ?probe,
                    count = 7u64,
                    text = "after"
                )),
            );
            reader.join().unwrap();
        });
        assert!(probe.unlocked.get());
        {
            let key = (subscriber.subscriber_id, id.clone());
            let spans = span_shard(&key).read();
            let fields = &spans[&key].fields;
            assert!(fields.iter().any(|field| {
                field.field == "value"
                    && matches!(&field.value, ValueTypes::v_str(value) if value == "formatted")
            }));
            assert!(fields.iter().any(|field| {
                field.field == "count" && matches!(&field.value, ValueTypes::v_u64(7))
            }));
            assert!(fields.iter().any(|field| {
                field.field == "text"
                    && matches!(&field.value, ValueTypes::v_str(value) if value == "after")
            }));
        }
        assert!(subscriber.try_close(id));
    }

    #[test]
    fn tracks_nested_reentrant_and_out_of_order_entries() {
        let subscriber = new_subscriber();
        let outer = new_span(&subscriber, Parent::Root);
        let inner = new_span(&subscriber, Parent::Root);
        assert!(subscriber.current_span().id().is_none());

        subscriber.enter(&outer);
        assert_eq!(subscriber.current_span().id(), Some(&outer));
        assert!(core::ptr::eq(
            subscriber.current_span().metadata().unwrap(),
            &META
        ));
        subscriber.enter(&inner);
        assert_eq!(subscriber.current_span().id(), Some(&inner));
        subscriber.enter(&outer);
        subscriber.exit(&inner);
        assert_eq!(subscriber.current_span().id(), Some(&outer));
        subscriber.exit(&outer);
        assert_eq!(subscriber.current_span().id(), Some(&outer));
        subscriber.exit(&outer);
        assert!(subscriber.current_span().id().is_none());
        assert!(subscriber.try_close(inner));
        assert!(subscriber.try_close(outer));
    }

    #[test]
    fn isolates_threads_and_subscribers_but_shares_subscriber_clones() {
        let subscriber = new_subscriber();
        let other = new_subscriber();
        let first = new_span(&subscriber, Parent::Root);
        let second = new_span(&other, Parent::Root);
        subscriber.enter(&first);
        other.enter(&second);
        assert_eq!(subscriber.clone().current_span().id(), Some(&first));
        assert_eq!(other.current_span().id(), Some(&second));

        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert!(subscriber.current_span().id().is_none());
                    subscriber.enter(&first);
                    assert_eq!(subscriber.current_span().id(), Some(&first));
                    subscriber.exit(&first);
                    assert!(subscriber.current_span().id().is_none());
                })
                .join()
                .unwrap();
        });
        assert_eq!(subscriber.current_span().id(), Some(&first));
        other.exit(&second);
        subscriber.exit(&first);
        assert!(other.try_close(second));
        assert!(subscriber.try_close(first));
    }

    #[test]
    fn retains_entered_spans_until_exit_and_removes_closed_spans() {
        let subscriber = new_subscriber();
        let id = new_span(&subscriber, Parent::Root);
        let cloned = subscriber.clone_span(&id);
        assert!(!subscriber.try_close(cloned));
        subscriber.enter(&id);
        assert!(!subscriber.try_close(id.clone()));
        assert_eq!(subscriber.current_span().id(), Some(&id));
        subscriber.exit(&id);
        assert!(subscriber.current_span().id().is_none());
        assert!(!contains(subscriber.subscriber_id, &id));
    }

    #[test]
    fn isolates_reused_span_ids_between_subscribers() {
        let first = new_subscriber();
        let second = new_subscriber();
        let id = Id::from_u64(1);
        let values = META.fields().value_set(&[]);
        let attrs = Attributes::new_root(&META, &values);
        for subscriber in [&first, &second] {
            create_span_data_for_new_span(subscriber.subscriber_id, &attrs, &id, None);
        }
        assert!(first.try_close(id.clone()));
        assert!(!contains(first.subscriber_id, &id));
        assert!(contains(second.subscriber_id, &id));
        second.enter(&id);
        assert_eq!(second.current_span().id(), Some(&id));
        second.exit(&id);
        assert!(second.try_close(id));
    }

    #[test]
    fn resolves_parents_and_keeps_them_alive_until_children_close() {
        let subscriber = new_subscriber();
        let parent = new_span(&subscriber, Parent::Root);
        subscriber.enter(&parent);
        let contextual = new_span(&subscriber, Parent::Current);
        let explicit = new_span(&subscriber, Parent::Explicit(&parent));
        let root = new_span(&subscriber, Parent::Root);
        {
            for id in [&contextual, &explicit] {
                let key = (subscriber.subscriber_id, id.clone());
                assert_eq!(
                    span_shard(&key).read()[&key].ids.parent_span_id,
                    Some(parent.into_non_zero_u64()),
                );
            }
            let key = (subscriber.subscriber_id, root.clone());
            assert_eq!(span_shard(&key).read()[&key].ids.parent_span_id, None);
        }
        subscriber.exit(&parent);
        assert!(!subscriber.try_close(parent.clone()));
        assert!(subscriber.try_close(contextual));
        assert!(contains(subscriber.subscriber_id, &parent));
        assert!(subscriber.try_close(explicit));
        assert!(!contains(subscriber.subscriber_id, &parent));
        assert!(subscriber.try_close(root));
    }

    #[test]
    fn releases_entered_references_when_a_thread_exits() {
        let subscriber = new_subscriber();
        for should_panic in [false, true] {
            let id = new_span(&subscriber, Parent::Root);
            std::thread::scope(|scope| {
                let result = scope
                    .spawn(|| {
                        subscriber.enter(&id);
                        assert!(!subscriber.try_close(id.clone()));
                        assert!(!should_panic, "Testing thread-exit cleanup after a panic");
                    })
                    .join();
                assert_eq!(result.is_err(), should_panic);
            });
            assert!(!contains(subscriber.subscriber_id, &id));
        }
    }

    #[test]
    fn closes_only_after_all_concurrent_references_are_released() {
        let subscriber = new_subscriber();
        let id = new_span(&subscriber, Parent::Root);
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..8 {
                let cloned = subscriber.clone_span(&id);
                let subscriber = &subscriber;
                let barrier = &barrier;
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    subscriber.try_close(cloned)
                }));
            }
            for handle in handles {
                assert!(!handle.join().unwrap());
            }
        });
        assert!(subscriber.try_close(id.clone()));
        assert!(!contains(subscriber.subscriber_id, &id));
    }

    #[cfg(feature = "std")]
    #[test]
    fn preserves_registry_parent_context_and_cleans_up_exporter_data() {
        use tracing_subscriber::prelude::*;

        let layer = LayerBuilder::new("RegistryLifetimeTests")
            .__build_for_test()
            .unwrap();
        let subscriber_id = layer.subscriber_id;
        let registry = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(registry, || {
            let parent = tracing::info_span!(parent: None, "registry parent");
            let parent_id = parent.id().unwrap();
            let entered = parent.enter();
            let child = tracing::info_span!("registry child");
            let child_id = child.id().unwrap();
            let key = (subscriber_id, child_id.clone());
            assert_eq!(
                span_shard(&key).read()[&key].ids.parent_span_id,
                Some(parent_id.into_non_zero_u64()),
            );
            drop(entered);
            drop(parent);
            assert!(contains(subscriber_id, &parent_id));
            drop(child);
            assert!(!contains(subscriber_id, &child_id));
            assert!(!contains(subscriber_id, &parent_id));
        });
    }

    #[derive(Default)]
    struct RecordingWriter(std::sync::Mutex<Option<(u64, u64, Option<SpanStrings>)>>);

    impl EventWriter<NormalOutput> for RecordingWriter {
        fn span_start(self: Pin<&Self>, _: SpanRef<'_>, _: u64, _: u32) {}

        fn span_stop(
            self: Pin<&Self>,
            _: (
                crate::native::time::CivilTime,
                crate::native::time::CivilTime,
            ),
            _: SpanRef<'_>,
            _: u64,
            _: u32,
        ) {
        }

        fn write_record(
            self: Pin<&Self>,
            _: crate::native::time::CivilTime,
            current_span: u64,
            parent_span: u64,
            _: &str,
            _: &tracing::Level,
            _: u64,
            _: u32,
            _: &tracing::Event<'_>,
            span_strings: Option<SpanStrings>,
        ) {
            *self.0.lock().unwrap() = Some((current_span, parent_span, span_strings));
        }
    }

    #[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
    #[test]
    fn fused_exit_releases_its_reference_and_cascades_to_the_parent() {
        for close_original in [false, true] {
            let subscriber = new_subscriber();
            let parent = new_span(&subscriber, Parent::Root);
            let child = new_span(&subscriber, Parent::Explicit(&parent));
            let writer = RecordingWriter::default();
            let mut retained = None;
            enter_span_retained(
                subscriber.subscriber_id,
                &child,
                Pin::new(&writer),
                1,
                0,
                None,
                |context| retained = Some(context),
            );
            assert!(!subscriber.try_close(parent.clone()));
            if close_original {
                assert!(!subscriber.try_close(child.clone()));
            }
            exit_span_releasing(
                subscriber.subscriber_id,
                &child,
                Pin::new(&writer),
                1,
                0,
                None,
                &mut retained,
            );
            assert!(retained.is_none());
            if !close_original {
                assert!(contains(subscriber.subscriber_id, &child));
                assert!(contains(subscriber.subscriber_id, &parent));
                assert!(subscriber.try_close(child.clone()));
            }
            assert!(!contains(subscriber.subscriber_id, &child));
            assert!(!contains(subscriber.subscriber_id, &parent));
        }
    }

    #[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
    #[test]
    fn failed_stop_leaves_the_entered_reference_owned_by_the_caller() {
        struct PanickingWriter;
        impl EventWriter<NormalOutput> for PanickingWriter {
            fn span_start(self: Pin<&Self>, _: SpanRef<'_>, _: u64, _: u32) {}
            fn span_stop(
                self: Pin<&Self>,
                _: (
                    crate::native::time::CivilTime,
                    crate::native::time::CivilTime,
                ),
                _: SpanRef<'_>,
                _: u64,
                _: u32,
            ) {
                panic!("Injected stop failure");
            }
            fn write_record(
                self: Pin<&Self>,
                _: crate::native::time::CivilTime,
                _: u64,
                _: u64,
                _: &str,
                _: &tracing::Level,
                _: u64,
                _: u32,
                _: &tracing::Event<'_>,
                _: Option<SpanStrings>,
            ) {
                unreachable!();
            }
        }
        let subscriber = new_subscriber();
        let id = new_span(&subscriber, Parent::Root);
        let mut retained = None;
        enter_span_retained(
            subscriber.subscriber_id,
            &id,
            Pin::new(&PanickingWriter),
            1,
            0,
            None,
            |context| retained = Some(context),
        );
        assert!(!subscriber.try_close(id.clone()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            exit_span_releasing(
                subscriber.subscriber_id,
                &id,
                Pin::new(&PanickingWriter),
                1,
                0,
                None,
                &mut retained,
            );
        }));
        assert!(result.is_err());
        let context = retained
            .take()
            .expect("Failed emission must not consume the reference");
        assert!(release_span(subscriber.subscriber_id, &context.id()));
        assert!(!contains(subscriber.subscriber_id, &id));
    }

    #[test]
    fn unparented_events_bypass_the_map_and_preserve_otel_context() {
        let _probe = LOCK_PROBES.lock().unwrap();
        let subscriber = new_subscriber();
        let writer = RecordingWriter::default();
        let (sender, receiver) = mpsc::channel();
        let guards: Vec<_> = SPAN_DATA.iter().map(|shard| shard.spans.write()).collect();
        std::thread::scope(|scope| {
            let writer = &writer;
            let reader = scope.spawn(move || {
                let values = META.fields().value_set(&[]);
                write_event(
                    subscriber.subscriber_id,
                    Pin::new(writer),
                    &tracing::Event::new(&META, &values),
                    "event",
                    1,
                    0,
                    None,
                    Some(SpanStrings {
                        trace_id: [b'a'; 32],
                        span_id: [b'b'; 16],
                        parent_span_id: Some([b'c'; 16]),
                    }),
                );
                sender.send(()).unwrap();
            });
            let result = receiver.recv_timeout(Duration::from_secs(5));
            drop(guards);
            reader.join().unwrap();
            result.unwrap();
        });
        let (current, related, strings) = writer.0.lock().unwrap().take().unwrap();
        assert_eq!((current, related), (0, 0));
        let strings = strings.unwrap();
        assert_eq!(strings.trace_id, [b'a'; 32]);
        assert_eq!(strings.span_id, [b'b'; 16]);
        assert_eq!(strings.parent_span_id, Some([b'c'; 16]));
    }

    #[test]
    fn forwards_local_and_otel_event_correlation() {
        let subscriber = new_subscriber();
        let parent = new_span(&subscriber, Parent::Root);
        let child = new_span(&subscriber, Parent::Explicit(&parent));
        let writer = RecordingWriter::default();
        let values = META.fields().value_set(&[]);
        let event = tracing::Event::new(&META, &values);
        write_event(
            subscriber.subscriber_id,
            Pin::new(&writer),
            &event,
            "event",
            1,
            0,
            Some(EventSpan::Id(&child)),
            None,
        );
        let (current, related, strings) = writer.0.lock().unwrap().take().unwrap();
        assert_eq!(current, child.into_u64());
        assert_eq!(related, parent.into_u64());
        let strings = strings.unwrap();
        assert_eq!(
            strings.span_id,
            crate::native::to_hex_utf8_bytes(child.into_u64())
        );
        assert_eq!(
            strings.parent_span_id,
            Some(crate::native::to_hex_utf8_bytes(parent.into_u64()))
        );

        let context = addref_span(subscriber.subscriber_id, &child);
        write_event(
            subscriber.subscriber_id,
            Pin::new(&writer),
            &event,
            "event",
            1,
            0,
            Some(EventSpan::Cached(context.ids)),
            None,
        );
        let (current, related, strings) = writer.0.lock().unwrap().take().unwrap();
        assert_eq!((current, related), (child.into_u64(), parent.into_u64()));
        let strings = strings.unwrap();
        assert_eq!(
            strings.span_id,
            crate::native::to_hex_utf8_bytes(child.into_u64())
        );
        assert_eq!(
            strings.parent_span_id,
            Some(crate::native::to_hex_utf8_bytes(parent.into_u64()))
        );
        assert!(!subscriber.try_close(child.clone()));

        let otel = SpanStrings {
            trace_id: [b'a'; 32],
            span_id: [b'b'; 16],
            parent_span_id: None,
        };
        write_event(
            subscriber.subscriber_id,
            Pin::new(&writer),
            &event,
            "event",
            1,
            0,
            Some(EventSpan::Id(&child)),
            Some(otel),
        );
        let (_, _, strings) = writer.0.lock().unwrap().take().unwrap();
        let strings = strings.unwrap();
        assert_eq!(strings.trace_id, [b'a'; 32]);
        assert_eq!(strings.span_id, [b'b'; 16]);

        write_event(
            subscriber.subscriber_id,
            Pin::new(&writer),
            &event,
            "event",
            1,
            0,
            None,
            None,
        );
        let (current, related, strings) = writer.0.lock().unwrap().take().unwrap();
        assert_eq!((current, related), (0, 0));
        assert!(strings.is_none());
        assert!(subscriber.try_close(child));
        assert!(subscriber.try_close(parent));
    }
}

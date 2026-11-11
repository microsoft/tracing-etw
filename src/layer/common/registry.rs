use super::*;
use hashbrown::{HashMap, hash_map::RawEntryMut};

const SPAN_HASHER: ahash::RandomState = ahash::RandomState::with_seeds(
    17_616_942_133_695_121_499,
    9_565_839_503_509_016_163,
    2_756_528_679_765_226_774,
    228_784_672_216_536_063,
);

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
            hash: core::hash::BuildHasher::hash_one(&SPAN_HASHER, key),
        }
    }

    #[inline]
    fn shard_index(self) -> usize {
        (self.hash >> 32) as usize & (SPAN_SHARD_COUNT - 1)
    }

    fn get(self, spans: &RegistryShard) -> Option<&RegistrySpanData> {
        spans
            .spans
            .raw_entry()
            .from_key_hashed_nocheck(self.hash, self.key)
            .map(|(_, data)| data)
    }

    fn entry(
        self,
        spans: &mut RegistryShard,
    ) -> RawEntryMut<'_, SpanKey, RegistrySpanData, ahash::RandomState> {
        spans
            .spans
            .raw_entry_mut()
            .from_key_hashed_nocheck(self.hash, self.key)
    }

    fn get_mut(self, spans: &mut RegistryShard) -> Option<&mut RegistrySpanData> {
        match self.entry(spans) {
            RawEntryMut::Occupied(entry) => Some(entry.into_mut()),
            RawEntryMut::Vacant(_) => None,
        }
    }

    // Clone the payload handle so callers can unlock the shard before emitting.
    fn payload(self) -> Option<Arc<RwLock<SpanData>>> {
        self.get(&shard(self).read())
            .map(|span| Arc::clone(&span.payload))
    }
}

struct RegistrySpanData {
    // Shared so native writes run without holding the shard lock.
    payload: Arc<RwLock<SpanData>>,
    ids: SpanIds,
    references: usize,
    retained_parent: Option<tracing::span::Id>,
}

impl RegistrySpanData {
    fn retain(&mut self) {
        self.references = self
            .references
            .checked_add(1)
            .expect("Span reference count overflow");
    }
}

type RegistrySpanIndex = HashMap<SpanKey, RegistrySpanData, ahash::RandomState>;

struct RegistryShard {
    spans: RegistrySpanIndex,
    // Released payload allocations, reused by later spans in this shard.
    free: Vec<Arc<RwLock<SpanData>>>,
}

static SPAN_DATA: [SpanShard<RegistryShard>; SPAN_SHARD_COUNT] = [const {
    SpanShard {
        spans: RwLock::new(RegistryShard {
            spans: HashMap::with_hasher(SPAN_HASHER),
            free: Vec::new(),
        }),
    }
}; SPAN_SHARD_COUNT];

fn shard(lookup: SpanLookup<'_>) -> &'static RwLock<RegistryShard> {
    &SPAN_DATA[lookup.shard_index()].spans
}

pub(crate) fn create_span_data_for_new_span(
    subscriber_id: u64,
    attrs: &tracing::span::Attributes<'_>,
    id: &tracing::span::Id,
    parent: Option<&tracing::span::Id>,
) {
    let data = PreparedSpanData::new(attrs).finish(id, parent);
    let key = (subscriber_id, id.clone());
    let lookup = SpanLookup::new(&key);
    let parent_key = parent.map(|parent| (subscriber_id, parent.clone()));
    let parent = parent_key.as_ref().map(SpanLookup::new);
    let unused = with_span_shards(
        &SPAN_DATA,
        lookup.shard_index(),
        parent.map(SpanLookup::shard_index),
        |spans, parents| insert_span_data(spans, parents, lookup, data, parent),
    );
    drop(unused);
}

fn insert_span_data(
    spans: &mut RegistryShard,
    parents: Option<&mut RegistryShard>,
    lookup: SpanLookup<'_>,
    data: SpanData,
    parent: Option<SpanLookup<'_>>,
) -> Option<SpanData> {
    if let Some(existing) = lookup.get_mut(spans) {
        // Multiple exporter layers can observe the same registry span.
        existing.retain();
        return Some(data);
    }
    let retained_parent = parent.and_then(|parent| {
        let data = parent.get_mut(parents.unwrap_or(&mut *spans))?;
        data.retain();
        Some(parent.key.1.clone())
    });
    let ids = data.ids;
    let payload = match spans.free.pop() {
        Some(mut cached) => {
            // Cached payloads are unique and their fields were already taken.
            *Arc::get_mut(&mut cached)
                .and_then(RwLock::get_mut)
                .expect("Cached span payload was still shared") = data;
            cached
        }
        None => Arc::new(RwLock::new(data)),
    };
    match lookup.entry(spans) {
        RawEntryMut::Vacant(entry) => {
            entry.insert_hashed_nocheck(
                lookup.hash,
                lookup.key.clone(),
                RegistrySpanData {
                    payload,
                    ids,
                    references: 1,
                    retained_parent,
                },
            );
            None
        }
        RawEntryMut::Occupied(_) => {
            unreachable!("Span became occupied while its shard was locked")
        }
    }
}

// Keeps released allocations alive until the shard lock is dropped.
struct ReleasedPayload {
    _payload: Option<Arc<RwLock<SpanData>>>,
    _fields: Box<[FieldValueIndex]>,
}

fn recycle_payload(
    free: &mut Vec<Arc<RwLock<SpanData>>>,
    mut payload: Arc<RwLock<SpanData>>,
) -> ReleasedPayload {
    if free.len() < CACHED_ENTRIES_PER_SHARD {
        // A concurrent enter/exit may still hold a clone; only reuse unique payloads.
        if let Some(data) = Arc::get_mut(&mut payload).and_then(RwLock::get_mut) {
            let fields = core::mem::take(&mut data.fields);
            free.push(payload);
            return ReleasedPayload {
                _payload: None,
                _fields: fields,
            };
        }
    }
    ReleasedPayload {
        _payload: Some(payload),
        _fields: Box::default(),
    }
}

pub(crate) fn release_span(subscriber_id: u64, id: &tracing::span::Id) -> bool {
    let mut key = (subscriber_id, id.clone());
    let mut closed = false;
    loop {
        let lookup = SpanLookup::new(&key);
        let (parent, released) = {
            let mut spans = shard(lookup).write();
            let mut entry = match lookup.entry(&mut spans) {
                RawEntryMut::Occupied(entry) => entry,
                // A layer sees callbacks for spans it never saw created after
                // tracing_subscriber::reload. Close runs from Span::drop, so
                // ignore it rather than panicking.
                RawEntryMut::Vacant(_) => return closed,
            };
            let span = entry.get_mut();
            span.references = span
                .references
                .checked_sub(1)
                .expect("Span reference count underflow");
            if span.references != 0 {
                return closed;
            }
            let removed = entry.remove();
            let released = recycle_payload(&mut spans.free, removed.payload);
            (removed.retained_parent, released)
        };
        closed = true;
        drop(released);
        // The child's retained reference keeps its parent alive between locks.
        match parent {
            Some(parent) => key.1 = parent,
            None => return true,
        }
    }
}

// Releases all span data stored for a registry layer that is being dropped.
pub(crate) fn release_subscriber(subscriber_id: u64) {
    for shard in &SPAN_DATA {
        // Payloads and retained parents belong to the same subscriber, so the
        // entries can be removed without reference-count adjustments. Drop
        // them after unlocking the shard.
        let released: Vec<_> = shard
            .spans
            .write()
            .spans
            .extract_if(|key, _| key.0 == subscriber_id)
            .collect();
        drop(released);
    }
}

#[cfg(feature = "opentelemetry")]
pub(crate) fn span_metadata(
    subscriber_id: u64,
    id: &tracing::span::Id,
) -> Option<&'static tracing_core::Metadata<'static>> {
    let key = (subscriber_id, id.clone());
    SpanLookup::new(&key)
        .payload()
        .map(|payload| payload.read().metadata)
}

pub(crate) fn enter_span<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let timestamp = Timestamp::now();
    let key = (subscriber_id, id.clone());
    // Unknown spans predate this layer (see release_span).
    let Some(payload) = SpanLookup::new(&key).payload() else {
        return;
    };
    write_span_start_data(
        &mut payload.write(),
        timestamp,
        writer,
        keyword,
        tag,
        otel_span_strings,
    );
}

pub(crate) fn exit_span<OutMode: OutputMode>(
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let timestamp = Timestamp::now();
    let key = (subscriber_id, id.clone());
    let Some(payload) = SpanLookup::new(&key).payload() else {
        return;
    };
    write_span_stop(
        &payload.read(),
        timestamp,
        writer,
        keyword,
        tag,
        otel_span_strings,
    );
}

pub(crate) fn update_span_values(
    subscriber_id: u64,
    id: &tracing::span::Id,
    values: &tracing::span::Record<'_>,
) {
    let mut updates = SpanUpdates::new(values);
    let key = (subscriber_id, id.clone());
    let Some(payload) = SpanLookup::new(&key).payload() else {
        return;
    };
    updates.apply(&mut payload.write());
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_event<OutMode: OutputMode>(
    subscriber_id: u64,
    writer: Pin<&impl EventWriter<OutMode>>,
    event: &tracing::Event<'_>,
    name: &str,
    keyword: u64,
    tag: u32,
    span: Option<&tracing::span::Id>,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    write_event_with(
        writer,
        event,
        name,
        keyword,
        tag,
        || {
            let ids = span.and_then(|id| {
                let key = (subscriber_id, id.clone());
                let lookup = SpanLookup::new(&key);
                lookup.get(&shard(lookup).read()).map(|span| span.ids)
            });
            (span.map_or(0, tracing::span::Id::into_u64), ids)
        },
        otel_span_strings,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LayerBuilder,
        layer::_EtwTracingSubscriber,
        layer::common::{
            self as standalone,
            tests::{LOCK_PROBES, META, RecordingWriter, new_subscriber},
        },
        native::NormalOutput,
    };
    use std::{sync::mpsc, time::Duration};
    use tracing::{
        Subscriber,
        callsite::Callsite,
        metadata::Kind,
        span::{Attributes, Id},
    };
    use tracing_subscriber::prelude::*;

    fn contains(subscriber_id: u64, id: &Id) -> bool {
        let key = (subscriber_id, id.clone());
        shard(SpanLookup::new(&key)).read().spans.contains_key(&key)
    }

    // Layers receive a new storage identity when attached to a subscriber.
    fn attached_id<Mode: OutputMode + 'static>(subscriber: &(impl Subscriber + 'static)) -> u64 {
        (subscriber as &dyn Subscriber)
            .downcast_ref::<_EtwTracingSubscriber<Mode, tracing_subscriber::Registry>>()
            .expect("The layer is attached to a registry")
            .subscriber_id
    }

    fn parent_span_id(subscriber_id: u64, id: &Id) -> Option<NonZeroU64> {
        let key = (subscriber_id, id.clone());
        shard(SpanLookup::new(&key)).read().spans[&key]
            .ids
            .parent_span_id
    }

    #[test]
    fn unrecognized_spans_are_ignored() {
        let id = Id::from_u64(u64::MAX);
        let writer = RecordingWriter::default();
        enter_span(u64::MAX, &id, Pin::new(&writer), 1, 0, None);
        exit_span(u64::MAX, &id, Pin::new(&writer), 1, 0, None);
        let values = META.fields().value_set(&[]);
        update_span_values(u64::MAX, &id, &tracing::span::Record::new(&values));
        assert!(!release_span(u64::MAX, &id));
    }

    #[test]
    fn reloading_a_layer_releases_its_spans_and_ignores_unknown_spans() {
        let (reloadable, handle) = tracing_subscriber::reload::Layer::new(
            LayerBuilder::new("RegistryReload")
                .__build_for_test()
                .unwrap(),
        );
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(reloadable));
        let old_id = handle.with_current(|layer| layer.subscriber_id).unwrap();
        tracing::dispatcher::with_default(&dispatch, || {
            let parent = tracing::info_span!("reloaded parent");
            let span = tracing::info_span!(parent: &parent, "reloaded", value = 1u64);
            let (parent_id, id) = (parent.id().unwrap(), span.id().unwrap());
            let entered = span.enter();
            assert!(contains(old_id, &id) && contains(old_id, &parent_id));
            handle
                .reload(
                    LayerBuilder::new("RegistryReloaded")
                        .__build_for_test()
                        .unwrap(),
                )
                .unwrap();
            // The replaced layer's span data is released with it.
            assert!(!contains(old_id, &id) && !contains(old_id, &parent_id));
            // The replacement ignores callbacks for spans it never saw.
            span.record("value", 2u64);
            drop(entered);
            drop(span);
            drop(parent);
        });
    }

    #[test]
    fn dropping_an_attached_layer_releases_its_spans() {
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry().with(
                LayerBuilder::new("RegistryDropTests")
                    .__build_for_test()
                    .unwrap(),
            ),
        );
        let subscriber_id = dispatch
            .downcast_ref::<_EtwTracingSubscriber<NormalOutput, tracing_subscriber::Registry>>()
            .unwrap()
            .subscriber_id;
        let values = META.fields().value_set(&[]);
        // Without a Span guard the span is never closed.
        let id = dispatch.new_span(&Attributes::new_root(&META, &values));
        assert!(contains(subscriber_id, &id));
        drop(dispatch);
        // Concurrent callsite-interest rebuilds in other tests can briefly
        // keep the subscriber alive, so allow those to finish.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while contains(subscriber_id, &id) {
            assert!(
                std::time::Instant::now() < deadline,
                "Dropping the layer must release its spans"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn layer_clones_attached_to_separate_registries_use_separate_storage() {
        let layer = LayerBuilder::new("RegistryCloneTests")
            .__build_for_test()
            .unwrap();
        let built_id = layer.subscriber_id;
        let first = tracing_subscriber::registry().with(layer.clone());
        let second = tracing_subscriber::registry().with(layer);
        let first_id = attached_id::<NormalOutput>(&first);
        let second_id = attached_id::<NormalOutput>(&second);
        assert_ne!(first_id, second_id);
        assert_ne!(first_id, built_id);

        // Both registries allocate the same first span ID.
        let first_span = tracing::subscriber::with_default(first, || {
            let span = tracing::info_span!("first", value = 1u64);
            let id = span.id().unwrap();
            assert!(contains(first_id, &id));
            assert!(!contains(second_id, &id));
            (span, id)
        });
        tracing::subscriber::with_default(second, || {
            let span = tracing::info_span!("second", value = 2u64);
            let id = span.id().unwrap();
            assert_eq!(id, first_span.1);
            let key = (first_id, id.clone());
            assert_eq!(
                SpanLookup::new(&key)
                    .payload()
                    .unwrap()
                    .read()
                    .metadata
                    .name(),
                "first"
            );
            let key = (second_id, id.clone());
            assert_eq!(
                SpanLookup::new(&key)
                    .payload()
                    .unwrap()
                    .read()
                    .metadata
                    .name(),
                "second"
            );
            drop(span);
            assert!(!contains(second_id, &id));
            assert!(contains(first_id, &id));
        });
    }

    #[test]
    fn contextual_parents_come_from_registry_ancestry() {
        use tracing_subscriber::filter::filter_fn;

        // Another layer enables every span, so the registry creates spans
        // that are disabled only for the ETW layer.
        struct EnableAll;
        impl<S: Subscriber> tracing_subscriber::Layer<S> for EnableAll {}

        let layer = LayerBuilder::new("RegistryAncestryTests")
            .__build_for_test()
            .unwrap()
            .with_filter(filter_fn(|metadata| metadata.name() != "disabled"));
        let registry = tracing_subscriber::registry().with(layer).with(EnableAll);
        let subscriber_id = attached_id::<NormalOutput>(&registry);
        tracing::subscriber::with_default(registry, || {
            let enabled = tracing::info_span!(parent: None, "enabled");
            let enabled_id = enabled.id().unwrap();
            let _enabled = enabled.enter();
            // An unrelated root span, disabled for this layer, is entered last.
            let disabled = tracing::info_span!(parent: None, "disabled");
            assert!(!disabled.is_disabled());
            let _disabled = disabled.enter();
            let unrelated = tracing::info_span!("unrelated");
            let unrelated_id = unrelated.id().unwrap();
            assert_eq!(parent_span_id(subscriber_id, &unrelated_id), None);

            // A disabled intermediate parent is skipped in favor of its ancestor.
            let nested_disabled = tracing::info_span!(parent: &enabled, "disabled");
            let nested = tracing::info_span!(parent: &nested_disabled, "nested");
            let nested_id = nested.id().unwrap();
            assert_eq!(
                parent_span_id(subscriber_id, &nested_id),
                Some(enabled_id.into_non_zero_u64())
            );
        });
    }

    #[test]
    fn registry_growth_preserves_precomputed_hashes_and_bucket_distribution() {
        let subscriber = new_subscriber();
        let subscriber_id = subscriber.subscriber_id;
        let ids: Vec<_> = (1..)
            .map(Id::from_u64)
            .filter(|id| {
                SpanLookup::new(&(subscriber_id, id.clone())).shard_index() == SPAN_SHARD_COUNT / 2
            })
            .take(256)
            .collect();
        let mut buckets = 0u8;
        for id in &ids {
            buckets |= 1 << (SpanLookup::new(&(subscriber_id, id.clone())).hash & 7);
            let values = META.fields().value_set(&[]);
            create_span_data_for_new_span(
                subscriber_id,
                &Attributes::new_root(&META, &values),
                id,
                None,
            );
        }
        assert!(buckets.count_ones() >= 6);
        for id in ids {
            assert!(contains(subscriber_id, &id));
            assert!(release_span(subscriber_id, &id));
        }
    }

    #[test]
    fn registry_entries_retain_each_parent_once() {
        let subscriber = new_subscriber();
        let parent = Id::from_u64(1);
        let child = Id::from_u64(2);
        let values = META.fields().value_set(&[]);
        create_span_data_for_new_span(
            subscriber.subscriber_id,
            &Attributes::new_root(&META, &values),
            &parent,
            None,
        );
        for _ in 0..2 {
            create_span_data_for_new_span(
                subscriber.subscriber_id,
                &Attributes::child_of(parent.clone(), &META, &values),
                &child,
                Some(&parent),
            );
        }
        for (id, expected) in [(&parent, 2), (&child, 2)] {
            let key = (subscriber.subscriber_id, id.clone());
            assert_eq!(
                shard(SpanLookup::new(&key)).read().spans[&key].references,
                expected
            );
        }
        assert!(!release_span(subscriber.subscriber_id, &parent));
        assert!(!release_span(subscriber.subscriber_id, &child));
        assert!(release_span(subscriber.subscriber_id, &child));
        assert!(!contains(subscriber.subscriber_id, &child));
        assert!(!contains(subscriber.subscriber_id, &parent));
    }

    #[test]
    fn preserves_registry_parent_context_and_cleans_up_exporter_data() {
        let layer = LayerBuilder::new("RegistryLifetimeTests")
            .__build_for_test()
            .unwrap();
        let registry = tracing_subscriber::registry().with(layer);
        let subscriber_id = attached_id::<NormalOutput>(&registry);
        tracing::subscriber::with_default(registry, || {
            let parent = tracing::info_span!(parent: None, "registry parent");
            let parent_id = parent.id().unwrap();
            let entered = parent.enter();
            let child = tracing::info_span!("registry child");
            let child_id = child.id().unwrap();
            let key = (subscriber_id, child_id.clone());
            assert_eq!(
                shard(SpanLookup::new(&key)).read().spans[&key]
                    .ids
                    .parent_span_id,
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

    #[test]
    fn backends_isolate_identical_keys_fields_correlation_and_lifetimes() {
        let subscriber = new_subscriber();
        let subscriber_id = subscriber.subscriber_id;
        let values = META.fields().value_set(&[]);
        let metadata = tracing::callsite! {
            name: "isolated backend fields",
            kind: Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields: value
        }
        .metadata();
        let parent = subscriber.new_span(&Attributes::new_root(&META, &values));
        let child = subscriber.new_span(&Attributes::new_root(
            metadata,
            &tracing::valueset!(metadata.fields(), value = 1u64),
        ));
        create_span_data_for_new_span(
            subscriber_id,
            &Attributes::new_root(&META, &values),
            &parent,
            None,
        );
        create_span_data_for_new_span(
            subscriber_id,
            &Attributes::child_of(
                parent.clone(),
                metadata,
                &tracing::valueset!(metadata.fields(), value = 1u64),
            ),
            &child,
            Some(&parent),
        );
        subscriber.enter(&child);
        let writer = RecordingWriter::default();
        enter_span(subscriber_id, &child, Pin::new(&writer), 1, 0, None);
        subscriber.record(
            &child,
            &tracing::span::Record::new(&tracing::valueset!(metadata.fields(), value = 7u64)),
        );
        update_span_values(
            subscriber_id,
            &child,
            &tracing::span::Record::new(&tracing::valueset!(metadata.fields(), value = 11u64)),
        );
        let key = (subscriber_id, child.clone());
        assert!(matches!(
            SpanLookup::new(&key).payload().unwrap().read().fields[0].value,
            ValueTypes::v_u64(11)
        ));
        assert!(matches!(
            standalone::find_span(subscriber_id, &child)
                .unwrap()
                .data
                .read()
                .fields[0]
                .value,
            ValueTypes::v_u64(7)
        ));
        let event = tracing::Event::new(&META, &values);
        write_event(
            subscriber_id,
            Pin::new(&writer),
            &event,
            "event",
            1,
            0,
            Some(&child),
            None,
        );
        let (current, related, strings) = writer.0.lock().unwrap().take().unwrap();
        assert_eq!((current, related), (child.into_u64(), parent.into_u64()));
        assert_eq!(
            strings.unwrap().parent_span_id,
            Some(crate::native::to_hex_utf8_bytes(parent.into_u64()))
        );
        standalone::write_event(
            subscriber_id,
            Pin::new(&writer),
            &event,
            "event",
            1,
            0,
            Some(EventSpan::Id(&child)),
            None,
        );
        let (current, related, strings) = writer.0.lock().unwrap().take().unwrap();
        assert_eq!((current, related), (child.into_u64(), 0));
        assert!(strings.unwrap().parent_span_id.is_none());
        exit_span(subscriber_id, &child, Pin::new(&writer), 1, 0, None);
        assert!(!release_span(subscriber_id, &parent));
        assert!(subscriber.try_close(parent.clone()));
        assert!(!subscriber.try_close(child.clone()));
        assert!(release_span(subscriber_id, &child));
        assert!(!contains(subscriber_id, &parent));
        assert!(!contains(subscriber_id, &child));
        assert!(standalone::tests::contains(subscriber_id, &child));
        assert_eq!(subscriber.current_span().id(), Some(&child));
        subscriber.exit(&child);
        assert!(!standalone::tests::contains(subscriber_id, &child));
    }

    #[test]
    fn registry_emission_runs_without_the_shard_lock() {
        let _probe = LOCK_PROBES.lock().unwrap();
        let subscriber = new_subscriber();
        let id = Id::from_u64(3);
        let values = META.fields().value_set(&[]);
        create_span_data_for_new_span(
            subscriber.subscriber_id,
            &Attributes::new_root(&META, &values),
            &id,
            None,
        );
        let writer = standalone::tests::UnlockedShardWriter {
            key: (subscriber.subscriber_id, id.clone()),
            lock: |key| drop(shard(SpanLookup::new(key)).write()),
        };
        enter_span(subscriber.subscriber_id, &id, Pin::new(&writer), 1, 0, None);
        exit_span(subscriber.subscriber_id, &id, Pin::new(&writer), 1, 0, None);
        assert!(release_span(subscriber.subscriber_id, &id));
    }

    #[test]
    fn registry_api_progresses_while_standalone_shards_are_locked() {
        let _probe = LOCK_PROBES.lock().unwrap();
        let layer = LayerBuilder::new("RegistryIndependentStorage")
            .__build_for_test()
            .unwrap();
        let registry = tracing_subscriber::registry().with(layer);
        let subscriber_id = attached_id::<NormalOutput>(&registry);
        let metadata = tracing::callsite! {
            name: "independent registry fields",
            kind: Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields: value
        }
        .metadata();
        let (sender, receiver) = mpsc::channel();
        let guards: Vec<_> = standalone::SPAN_DATA
            .iter()
            .map(|shard| shard.spans.write())
            .collect();
        std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let dispatch = tracing::Dispatch::new(registry);
                tracing::dispatcher::with_default(&dispatch, || {
                    let values = META.fields().value_set(&[]);
                    let parent = dispatch.new_span(&Attributes::new_root(&META, &values));
                    dispatch.enter(&parent);
                    let child = dispatch.new_span(&Attributes::new(
                        metadata,
                        &tracing::valueset!(metadata.fields(), value = 1u64),
                    ));
                    dispatch.enter(&child);
                    dispatch.record(
                        &child,
                        &tracing::span::Record::new(&tracing::valueset!(
                            metadata.fields(),
                            value = 7u64
                        )),
                    );
                    dispatch.event(&tracing::Event::new(&META, &values));
                    dispatch.event(&tracing::Event::new_child_of(
                        Some(parent.clone()),
                        &META,
                        &values,
                    ));
                    dispatch.exit(&child);
                    dispatch.exit(&parent);
                    dispatch.try_close(parent.clone());
                    assert!(dispatch.try_close(child.clone()));
                    assert!(!contains(subscriber_id, &child));
                    assert!(!contains(subscriber_id, &parent));
                });
                sender.send(()).unwrap();
            });
            let result = receiver.recv_timeout(Duration::from_secs(5));
            drop(guards);
            worker.join().unwrap();
            result.expect("Registry callbacks must not acquire standalone shard locks");
        });
    }
}

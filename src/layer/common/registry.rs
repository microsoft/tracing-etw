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

    fn get(self, spans: &RegistrySpanIndex) -> Option<&RegistrySpanData> {
        spans
            .raw_entry()
            .from_key_hashed_nocheck(self.hash, self.key)
            .map(|(_, data)| data)
    }

    fn entry(
        self,
        spans: &mut RegistrySpanIndex,
    ) -> RawEntryMut<'_, SpanKey, RegistrySpanData, ahash::RandomState> {
        spans
            .raw_entry_mut()
            .from_key_hashed_nocheck(self.hash, self.key)
    }

    fn get_mut(self, spans: &mut RegistrySpanIndex) -> Option<&mut RegistrySpanData> {
        match self.entry(spans) {
            RawEntryMut::Occupied(entry) => Some(entry.into_mut()),
            RawEntryMut::Vacant(_) => None,
        }
    }
}

struct RegistrySpanData {
    data: SpanData,
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

static SPAN_DATA: [SpanShard<RegistrySpanIndex>; SPAN_SHARD_COUNT] = [const {
    SpanShard {
        spans: RwLock::new(HashMap::with_hasher(SPAN_HASHER)),
    }
}; SPAN_SHARD_COUNT];

fn shard(lookup: SpanLookup<'_>) -> &'static RwLock<RegistrySpanIndex> {
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
    spans: &mut RegistrySpanIndex,
    parents: Option<&mut RegistrySpanIndex>,
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
    match lookup.entry(spans) {
        RawEntryMut::Vacant(entry) => {
            entry.insert_hashed_nocheck(
                lookup.hash,
                lookup.key.clone(),
                RegistrySpanData {
                    data,
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

pub(crate) fn release_span(subscriber_id: u64, id: &tracing::span::Id) -> bool {
    let mut key = (subscriber_id, id.clone());
    let mut closed = false;
    loop {
        let lookup = SpanLookup::new(&key);
        let mut removed = {
            let mut spans = shard(lookup).write();
            let mut entry = match lookup.entry(&mut spans) {
                RawEntryMut::Occupied(entry) => entry,
                RawEntryMut::Vacant(_) => panic!("Close of an unrecognized span"),
            };
            let span = entry.get_mut();
            span.references = span
                .references
                .checked_sub(1)
                .expect("Span reference count underflow");
            if span.references != 0 {
                return closed;
            }
            entry.remove()
        };
        closed = true;
        let parent = removed.retained_parent.take();
        drop(removed);
        // The child's retained reference keeps its parent alive between locks.
        match parent {
            Some(parent) => key.1 = parent,
            None => return true,
        }
    }
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
    let lookup = SpanLookup::new(&key);
    let mut spans = shard(lookup).write();
    let Some(span) = lookup.get_mut(&mut spans) else {
        debug_assert!(false, "Enter of unrecognized span");
        return;
    };
    write_span_start_data(
        &mut span.data,
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
    let lookup = SpanLookup::new(&key);
    let spans = shard(lookup).read();
    let Some(span) = lookup.get(&spans) else {
        debug_assert!(false, "Exit of unrecognized span");
        return;
    };
    write_span_stop(
        &span.data,
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
    let lookup = SpanLookup::new(&key);
    let mut spans = shard(lookup).write();
    let Some(span) = lookup.get_mut(&mut spans) else {
        debug_assert!(false, "Event on unrecognized span");
        return;
    };
    updates.apply(&mut span.data);
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
                lookup.get(&shard(lookup).read()).map(|span| span.data.ids)
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
        layer::common::{
            self as standalone,
            tests::{LOCK_PROBES, META, RecordingWriter, new_subscriber},
        },
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
        shard(SpanLookup::new(&key)).read().contains_key(&key)
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
            let key = (subscriber_id, id.clone());
            assert!(shard(SpanLookup::new(&key)).read().contains_key(&key));
            assert!(release_span(subscriber_id, &id));
        }
    }

    #[test]
    fn registry_entries_stay_inline_and_retain_each_parent_once() {
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
                shard(SpanLookup::new(&key)).read()[&key].references,
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
                shard(SpanLookup::new(&key)).read()[&key]
                    .data
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
            shard(SpanLookup::new(&key)).read()[&key].data.fields[0].value,
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
    fn registry_api_progresses_while_standalone_shards_are_locked() {
        let _probe = LOCK_PROBES.lock().unwrap();
        let layer = LayerBuilder::new("RegistryIndependentStorage")
            .__build_for_test()
            .unwrap();
        let subscriber_id = layer.subscriber_id;
        let registry = tracing_subscriber::registry().with(layer);
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

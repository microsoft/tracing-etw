use core::{num::NonZeroU64, pin::Pin};
extern crate alloc;
use alloc::{boxed::Box, vec::Vec};

use hashbrown::HashMap;

use crate::{
    native::{EventWriter, OutputMode, time::Timestamp},
    statics::GLOBAL_ACTIVITY_SEED,
    sync::RwLock,
    values::{
        ValueTypes,
        span_values::{FieldValueIndex, SpanValueVisitor},
    },
};

// DOS resistance is not needed in this hash
static SPAN_DATA: RwLock<HashMap<(u64, tracing::span::Id), SpanData, ahash::RandomState>> =
    RwLock::new(HashMap::with_hasher(ahash::RandomState::with_seeds(
        17_616_942_133_695_121_499,
        9_565_839_503_509_016_163,
        2_756_528_679_765_226_774,
        228_784_672_216_536_063,
    )));

pub(crate) struct SpanIds {
    pub(crate) span_id: NonZeroU64,
    pub(crate) parent_span_id: Option<NonZeroU64>, // sizeof(Option<NonZeroU64>) == sizeof(u64) is guaranteed by the standard
}

#[doc(hidden)]
pub struct SpanStrings {
    pub(crate) trace_id: [u8; 32], // UTF-8 hex bytes, only non-zero if the otel feature is enabled and the span has a valid trace ID
    pub(crate) span_id: [u8; 16], // UTF-8 hex bytes, either the otel span ID if the otel feature is enabled, or the local span ID
    pub(crate) parent_span_id: Option<[u8; 16]>, // UTF-8 hex bytes, only Some if the otel feature is enabled and the span has a valid parent span ID
}

impl SpanStrings {
    fn build(span_data: &SpanData, otel_span_strings: Option<OtelSpanStrings>) -> SpanStrings {
        otel_span_strings.unwrap_or(SpanStrings {
            trace_id: [0; 32],
            span_id: crate::native::to_hex_utf8_bytes(span_data.ids.span_id.into()),
            parent_span_id: span_data
                .ids
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

        SpanData {
            fields: v.into_boxed_slice(),
            activity_id: *GLOBAL_ACTIVITY_SEED,
            related_activity_id: *GLOBAL_ACTIVITY_SEED,
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
        fields: &mut data.fields,
    });

    let mut spans = SPAN_DATA.write();
    if let Some(existing) = spans.get_mut(&(subscriber_id, id.clone())) {
        // Multiple exporter layers can observe the same registry span.
        existing.ref_count = existing
            .ref_count
            .checked_add(1)
            .expect("Span reference count overflow");
        return;
    }
    if let Some(parent) = parent {
        if let Some(parent_data) = spans.get_mut(&(subscriber_id, parent.clone())) {
            parent_data.ref_count = parent_data
                .ref_count
                .checked_add(1)
                .expect("Span reference count overflow");
            data.retained_parent = Some(parent.clone());
        }
    }
    spans.insert((subscriber_id, id.clone()), data);
}

pub(crate) fn addref_span(subscriber_id: u64, id: &tracing::span::Id) {
    let mut spans = SPAN_DATA.write();
    let span = spans
        .get_mut(&(subscriber_id, id.clone()))
        .expect("Clone or enter of an unrecognized span");
    span.ref_count = span
        .ref_count
        .checked_add(1)
        .expect("Span reference count overflow");
}

pub(crate) fn span_metadata(
    subscriber_id: u64,
    id: &tracing::span::Id,
) -> &'static tracing_core::Metadata<'static> {
    SPAN_DATA
        .read()
        .get(&(subscriber_id, id.clone()))
        .expect("Current span is unrecognized")
        .metadata
}

pub(crate) fn release_span(subscriber_id: u64, id: &tracing::span::Id) -> bool {
    let mut spans = SPAN_DATA.write();
    let mut current = (subscriber_id, id.clone());
    loop {
        let span = spans
            .get_mut(&current)
            .expect("Close of an unrecognized span");
        span.ref_count = span
            .ref_count
            .checked_sub(1)
            .expect("Span reference count underflow");
        if span.ref_count != 0 {
            return current.1 != *id;
        }
        let span = spans.remove(&current).expect("Closed span is unrecognized");
        match span.retained_parent {
            Some(parent) => current.1 = parent,
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

    let mut span_data_guard = SPAN_DATA.write();
    let data = if let Some(data) = span_data_guard.get_mut(&(subscriber_id, id.clone())) {
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
    subscriber_id: u64,
    id: &tracing::span::Id,
    writer: Pin<&impl EventWriter<OutMode>>,
    keyword: u64,
    tag: u32,
    otel_span_strings: Option<OtelSpanStrings>,
) {
    let stop_timestamp = Timestamp::now();

    let span_data_guard = SPAN_DATA.read();
    let data = if let Some(data) = span_data_guard.get(&(subscriber_id, id.clone())) {
        data
    } else {
        debug_assert!(false, "Exit of unrecognized span");
        return;
    };

    writer.span_stop(
        (data.start_time.into(), stop_timestamp.into()),
        SpanRef {
            data,
            span_strings: SpanStrings::build(data, otel_span_strings),
        },
        keyword,
        tag,
    );
}

pub(crate) fn update_span_values(
    subscriber_id: u64,
    id: &tracing::span::Id,
    values: &tracing::span::Record<'_>,
) {
    let mut span_data_guard = SPAN_DATA.write();
    let data = if let Some(data) = span_data_guard.get_mut(&(subscriber_id, id.clone())) {
        data
    } else {
        debug_assert!(false, "Event on unrecognized span");
        return;
    };

    values.record(&mut SpanValueVisitor {
        fields: &mut data.fields,
    });
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
    let timestamp = Timestamp::now();

    let spans = SPAN_DATA.read();
    let data = span.and_then(|id| spans.get(&(subscriber_id, id.clone())));
    let current_span = span.map_or(0, |id| id.into_u64());
    let parent_span = data
        .and_then(|data| data.ids.parent_span_id)
        .map_or(0, |id| id.get());
    let span_strings = match data {
        Some(data) => Some(SpanStrings::build(data, otel_span_strings)),
        None => otel_span_strings,
    };
    drop(spans);

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
    use tracing::{
        Metadata, Subscriber,
        callsite::DefaultCallsite,
        field::FieldSet,
        metadata::Kind,
        span::{Attributes, Id},
    };

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
        SPAN_DATA.read().contains_key(&(subscriber_id, id.clone()))
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
            let spans = SPAN_DATA.read();
            for id in [&contextual, &explicit] {
                assert_eq!(
                    spans[&(subscriber.subscriber_id, id.clone())]
                        .ids
                        .parent_span_id,
                    Some(parent.into_non_zero_u64()),
                );
            }
            assert_eq!(
                spans[&(subscriber.subscriber_id, root.clone())]
                    .ids
                    .parent_span_id,
                None
            );
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
        let id = new_span(&subscriber, Parent::Root);
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    subscriber.enter(&id);
                    assert!(!subscriber.try_close(id.clone()));
                })
                .join()
                .unwrap();
        });
        assert!(!contains(subscriber.subscriber_id, &id));
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
            assert_eq!(
                SPAN_DATA.read()[&(subscriber_id, child_id.clone())]
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
            Some(&child),
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
            Some(&child),
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

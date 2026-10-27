use alloc::vec::Vec;
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    layer::common,
    native::{ContextWriter, EventContext, ProviderTraits},
    statics::get_event_metadata,
};

use super::*;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[cold]
fn reserve_ids(counter: &AtomicU64, count: u64) -> (u64, u64) {
    let mut id = counter.load(Ordering::Relaxed);
    loop {
        let end = id.saturating_add(count);
        assert!(end > id, "Subscriber IDs exhausted");
        match counter.compare_exchange_weak(id, end, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return (id, end),
            Err(current) => id = current,
        }
    }
}

pub(crate) fn next_id() -> u64 {
    reserve_ids(&NEXT_ID, 1).0
}

struct ThreadState {
    spans: Vec<common::SpanHandle>,
    shard: Option<usize>,
}

impl ThreadState {
    fn current_span_context(&self, subscriber_id: u64) -> Option<common::SpanContext> {
        self.spans
            .iter()
            .rev()
            .find(|span| span.subscriber_id() == subscriber_id)
            .map(common::SpanHandle::context)
    }
}

crate::tls::thread_local! {
    static THREAD_STATE: LocalState = const {
        LocalState {
            entered: RefCell::new(ThreadState {
                spans: Vec::new(),
                shard: None,
            }),
            context: EventContext::new(),
        }
    };
}

struct LocalState {
    entered: RefCell<ThreadState>,
    context: EventContext,
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn with_event_context<R>(f: impl FnOnce(&EventContext) -> R) -> R {
    THREAD_STATE.with(|state| f(&state.context))
}

impl<OutMode: OutputMode> _EtwTracingSubscriber<OutMode> {
    fn current_span_context(&self) -> Option<common::SpanContext> {
        THREAD_STATE.with(|state| {
            state
                .entered
                .borrow()
                .current_span_context(self.subscriber_id)
        })
    }

    fn with_entered_span<R>(
        &self,
        id: &tracing_core::span::Id,
        f: impl FnOnce(&common::SpanHandle) -> R,
    ) -> Option<R> {
        crate::tls::try_with(&THREAD_STATE, |state| {
            let state = state.entered.try_borrow().ok()?;
            state
                .spans
                .iter()
                .rev()
                .find(|span| span.matches(self.subscriber_id, id))
                .map(f)
        })
        .flatten()
    }
}

impl<OutMode: OutputMode + 'static> tracing::Subscriber for _EtwTracingSubscriber<OutMode>
where
    crate::native::Provider<OutMode>: crate::native::EventWriter<OutMode>,
{
    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing_core::Interest {
        let etw_meta = get_event_metadata(&metadata.callsite());
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.default_keyword
        };

        if crate::native::Provider::<OutMode>::supports_enable_callback() {
            if self.provider.enabled(metadata.level(), keyword) {
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

    // Only called if register_callsite returned Interest::sometimes
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        self.is_enabled(&metadata.callsite(), metadata.level())
    }

    fn new_span(&self, attrs: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        let (shard, parent) = THREAD_STATE.with(|state| {
            let mut state = state.entered.borrow_mut();
            let shard = *state.shard.get_or_insert_with(common::next_span_shard);
            let parent = if attrs.is_contextual() {
                state
                    .current_span_context(self.subscriber_id)
                    .map(common::SpanContext::id)
            } else {
                attrs.parent().cloned()
            };
            (shard, parent)
        });

        common::create_span_data_for_new_span(self.subscriber_id, attrs, parent.as_ref(), shard)
    }

    fn clone_span(&self, id: &tracing_core::span::Id) -> tracing_core::span::Id {
        common::addref_span(self.subscriber_id, id);
        id.clone()
    }

    fn try_close(&self, id: tracing_core::span::Id) -> bool {
        common::release_span(self.subscriber_id, &id)
    }

    fn current_span(&self) -> tracing_core::span::Current {
        self.current_span_context()
            .map_or_else(tracing_core::span::Current::none, |context| {
                tracing_core::span::Current::new(context.id(), context.metadata)
            })
    }

    fn record(&self, id: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        // User formatting may reenter tracing, so finish it before borrowing TLS.
        let mut updates = common::SpanUpdates::new(values);
        if self
            .with_entered_span(id, |handle| handle.update_values(&mut updates))
            .is_none()
        {
            common::update_span_values_prepared(self.subscriber_id, id, &mut updates);
        }
    }

    fn record_follows_from(
        &self,
        _span: &tracing_core::span::Id,
        _follows: &tracing_core::span::Id,
    ) {
        // Do nothing
    }

    fn event(&self, event: &tracing::Event<'_>) {
        let etw_meta = get_event_metadata(&event.metadata().callsite());
        let (name, keyword, tag) = if let Some(meta) = etw_meta {
            (event.metadata().name(), meta.kw, meta.event_tag)
        } else {
            (event.metadata().name(), self.default_keyword, 0)
        };

        THREAD_STATE.with(|state| {
            let span = if event.is_contextual() {
                state
                    .entered
                    .borrow()
                    .current_span_context(self.subscriber_id)
                    .map(|context| common::EventSpan::Cached(context.ids))
            } else {
                event.parent().map(|id| {
                    state
                        .entered
                        .try_borrow()
                        .ok()
                        .and_then(|entered| {
                            entered
                                .spans
                                .iter()
                                .rev()
                                .find(|span| span.matches(self.subscriber_id, id))
                                .map(|handle| common::EventSpan::Cached(handle.context().ids))
                        })
                        .unwrap_or(common::EventSpan::Id(id))
                })
            };

            let writer = ContextWriter::new(self.provider.as_ref(), &state.context);
            common::write_event(
                self.subscriber_id,
                Pin::new(&writer),
                event,
                name,
                keyword,
                tag,
                span,
                None,
            )
        })
    }

    fn enter(&self, id: &tracing_core::span::Id) {
        THREAD_STATE.with(|local| {
            let writer = ContextWriter::new(self.provider.as_ref(), &local.context);
            let mut state = local.entered.borrow_mut();
            // Initialize TLS and reserve capacity before retaining an entry.
            state.spans.reserve(1);
            let cached = state
                .spans
                .iter()
                .rev()
                .find(|span| span.matches(self.subscriber_id, id))
                .cloned();
            let register = |handle| {
                state.spans.push(handle);
                drop(state);
            };
            if let Some(handle) = cached {
                common::enter_span_cached(
                    handle,
                    Pin::new(&writer),
                    self.default_keyword,
                    0,
                    None,
                    register,
                );
            } else {
                common::enter_span_retained(
                    self.subscriber_id,
                    id,
                    Pin::new(&writer),
                    self.default_keyword,
                    0,
                    None,
                    register,
                );
            }
        });
    }

    fn exit(&self, id: &tracing_core::span::Id) {
        THREAD_STATE.with(|local| {
            let entered = {
                let mut state = local.entered.borrow_mut();
                let index = state
                    .spans
                    .iter()
                    .rposition(|span| span.matches(self.subscriber_id, id))
                    .expect("Exit of a span that is not entered on this thread");
                state.spans.remove(index)
            };
            // Spans don't have callsites to store keyword/tag metadata on,
            // so we must use the defaults.
            let writer = ContextWriter::new(self.provider.as_ref(), &local.context);
            common::exit_span_cached(&entered, Pin::new(&writer), self.default_keyword, 0, None);
            drop(entered);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_core::{Subscriber, callsite::Callsite};

    #[cfg(all(not(feature = "std"), any(windows, target_os = "linux")))]
    #[test]
    fn native_span_emission_uses_one_tls_lookup_per_operation() {
        fn exercise<Mode: OutputMode + 'static>(subscriber: _EtwTracingSubscriber<Mode>)
        where
            crate::native::Provider<Mode>: crate::native::EventWriter<Mode>,
        {
            let metadata = tracing::callsite! {
                name: "single TLS lookup",
                kind: tracing::metadata::Kind::SPAN,
                target: module_path!(),
                level: tracing::Level::INFO,
                fields: value
            }
            .metadata();
            let id = subscriber.new_span(&tracing::span::Attributes::new_root(
                metadata,
                &tracing::valueset!(metadata.fields(), value = 1u64),
            ));
            let check = |operation: &dyn Fn()| {
                let before = crate::tls::access_count();
                operation();
                assert_eq!(crate::tls::access_count() - before, 1);
            };
            check(&|| subscriber.enter(&id));
            check(&|| subscriber.enter(&id));
            check(&|| {
                subscriber.event(&tracing::Event::new(
                    metadata,
                    &tracing::valueset!(metadata.fields(), value = 2u64),
                ))
            });
            check(&|| {
                subscriber.event(&tracing::Event::new_child_of(
                    Some(id.clone()),
                    metadata,
                    &tracing::valueset!(metadata.fields(), value = 2u64),
                ))
            });
            check(&|| subscriber.exit(&id));
            check(&|| subscriber.exit(&id));
            assert!(subscriber.try_close(id));
        }
        exercise(
            crate::LayerBuilder::new("SingleTlsLookup")
                .build_subscriber()
                .unwrap(),
        );
        exercise(
            crate::LayerBuilder::new_common_schema_events("SingleTlsLookupCS")
                .build_subscriber()
                .unwrap(),
        );
    }

    #[test]
    fn event_formatting_can_query_and_record_the_current_span() {
        let subscriber = crate::LayerBuilder::new("ReentrantContextRead")
            .build_subscriber()
            .unwrap();
        let metadata = tracing::callsite! {
            name: "formatting context",
            kind: tracing::metadata::Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields: value
        }
        .metadata();
        let id = subscriber.new_span(&tracing::span::Attributes::new_root(
            metadata,
            &tracing::valueset!(metadata.fields(), value = 1u64),
        ));
        subscriber.enter(&id);
        struct Probe<'a> {
            subscriber: &'a _EtwTracingSubscriber<crate::NormalOutput>,
            id: &'a tracing::span::Id,
        }
        impl core::fmt::Debug for Probe<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let current = self.subscriber.current_span();
                assert_eq!(current.id(), Some(self.id));
                let metadata = current.metadata().unwrap();
                self.subscriber.record(
                    self.id,
                    &tracing::span::Record::new(&tracing::valueset!(
                        metadata.fields(),
                        value = 7u64
                    )),
                );
                f.write_str("context available")
            }
        }
        let probe = Probe {
            subscriber: &subscriber,
            id: &id,
        };
        subscriber.event(&tracing::Event::new(
            metadata,
            &tracing::valueset!(metadata.fields(), value = ?probe),
        ));
        subscriber.record(
            &id,
            &tracing::span::Record::new(&tracing::valueset!(metadata.fields(), value = ?probe)),
        );
        subscriber.exit(&id);
        assert!(subscriber.try_close(id));
    }

    #[test]
    fn concurrent_recycled_span_ids_do_not_overlap() {
        let subscriber = crate::LayerBuilder::new("ConcurrentSlabIds")
            .build_subscriber()
            .unwrap();
        let metadata = tracing::callsite! {
            name: "concurrent slot reuse",
            kind: tracing::metadata::Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields:
        }
        .metadata();
        let mut ids = Vec::new();
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 0..4 {
                let subscriber = &subscriber;
                workers.push(scope.spawn(move || {
                    let values = metadata.fields().value_set(&[]);
                    (0..128)
                        .map(|_| {
                            let id = subscriber
                                .new_span(&tracing::span::Attributes::new_root(metadata, &values));
                            assert!(subscriber.try_close(id.clone()));
                            id.into_u64()
                        })
                        .collect::<Vec<_>>()
                }));
            }
            for worker in workers {
                ids.extend(worker.join().unwrap());
            }
        });
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4 * 128);
    }

    #[test]
    fn subscriber_id_reservations_do_not_wrap() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(reserve_ids(&counter, 1).0, u64::MAX - 1);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reserve_ids(&counter, 1)));
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn entering_an_unknown_span_panics_even_in_release_builds() {
        let subscriber = crate::LayerBuilder::new("UnknownEntryTests")
            .build_subscriber()
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            subscriber.enter(&tracing::span::Id::from_u64(u64::MAX));
        }));
        assert!(result.is_err());
        assert!(subscriber.current_span().id().is_none());
    }

    #[test]
    fn releases_the_entered_reference_if_tls_access_panics() {
        let builder = crate::LayerBuilder::new("FailedEnterTests");
        #[cfg(feature = "std")]
        let subscriber = builder.build_subscriber().unwrap();
        #[cfg(not(feature = "std"))]
        let subscriber = builder.build().unwrap();
        let metadata = tracing::callsite! {
            name: "failed span entry",
            kind: tracing::metadata::Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields:
        }
        .metadata();
        let values = metadata.fields().value_set(&[]);
        let id = subscriber.new_span(&tracing_core::span::Attributes::new_root(metadata, &values));
        let result = THREAD_STATE.with(|state| {
            let _entered = state.entered.borrow_mut();
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subscriber.enter(&id)))
        });
        assert!(result.is_err());
        assert!(subscriber.current_span().id().is_none());
        assert!(subscriber.try_close(id));
    }

    #[test]
    fn recording_works_when_the_entered_cache_is_borrowed() {
        let subscriber = crate::LayerBuilder::new("BorrowedCacheTests")
            .build_subscriber()
            .unwrap();
        let metadata = tracing::callsite! {
            name: "uncached ID operations",
            kind: tracing::metadata::Kind::SPAN,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields:
        }
        .metadata();
        let values = metadata.fields().value_set(&[]);
        let id = subscriber.new_span(&tracing_core::span::Attributes::new_root(metadata, &values));
        THREAD_STATE.with(|state| {
            let _entered = state.entered.borrow_mut();
            let cloned = subscriber.clone_span(&id);
            subscriber.record(&id, &tracing_core::span::Record::new(&values));
            assert!(!subscriber.try_close(cloned));
        });
        assert!(subscriber.try_close(id));
    }
}

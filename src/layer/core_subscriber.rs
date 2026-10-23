use alloc::vec::Vec;
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{layer::common, native::ProviderTraits, statics::get_event_metadata};

use super::*;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const ID_BATCH_SIZE: u64 = 1024;

#[cold]
fn reserve_ids(counter: &AtomicU64, count: u64) -> (u64, u64) {
    let mut id = counter.load(Ordering::Relaxed);
    loop {
        let end = id.saturating_add(count);
        assert!(end > id, "Span and subscriber IDs exhausted");
        match counter.compare_exchange_weak(id, end, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return (id, end),
            Err(current) => id = current,
        }
    }
}

pub(crate) fn next_id() -> u64 {
    reserve_ids(&NEXT_ID, 1).0
}

struct IdBatch {
    next: u64,
    end: u64,
}

impl IdBatch {
    #[inline]
    fn next(&mut self, counter: &AtomicU64) -> u64 {
        if self.next == self.end {
            (self.next, self.end) = reserve_ids(counter, ID_BATCH_SIZE);
        }
        let id = self.next;
        self.next += 1;
        id
    }
}

struct ThreadState {
    spans: Vec<common::SpanHandle>,
    ids: IdBatch,
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
    static THREAD_STATE: RefCell<ThreadState> = const {
        RefCell::new(ThreadState {
            spans: Vec::new(),
            ids: IdBatch { next: 0, end: 0 },
        })
    };
}

impl<OutMode: OutputMode> _EtwTracingSubscriber<OutMode> {
    fn current_span_context(&self) -> Option<common::SpanContext> {
        THREAD_STATE.with_borrow(|state| state.current_span_context(self.subscriber_id))
    }

    fn with_entered_span<R>(
        &self,
        id: &tracing_core::span::Id,
        f: impl FnOnce(&common::SpanHandle) -> R,
    ) -> Option<R> {
        crate::tls::try_with(&THREAD_STATE, |state| {
            let state = state.try_borrow().ok()?;
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
        let (id, parent) = THREAD_STATE.with_borrow_mut(|state| {
            let id = tracing::span::Id::from_u64(state.ids.next(&NEXT_ID));
            let parent = if attrs.is_contextual() {
                state
                    .current_span_context(self.subscriber_id)
                    .map(common::SpanContext::id)
            } else {
                attrs.parent().cloned()
            };
            (id, parent)
        });

        common::create_span_data_for_new_span(self.subscriber_id, attrs, &id, parent.as_ref());

        id
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

        let span = if event.is_contextual() {
            self.current_span_context()
                .map(|context| common::EventSpan::Cached(context.ids))
        } else {
            event.parent().map(|id| {
                self.with_entered_span(id, |handle| common::EventSpan::Cached(handle.context().ids))
                    .unwrap_or(common::EventSpan::Id(id))
            })
        };

        common::write_event(
            self.subscriber_id,
            self.provider.as_ref(),
            event,
            name,
            keyword,
            tag,
            span,
            None,
        )
    }

    fn enter(&self, id: &tracing_core::span::Id) {
        THREAD_STATE.with(|state| {
            let mut state = state.borrow_mut();
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
                    self.provider.as_ref(),
                    self.default_keyword,
                    0,
                    None,
                    register,
                );
            } else {
                common::enter_span_retained(
                    self.subscriber_id,
                    id,
                    self.provider.as_ref(),
                    self.default_keyword,
                    0,
                    None,
                    register,
                );
            }
        });
    }

    fn exit(&self, id: &tracing_core::span::Id) {
        let entered = THREAD_STATE.with_borrow_mut(|state| {
            let index = state
                .spans
                .iter()
                .rposition(|span| span.matches(self.subscriber_id, id))
                .expect("Exit of a span that is not entered on this thread");
            state.spans.remove(index)
        });
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::exit_span_cached(
            &entered,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            None,
        );
        drop(entered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_core::{Subscriber, callsite::Callsite};

    #[test]
    fn concurrent_id_batches_do_not_overlap_single_id_reservations() {
        let counter = AtomicU64::new(1);
        let mut ids = alloc::vec![reserve_ids(&counter, 1).0];
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 0..4 {
                let counter = &counter;
                workers.push(scope.spawn(move || {
                    let mut batch = IdBatch { next: 0, end: 0 };
                    (0..3 * ID_BATCH_SIZE)
                        .map(|_| batch.next(counter))
                        .collect::<Vec<_>>()
                }));
            }
            ids.push(reserve_ids(&counter, 1).0);
            for worker in workers {
                ids.extend(worker.join().unwrap());
            }
        });
        ids.sort_unstable();
        assert_eq!(
            ids,
            (1..counter.load(Ordering::Relaxed)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn final_id_batch_does_not_wrap_or_reuse_ids() {
        let counter = AtomicU64::new(u64::MAX - 3);
        let mut batch = IdBatch { next: 0, end: 0 };
        assert_eq!(batch.next(&counter), u64::MAX - 3);
        assert_eq!(batch.next(&counter), u64::MAX - 2);
        assert_eq!(batch.next(&counter), u64::MAX - 1);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| batch.next(&counter)));
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
        let result = THREAD_STATE.with_borrow_mut(|_| {
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
        THREAD_STATE.with_borrow_mut(|_| {
            let cloned = subscriber.clone_span(&id);
            subscriber.record(&id, &tracing_core::span::Record::new(&values));
            assert!(!subscriber.try_close(cloned));
        });
        assert!(subscriber.try_close(id));
    }
}

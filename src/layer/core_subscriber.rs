use alloc::vec::Vec;
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{layer::common, native::ProviderTraits, statics::get_event_metadata};

use super::*;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_id() -> u64 {
    let mut id = NEXT_ID.load(Ordering::Relaxed);
    loop {
        let next = id
            .checked_add(1)
            .expect("Span and subscriber IDs exhausted");
        match NEXT_ID.compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return id,
            Err(current) => id = current,
        }
    }
}

struct EnteredSpan {
    subscriber_id: u64,
    id: tracing_core::span::Id,
}

impl Drop for EnteredSpan {
    fn drop(&mut self) {
        common::release_span(self.subscriber_id, &self.id);
    }
}

std::thread_local! {
    static ENTERED_SPANS: RefCell<Vec<EnteredSpan>> = const { RefCell::new(Vec::new()) };
}

impl<OutMode: OutputMode> _EtwTracingSubscriber<OutMode> {
    fn current_span_id(&self) -> Option<tracing_core::span::Id> {
        ENTERED_SPANS.with_borrow(|spans| {
            spans
                .iter()
                .rev()
                .find(|span| span.subscriber_id == self.subscriber_id)
                .map(|span| span.id.clone())
        })
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
        let id = tracing::span::Id::from_u64(next_id());
        let parent = if attrs.is_contextual() {
            self.current_span_id()
        } else {
            attrs.parent().cloned()
        };

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
        self.current_span_id()
            .map_or_else(tracing_core::span::Current::none, |id| {
                tracing_core::span::Current::new(
                    id.clone(),
                    common::span_metadata(self.subscriber_id, &id),
                )
            })
    }

    fn record(&self, id: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        common::update_span_values(self.subscriber_id, id, values)
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
            self.current_span_id()
        } else {
            event.parent().cloned()
        };

        common::write_event(
            self.subscriber_id,
            self.provider.as_ref(),
            event,
            name,
            keyword,
            tag,
            span.as_ref(),
            None,
        )
    }

    fn enter(&self, id: &tracing_core::span::Id) {
        common::addref_span(self.subscriber_id, id);
        ENTERED_SPANS.with_borrow_mut(|spans| {
            spans.push(EnteredSpan {
                subscriber_id: self.subscriber_id,
                id: id.clone(),
            });
        });
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::enter_span(
            self.subscriber_id,
            id,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            None,
        );
    }

    fn exit(&self, id: &tracing_core::span::Id) {
        let entered = ENTERED_SPANS.with_borrow_mut(|spans| {
            let index = spans
                .iter()
                .rposition(|span| span.subscriber_id == self.subscriber_id && span.id == *id)
                .expect("Exit of a span that is not entered on this thread");
            spans.remove(index)
        });
        // Spans don't have callsites to store keyword/tag metadata on,
        // so we must use the defaults.
        common::exit_span(
            self.subscriber_id,
            id,
            self.provider.as_ref(),
            self.default_keyword,
            0,
            None,
        );
        drop(entered);
    }
}

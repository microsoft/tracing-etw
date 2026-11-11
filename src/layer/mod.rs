use core::{marker::PhantomData, pin::Pin};
extern crate alloc;
use alloc::sync::Arc;

use tracing_core::callsite;

use crate::{
    native::{OutputMode, ProviderTraits},
    statics::*,
};

// This struct needs to be public as it implements the tracing traits.
#[doc(hidden)]
pub struct _EtwTracingSubscriber<OutMode: OutputMode, S = ()> {
    pub(crate) provider: Pin<Arc<crate::native::Provider<OutMode>>>,
    pub(crate) default_keyword: u64,
    pub(crate) subscriber_id: u64,
    #[cfg(feature = "opentelemetry")]
    pub(crate) dispatch: registry_subscriber::Dispatches,
    // Set once this registry layer has stored span data under subscriber_id,
    // so that dropping it releases that data.
    #[cfg(feature = "std")]
    pub(crate) attached: core::sync::atomic::AtomicBool,
    pub(crate) _p: PhantomData<S>,
}

/// A tracing layer that writes events and spans as ETW events or user_events.
///
/// Use [`LayerBuilder`](crate::LayerBuilder) to construct this type.
/// With `std`, `S` is the registry subscriber used by the layer. The standalone
/// [`tracing::Subscriber`] implementation uses `S = ()`.
pub type EtwLayer<S, OutMode> = _EtwTracingSubscriber<OutMode, S>;

impl<OutMode: OutputMode, S> _EtwTracingSubscriber<OutMode, S> {
    pub(crate) fn is_enabled(
        &self,
        callsite: &callsite::Identifier,
        level: &tracing_core::Level,
    ) -> bool {
        let etw_meta = get_event_metadata(callsite);
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.default_keyword
        };

        self.provider.enabled(level, keyword)
    }
}

impl<OutMode: OutputMode, S> Clone for _EtwTracingSubscriber<OutMode, S> {
    fn clone(&self) -> Self {
        _EtwTracingSubscriber {
            provider: self.provider.clone(),
            default_keyword: self.default_keyword,
            // Standalone subscriber clones intentionally share span storage.
            // Registry layers receive a new identity when attached (on_layer).
            subscriber_id: self.subscriber_id,
            // A cloned layer captures its own dispatch when registered.
            #[cfg(feature = "opentelemetry")]
            dispatch: Default::default(),
            // Only the attached instance owns its stored span data.
            #[cfg(feature = "std")]
            attached: Default::default(),
            _p: PhantomData,
        }
    }
}

// A registry layer can be dropped while its spans are still alive, e.g. when
// it is replaced through tracing_subscriber::reload. Its span data is keyed by
// its own identity and would otherwise never be released.
#[cfg(feature = "std")]
impl<OutMode: OutputMode, S> Drop for _EtwTracingSubscriber<OutMode, S> {
    fn drop(&mut self) {
        if *self.attached.get_mut() {
            common::registry::release_subscriber(self.subscriber_id);
        }
    }
}

pub mod core_subscriber;
#[cfg(feature = "std")]
pub mod registry_subscriber;

pub(crate) mod common;

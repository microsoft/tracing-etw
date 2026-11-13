pub(crate) mod filter;

mod layer_impl;

use crate::{layer::_EtwTracingSubscriber, native::OutputMode};

/// A [`Filter`](tracing_subscriber::layer::Filter) for [`EtwLayer`](crate::EtwLayer)
/// that determines if an event or span is enabled based on whether an ETW or
/// user_events session is currently collecting events matching its level and keyword.
pub struct EtwFilter<S, OutMode: OutputMode> {
    pub(crate) layer: _EtwTracingSubscriber<OutMode, S>,
}

/// Dispatchers a registry layer has been registered with, used to look up
/// OpenTelemetry context. A subscriber can be shared by several dispatchers
/// (e.g. through `Arc`), and any one that is still alive can be used.
#[cfg(feature = "opentelemetry")]
#[derive(Default)]
pub(crate) struct Dispatches(std::sync::RwLock<std::vec::Vec<tracing::dispatcher::WeakDispatch>>);

#[cfg(feature = "opentelemetry")]
impl Dispatches {
    pub(crate) fn register(&self, dispatch: &tracing::Dispatch) {
        let mut registered = self.0.write().unwrap_or_else(|e| e.into_inner());
        registered.retain(|weak| weak.upgrade().is_some());
        registered.push(dispatch.downgrade());
    }

    pub(crate) fn upgrade(&self) -> Option<tracing::Dispatch> {
        let registered = self.0.read().unwrap_or_else(|e| e.into_inner());
        registered
            .iter()
            .find_map(tracing::dispatcher::WeakDispatch::upgrade)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    #[cfg(test)]
    pub(crate) fn registered(&self) -> std::vec::Vec<tracing::dispatcher::WeakDispatch> {
        self.0.read().unwrap().clone()
    }
}

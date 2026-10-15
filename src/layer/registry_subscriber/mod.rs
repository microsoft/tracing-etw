pub(crate) mod filter;

mod layer_impl;

use crate::{layer::_EtwTracingSubscriber, native::OutputMode};

/// A [`Filter`](tracing_subscriber::layer::Filter) for [`EtwLayer`](crate::EtwLayer)
/// that determines if an event or span is enabled based on whether an ETW or
/// user_events session is currently collecting events matching its level and keyword.
pub struct EtwFilter<S, OutMode: OutputMode> {
    pub(crate) layer: _EtwTracingSubscriber<OutMode, S>,
}

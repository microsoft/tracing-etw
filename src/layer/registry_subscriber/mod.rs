pub(crate) mod filter;

mod layer_impl;

use crate::{
    layer::_EtwTracingSubscriber,
    native::OutputMode
};

// This struct needs to be public as it implements the tracing_subscriber::Layer::Filter trait.
#[doc(hidden)]
pub struct EtwFilter<S, OutMode: OutputMode> {
    pub(crate) layer: _EtwTracingSubscriber<OutMode, S>,
}

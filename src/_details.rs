// Module for internal structs that need to be publically accessible,
// but should not be directly used by consumers of the crate.
//
// Implementations for these structs are contained in other files.

use std::{marker::PhantomData, pin::Pin, sync::Arc};

use crate::native::ProviderTypes;

// Public with public fields because the `etw_event!` macro needs to create it at invocation site.
#[doc(hidden)]
pub struct EventMetadata {
    pub kw: u64,
    pub identity: tracing::callsite::Identifier,
    pub event_tag: u32,
}

// This struct needs to be public as it implements the tracing_subscriber::Layer trait.
#[doc(hidden)]
pub struct EtwLayer<S, Mode: ProviderTypes>
where
    Mode::Provider: crate::native::EventWriter<Mode> + 'static
{
    pub(crate) provider: Pin<Arc<Mode::Provider>>,
    pub(crate) default_keyword: u64,
    pub(crate) _p: PhantomData<S>,
}

// This struct needs to be public as it implements the tracing_subscriber::Layer::Filter trait.
#[doc(hidden)]
pub struct EtwFilter<S, Mode: ProviderTypes>
where
    Mode::Provider: crate::native::EventWriter<Mode> + 'static
{
    pub(crate) provider: Pin<Arc<Mode::Provider>>,
    pub(crate) default_keyword: u64,
    pub(crate) _p: PhantomData<S>,
    pub(crate) _m: PhantomData<Mode>,
}

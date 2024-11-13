mod layer_impl;

#[cfg(any(not(feature = "global_filter"), docsrs))]
mod filter;

use std::{marker::PhantomData, pin::Pin, sync::Arc};

use tracing::Subscriber;
use tracing_core::callsite;
use tracing_subscriber::registry::LookupSpan;

use crate::{native::{EventWriter, ProviderTypes}, statics::EVENT_METADATA};

pub(crate) struct _EtwLayer<S, Mode: ProviderTypes>
where
    Mode::Provider: crate::native::EventWriter<Mode> + 'static
{
    pub(crate) provider: Pin<Arc<Mode::Provider>>,
    pub(crate) default_keyword: u64,
    pub(crate) _p: PhantomData<S>,
}

impl<S, Mode: ProviderTypes> Clone for _EtwLayer<S, Mode>
where 
    Mode::Provider: crate::native::EventWriter<Mode> + 'static
{
    fn clone(&self) -> Self {
        _EtwLayer {
            provider: self.provider.clone(),
            default_keyword: self.default_keyword,
            _p: PhantomData
        }
    }
}

// This struct needs to be public as it implements the tracing_subscriber::Layer and tracing_subscriber::Layer::Filter traits.
#[doc(hidden)]
pub struct EtwLayer<S, Mode: ProviderTypes>
where
    Mode::Provider: EventWriter<Mode> + 'static
{
    pub(crate) layer: _EtwLayer<S, Mode>
}

// This struct needs to be public as it implements the tracing_subscriber::Layer::Filter trait.
#[doc(hidden)]
#[cfg(any(not(feature = "global_filter"), docsrs))]
pub struct EtwFilter<S, Mode: ProviderTypes>
where
    Mode::Provider: EventWriter<Mode> + 'static
{
    pub(crate) layer: _EtwLayer<S, Mode>
}

impl<S, Mode> _EtwLayer<S, Mode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    Mode: ProviderTypes + 'static,
    Mode::Provider: EventWriter<Mode> + 'static,
{
    fn is_enabled(&self, callsite: &callsite::Identifier, level: &tracing_core::Level) -> bool {
        let etw_meta = EVENT_METADATA.get(callsite);
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.default_keyword
        };

        self.provider.enabled(level, keyword)
    }
}

mod layer_impl;

#[cfg(any(not(feature = "global_filter"), docsrs))]
mod filter;

use std::{marker::PhantomData, pin::Pin, sync::Arc};

use tracing::Subscriber;
use tracing_core::callsite;
use tracing_subscriber::registry::LookupSpan;

use crate::{
    native::{OutputMode, ProviderTraits},
    statics::get_event_metadata,
};

pub(crate) struct _EtwLayer<S, OutMode: OutputMode> {
    pub(crate) provider: Pin<Arc<crate::native::Provider<OutMode>>>,
    pub(crate) default_keyword: u64,
    pub(crate) _p: PhantomData<S>,
}

impl<S, OutMode: OutputMode> Clone for _EtwLayer<S, OutMode> {
    fn clone(&self) -> Self {
        _EtwLayer {
            provider: self.provider.clone(),
            default_keyword: self.default_keyword,
            _p: PhantomData,
        }
    }
}

// This struct needs to be public as it implements the tracing_subscriber::Layer and tracing_subscriber::Layer::Filter traits.
#[doc(hidden)]
pub struct EtwLayer<S, OutMode: OutputMode> {
    pub(crate) layer: _EtwLayer<S, OutMode>,
}

// This struct needs to be public as it implements the tracing_subscriber::Layer::Filter trait.
#[doc(hidden)]
#[cfg(any(not(feature = "global_filter"), docsrs))]
pub struct EtwFilter<S, OutMode: OutputMode> {
    pub(crate) layer: _EtwLayer<S, OutMode>,
}

impl<S, OutMode: OutputMode> _EtwLayer<S, OutMode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn is_enabled(&self, callsite: &callsite::Identifier, level: &tracing_core::Level) -> bool {
        let etw_meta = get_event_metadata(callsite);
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.default_keyword
        };

        self.provider.enabled(level, keyword)
    }
}

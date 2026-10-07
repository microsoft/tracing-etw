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

/// A [`Layer`](tracing_subscriber::Layer) that writes `tracing` events
/// and spans as ETW events or user_events.
///
/// Use [`LayerBuilder`](crate::LayerBuilder) to construct this type.
pub struct EtwLayer<S, OutMode: OutputMode> {
    pub(crate) layer: _EtwLayer<S, OutMode>,
}

/// A [`Filter`](tracing_subscriber::layer::Filter) for [`EtwLayer`] that
/// determines if an event or span is enabled based on whether an ETW or
/// user_events session is currently collecting events matching its level
/// and keyword.
#[cfg_attr(docsrs, doc(cfg(not(feature = "global_filter"))))]
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

use core::{marker::PhantomData, pin::Pin};
extern crate alloc;
use alloc::sync::Arc;

use tracing_core::callsite;

use crate::{native::{OutputMode, ProviderTraits}, statics::*};

// This struct needs to be public as it implements the tracing traits.
#[doc(hidden)]
pub struct _EtwTracingSubscriber<OutMode: OutputMode, S = ()> {
    pub(crate) provider: Pin<Arc<crate::native::Provider<OutMode>>>,
    pub(crate) default_keyword: u64,
    pub(crate) _p: PhantomData<S>,
}

impl<OutMode: OutputMode, S> _EtwTracingSubscriber<OutMode, S>
{
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
            _p: PhantomData,
        }
    }
}

pub mod core_subscriber;
#[cfg(any(feature = "std", docsrs))]
pub mod registry_subscriber;

pub(crate) mod common;

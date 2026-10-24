use tracing::Subscriber;
use tracing_subscriber::{layer::Filter, registry::LookupSpan};

use crate::{
    native::{OutputMode, ProviderTraits},
    statics::get_event_metadata,
};

use super::EtwFilter;

impl<S, OutMode: OutputMode> Filter<S> for EtwFilter<S, OutMode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn callsite_enabled(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        let etw_meta = get_event_metadata(&metadata.callsite());
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.layer.default_keyword
        };

        if crate::native::Provider::<OutMode>::supports_enable_callback() {
            if self.layer.provider.enabled(metadata.level(), keyword) {
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

    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.layer
            .is_enabled(&metadata.callsite(), metadata.level())
    }

    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.layer
            .is_enabled(&event.metadata().callsite(), event.metadata().level())
    }
}

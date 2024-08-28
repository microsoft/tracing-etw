use tracing::Subscriber;
use tracing_subscriber::{layer::Filter, registry::LookupSpan};

use crate::{native::{EventWriter, ProviderTypes}, statics::EVENT_METADATA};

use super::*;

impl<S, Mode> Filter<S> for EtwFilter<S, Mode>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    Mode: ProviderTypes + 'static,
    Mode::Provider: EventWriter<Mode> + 'static,
{
    fn callsite_enabled(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        let etw_meta = EVENT_METADATA.get(&metadata.callsite());
        let keyword = if let Some(meta) = etw_meta {
            meta.kw
        } else {
            self.layer.default_keyword
        };

        if Mode::supports_enable_callback() {
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
        self.layer.is_enabled(&metadata.callsite(), metadata.level())
    }

    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.layer.is_enabled(&event.metadata().callsite(), event.metadata().level())
    }
}

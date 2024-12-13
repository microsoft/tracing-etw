use std::marker::PhantomData;
#[allow(unused_imports)]
use std::{pin::Pin, sync::Arc};

#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing::metadata::LevelFilter;
use tracing::Subscriber;
#[allow(unused_imports)]
use tracing_subscriber::filter::{combinator::And, FilterExt, Filtered, Targets};
use tracing_subscriber::registry::LookupSpan;
#[allow(unused_imports)]
use tracing_subscriber::{layer::Filter, Layer};

use crate::error::EtwError;
#[cfg(any(not(feature = "global_filter"), docsrs))]
use crate::layer::EtwFilter;
use crate::layer::{EtwLayer, _EtwLayer};
use crate::native::{
    CommonSchemaOutput, EventWriter, GuidWrapper, NormalOutput, OutputMode, ProviderTraits,
};

/// Builds a [tracing_subscriber::Layer] that will logs events from a single
/// ETW or user_events provider. Use [LayerBuilder::new] to construct a new
/// builder for the given provider name. Use the `with_*` methods to set
/// additional properties for the provider, such as the keyword to use
/// for events (default: 1) or a specific provider GUID (default: a hash of
/// the provider name).
///
/// Use [LayerBuilder::new_common_schema_events] to create a layer that
/// will log events in the Common Schema 4.0 mapping. Only use this if
/// you know that you need events in this format.
///
/// Multiple `tracing_etw` layers can be created at the same time,
/// with different provider names/IDs, keywords, or output formats.
/// (Target filters)[tracing_subscriber::filter] can then be used to direct
/// specific events to specific layers.
pub struct LayerBuilder<OutMode: OutputMode> {
    provider_name: Box<str>,
    provider_id: GuidWrapper,
    provider_group: Option<crate::native::ProviderGroupType>,
    default_keyword: u64,
    _o: PhantomData<OutMode>,
}

impl LayerBuilder<NormalOutput> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: &str) -> LayerBuilder<NormalOutput> {
        LayerBuilder::<NormalOutput> {
            provider_name: name.to_string().into_boxed_str(),
            provider_id: GuidWrapper::from_name(name),
            provider_group: None,
            default_keyword: 1,
            _o: PhantomData,
        }
    }
}

impl LayerBuilder<CommonSchemaOutput> {
    /// For advanced scenarios.
    /// Emit events that follow the Common Schema 4.0 mapping.
    /// Recommended only for compatibility with specialized event consumers.
    /// Most ETW consumers will not benefit from events in this schema, and
    /// may perform worse. Common Schema events are much slower to generate
    /// and should not be enabled unless absolutely necessary.
    pub fn new_common_schema_events(name: &str) -> LayerBuilder<CommonSchemaOutput> {
        LayerBuilder::<CommonSchemaOutput> {
            provider_name: name.to_string().into_boxed_str(),
            provider_id: GuidWrapper::from_name(name),
            provider_group: None,
            default_keyword: 1,
            _o: PhantomData,
        }
    }
}

impl<OutMode: OutputMode + 'static> LayerBuilder<OutMode> {
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id<G>(mut self, guid: &G) -> Self
    where
        for<'a> &'a G: Into<GuidWrapper>,
    {
        self.provider_id = guid.into();
        self
    }

    /// Get the current provider ID that will be used for the ETW provider.
    /// This is a convenience function to help with tools that do not implement
    /// the standard provider name to ID algorithm.
    pub fn get_provider_id(&self) -> GuidWrapper {
        self.provider_id
    }

    /// Set the keyword used for events that do not explicitly set a keyword.
    ///
    /// Events logged with the [crate::etw_event!] macro specify a keyword for the event.
    /// Events and spans logged with the [tracing::event!], [tracing::span!],
    /// or other similar `tracing` macros will use the default keyword.
    ///
    /// If this method is not called, the default keyword will be `1`.
    ///
    /// Keyword value `0` is special in ETW (but not user_events), and should
    /// not be used.
    pub fn with_default_keyword(mut self, kw: u64) -> Self {
        self.default_keyword = kw;
        self
    }

    /// For advanced scenarios.
    /// Set the provider group to join this provider to.
    pub fn with_provider_group<G>(mut self, group_id: &G) -> Self
    where
        for<'a> &'a G: Into<crate::native::ProviderGroupType>,
    {
        self.provider_group = Some(group_id.into());
        self
    }

    fn validate_config(&self) -> Result<(), EtwError> {
        crate::native::Provider::<OutMode>::is_valid_provider(&self.provider_name).and_then(|_| {
            self.provider_group.as_ref().map_or_else(
                || Ok(()),
                |group| {
                    crate::native::Provider::<OutMode>::is_valid_group(&self.provider_name, group)
                },
            )
        })
    }

    #[cfg(any(not(feature = "global_filter"), docsrs))]
    fn build_target_filter(&self, target: &'static str) -> Targets {
        let mut targets = Targets::new().with_target(&*self.provider_name, LevelFilter::TRACE);

        if !target.is_empty() {
            targets = targets.with_target(target, LevelFilter::TRACE)
        }

        targets
    }

    fn build_layer<S>(&self) -> EtwLayer<S, OutMode>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        crate::native::Provider<OutMode>: EventWriter<OutMode>,
    {
        EtwLayer::<S, OutMode> {
            layer: _EtwLayer {
                provider: crate::native::Provider::<OutMode>::new(
                    &self.provider_name,
                    &self.provider_id,
                    &self.provider_group,
                    self.default_keyword,
                ),
                default_keyword: self.default_keyword,
                _p: PhantomData,
            },
        }
    }

    #[cfg(any(not(feature = "global_filter"), docsrs))]
    fn build_filter<S>(&self, layer: _EtwLayer<S, OutMode>) -> EtwFilter<S, OutMode>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        EtwFilter::<S, OutMode> { layer }
    }

    #[cfg_attr(docsrs, doc(cfg(feature = "global_filter")))]
    #[cfg(any(feature = "global_filter", docsrs))]
    pub fn build_global_filter<S>(self) -> Result<EtwLayer<S, Mode>, EtwError>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        Mode::Provider: EventWriter<Mode> + 'static,
    {
        self.validate_config()?;

        Ok(self.build_layer())
    }

    #[allow(clippy::type_complexity)]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "global_filter"))))]
    #[cfg(any(not(feature = "global_filter"), docsrs))]
    pub fn build<S>(
        self,
    ) -> Result<Filtered<EtwLayer<S, OutMode>, EtwFilter<S, OutMode>, S>, EtwError>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        crate::native::Provider<OutMode>: EventWriter<OutMode>,
    {
        self.validate_config()?;

        let layer = self.build_layer();

        let filter = self.build_filter(layer.layer.clone());

        Ok(layer.with_filter(filter))
    }

    /// Constructs the configured layer with a target [tracing_subscriber::filter] applied.
    /// This can be used to target specific events to specific layers, and in effect allow
    /// specific events to be logged only from specific ETW/user_event providers.
    #[allow(clippy::type_complexity)]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "global_filter"))))]
    #[cfg(any(not(feature = "global_filter"), docsrs))]
    pub fn build_with_target<S>(
        self,
        target: &'static str,
    ) -> Result<Filtered<EtwLayer<S, OutMode>, And<EtwFilter<S, OutMode>, Targets, S>, S>, EtwError>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        crate::native::Provider<OutMode>: EventWriter<OutMode>,
    {
        self.validate_config()?;

        let layer = self.build_layer();

        let filter = self.build_filter(layer.layer.clone());

        let targets = self.build_target_filter(target);

        Ok(layer.with_filter(filter.and(targets)))
    }
}

#[cfg(test)]
mod test {
    use tracing_subscriber::{self, prelude::*};

    use crate::native::GuidWrapper;

    use super::LayerBuilder;

    #[test]
    fn build_normal() {
        tracing_subscriber::registry()
            .with(LayerBuilder::new("test_build_normal").build().unwrap());
    }

    #[test]
    fn build_with_target() {
        tracing_subscriber::registry().with(
            LayerBuilder::new("test_build_with_target")
                .with_default_keyword(5)
                .build_with_target("asdf")
                .unwrap(),
        );
    }

    #[test]
    fn build_provider_id() {
        let provider_id = GuidWrapper::from_name("name");
        tracing_subscriber::registry().with(
            LayerBuilder::new("test_build_provider_id")
                .with_provider_id(&provider_id)
                .build()
                .unwrap(),
        );
    }
}

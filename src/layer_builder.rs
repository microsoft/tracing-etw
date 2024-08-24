use std::marker::PhantomData;
#[allow(unused_imports)]
use std::{pin::Pin, sync::Arc};

#[allow(unused_imports)] // Many imports are used exclusively by feature-gated code
use tracing::metadata::LevelFilter;
use tracing::Subscriber;
#[allow(unused_imports)]
use tracing_subscriber::filter::{combinator::And, FilterExt, Filtered, Targets};
#[allow(unused_imports)]
use tracing_subscriber::{Layer, layer::Filter};
use tracing_subscriber::registry::LookupSpan;


#[cfg(any(not(feature = "global_filter"), docsrs))]
use crate::layer::EtwFilter;
use crate::layer::{EtwLayer, _EtwLayer};
use crate::native::{EventWriter, GuidWrapper, ProviderTypes};
use crate::{error::EtwError, native};

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
pub struct LayerBuilder<Mode>
where
    Mode: ProviderTypes
{
    provider_name: String,
    provider_id: GuidWrapper,
    provider_group: Option<Mode::ProviderGroupType>,
    default_keyword: u64,
    _m: PhantomData<Mode>,
}

impl LayerBuilder<native::Provider> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: &str) -> LayerBuilder<native::Provider> {
        LayerBuilder::<native::Provider> {
            provider_name: name.to_owned(),
            provider_id: GuidWrapper::from_name(name),
            provider_group: None,
            default_keyword: 1,
            _m: PhantomData,
        }
    }
}

impl LayerBuilder<native::common_schema::Provider> {
    /// For advanced scenarios.
    /// Emit events that follow the Common Schema 4.0 mapping.
    /// Recommended only for compatibility with specialized event consumers.
    /// Most ETW consumers will not benefit from events in this schema, and
    /// may perform worse. Common Schema events are much slower to generate
    /// and should not be enabled unless absolutely necessary.
    #[cfg(any(feature = "common_schema", docsrs))]
    pub fn new_common_schema_events(
        name: &str,
    ) -> LayerBuilder<native::common_schema::Provider> {
        LayerBuilder::<native::common_schema::Provider> {
            provider_name: name.to_owned(),
            provider_id: GuidWrapper::from_name(name),
            provider_group: None,
            default_keyword: 1,
            _m: PhantomData,
        }
    }
}

impl<Mode> LayerBuilder<Mode>
where
    Mode: ProviderTypes + 'static,
{
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id<G>(mut self, guid: &G) -> Self
    where
        for<'a> &'a G: Into<GuidWrapper>
    {
        self.provider_id = guid.into();
        self
    }

    /// Get the current provider ID that will be used for the ETW provider.
    /// This is a convenience function to help with tools that do not implement
    /// the standard provider name to ID algorithm.
    pub fn get_provider_id(&self) -> GuidWrapper {
        self.provider_id.clone()
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
        for <'a> &'a G: Into<Mode::ProviderGroupType>,
    {
        self.provider_group = Some(group_id.into());
        self
    }

    fn validate_config(&self) -> Result<(), EtwError> {
        #[cfg(target_os = "linux")]
        {
            if self
                .provider_name
                .contains(|f: char| !f.is_ascii_alphanumeric() && f != '_')
            {
                // The perf command is very particular about the provider names it accepts.
                // The Linux kernel itself cares less, and other event consumers should also presumably not need this check.
                return Err(EtwError::InvalidProviderNameCharacters(self.provider_name.clone()));
            }

            let group_name_len = match &self.provider_group {
                None => 0,
                Some(ref name) => Mode::get_provider_group(&name).as_ref().len()
            };

            if self.provider_name.len() + group_name_len >= 234 {
                return Err(EtwError::TooManyCharacters(self.provider_name.len() + group_name_len));
            }
        }

        match &self.provider_group {
            None => Ok(()),
            Some(value) => Mode::is_valid(value)
        }
    }

    #[cfg(any(not(feature = "global_filter"), docsrs))]
    fn build_target_filter(&self, target: &'static str) -> Targets {
        let mut targets = Targets::new().with_target(&self.provider_name, LevelFilter::TRACE);

        #[cfg(target_os = "linux")]
        match self.provider_group {
            None => {}
            Some(ref name) => {
                targets = targets.with_target(Mode::get_provider_group(name).as_ref(), LevelFilter::TRACE);
            }
        }

        if !target.is_empty() {
            targets = targets.with_target(target, LevelFilter::TRACE)
        }

        targets
    }

    fn build_layer<S>(&self) -> EtwLayer<S, Mode>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        Mode::Provider: EventWriter<Mode> + 'static,
    {
        EtwLayer::<S, Mode> {
            layer: _EtwLayer {
                provider: Mode::Provider::new(
                    &self.provider_name,
                    &self.provider_id,
                    &self.provider_group,
                    self.default_keyword,
                ),
                default_keyword: self.default_keyword,
                _p: PhantomData,
            }
        }
    }

    #[cfg(any(not(feature = "global_filter"), docsrs))]
    fn build_filter<S>(&self, layer: _EtwLayer<S, Mode>) -> EtwFilter<S, Mode>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        Mode::Provider: EventWriter<Mode> + 'static,
    {
        EtwFilter::<S, Mode> {
            layer
        }
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
    pub fn build<S>(self) -> Result<Filtered<EtwLayer<S, Mode>, EtwFilter<S, Mode>, S>, EtwError>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        Mode::Provider: EventWriter<Mode> + 'static,
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
    ) -> Result<Filtered<EtwLayer<S, Mode>, And<EtwFilter<S, Mode>, Targets, S>, S>, EtwError>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        Mode::Provider: EventWriter<Mode> + 'static,
    {
        self.validate_config()?;

        let layer = self.build_layer();

        let filter = self.build_filter(layer.layer.clone());

        let targets = self.build_target_filter(target);

        Ok(layer.with_filter(filter.and(targets)))
    }
}

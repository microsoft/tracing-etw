use crate::values::*;
use crate::statics::GLOBAL_ACTIVITY_SEED;
use crate::error::EtwError;
use eventheader::*;
use eventheader_dynamic::EventBuilder;
use std::{cell::RefCell, ops::DerefMut, pin::Pin, sync::Arc, time::SystemTime};
use tracing_subscriber::registry::{LookupSpan, SpanRef};

extern "C" {
    #[link_name = "__start__etw_kw"]
    pub(crate) static mut _start__etw_kw: usize;
    #[link_name = "__stop__etw_kw"]
    pub(crate) static mut _stop__etw_kw: usize;
}

#[link_section = "_etw_kw"]
#[used]
static mut ETW_META_PTR: *const crate::_details::EventMetadata = core::ptr::null();

thread_local! {static EBW: std::cell::RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

impl<T> AddFieldAndValue<T> for &'_ mut eventheader_dynamic::EventBuilder {
    fn add_field_value(&mut self, fv: &FieldAndValue) {
        match fv.value {
            ValueTypes::None => (),
            ValueTypes::v_u64(u) => {
                self.add_value(fv.field_name, *u, FieldFormat::Default, 0);
            }
            ValueTypes::v_i64(i) => {
                self.add_value(fv.field_name, *i, FieldFormat::SignedInt, 0);
            }
            ValueTypes::v_u128(u) => {
                self.add_value(fv.field_name, u.to_le_bytes(), FieldFormat::Default, 0);
            }
            ValueTypes::v_i128(i) => {
                self.add_value(fv.field_name, i.to_le_bytes(), FieldFormat::Default, 0);
            }
            ValueTypes::v_f64(f) => {
                self.add_value(fv.field_name, *f, FieldFormat::Float, 0);
            }
            ValueTypes::v_bool(b) => {
                self.add_value(fv.field_name, *b, FieldFormat::Boolean, 0);
            }
            ValueTypes::v_str(ref s) => {
                self.add_str(fv.field_name, s.as_ref(), FieldFormat::Default, 0);
            }
            ValueTypes::v_char(c) => {
                self.add_value(fv.field_name, *c, FieldFormat::StringUtf, 0);
            }
        }
    }
}

#[doc(hidden)]
pub struct Provider {
    provider: std::sync::RwLock<eventheader_dynamic::Provider>,
}

impl crate::native::ProviderTypes for Provider {
    type Provider = Self;
    type ProviderGroupType = std::borrow::Cow<'static, str>;

    #[inline(always)]
    fn supports_enable_callback() -> bool {
        false
    }

    fn is_valid(value: &Self::ProviderGroupType) -> Result<(), EtwError> {
        if !eventheader_dynamic::ProviderOptions::is_valid_option_value(value) {
            Err(EtwError::InvalidProviderGroupCharacters(value.clone().into()))
        }
        else
        {
            Ok(())
        }
    }

    fn get_provider_group(value: &Self::ProviderGroupType) -> impl AsRef<str> {
        value.clone()
    }
}

impl Provider {
    fn find_set(
        self: Pin<&Self>,
        level: eventheader_dynamic::Level,
        keyword: u64,
    ) -> Option<Arc<eventheader_dynamic::EventSet>> {
        self.get_provider().read().unwrap().find_set(level, keyword)
    }

    fn register_set(
        self: Pin<&Self>,
        level: eventheader_dynamic::Level,
        keyword: u64,
    ) -> Arc<eventheader_dynamic::EventSet> {
        self.get_provider()
            .write()
            .unwrap()
            .register_set(level, keyword)
    }

    fn get_provider(self: Pin<&Self>) -> Pin<&std::sync::RwLock<eventheader_dynamic::Provider>> {
        unsafe { self.map_unchecked(|s| &s.provider) }
    }

    #[inline]
    const fn map_level(level: &tracing_core::Level) -> eventheader_dynamic::Level {
        match *level {
            tracing_core::Level::ERROR => eventheader_dynamic::Level::Error,
            tracing_core::Level::WARN => eventheader_dynamic::Level::Warning,
            tracing_core::Level::INFO => eventheader_dynamic::Level::Informational,
            tracing_core::Level::DEBUG => eventheader_dynamic::Level::Verbose,
            tracing_core::Level::TRACE => eventheader_dynamic::Level::from_int(eventheader_dynamic::Level::Verbose.as_int() + 1),
        }
    }
}

impl crate::native::EventWriter<Provider> for Provider {
    fn new<G>(
        provider_name: &str,
        _: &G,
        provider_group: &Option<<Self as crate::native::ProviderTypes>::ProviderGroupType>,
        default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<crate::native::GuidWrapper>,
    {
        let mut options = eventheader_dynamic::Provider::new_options();
        if let Some(ref name) = provider_group {
            options = *options.group_name(name);
        }
        let mut provider = eventheader_dynamic::Provider::new(provider_name, &options);

        // Keywords are static, but levels are dynamic so we have to register them all
        for event in crate::statics::event_metadata() {
            provider.register_set(
                Self::map_level(&tracing::Level::ERROR),
                event.kw,
            );
            provider.register_set(
                Self::map_level(&tracing::Level::WARN),
                event.kw,
            );
            provider.register_set(
                Self::map_level(&tracing::Level::INFO),
                event.kw,
            );
            provider.register_set(
                Self::map_level(&tracing::Level::DEBUG),
                event.kw,
            );
            provider.register_set(
                Self::map_level(&tracing::Level::TRACE),
                event.kw,
            );
        }

        provider.register_set(
            Self::map_level(&tracing::Level::ERROR),
            default_keyword,
        );
        provider.register_set(
            Self::map_level(&tracing::Level::WARN),
            default_keyword,
        );
        provider.register_set(
            Self::map_level(&tracing::Level::INFO),
            default_keyword,
        );
        provider.register_set(
            Self::map_level(&tracing::Level::DEBUG),
            default_keyword,
        );
        provider.register_set(
            Self::map_level(&tracing::Level::TRACE),
            default_keyword,
        );

        Arc::pin(Provider {
            provider: std::sync::RwLock::new(provider),
        })
    }

    #[inline]
    fn enabled(&self, level: &tracing_core::Level, keyword: u64) -> bool {
        let es = self
            .provider
            .read()
            .unwrap()
            .find_set(Self::map_level(level), keyword);
        if let Some(s) = es { s.enabled() } else { false }
    }

    fn span_start<'a, 'b, R>(
        self: Pin<&Self>,
        span: &'b SpanRef<'a, R>,
        timestamp: SystemTime,
        activity_id: &[u8; 16],
        related_activity_id: &[u8; 16],
        fields: &'b [crate::values::FieldValueIndex],
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
    ) where
        R: LookupSpan<'a>,
    {
        let span_name = span.name();

        let es = if let Some(es) = self.find_set(Self::map_level(level), keyword) {
            es
        } else {
            self.register_set(Self::map_level(level), keyword)
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(span_name, event_tag as u16);
            eb.opcode(Opcode::ActivityStart);

            eb.add_value(
                "start time",
                timestamp
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                FieldFormat::Time,
                0,
            );

            for f in fields {
                <&mut EventBuilder as AddFieldAndValue<EventBuilder>>::add_field_value(
                    &mut eb.deref_mut(),
                    &FieldAndValue {
                        field_name: f.field,
                        value: &f.value,
                    },
                );
            }

            let _ = eb.write(
                &es,
                if activity_id[0] != 0 {
                    Some(activity_id)
                } else {
                    None
                },
                if related_activity_id[0] != 0 {
                    Some(related_activity_id)
                } else {
                    None
                },
            );
        });
    }

    fn span_stop<'a, 'b, R>(
        self: Pin<&Self>,
        span: &'b SpanRef<'a, R>,
        start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        activity_id: &[u8; 16],
        related_activity_id: &[u8; 16],
        fields: &'b [crate::values::FieldValueIndex],
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
    ) where
        R: LookupSpan<'a>,
    {
        let span_name = span.name();

        let es = if let Some(es) = self.find_set(Self::map_level(level), keyword) {
            es
        } else {
            self.register_set(Self::map_level(level), keyword)
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(span_name, event_tag as u16);
            eb.opcode(Opcode::ActivityStop);

            eb.add_value(
                "stop time",
                start_stop_times
                    .1
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                FieldFormat::Time,
                0,
            );

            for f in fields {
                <&mut EventBuilder as AddFieldAndValue<EventBuilder>>::add_field_value(
                    &mut eb.deref_mut(),
                    &FieldAndValue {
                        field_name: f.field,
                        value: &f.value,
                    },
                );
            }

            let _ = eb.write(
                &es,
                if activity_id[0] != 0 {
                    Some(activity_id)
                } else {
                    None
                },
                if related_activity_id[0] != 0 {
                    Some(related_activity_id)
                } else {
                    None
                },
            );
        });
    }

    fn write_record(
        self: Pin<&Self>,
        timestamp: SystemTime,
        current_span: u64,
        parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
    ) {
        let es = if let Some(es) = self.find_set(Self::map_level(level), keyword) {
            es
        } else {
            self.register_set(Self::map_level(level), keyword)
        };

        let mut activity_id: [u8; 16] = *GLOBAL_ACTIVITY_SEED;
        activity_id[0] = if current_span != 0 {
            let (_, half) = activity_id.split_at_mut(8);
            half.copy_from_slice(&current_span.to_le_bytes());
            1
        } else {
            0
        };

        let mut related_activity_id: [u8; 16] = *GLOBAL_ACTIVITY_SEED;
        related_activity_id[0] = if parent_span != 0 {
            let (_, half) = related_activity_id.split_at_mut(8);
            half.copy_from_slice(&parent_span.to_le_bytes());
            1
        } else {
            0
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(event_name, event_tag as u16);
            eb.opcode(Opcode::Info);

            eb.add_value(
                "time",
                timestamp
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                FieldFormat::Time,
                0,
            );

            let mut visitor = VisitorWrapper::from(eb.deref_mut());
            event.record(&mut visitor);

            let _ = eb.write(
                &es,
                if activity_id[0] != 0 {
                    Some(&activity_id)
                } else {
                    None
                },
                if related_activity_id[0] != 0 {
                    Some(&related_activity_id)
                } else {
                    None
                },
            );
        });
    }
}

use crate::{error::EtwError, values::*};
use crate::statics::GLOBAL_ACTIVITY_SEED;
use chrono::{Datelike, Timelike};
use std::{cell::RefCell, ops::DerefMut, pin::Pin, sync::Arc, time::SystemTime};
use tracelogging::*;
use tracelogging_dynamic::EventBuilder;
use tracing_subscriber::registry::{LookupSpan, SpanRef};

// Items within this .rsdata section will be sorted alphabetically, thus the start is named with "0", the end "9", and each metadata "5".
// If these statics aren't mut then everything will silently fail to work.
#[allow(non_upper_case_globals)]
#[link_section = ".rsdata$zRSETW0"]
pub(crate) static mut _start__etw_kw: usize = 0;
#[allow(non_upper_case_globals)]
#[link_section = ".rsdata$zRSETW9"]
pub(crate) static mut _stop__etw_kw: usize = 0;

thread_local! {static EBW: std::cell::RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

struct Win32SystemTime {
    st: [u16; 8],
}

impl From<std::time::SystemTime> for Win32SystemTime {
    fn from(value: std::time::SystemTime) -> Self {
        let dt: chrono::DateTime<chrono::Utc> = chrono::DateTime::from(value);

        Win32SystemTime {
            st: [
                dt.year() as u16,
                dt.month() as u16,
                0,
                dt.day() as u16,
                dt.hour() as u16,
                dt.minute() as u16,
                dt.second() as u16,
                (dt.nanosecond() / 1000000) as u16,
            ],
        }
    }
}

impl<T> AddFieldAndValue<T> for &'_ mut tracelogging_dynamic::EventBuilder {
    fn add_field_value(&mut self, fv: &FieldAndValue) {
        match fv.value {
            ValueTypes::None => (),
            ValueTypes::v_u64(u) => {
                self.add_u64(fv.field_name, *u, OutType::Default, 0);
            }
            ValueTypes::v_i64(i) => {
                self.add_i64(fv.field_name, *i, OutType::Default, 0);
            }
            ValueTypes::v_u128(u) => {
                self.add_binary(fv.field_name, u.to_le_bytes(), OutType::Default, 0);
            }
            ValueTypes::v_i128(i) => {
                self.add_binary(fv.field_name, i.to_le_bytes(), OutType::Default, 0);
            }
            ValueTypes::v_f64(f) => {
                self.add_f64(fv.field_name, *f, OutType::Default, 0);
            }
            ValueTypes::v_bool(b) => {
                self.add_bool32(fv.field_name, *b as i32, OutType::Default, 0);
            }
            ValueTypes::v_str(ref s) => {
                self.add_str8(fv.field_name, s.as_ref(), OutType::Utf8, 0);
            }
            ValueTypes::v_char(c) => {
                // Or add_str16 with a 1-char (BMP) or 2-char (surrogate-pair) string.
                self.add_u16(fv.field_name, *c as u16, OutType::String, 0);
            }
        }
    }
}

fn callback_fn(
    _source_id: &Guid,
    _event_control_code: u32,
    _level: Level,
    _match_any_keyword: u64,
    _match_all_keyword: u64,
    _filter_data: usize,
    _callback_context: usize,
) {
    // Every time the enablement changes, reset the event-enabled cache
    tracing::callsite::rebuild_interest_cache();
}

#[doc(hidden)]
pub struct Provider {
    provider: tracelogging_dynamic::Provider,
}

impl crate::native::ProviderTypes for Provider {
    type Provider = Self;
    type ProviderGroupType = crate::native::native_guid;

    #[inline(always)]
    fn supports_enable_callback() -> bool {
        true
    }

    fn is_valid(value: &Self::ProviderGroupType) -> Result<(), EtwError> {
        if value == &crate::native::native_guid::zero() {
            Err(EtwError::EmptyProviderGroupGuid)
        }
        else {
            Ok(())
        }
    }
}

impl Provider {
    #[inline(always)]
    fn get_provider(self: Pin<&Self>) -> Pin<&tracelogging_dynamic::Provider> {
        unsafe { self.map_unchecked(|s| &s.provider) }
    }

    #[inline]
    const fn map_level(level: &tracing_core::Level) -> tracelogging::Level {
        match *level {
            tracing_core::Level::ERROR => tracelogging::Level::Error,
            tracing_core::Level::WARN => tracelogging::Level::Warning,
            tracing_core::Level::INFO => tracelogging::Level::Informational,
            tracing_core::Level::DEBUG => tracelogging::Level::Verbose,
            tracing_core::Level::TRACE => tracelogging::Level::from_int(tracelogging::Level::Verbose.as_int() + 1),
        }
    }
}

impl super::EventWriter<Provider> for Provider {
    fn new<G>(
        provider_name: &str,
        provider_id: &G,
        provider_group: &Option<<Self as crate::native::ProviderTypes>::ProviderGroupType>,
        _default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<crate::native::GuidWrapper>,
    {
        let mut options = tracelogging_dynamic::Provider::options();
        if let Some(guid) = provider_group {
            options.group_id(guid);
        }

        options.callback(callback_fn, 0);

        let wrapper = Arc::pin(Self {
            provider: tracelogging_dynamic::Provider::new_with_id(
                provider_name,
                &options,
                &provider_id.into().into(),
            ),
        });
        unsafe {
            wrapper.as_ref().get_provider().register();
        }

        wrapper
    }

    #[inline]
    fn enabled(&self, level: &tracing_core::Level, keyword: u64) -> bool {
        self.provider
            .enabled(Self::map_level(level), keyword)
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

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(span_name, Self::map_level(level), keyword, event_tag);
            eb.opcode(Opcode::Start);

            eb.add_systemtime(
                "start time",
                &Into::<Win32SystemTime>::into(timestamp).st,
                OutType::DateTimeUtc,
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

            let act = tracelogging_dynamic::Guid::from_bytes_le(activity_id);
            let related = tracelogging_dynamic::Guid::from_bytes_le(related_activity_id);
            let _ = eb.write(
                &self.get_provider(),
                if activity_id[0] != 0 {
                    Some(&act)
                } else {
                    None
                },
                if related_activity_id[0] != 0 {
                    Some(&related)
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

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(span_name, Self::map_level(level), keyword, event_tag);
            eb.opcode(Opcode::Stop);

            eb.add_systemtime(
                "stop time",
                &Into::<Win32SystemTime>::into(start_stop_times.1).st,
                OutType::DateTimeUtc,
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

            let act = tracelogging_dynamic::Guid::from_bytes_le(activity_id);
            let related = tracelogging_dynamic::Guid::from_bytes_le(related_activity_id);
            let _ = eb.write(
                &self.get_provider(),
                if activity_id[0] != 0 {
                    Some(&act)
                } else {
                    None
                },
                if related_activity_id[0] != 0 {
                    Some(&related)
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

            eb.reset(event_name, Self::map_level(level), keyword, event_tag);
            eb.opcode(Opcode::Info);

            eb.add_systemtime(
                "time",
                &Into::<Win32SystemTime>::into(timestamp).st,
                OutType::DateTimeUtc,
                0,
            );

            event.record(&mut VisitorWrapper::from(eb.deref_mut()));

            let act = tracelogging_dynamic::Guid::from_bytes_le(&activity_id);
            let related = tracelogging_dynamic::Guid::from_bytes_le(&related_activity_id);
            let _ = eb.write(
                &self.get_provider(),
                if activity_id[0] != 0 {
                    Some(&act)
                } else {
                    None
                },
                if related_activity_id[0] != 0 {
                    Some(&related)
                } else {
                    None
                },
            );
        });
    }
}

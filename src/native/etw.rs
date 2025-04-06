use core::{cell::RefCell, marker::PhantomData, ops::DerefMut, pin::Pin};
extern crate alloc;
use alloc::{string::String, sync::Arc};

use chrono::{Datelike, Timelike};
use tracelogging::*;
use tracelogging_dynamic::EventBuilder;

use crate::{
    error::EtwError,
    native::{CommonSchemaOutput, NormalOutput, OutputMode},
    statics::GLOBAL_ACTIVITY_SEED,
    values::{event_values::*, *},
};

// Items within this .rdata section will be sorted alphabetically, thus the start is named with "0", the end "9", and each metadata "5".
// If these statics aren't mut then everything will silently fail to work.
#[allow(non_upper_case_globals)]
#[unsafe(link_section = ".rdata$zRSETW0")]
pub(crate) static mut _start__etw_kw: usize = 0;
#[allow(non_upper_case_globals)]
#[unsafe(link_section = ".rdata$zRSETW9")]
pub(crate) static mut _stop__etw_kw: usize = 0;

pub(crate) type ProviderGroupType = crate::native::native_guid;

thread_local! {static EBW: RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

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

impl AddFieldAndValue for &'_ mut tracelogging_dynamic::EventBuilder {
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
            ValueTypes::v_str(s) => {
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
pub struct Provider<Mode: OutputMode> {
    provider: tracelogging_dynamic::Provider,
    _mode: PhantomData<Mode>,
}

impl<Mode: OutputMode> crate::native::ProviderTraits for Provider<Mode> {
    #[inline(always)]
    fn supports_enable_callback() -> bool {
        true
    }

    fn is_valid_provider(_provider_name: &str) -> Result<(), EtwError> {
        Ok(())
    }

    fn is_valid_group(_provider_name: &str, value: &ProviderGroupType) -> Result<(), EtwError> {
        if value == &crate::native::native_guid::zero() {
            Err(EtwError::EmptyProviderGroupGuid)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn enabled(&self, level: &tracing_core::Level, keyword: u64) -> bool {
        self.provider.enabled(Self::map_level(level), keyword)
    }

    fn new<G>(
        provider_name: &str,
        provider_id: &G,
        provider_group: &Option<ProviderGroupType>,
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
            _mode: PhantomData,
        });
        unsafe {
            wrapper.as_ref().get_provider().register();
        }

        wrapper
    }
}

impl<Mode: OutputMode> Provider<Mode> {
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
            tracing_core::Level::TRACE => {
                tracelogging::Level::from_int(tracelogging::Level::Verbose.as_int() + 1)
            }
        }
    }
}

impl<Mode: OutputMode> super::EventWriter<NormalOutput> for Provider<Mode> {
    fn span_start<'a, 'b>(
        self: Pin<&Self>,
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    ) {
        EBW.with_borrow_mut(|mut eb| {
            eb.reset(
                data.name(),
                Self::map_level(&data.level()),
                keyword,
                event_tag,
            );
            eb.opcode(Opcode::Start);

            eb.add_systemtime(
                "start time",
                &Into::<Win32SystemTime>::into(data.timestamp()).st,
                OutType::DateTimeUtc,
                0,
            );

            for f in data.fields() {
                <&mut EventBuilder as AddFieldAndValue>::add_field_value(
                    &mut eb.deref_mut(),
                    &FieldAndValue {
                        field_name: f.field,
                        value: &f.value,
                    },
                );
            }

            let act = tracelogging_dynamic::Guid::from_bytes_le(data.activity_id());
            let related = tracelogging_dynamic::Guid::from_bytes_le(data.related_activity_id());
            let _ = eb.write(
                &self.get_provider(),
                if data.activity_id()[0] != 0 {
                    Some(&act)
                } else {
                    None
                },
                if data.related_activity_id()[0] != 0 {
                    Some(&related)
                } else {
                    None
                },
            );
        });
    }

    fn span_stop<'a, 'b>(
        self: Pin<&Self>,
        start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    ) {
        EBW.with_borrow_mut(|mut eb| {
            eb.reset(
                data.name(),
                Self::map_level(&data.level()),
                keyword,
                event_tag,
            );
            eb.opcode(Opcode::Stop);

            eb.add_systemtime(
                "stop time",
                &Into::<Win32SystemTime>::into(start_stop_times.1).st,
                OutType::DateTimeUtc,
                0,
            );

            for f in data.fields() {
                <&mut EventBuilder as AddFieldAndValue>::add_field_value(
                    &mut eb.deref_mut(),
                    &FieldAndValue {
                        field_name: f.field,
                        value: &f.value,
                    },
                );
            }

            let act = tracelogging_dynamic::Guid::from_bytes_le(data.activity_id());
            let related = tracelogging_dynamic::Guid::from_bytes_le(data.related_activity_id());
            let _ = eb.write(
                &self.get_provider(),
                if data.activity_id()[0] != 0 {
                    Some(&act)
                } else {
                    None
                },
                if data.related_activity_id()[0] != 0 {
                    Some(&related)
                } else {
                    None
                },
            );
        });
    }

    fn write_record(
        self: Pin<&Self>,
        timestamp: std::time::SystemTime,
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

        EBW.with_borrow_mut(|mut eb| {
            eb.reset(event_name, Self::map_level(level), keyword, event_tag);
            eb.opcode(Opcode::Info);

            eb.add_systemtime(
                "time",
                &Into::<Win32SystemTime>::into(timestamp).st,
                OutType::DateTimeUtc,
                0,
            );

            event.record(&mut EventBuilderVisitorWrapper::from(eb.deref_mut()));

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

struct CommonSchemaPartCBuilder<'a> {
    eb: &'a mut EventBuilder,
}

impl<'a> CommonSchemaPartCBuilder<'a> {
    fn make_visitor(
        eb: &'a mut EventBuilder,
    ) -> EventBuilderVisitorWrapper<CommonSchemaPartCBuilder<'a>> {
        EventBuilderVisitorWrapper::from(CommonSchemaPartCBuilder { eb })
    }
}

impl AddFieldAndValue for CommonSchemaPartCBuilder<'_> {
    fn add_field_value(&mut self, fv: &FieldAndValue) {
        let mut field_name: &'static str = fv.field_name;

        if field_name == "message" {
            field_name = "Body";
            assert!(matches!(fv.value, ValueTypes::v_str(_)));
        }

        <&mut EventBuilder as AddFieldAndValue>::add_field_value(
            &mut self.eb,
            &FieldAndValue {
                field_name,
                value: fv.value,
            },
        );
    }
}

impl<Mode: OutputMode> super::EventWriter<CommonSchemaOutput> for Provider<Mode> {
    fn span_start<'a, 'b>(
        self: Pin<&Self>,
        _data: crate::layer::common::SpanRef,
        _keyword: u64,
        _event_tag: u32,
    ) {
    }

    fn span_stop<'a, 'b>(
        self: Pin<&Self>,
        start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        data: crate::layer::common::SpanRef,
        keyword: u64,
        event_tag: u32,
    ) {
        // We need a UTF-8 rather than raw bytes, so we can't use data.activity_id() here
        let span_id = super::to_hex_utf8_bytes(data.id());

        EBW.with_borrow_mut(|mut eb| {
            eb.reset(
                data.name(),
                Self::map_level(&data.level()),
                keyword,
                event_tag,
            );
            eb.opcode(Opcode::Info);

            // Promoting values from PartC to PartA extensions is apparently just a draft spec
            // and not necessary / supported by consumers.
            // let exts = json::extract_common_schema_parta_exts(attributes);

            eb.add_u16("__csver__", 0x0401, OutType::Signed, 0);
            eb.add_struct("PartA", 2 /* + exts.len() as u8*/, 0);
            {
                let time: String = chrono::DateTime::to_rfc3339(
                    &chrono::DateTime::<chrono::Utc>::from(start_stop_times.1),
                );
                eb.add_str8("time", time, OutType::Utf8, 0);

                eb.add_struct("ext_dt", 2, 0);
                {
                    eb.add_str8("traceId", "", OutType::Utf8, 0); // TODO
                    eb.add_str8("spanId", span_id, OutType::Utf8, 0);
                }
            }

            // if !span_data.links.is_empty() {
            //     self.add_struct("PartB", 5, 0);
            //     {
            //         self.add_str8("_typeName", "SpanLink", OutType::Utf8, 0);
            //         self.add_str8("fromTraceId", &traceId, OutType::Utf8, 0);
            //         self.add_str8("fromSpanId", &spanId, OutType::Utf8, 0);
            //         self.add_str8("toTraceId", "SpanLink", OutType::Utf8, 0);
            //         self.add_str8("toSpanId", "SpanLink", OutType::Utf8, 0);
            //     }
            // }

            let parent_span = data.parent();
            let partb_field_count = 3 + if parent_span.is_some() { 1 } else { 0 };

            eb.add_struct("PartB", partb_field_count, 0);
            {
                eb.add_str8("_typeName", "Span", OutType::Utf8, 0);

                if let Some(id) = parent_span {
                    eb.add_str8("parentId", super::to_hex_utf8_bytes(id), OutType::Utf8, 0);
                }

                eb.add_str8("name", data.name(), OutType::Utf8, 0);

                eb.add_str8(
                    "startTime",
                    chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                        start_stop_times.0,
                    )),
                    OutType::Utf8,
                    0,
                );
            }

            let partc_field_count = data.field_count() as u8;

            eb.add_struct("PartC", partc_field_count, 0);
            {
                let mut pfv = CommonSchemaPartCBuilder { eb: eb.deref_mut() };

                for f in data.fields() {
                    <CommonSchemaPartCBuilder<'_> as AddFieldAndValue>::add_field_value(
                        &mut pfv,
                        &FieldAndValue {
                            field_name: f.field,
                            value: &f.value,
                        },
                    );
                }
            }

            let _ = eb.write(&self.get_provider(), None, None);
        });
    }

    fn write_record(
        self: Pin<&Self>,
        timestamp: std::time::SystemTime,
        current_span: u64,
        _parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
    ) {
        EBW.with_borrow_mut(|mut eb| {
            eb.reset(event_name, Self::map_level(level), keyword, event_tag);
            eb.opcode(Opcode::Info);

            // Promoting values from PartC to PartA extensions is apparently just a draft spec
            // and not necessary / supported by consumers.
            // let exts = json::extract_common_schema_parta_exts(attributes);

            eb.add_u16("__csver__", 0x0401, OutType::Signed, 0);
            eb.add_struct(
                "PartA",
                1 + if current_span != 0 { 1 } else { 0 }, /* + exts.len() as u8*/
                0,
            );
            {
                let time: String =
                    chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(timestamp));
                eb.add_str8("time", time, OutType::Utf8, 0);

                if current_span != 0 {
                    eb.add_struct("ext_dt", 2, 0);
                    {
                        eb.add_str8("traceId", "", OutType::Utf8, 0); // TODO
                        eb.add_str8(
                            "spanId",
                            super::to_hex_utf8_bytes(current_span),
                            OutType::Utf8,
                            0,
                        );
                    }
                }
            }

            eb.add_struct("PartB", 3, 0);
            {
                eb.add_str8("_typeName", "Log", OutType::Utf8, 0);
                eb.add_str8("name", event_name, OutType::Utf8, 0);

                eb.add_str8(
                    "eventTime",
                    chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(timestamp)),
                    OutType::Utf8,
                    0,
                );
            }

            let partc_field_count = event.fields().count() as u8;

            eb.add_struct("PartC", partc_field_count, 0);
            {
                let mut visitor = CommonSchemaPartCBuilder::make_visitor(eb.deref_mut());
                event.record(&mut visitor);
            }

            let _ = eb.write(&self.get_provider(), None, None);
        });
    }
}

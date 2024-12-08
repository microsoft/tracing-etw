use crate::{
    error::EtwError,
    values::{event_values::*, *},
};
use std::{
    cell::RefCell,
    io::{Cursor, Write},
    mem::MaybeUninit,
    ops::DerefMut,
    pin::Pin,
    sync::Arc,
    time::SystemTime,
};
use tracelogging::*;
use tracelogging_dynamic::EventBuilder;
use tracing_subscriber::registry::{LookupSpan, SpanRef};

thread_local! {static EBW: std::cell::RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

pub(crate) struct CommonSchemaPartCBuilder<'a> {
    pub(crate) eb: &'a mut EventBuilder,
}

impl<'a> CommonSchemaPartCBuilder<'a> {
    fn make_visitor(eb: &'a mut EventBuilder) -> EventBuilderVisitorWrapper<CommonSchemaPartCBuilder<'a>> {
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

#[doc(hidden)]
pub struct CommonSchemaProvider {
    provider: tracelogging_dynamic::Provider,
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

impl CommonSchemaProvider {
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

impl crate::native::ProviderTypes for CommonSchemaProvider {
    type Provider = Self;
    type ProviderGroupType = crate::native::native_guid;

    #[inline(always)]
    fn supports_enable_callback() -> bool {
        true
    }

    fn is_valid(value: &Self::ProviderGroupType) -> Result<(), EtwError> {
        if value == &crate::native::native_guid::zero() {
            Err(EtwError::EmptyProviderGroupGuid)
        } else {
            Ok(())
        }
    }
}

impl crate::native::EventWriter<CommonSchemaProvider> for CommonSchemaProvider {
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
        self.provider.enabled(Self::map_level(level), keyword)
    }

    fn span_start<'a, 'b, R>(
        self: Pin<&Self>,
        _span: &'b SpanRef<'a, R>,
        _timestamp: SystemTime,
        _activity_id: &[u8; 16],
        _related_activity_id: &[u8; 16],
        _fields: &'b [crate::values::span_values::FieldValueIndex],
        _level: &tracing_core::Level,
        _keyword: u64,
        _event_tag: u32,
    ) where
        R: LookupSpan<'a>,
    {
    }

    fn span_stop<'a, 'b, R>(
        self: Pin<&Self>,
        span: &'b SpanRef<'a, R>,
        start_stop_times: (std::time::SystemTime, std::time::SystemTime),
        _activity_id: &[u8; 16],
        _related_activity_id: &[u8; 16],
        fields: &'b [crate::values::span_values::FieldValueIndex],
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
    ) where
        R: LookupSpan<'a>,
    {
        let span_name = span.name();

        let span_id = unsafe {
            let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
            let mut cur = Cursor::new((*span_id.as_mut_ptr()).as_mut_slice());
            write!(&mut cur, "{:16x}", span.id().into_u64()).expect("!write");
            span_id.assume_init()
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(span_name, Self::map_level(level), keyword, event_tag);
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

            let span_parent = span.parent();
            let partb_field_count = 3 + if span_parent.is_some() { 1 } else { 0 };

            eb.add_struct("PartB", partb_field_count, 0);
            {
                eb.add_str8("_typeName", "Span", OutType::Utf8, 0);

                if let Some(parent) = span_parent {
                    let parent_span_id = unsafe {
                        let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
                        let mut cur = Cursor::new((*span_id.as_mut_ptr()).as_mut_slice());
                        write!(&mut cur, "{:16x}", parent.id().into_u64()).expect("!write");
                        span_id.assume_init()
                    };

                    eb.add_str8("parentId", parent_span_id, OutType::Utf8, 0);
                }

                eb.add_str8("name", span_name, OutType::Utf8, 0);

                eb.add_str8(
                    "startTime",
                    chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                        start_stop_times.0,
                    )),
                    OutType::Utf8,
                    0,
                );
            }

            let partc_field_count = span.fields().len() as u8;

            eb.add_struct("PartC", partc_field_count, 0);
            {
                let mut pfv = CommonSchemaPartCBuilder { eb: eb.deref_mut() };

                for f in fields {
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
        timestamp: SystemTime,
        current_span: u64,
        _parent_span: u64,
        event_name: &str,
        level: &tracing_core::Level,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
    ) {
        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

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
                        let span_id = unsafe {
                            let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
                            let mut cur = Cursor::new((*span_id.as_mut_ptr()).as_mut_slice());
                            write!(&mut cur, "{:16x}", current_span).expect("!write");
                            span_id.assume_init()
                        };

                        eb.add_str8("traceId", "", OutType::Utf8, 0); // TODO
                        eb.add_str8("spanId", span_id, OutType::Utf8, 0);
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

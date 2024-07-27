use crate::{map_level, values::*};
use eventheader::*;
use eventheader_dynamic::EventBuilder;
use std::{
    cell::RefCell,
    io::{Cursor, Write},
    mem::MaybeUninit,
    ops::DerefMut,
    pin::Pin,
    sync::Arc,
    time::SystemTime,
};
use tracing_subscriber::registry::{LookupSpan, SpanRef};

use crate::native::ProviderGroup;

thread_local! {static EBW: std::cell::RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

pub(crate) struct CommonSchemaPartCBuilder<'a> {
    pub(crate) eb: &'a mut EventBuilder,
}

impl<'a> CommonSchemaPartCBuilder<'a> {
    fn make_visitor(eb: &'a mut EventBuilder) -> VisitorWrapper<CommonSchemaPartCBuilder<'a>> {
        VisitorWrapper::from(CommonSchemaPartCBuilder { eb })
    }
}

impl<T> AddFieldAndValue<T> for CommonSchemaPartCBuilder<'_> {
    fn add_field_value(&mut self, fv: &FieldAndValue) {
        let mut field_name: &'static str = fv.field_name;

        if field_name == "message" {
            field_name = "Body";
            assert!(matches!(fv.value, ValueTypes::v_str(_)));
        }

        <&mut EventBuilder as AddFieldAndValue<EventBuilder>>::add_field_value(
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
    provider: std::sync::RwLock<eventheader_dynamic::Provider>,
}

impl CommonSchemaProvider {
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
}

impl crate::native::EventWriter for CommonSchemaProvider {
    fn new<G>(
        provider_name: &str,
        _: &G,
        provider_group: &ProviderGroup,
        default_keyword: u64,
    ) -> Pin<Arc<Self>>
    where
        for<'a> &'a G: Into<crate::native::GuidWrapper>,
    {
        let mut options = eventheader_dynamic::Provider::new_options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(name);
        }
        let mut provider = eventheader_dynamic::Provider::new(provider_name, &options);

        for event in &*crate::EVENT_METADATA {
            provider.register_set(
                eventheader_dynamic::Level::from_int(map_level(&tracing::Level::ERROR)),
                event.kw,
            );
            provider.register_set(
                eventheader_dynamic::Level::from_int(map_level(&tracing::Level::WARN)),
                event.kw,
            );
            provider.register_set(
                eventheader_dynamic::Level::from_int(map_level(&tracing::Level::INFO)),
                event.kw,
            );
            provider.register_set(
                eventheader_dynamic::Level::from_int(map_level(&tracing::Level::DEBUG)),
                event.kw,
            );
            provider.register_set(
                eventheader_dynamic::Level::from_int(map_level(&tracing::Level::TRACE)),
                event.kw,
            );
        }

        provider.register_set(
            eventheader_dynamic::Level::from_int(map_level(&tracing::Level::ERROR)),
            default_keyword,
        );
        provider.register_set(
            eventheader_dynamic::Level::from_int(map_level(&tracing::Level::WARN)),
            default_keyword,
        );
        provider.register_set(
            eventheader_dynamic::Level::from_int(map_level(&tracing::Level::INFO)),
            default_keyword,
        );
        provider.register_set(
            eventheader_dynamic::Level::from_int(map_level(&tracing::Level::DEBUG)),
            default_keyword,
        );
        provider.register_set(
            eventheader_dynamic::Level::from_int(map_level(&tracing::Level::TRACE)),
            default_keyword,
        );

        Arc::pin(Self {
            provider: std::sync::RwLock::new(provider),
        })
    }

    #[inline]
    fn enabled(&self, level: u8, keyword: u64) -> bool {
        let es = self
            .provider
            .read()
            .unwrap()
            .find_set(eventheader_dynamic::Level::from_int(level), keyword);
        if let Some(s) = es { s.enabled() } else { false }
    }

    #[inline(always)]
    fn supports_enable_callback() -> bool {
        false
    }

    fn span_start<'a, 'b, R>(
        self: Pin<&Self>,
        _span: &'b SpanRef<'a, R>,
        _timestamp: SystemTime,
        _activity_id: &[u8; 16],
        _related_activity_id: &[u8; 16],
        _fields: &'b [crate::values::FieldValueIndex],
        _level: u8,
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
        fields: &'b [crate::values::FieldValueIndex],
        level: u8,
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

        let es = if let Some(es) = self.find_set(level.into(), keyword) {
            es
        } else {
            self.register_set(level.into(), keyword)
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(span_name, event_tag as u16);
            eb.opcode(Opcode::Info);

            // Promoting values from PartC to PartA extensions is apparently just a draft spec
            // and not necessary / supported by consumers.
            // let exts = json::extract_common_schema_parta_exts(attributes);

            eb.add_value("__csver__", 0x0401, FieldFormat::SignedInt, 0);
            eb.add_struct("PartA", 2 /* + exts.len() as u8*/, 0);
            {
                let time: String = chrono::DateTime::to_rfc3339(
                    &chrono::DateTime::<chrono::Utc>::from(start_stop_times.1),
                );
                eb.add_str("time", time, FieldFormat::Default, 0);

                eb.add_struct("ext_dt", 2, 0);
                {
                    eb.add_str("traceId", "", FieldFormat::Default, 0); // TODO
                    eb.add_str("spanId", span_id, FieldFormat::Default, 0);
                }
            }

            // if !span_data.links.is_empty() {
            //     self.add_struct("PartB", 5, 0);
            //     {
            //         self.add_str8("_typeName", "SpanLink", FieldFormat::Default, 0);
            //         self.add_str8("fromTraceId", &traceId, FieldFormat::Default, 0);
            //         self.add_str8("fromSpanId", &spanId, FieldFormat::Default, 0);
            //         self.add_str8("toTraceId", "SpanLink", FieldFormat::Default, 0);
            //         self.add_str8("toSpanId", "SpanLink", FieldFormat::Default, 0);
            //     }
            // }

            let span_parent = span.parent();
            let partb_field_count = 3 + if span_parent.is_some() { 1 } else { 0 };

            eb.add_struct("PartB", partb_field_count, 0);
            {
                eb.add_str("_typeName", "Span", FieldFormat::Default, 0);

                if let Some(parent) = span_parent {
                    let parent_span_id = unsafe {
                        let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
                        let mut cur = Cursor::new((*span_id.as_mut_ptr()).as_mut_slice());
                        write!(&mut cur, "{:16x}", parent.id().into_u64()).expect("!write");
                        span_id.assume_init()
                    };

                    eb.add_str("parentId", parent_span_id, FieldFormat::Default, 0);
                }

                eb.add_str("name", span_name, FieldFormat::Default, 0);

                eb.add_str(
                    "startTime",
                    &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                        start_stop_times.0,
                    )),
                    FieldFormat::Default,
                    0,
                );
            }

            let partc_field_count = span.fields().len() as u8;

            eb.add_struct("PartC", partc_field_count, 0);
            {
                let mut pfv = CommonSchemaPartCBuilder { eb: eb.deref_mut() };

                for f in fields {
                    <CommonSchemaPartCBuilder<'_> as AddFieldAndValue<
                        CommonSchemaPartCBuilder<'_>,
                    >>::add_field_value(
                        &mut pfv,
                        &FieldAndValue {
                            field_name: f.field,
                            value: &f.value,
                        },
                    );
                }
            }

            let _ = eb.write(&es, None, None);
        });
    }

    fn write_record(
        self: Pin<&Self>,
        timestamp: SystemTime,
        current_span: u64,
        _parent_span: u64,
        event_name: &str,
        level: u8,
        keyword: u64,
        event_tag: u32,
        event: &tracing::Event<'_>,
    ) {
        let es = if let Some(es) = self.find_set(level.into(), keyword) {
            es
        } else {
            self.register_set(level.into(), keyword)
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(event_name, event_tag as u16);
            eb.opcode(Opcode::Info);

            // Promoting values from PartC to PartA extensions is apparently just a draft spec
            // and not necessary / supported by consumers.
            // let exts = json::extract_common_schema_parta_exts(attributes);

            eb.add_value("__csver__", 0x0401, FieldFormat::SignedInt, 0);
            eb.add_struct(
                "PartA",
                1 + if current_span != 0 { 1 } else { 0 }, /* + exts.len() as u8*/
                0,
            );
            {
                let time: String =
                    chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(timestamp));
                eb.add_str("time", time, FieldFormat::Default, 0);

                if current_span != 0 {
                    eb.add_struct("ext_dt", 2, 0);
                    {
                        let span_id = unsafe {
                            let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
                            let mut cur = Cursor::new((*span_id.as_mut_ptr()).as_mut_slice());
                            write!(&mut cur, "{:16x}", current_span).expect("!write");
                            span_id.assume_init()
                        };

                        eb.add_str("traceId", "", FieldFormat::Default, 0); // TODO
                        eb.add_str("spanId", span_id, FieldFormat::Default, 0);
                    }
                }
            }

            eb.add_struct("PartB", 3, 0);
            {
                eb.add_str("_typeName", "Log", FieldFormat::Default, 0);
                eb.add_str("name", event_name, FieldFormat::Default, 0);

                eb.add_str(
                    "eventTime",
                    &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                        timestamp,
                    )),
                    FieldFormat::Default,
                    0,
                );
            }

            let partc_field_count = event.fields().count() as u8;

            eb.add_struct("PartC", partc_field_count, 0);
            {
                let mut visitor = CommonSchemaPartCBuilder::make_visitor(eb.deref_mut());
                event.record(&mut visitor);
            }

            let _ = eb.write(&es, None, None);
        });
    }
}

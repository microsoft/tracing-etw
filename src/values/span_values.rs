use core::fmt::Write;
extern crate alloc;
use alloc::string::ToString;

use tracing::field;

use crate::values::*;

#[doc(hidden)]
#[derive(Default)]
pub struct FieldValueIndex {
    pub(crate) field: &'static str,
    pub(crate) value: ValueTypes,
    pub(crate) sort_index: u8,
}

// Formats tracing values into an owned representation for the caller.
pub(crate) struct SpanValueVisitor<F> {
    pub(crate) record: F,
}

// Leave the old value in `value` so the caller can drop it after unlocking.
pub(crate) fn update_span_value(
    fields: &mut [FieldValueIndex],
    field_name: &'static str,
    value: &mut ValueTypes,
) {
    let res = fields.binary_search_by_key(&field_name, |idx| fields[idx.sort_index as usize].field);
    if let Ok(idx) = res {
        let index = fields[idx].sort_index as usize;
        core::mem::swap(&mut fields[index].value, value);
    }
}

impl<F: FnMut(&'static str, ValueTypes)> field::Visit for SpanValueVisitor<F> {
    fn record_debug(&mut self, field: &field::Field, value: &dyn core::fmt::Debug) {
        // Format on the stack, then copy once into an exactly sized string
        // instead of growing a heap buffer.
        let mut string = FormatBuffer::<FORMAT_INLINE_CAPACITY>::new();
        if write!(string, "{:?}", value).is_err() {
            return;
        }

        (self.record)(
            field.name(),
            ValueTypes::v_str(Cow::from(string.into_string())),
        );
    }

    fn record_f64(&mut self, field: &field::Field, value: f64) {
        (self.record)(field.name(), ValueTypes::v_f64(value));
    }

    fn record_i64(&mut self, field: &field::Field, value: i64) {
        (self.record)(field.name(), ValueTypes::v_i64(value));
    }

    fn record_u64(&mut self, field: &field::Field, value: u64) {
        (self.record)(field.name(), ValueTypes::v_u64(value));
    }

    fn record_i128(&mut self, field: &field::Field, value: i128) {
        (self.record)(field.name(), ValueTypes::v_i128(value));
    }

    fn record_u128(&mut self, field: &field::Field, value: u128) {
        (self.record)(field.name(), ValueTypes::v_u128(value));
    }

    fn record_bool(&mut self, field: &field::Field, value: bool) {
        (self.record)(field.name(), ValueTypes::v_bool(value));
    }

    fn record_str(&mut self, field: &field::Field, value: &str) {
        (self.record)(
            field.name(),
            ValueTypes::v_str(Cow::from(value.to_string())),
        );
    }

    // record_error is intentionally not overridden. tracing-core declares it only
    // with its own `std` feature, which feature unification can enable regardless
    // of this crate's features. Its default records errors as Display strings.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::event_values::{AddFieldAndValue, EventBuilderVisitorWrapper};
    use alloc::vec::Vec;
    use tracing::callsite::Callsite;

    #[derive(Debug)]
    struct TestError;

    impl core::fmt::Display for TestError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("test error")
        }
    }

    impl core::error::Error for TestError {}

    struct Recorder(Vec<(&'static str, ValueTypes)>);

    impl AddFieldAndValue for &mut Recorder {
        fn add_field_value(&mut self, fv: &FieldAndValue) {
            self.0.push((fv.field_name, fv.value.clone()));
        }

        fn add_str_value(&mut self, field_name: &'static str, value: &str) {
            self.0
                .push((field_name, ValueTypes::v_str(value.to_string().into())));
        }
    }

    fn assert_error_string(recorded: &[(&'static str, ValueTypes)]) {
        assert!(matches!(
            recorded,
            [("error", ValueTypes::v_str(value))] if value == "test error"
        ));
    }

    #[test]
    fn span_and_event_visitors_record_errors_as_strings() {
        let metadata = tracing::callsite! {
            name: "error field",
            kind: tracing::metadata::Kind::EVENT,
            target: module_path!(),
            level: tracing::Level::INFO,
            fields: error
        }
        .metadata();
        let error: &(dyn core::error::Error + 'static) = &TestError;
        let record = |visitor: &mut dyn field::Visit| {
            tracing::valueset!(metadata.fields(), error = error).record(visitor)
        };

        let mut spans = Vec::new();
        record(&mut SpanValueVisitor {
            record: |name, value| spans.push((name, value)),
        });
        assert_error_string(&spans);

        let mut events = Recorder(Vec::new());
        record(&mut EventBuilderVisitorWrapper::from(&mut events));
        assert_error_string(&events.0);
    }
}

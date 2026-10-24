use core::fmt::Write;

use tracing::field;

use crate::values::*;

// Implemented on the EventBuilder types
#[allow(dead_code)]
pub(crate) trait AddFieldAndValue {
    fn add_field_value(&mut self, fv: &crate::values::FieldAndValue);

    // Adds a borrowed string without first converting it to an owned value.
    fn add_str_value(&mut self, field_name: &'static str, value: &str);
}

// We need a wrapper because we cannot implement an external trait (field::Visit) on an external type (EventBuilder)
#[allow(dead_code)]
pub(crate) struct EventBuilderVisitorWrapper<T: AddFieldAndValue> {
    wrapped: T,
}

// Construct a wrapper from the EventBuilder type
impl<T: AddFieldAndValue> From<T> for EventBuilderVisitorWrapper<T> {
    fn from(value: T) -> Self {
        EventBuilderVisitorWrapper { wrapped: value }
    }
}

impl<T: AddFieldAndValue> field::Visit for EventBuilderVisitorWrapper<T> {
    fn record_debug(&mut self, field: &field::Field, value: &dyn core::fmt::Debug) {
        // Event fields are copied into the builder immediately, so format on
        // the stack instead of allocating an owned string.
        let mut string = FormatBuffer::<FORMAT_INLINE_CAPACITY>::new();
        if write!(string, "{:?}", value).is_err() {
            return;
        }

        self.wrapped.add_str_value(field.name(), string.as_str())
    }

    fn record_f64(&mut self, field: &field::Field, value: f64) {
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value),
        })
    }

    fn record_i64(&mut self, field: &field::Field, value: i64) {
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value),
        })
    }

    fn record_u64(&mut self, field: &field::Field, value: u64) {
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value),
        })
    }

    fn record_i128(&mut self, field: &field::Field, value: i128) {
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value),
        })
    }

    fn record_u128(&mut self, field: &field::Field, value: u128) {
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value),
        })
    }

    fn record_bool(&mut self, field: &field::Field, value: bool) {
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value),
        })
    }

    fn record_str(&mut self, field: &field::Field, value: &str) {
        self.wrapped.add_str_value(field.name(), value)
    }

    // record_error uses tracing-core's Display-string default; see SpanValueVisitor.
}

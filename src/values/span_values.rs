use std::fmt::Write;

use tracing::field;

use crate::values::*;

#[doc(hidden)]
#[derive(Default)]
pub struct FieldValueIndex {
    pub(crate) field: &'static str,
    pub(crate) value: ValueTypes,
    pub(crate) sort_index: u8,
}

// Stores the values for a span, so we can update them while the span is alive and output all the values
// when the span ends.
pub(crate) struct SpanValueVisitor<'a> {
    pub(crate) fields: &'a mut [FieldValueIndex],
}

impl SpanValueVisitor<'_> {
    fn update_value(&mut self, field_name: &'static str, value: ValueTypes) {
        let res = self.fields.binary_search_by_key(&field_name, |idx| {
            self.fields[idx.sort_index as usize].field
        });
        if let Ok(idx) = res {
            self.fields[self.fields[idx].sort_index as usize].value = value;
        } else {
            // We don't support (and don't need to support) adding new fields that weren't in the original metadata
        }
    }
}

impl field::Visit for SpanValueVisitor<'_> {
    fn record_debug(&mut self, field: &field::Field, value: &dyn std::fmt::Debug) {
        let mut string = String::with_capacity(10); // Just a guess
        if write!(string, "{:?}", value).is_err() {
            return;
        }

        self.update_value(field.name(), ValueTypes::v_str(Cow::from(string)));
    }

    fn record_f64(&mut self, field: &field::Field, value: f64) {
        self.update_value(field.name(), ValueTypes::v_f64(value));
    }

    fn record_i64(&mut self, field: &field::Field, value: i64) {
        self.update_value(field.name(), ValueTypes::v_i64(value));
    }

    fn record_u64(&mut self, field: &field::Field, value: u64) {
        self.update_value(field.name(), ValueTypes::v_u64(value));
    }

    fn record_i128(&mut self, field: &field::Field, value: i128) {
        self.update_value(field.name(), ValueTypes::v_i128(value));
    }

    fn record_u128(&mut self, field: &field::Field, value: u128) {
        self.update_value(field.name(), ValueTypes::v_u128(value));
    }

    fn record_bool(&mut self, field: &field::Field, value: bool) {
        self.update_value(field.name(), ValueTypes::v_bool(value));
    }

    fn record_str(&mut self, field: &field::Field, value: &str) {
        self.update_value(
            field.name(),
            ValueTypes::v_str(Cow::from(value.to_string())),
        );
    }

    fn record_error(&mut self, _field: &field::Field, _value: &(dyn std::error::Error + 'static)) {}
}

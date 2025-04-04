use core::fmt::Write;
extern crate alloc;
use alloc::string::{String, ToString};

use tracing::field;

use crate::values::*;

// Implemented on the EventBuilder types
pub(crate) trait AddFieldAndValue {
    fn add_field_value(&mut self, fv: &crate::values::FieldAndValue);
}

// We need a wrapper because we cannot implement an external trait (field::Visit) on an external type (EventBuilder)
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
        let mut string = String::with_capacity(10);
        if write!(string, "{:?}", value).is_err() {
            // TODO: Needs to do a heap allocation
            return;
        }

        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(string),
        })
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
        self.wrapped.add_field_value(&FieldAndValue {
            field_name: field.name(),
            value: &ValueTypes::from(value.to_string()),
        })
    }

    #[cfg(feature = "std")]
    fn record_error(&mut self, _field: &field::Field, _value: &(dyn std::error::Error + 'static)) {}
}

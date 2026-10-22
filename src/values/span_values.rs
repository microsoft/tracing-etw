use core::fmt::Write;
extern crate alloc;
use alloc::string::{String, ToString};

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
        let mut string = String::with_capacity(10); // Just a guess
        if write!(string, "{:?}", value).is_err() {
            return;
        }

        (self.record)(field.name(), ValueTypes::v_str(Cow::from(string)));
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

    #[cfg(feature = "std")]
    fn record_error(&mut self, _field: &field::Field, _value: &(dyn core::error::Error + 'static)) {
    }
}

pub(crate) mod event_values;
pub(crate) mod span_values;

extern crate alloc;
use alloc::{borrow::Cow, string::String};

#[allow(non_camel_case_types, dead_code)]
#[derive(Default, Clone)]
#[doc(hidden)]
pub enum ValueTypes {
    #[default]
    None,
    v_u64(u64),
    v_i64(i64),
    v_u128(u128),
    v_i128(i128),
    v_f64(f64),
    v_bool(bool),
    v_str(Cow<'static, str>), // Would be nice if we didn't have to do a heap allocation
    v_char(char),
}

impl From<u64> for ValueTypes {
    fn from(value: u64) -> Self {
        ValueTypes::v_u64(value)
    }
}

impl From<i64> for ValueTypes {
    fn from(value: i64) -> Self {
        ValueTypes::v_i64(value)
    }
}

impl From<u128> for ValueTypes {
    fn from(value: u128) -> Self {
        ValueTypes::v_u128(value)
    }
}

impl From<i128> for ValueTypes {
    fn from(value: i128) -> Self {
        ValueTypes::v_i128(value)
    }
}

impl From<f64> for ValueTypes {
    fn from(value: f64) -> Self {
        ValueTypes::v_f64(value)
    }
}

impl From<bool> for ValueTypes {
    fn from(value: bool) -> Self {
        ValueTypes::v_bool(value)
    }
}

impl From<&'static str> for ValueTypes {
    fn from(value: &'static str) -> Self {
        ValueTypes::v_str(Cow::from(value))
    }
}

impl From<String> for ValueTypes {
    fn from(value: String) -> Self {
        ValueTypes::v_str(Cow::from(value))
    }
}

impl From<char> for ValueTypes {
    fn from(value: char) -> Self {
        ValueTypes::v_char(value)
    }
}

pub(crate) struct FieldAndValue<'a> {
    #[allow(dead_code)]
    pub(crate) field_name: &'static str,
    #[allow(dead_code)]
    pub(crate) value: &'a ValueTypes,
}

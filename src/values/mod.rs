pub(crate) mod event_values;
pub(crate) mod span_values;

extern crate alloc;
use alloc::{borrow::Cow, string::String};
use core::{fmt, mem::MaybeUninit, ptr};

/// Inline capacity for formatting field values without a heap allocation.
pub(crate) const FORMAT_INLINE_CAPACITY: usize = 512;

/// A string builder that formats into inline storage and spills to the heap
/// only when the output exceeds `N` bytes.
pub(crate) struct FormatBuffer<const N: usize> {
    inline: [MaybeUninit<u8>; N],
    len: usize,
    spilled: Option<String>,
}

impl<const N: usize> FormatBuffer<N> {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            inline: [MaybeUninit::uninit(); N],
            len: 0,
            spilled: None,
        }
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        match &self.spilled {
            Some(spilled) => spilled,
            // SAFETY: The first `len` bytes were initialized by copying whole
            // `&str` values, so they are valid UTF-8.
            None => unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                    self.inline.as_ptr().cast(),
                    self.len,
                ))
            },
        }
    }

    /// Converts to an owned string with a single exactly sized allocation
    /// when the contents are still inline.
    pub(crate) fn into_string(self) -> String {
        match self.spilled {
            Some(spilled) => spilled,
            None => String::from(self.as_str()),
        }
    }
}

impl<const N: usize> fmt::Write for FormatBuffer<N> {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(spilled) = &mut self.spilled {
            spilled.push_str(s);
        } else if s.len() <= N - self.len {
            // SAFETY: The destination has room for `s` after the initialized
            // prefix, and the source cannot overlap this buffer.
            unsafe {
                ptr::copy_nonoverlapping(
                    s.as_ptr(),
                    self.inline.as_mut_ptr().add(self.len).cast(),
                    s.len(),
                );
            }
            self.len += s.len();
        } else {
            let mut spilled = String::with_capacity((self.len + s.len()).max(2 * N));
            spilled.push_str(self.as_str());
            spilled.push_str(s);
            self.spilled = Some(spilled);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn format_buffer_spills_without_losing_inline_contents() {
        let (greek, f, ghij) = ("αβγ", "f", "ghij");
        let mut buffer = FormatBuffer::<8>::new();
        write!(buffer, "{greek}").unwrap();
        assert_eq!(buffer.as_str(), "αβγ");
        buffer.write_str("de").unwrap();
        assert_eq!(buffer.as_str(), "αβγde");
        assert!(buffer.spilled.is_none());
        write!(buffer, "{f}{ghij}").unwrap();
        assert_eq!(buffer.as_str(), "αβγdefghij");
        assert!(buffer.spilled.is_some());
        assert_eq!(buffer.into_string(), "αβγdefghij");

        let mut inline = FormatBuffer::<8>::new();
        write!(inline, "{:?}", 7u8).unwrap();
        let owned = inline.into_string();
        assert_eq!((owned.as_str(), owned.capacity()), ("7", 1));
    }
}

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

#[allow(dead_code)]
pub(crate) struct FieldAndValue<'a> {
    #[allow(dead_code)]
    pub(crate) field_name: &'static str,
    #[allow(dead_code)]
    pub(crate) value: &'a ValueTypes,
}

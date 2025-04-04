extern crate alloc;
use alloc::string::String;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EtwError {
    #[error("Provider group GUID must not be zeros")]
    EmptyProviderGroupGuid,
    #[error("Provider group names must be lower case ASCII or numeric digits: {0:?}")]
    InvalidProviderGroupCharacters(String),
    #[error("Linux provider names must be ASCII alphanumeric: {0:?}")]
    InvalidProviderNameCharacters(String),
    #[error("Linux provider name and provider group must less than 234 characters combined. Current length: {0:?}")]
    TooManyCharacters(usize),
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EtwError {
    #[error("Provider group GUID must not be zeroes")]
    EmptyProviderGroupGuid,
    #[error("Provider group names must be lower case ASCII or numeric digits: {0:?}")]
    InvalidProviderGroupCharacters(String),
    #[error("Linux provider names must be ASCII alphanumeric: {0:?}")]
    InvalidProviderNameCharacters(String)
}

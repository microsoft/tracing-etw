// Module for internal structs that need to be publicly accessible,
// but should not be directly used by consumers of the crate.
//
// Implementations for these structs are contained in other files.



// Public with public fields because the `etw_event!` macro needs to create it at invocation site.
#[doc(hidden)]
pub struct EventMetadata {
    pub kw: u64,
    pub identity: tracing::callsite::Identifier,
    pub event_tag: u32,
}

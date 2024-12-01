// Module for internal structs that need to be publicly accessible,
// but should not be directly used by consumers of the crate.
//
// Implementations for these structs are contained in other files.

// Public with public fields because the `etw_event!` macro needs to create it at invocation site.
#[doc(hidden)]
#[repr(C)]
pub struct EventMetadata {
    pub kw: u64,
    pub identity: tracing::callsite::Identifier,
    pub event_tag: u32,
}

// An EventMetadata with a hash, because Identity doesn't implement comparisons but we need a stable ordering.
#[derive(Clone)]
pub(crate) struct ParsedEventMetadata {
    pub(crate) identity_hash: u64,
    pub(crate) meta: &'static EventMetadata
}

//#[cfg(any(not(feature = "registry"), docsrs))]
mod core_subscriber;

#[cfg(any(feature = "registry", docsrs))]
pub mod registry_subscriber;

pub(crate) mod common;

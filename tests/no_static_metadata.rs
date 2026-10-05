use tracing_etw::LayerBuilder;
use tracing_subscriber::prelude::*;

#[cfg(target_os = "windows")]
#[used]
#[unsafe(link_section = ".rdata$zRSETW5")]
static mut NULL_METADATA_1: *const tracing_etw::_details::EventMetadata = std::ptr::null();

#[cfg(target_os = "windows")]
#[used]
#[unsafe(link_section = ".rdata$zRSETW5")]
static mut NULL_METADATA_2: *const tracing_etw::_details::EventMetadata = std::ptr::null();

#[test]
fn tracing_event_without_static_metadata() {
    let layer = LayerBuilder::new("NoStaticMetadata")
        .with_default_keyword(1)
        .__build_for_test()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || tracing::info!("Hello"));
}

use tracing::Level;
use tracing_etw::{etw_event, LayerBuilder};
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

fn main() {
    tracing_subscriber::registry()
        .with(LayerBuilder::new_common_schema_events("ExampleProvEtwEvent_CS").build().unwrap())
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        .init();

    etw_event!(name: "EtwEventName", Level::ERROR, 5, "An event with a name and keyword!");
}

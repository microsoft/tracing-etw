use tracing::Level;
use tracing_etw::{LayerBuilder, etw_event};
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

fn main() {
    tracing_subscriber::registry()
        .with(LayerBuilder::new("ExampleProvEtwEvent").build().unwrap())
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        .init();

    etw_event!(name: "EtwEventName1", Level::ERROR, 1, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName2", Level::WARN, 2, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName3", Level::INFO, 3, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName4", Level::DEBUG, 4, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName5", Level::TRACE, 5, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName6", Level::TRACE, 6, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName7", Level::DEBUG, 7, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName8", Level::INFO, 8, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName9", Level::WARN, 9, "An event with a name and keyword!");
    etw_event!(name: "EtwEventName10", Level::ERROR, 10, "An event with a name and keyword!");
}

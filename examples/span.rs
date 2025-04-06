use tracing::{Level, event, span};
use tracing_etw::LayerBuilder;
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

fn main() {
    tracing_subscriber::registry()
        .with(
            LayerBuilder::new_common_schema_events("ExampleProvSpan_CS")
                .build()
                .unwrap(),
        )
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        .init();

    let span = span!(
        Level::INFO,
        "span name",
        fieldC = b'x',
        fieldB = "asdf",
        fieldA = 7,
        "inside {}!",
        "main"
    );
    let _ = span.enter();

    event!(Level::ERROR, "error event");

    span.record("fieldB", 12345);
}

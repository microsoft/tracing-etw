use tracing::{event, Level};
use tracing_etw::LayerBuilder;
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

fn main() {
    tracing_subscriber::registry()
        .with(LayerBuilder::new("ExampleProvBasic").build().unwrap()) // Collects everything
        .with(
            LayerBuilder::new_common_schema_events("ExampleProvBasic_CS")
                .build_with_target("geneva")
                .unwrap(),
        )
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        .init();

    #[allow(non_snake_case)]
    let fieldB = "asdf";
    event!(
        Level::INFO,
        fieldC = b'x',
        fieldB,
        fieldA = 7,
        "inside {}!",
        "main"
    );

    event!(target: "geneva", Level::ERROR, "error event");
}

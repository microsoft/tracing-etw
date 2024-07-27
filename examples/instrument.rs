use tracing::instrument;
use tracing_etw::LayerBuilder;
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

#[instrument]
fn test_function(x: u32, y: f64) -> f64 {
    x as f64 + y
}

fn main() {
    tracing_subscriber::registry()
        .with(LayerBuilder::new("ExampleProvInstrument").build()) // Collects everything
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        .init();

    test_function(1, 2.0);
}

use tracing::{error_span, event, span, Level};
use tracing_etw::LayerBuilder;
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

#[test]
fn span_test_1() {
    tracing_subscriber::registry()
        .with(
            LayerBuilder::new("SpanTests")
                .__build_for_test()
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
    let _one = span.enter();
    let _two = span.enter();

    let span2 = error_span!("span 2");
    let _three = span2.enter();

    event!(Level::ERROR, "error event");

    span.record("fieldB", 12345);
}

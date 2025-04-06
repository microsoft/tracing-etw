//#![cfg(feature = "std")]

use tracing::{Level, error_span, event, span};
use tracing_etw::LayerBuilder;

#[test]
fn subscriber_span_test_1() {
    let layer = LayerBuilder::new("SpanTests").build_subscriber().unwrap();
    let dispatch = tracing_core::Dispatch::new(layer);
    tracing_core::dispatcher::set_global_default(dispatch).expect("Set dispatcher");

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

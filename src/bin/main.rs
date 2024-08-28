#![allow(unused_imports)]

use tracing::{event, span, Level};
use tracing_etw::{etw_event, LayerBuilder};
use tracing_subscriber::{self, fmt::format::FmtSpan, prelude::*};

// #[tracing::instrument]
// fn test_function(x: u32, y: f64) {

// }

fn main() {
    let registry = tracing_subscriber::registry();

    #[cfg(not(feature = "global_filter"))]
    let registry = registry.with(LayerBuilder::new("test").build().unwrap());
    #[cfg(all(not(feature = "global_filter"), feature = "common_schema"))]
    let registry = registry.with(LayerBuilder::new_common_schema_events("test2").build_with_target("geneva").unwrap());
    #[cfg(not(feature = "global_filter"))]
    let registry = registry.with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE));
    
    #[cfg(feature = "global_filter")]
    let registry = registry.with(LayerBuilder::new("test").build_global_filter().unwrap());

    registry.init();

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

    // Construct a new span named "my span" with trace log level.
    let span = span!(Level::INFO, "my_span", field3 = 4.5, field2 = 0, field1 = 0);

    // Enter the span, returning a guard object.
    let _enter = span.enter();
    span.record("field1", 12345);

    span.record("field2", "9876f64");

    span!(Level::INFO, "nested_span", key = "value").in_scope(|| {
        event!(Level::ERROR, "oh noes!");
    });

    //test_function(5, 83.29032);

    drop(_enter);
    drop(span);

    etw_event!(target: "geneva", name: "EtwEventName", Level::ERROR, 5, "Only for geneva!");
}

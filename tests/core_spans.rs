use tracing::{
    Event, Level, Metadata, Subscriber, error_span, event,
    level_filters::LevelFilter,
    span::{Attributes, Id, Record},
    subscriber::Interest,
};
use tracing_etw::LayerBuilder;

// Keep the public output-mode marker usable as a trait object.
const _: Option<&dyn tracing_etw::OutputMode> = None;

// Without an ETW/user_events collector every callsite would be disabled, so
// force enablement to exercise standalone span handling.
struct Unfiltered<S>(S);

impl<S: Subscriber> Subscriber for Unfiltered<S> {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::always()
    }

    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        self.0.new_span(attrs)
    }

    fn record(&self, id: &Id, values: &Record<'_>) {
        self.0.record(id, values);
    }

    fn record_follows_from(&self, id: &Id, follows: &Id) {
        self.0.record_follows_from(id, follows);
    }

    fn event(&self, event: &Event<'_>) {
        self.0.event(event);
    }

    fn enter(&self, id: &Id) {
        self.0.enter(id);
    }

    fn exit(&self, id: &Id) {
        self.0.exit(id);
    }

    fn clone_span(&self, id: &Id) -> Id {
        self.0.clone_span(id)
    }

    fn try_close(&self, id: Id) -> bool {
        self.0.try_close(id)
    }

    fn current_span(&self) -> tracing_core::span::Current {
        self.0.current_span()
    }
}

#[test]
fn subscriber_span_test_1() {
    let layer = LayerBuilder::new("SpanTests").build_subscriber().unwrap();
    let dispatch = tracing_core::Dispatch::new(Unfiltered(layer));
    tracing_core::dispatcher::set_global_default(dispatch).expect("Set dispatcher");

    let span = tracing::span!(
        Level::INFO,
        "span name",
        fieldC = b'x',
        fieldB = "asdf",
        fieldA = 7,
        "inside {}!",
        "main"
    );
    assert!(!span.is_disabled());
    let _one = span.enter();
    let _two = span.enter();
    assert_eq!(tracing::Span::current().id(), span.id());

    let span2 = error_span!("span 2");
    assert!(!span2.is_disabled());
    let _three = span2.enter();
    assert_eq!(tracing::Span::current().id(), span2.id());

    event!(Level::ERROR, "error event");

    span.record("fieldB", 12345);
}

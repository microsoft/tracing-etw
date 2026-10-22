use criterion::{Criterion, criterion_group, criterion_main};
#[cfg(windows)]
use std::any::TypeId;
#[cfg(windows)]
use tracing::{Dispatch, Event, Level, Metadata, Subscriber, event, span};
#[cfg(windows)]
use tracing_core::{
    Interest, LevelFilter,
    span::{Attributes, Current, Id, Record},
};
#[cfg(windows)]
use tracing_etw::{LayerBuilder, etw_event};
#[cfg(all(windows, feature = "std"))]
use tracing_subscriber::prelude::*;

// Bypass subscriber enablement so both implementations perform the same work
// without a collector. The std layer also needs its native filter removed.
#[cfg(windows)]
struct Unfiltered<S>(S);

#[cfg(windows)]
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

    fn on_register_dispatch(&self, dispatch: &Dispatch) {
        self.0.on_register_dispatch(dispatch);
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

    fn current_span(&self) -> Current {
        self.0.current_span()
    }

    unsafe fn downcast_raw(&self, id: TypeId) -> Option<*const ()> {
        // SAFETY: The wrapped subscriber owns the returned downcast pointer.
        unsafe { self.0.downcast_raw(id) }
    }
}

#[cfg(windows)]
pub fn etw_benchmark(c: &mut Criterion) {
    eprintln!(
        "ETW benchmark: {}, OpenTelemetry feature: {}",
        if cfg!(feature = "std") {
            "std (span registry)"
        } else {
            "no_std (standalone subscriber)"
        },
        cfg!(feature = "opentelemetry"),
    );

    // Drop this dispatch before registering Unfiltered, so Interest::always()
    // cannot turn the disabled callsites' cached Interest::never() into sometimes().
    {
        let builder = LayerBuilder::new("etw_bench_disabled");
        #[cfg(feature = "std")]
        let subscriber = tracing_subscriber::registry().with(builder.build_layer().unwrap());
        #[cfg(not(feature = "std"))]
        let subscriber = builder.build_subscriber().unwrap();
        let dispatch = Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            assert!(
                !tracing::enabled!(Level::INFO),
                "The disabled benchmark provider must not be enabled by an ETW collector"
            );
            let mut disabled_group = c.benchmark_group("disabled");
            disabled_group.warm_up_time(std::time::Duration::from_millis(250));

            disabled_group.bench_function("span", |b| {
                b.iter(|| {
                    let span = span!(Level::INFO, "Disabled span!");
                    let _entered = span.enter();
                })
            });

            disabled_group.bench_function("event", |b| {
                b.iter(|| {
                    event!(Level::INFO, "Disabled event!");
                })
            });
        });
    }

    let builder = LayerBuilder::new("etw_bench");
    #[cfg(feature = "std")]
    let subscriber = tracing_subscriber::registry().with(builder.__build_for_test().unwrap());
    #[cfg(not(feature = "std"))]
    let subscriber = builder.build_subscriber().unwrap();
    let dispatch = Dispatch::new(Unfiltered(subscriber));
    tracing::dispatcher::with_default(&dispatch, || {
        {
            let span = span!(Level::INFO, "Benchmark validation");
            assert!(
                !span.is_disabled(),
                "Processing benchmarks must create spans"
            );
            let _entered = span.enter();
            assert_eq!(tracing::Span::current().id(), span.id());
        }

        // Spans
        {
            let mut span_group = c.benchmark_group("unfiltered/spans");
            span_group.warm_up_time(std::time::Duration::from_millis(500));

            span_group.bench_function("empty", |b| {
                b.iter(|| {
                    let span = span!(Level::INFO, "Enabled span!");
                    let _entered = span.enter();
                })
            });

            span_group.bench_function("3 fields", |b| {
                b.iter(|| {
                    let span = span!(
                        Level::INFO,
                        "Enabled span!",
                        field1 = 1,
                        field2 = "asdf",
                        field3 = 1.1
                    );
                    let _entered = span.enter();
                })
            });

            span_group.bench_function("3 fields+record", |b| {
                b.iter(|| {
                    let field1 = 1;
                    let field2 = "asdf";
                    let field3 = 1.1;
                    let span = span!(Level::INFO, "Enabled span!", field1, field2, field3);
                    let _entered = span.enter();
                    span.record("field1", 5.5);
                    span.record("invalid", field2);
                    span.record("field2", 1000);
                })
            });
        }

        // Events
        {
            let mut event_group = c.benchmark_group("unfiltered/events");
            event_group.warm_up_time(std::time::Duration::from_millis(500));

            event_group.bench_function("empty", |b| {
                b.iter(|| {
                    event!(Level::INFO, "Enabled event!");
                })
            });

            event_group.bench_function("3 fields", |b| {
                b.iter(|| {
                    event!(
                        Level::INFO,
                        field1 = 1,
                        field2 = "asdf",
                        field3 = 1.1,
                        "Enabled event!"
                    );
                })
            });
        }

        // etw_events
        {
            let mut event_group = c.benchmark_group("unfiltered/etw_events");
            event_group.warm_up_time(std::time::Duration::from_millis(500));

            event_group.bench_function("empty", |b| {
                b.iter(|| {
                    etw_event!(name: "evtname", Level::INFO, 1, "Enabled event!");
                })
            });

            event_group.bench_function("3 fields", |b| {
                b.iter(|| {
                    etw_event!(
                        name: "evtname",
                        Level::INFO,
                        1,
                        { field1 = 1, field2 = "asdf", field3 = 1.1 },
                        "Enabled event!"
                    );
                })
            });
        }
    });
}

#[cfg(not(windows))]
pub fn etw_benchmark(_c: &mut Criterion) {}

criterion_group!(benches, etw_benchmark);
criterion_main!(benches);

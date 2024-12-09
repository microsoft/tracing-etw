#![allow(unused_imports, dead_code)]

use criterion::{criterion_group, criterion_main, Criterion};
#[cfg(target_os = "windows")]
use etw_helpers::{FileMode, SessionBuilder};
use tracing::{event, span, Level};
use tracing_etw::*;
use tracing_subscriber::{self, prelude::*};

#[cfg(target_os = "windows")]
pub fn etw_benchmark(c: &mut Criterion) {
    let builder = LayerBuilder::new("etw_bench");
    let provider_id = builder.get_provider_id();
    let _subscriber = tracing_subscriber::registry()
        .with(builder.build().unwrap())
        .init();

    let etw_session = SessionBuilder::new_file_mode(
        "tokio-tracing-etw-bench",
        "etw_bench.etl",
        FileMode::Sequential,
    )
    .buffer_counts(128, 128, 128)
    .realtime_event_delivery();
    let session = etw_session.start(true).expect("can't start etw session");

    // Disabled provider
    {
        let mut disabled_group = c.benchmark_group("disabled");
        disabled_group.warm_up_time(std::time::Duration::from_millis(250));

        disabled_group.bench_function("span", |b| {
            b.iter(|| {
                let span = span!(Level::INFO, "Disabled span!");
                let _ = span.enter();
            })
        });

        disabled_group.bench_function("event", |b| {
            b.iter(|| {
                event!(Level::INFO, "Disabled event!");
            })
        });
    }

    session
        .enable_provider(&provider_id.to_u128().into(), 0xFF)
        .expect("can't enable provider to session");

    // Spans
    {
        let mut span_group = c.benchmark_group("spans");
        span_group.warm_up_time(std::time::Duration::from_millis(500));

        span_group.bench_function("empty", |b| {
            b.iter(|| {
                let span = span!(Level::INFO, "Enabled span!");
                let _ = span.enter();
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
                let _ = span.enter();
            })
        });

        span_group.bench_function("3 fields+record", |b| {
            b.iter(|| {
                let field1 = 1;
                let field2 = "asdf";
                let field3 = 1.1;
                let span = span!(Level::INFO, "Enabled span!", field1, field2, field3);
                let _ = span.enter();
                span.record("field1", 5.5);
                span.record("invalid", field2);
                span.record("field2", 1000);
            })
        });
    }

    // Events
    {
        let mut event_group = c.benchmark_group("events");
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
        let mut event_group = c.benchmark_group("etw_events");
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
                    field1 = 1,
                    field2 = "asdf",
                    field3 = 1.1,
                    "Enabled event!"
                );
            })
        });
    }
}

#[cfg(not(target_os = "windows"))]
pub fn etw_benchmark(_c: &mut Criterion) {}

criterion_group!(benches, etw_benchmark);
criterion_main!(benches);

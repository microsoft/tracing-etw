# tracing-etw-user_events

[ETW][etw] and [EventHeader formatted user_events][eventheader] layer for [`tracing`].

[![Crates.io][crates-badge]][crates-url]
[![Documentation][docs-badge]][docs-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]
![maintenance status][maint-badge]

## Overview

This layer emits tracing events as Windows ETW events or Linux user-mode tracepoints
(user_events with the [EventHeader](https://github.com/microsoft/LinuxTracepoints/tree/main/libeventheader-tracepoint)
encoding; requires a Linux 6.4+ kernel).

*Note*: Linux kernels without user_events support will not log any events.

## Standalone subscriber

The default `std` feature enables integration with `tracing-subscriber`'s span
registry. With `default-features = false`, `LayerBuilder::build()` instead
constructs a standalone `tracing::Subscriber`; `build_subscriber()` explicitly
selects this implementation with either feature configuration.

The standalone subscriber tracks entered spans per thread and subscriber,
including nested and repeated entries. Span clones, entered spans, and live
children retain span data until their references are released. Contextual
children and events use the currently entered span; explicit parents and
explicit roots override that context.

Disabling the `std` feature selects `no_std + alloc` on Windows and Linux.
The native backends still require operating-system services and an allocator;
this is not support for a freestanding `no_std` target. The default
`opentelemetry` feature also enables `std`, so disable default features to
select the standalone-only configuration.

```toml
[dependencies]
tracing-etw = { version = "0.3", default-features = false }
tracing = { version = "0.1", default-features = false }
```

Use `LayerBuilder::build()` or `build_subscriber()` and install the subscriber
with `tracing::subscriber::set_global_default`. A no-std executable must also
provide its panic handler and native startup, in addition to an allocator.
Neither `tracing-subscriber` nor OpenTelemetry is required in this configuration.

Without `std`, span stacks and reusable event builders use Windows FLS slots
or Linux pthread keys. Values are initialized on first access and dropped by
native thread-exit callbacks, releasing references held by entered spans.
Windows FLS also isolates values between fibers and runs cleanup on fiber
deletion. Recursive initialization and access during destruction panic.
Native keys live for the process lifetime; per-thread values are freed at
exit. With `std`, standard-library thread-local storage is unchanged.
Cleanup follows native thread/fiber teardown; process termination does not
guarantee that destructors run.
Native keys are published atomically after initialization, so subsequent key
lookups do not acquire a lock.

Hashing uses explicitly seeded `ahash::RandomState` values. The dependency's
standard-library and runtime-RNG default features are disabled.

The standalone subscriber caches immutable span IDs, parent IDs, and metadata
in entered-span stacks. Current-span queries and contextual event correlation
therefore avoid the shared span map, as do events without a parent span.
Recorded span values are formatted before acquiring the map lock; replaced
values and removed span payloads are destroyed after releasing it. Span
start/stop encoding still borrows the payload under its shard's lock.

Span storage uses 32 independently locked, cache-line-aligned shards selected
by a hash of the full `(subscriber_id, span_id)` key. Each lookup hashes the
key once and reuses that hash for map access. Shard selection uses middle
hash bits, leaving the map's bucket and control-byte bits independent.
Parent/child creation locks distinct shards in ascending index order;
cascading close operations release each shard before accessing the parent.

Shared span storage and the Linux provider use blocking read/write locks with
poisoning when `std` is enabled. Without `std`, Windows uses native `SRWLOCK`
locks and Linux uses `libc::pthread_rwlock_t`; neither provides lock poisoning.
Linux locks are initialized on first use in stable heap allocations, allowing
their Rust owners to move without relocating initialized pthread objects.
Activity-seed and event-metadata initialization use the same native blocking
locks on Windows and Linux, with no spin-lock dependency. Other targets retain
standard-library synchronization for the no-op exporter. These paths are not
interrupt-safe or reentrant. Lazy initialization remains poisoned if its
initializer panics. Once initialized, lazy values use an Acquire/Release atomic
publication fast path and do not acquire a native lock on subsequent reads.

Wall-clock timestamps use Windows `GetSystemTimePreciseAsFileTime` (`FILETIME`)
or Unix `clock_gettime(CLOCK_REALTIME)` (`timespec`), without Rust standard-library
clock APIs. Calendar conversion and RFC3339 formatting are implemented locally.
Other targets retain the no-op exporter and do not emit timestamps.

## MSV

Minimum supported Rust version is 1.85 (edition 2024).

## ETW benchmarks

On Windows, compare the registry-backed `std` implementation with the
standalone `no_std` implementation using separate feature builds:

```text
cargo bench --bench etw --no-default-features --features std -- --save-baseline std
cargo bench --bench etw --no-default-features -- --baseline std
```

The second run compares `no_std` against the saved `std` results. Benchmark
IDs are identical across configurations, and the startup banner identifies
the active implementation. Disable default features in both commands to
exclude OpenTelemetry from the comparison.

The `disabled` group uses normal provider filtering. The `unfiltered` groups
force span/event processing in both implementations, including native event
construction, without requiring an ETW collector. These groups do not measure
the disabled-call fast path and do not imply that events were collected.
Keep external collectors stopped for a collector-free comparison; collecting
the `etw_bench` provider adds native ETW delivery costs. The
`etw_bench_disabled` provider must remain disabled.

The `unfiltered/4 threads/span+event` case runs four persistent workers against
one shared subscriber. Each worker repeatedly creates a three-field span,
enters and records it, emits a contextual three-field event, then exits and
closes the span. Thread creation, dispatch setup, and per-thread warm-up are
outside the measurement. Workers start each batch together; batch coordination
is timed, but there is no per-operation synchronization in the harness.
One Criterion iteration represents four span-plus-event operations, and
throughput reports their aggregate rate. Run just this case by adding
`"unfiltered/4 threads"` before the baseline arguments after `--`.

Criterion and the benchmark harness use `std` in both runs; `no_std` refers
to the library's feature configuration, native locks/TLS, and standalone
span tracking. Use `-- --test` instead of the baseline arguments for a quick
execution of every benchmark case.

## no_std coverage

`cargo test --no-default-features` exercises the standalone subscriber, but its
test harness and dev dependencies use `std`. The separate consumer in
`tests/no_std` avoids that feature unification and links and runs without Rust
`std`, using a native allocator, panic handler, C startup, and OS thread.
It exercises both output modes, span lifetime/current-span tracking, native
thread-exit cleanup, and the exported event macro without extra helper
dependencies. Its panic handler makes an accidental transitive `std` dependency
a compile error.
The release profile uses aborting panics and LTO to discard unused unwind
references from the prebuilt Linux `alloc` library.

CI runs this consumer on Windows and Linux. Run it locally with
`cargo run --release --manifest-path tests/no_std/Cargo.toml`.

[etw]: https://learn.microsoft.com/windows/win32/etw/about-event-tracing
[eventheader]: https://github.com/microsoft/LinuxTracepoints/tree/main/libeventheader-tracepoint
[`tracing`]: https://crates.io/crates/tracing

[crates-badge]: https://img.shields.io/crates/v/tracing-etw.svg
[crates-url]: https://crates.io/crates/tracing-etw
[docs-badge]: https://docs.rs/tracing-etw/badge.svg
[docs-url]: https://docs.rs/tracing-etw
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: LICENSE
[actions-badge]: https://github.com/microsoft/tracing-etw/actions/workflows/rust.yml/badge.svg
[actions-url]:https://github.com/microsoft/tracing-etw/actions/workflows/rust.yml
[maint-badge]: https://img.shields.io/badge/maintenance-experimental-blue.svg

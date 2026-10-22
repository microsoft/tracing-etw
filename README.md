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

Shared span storage and the Linux provider use blocking read/write locks with
poisoning when `std` is enabled. Without `std`, Windows uses native `SRWLOCK`
locks and Linux uses `libc::pthread_rwlock_t`; neither provides lock poisoning.
Linux locks are initialized on first use in stable heap allocations, allowing
their Rust owners to move without relocating initialized pthread objects.
Activity-seed and event-metadata initialization use the same native blocking
locks on Windows and Linux, with no spin-lock dependency. Other targets retain
standard-library synchronization for the no-op exporter. These paths are not
interrupt-safe or reentrant. Lazy initialization remains poisoned if its
initializer panics.

Wall-clock timestamps use Windows `GetSystemTimePreciseAsFileTime` (`FILETIME`)
or Unix `clock_gettime(CLOCK_REALTIME)` (`timespec`), without Rust standard-library
clock APIs. Calendar conversion and RFC3339 formatting are implemented locally.
Other targets retain the no-op exporter and do not emit timestamps.

## MSV

Minimum supported Rust version is 1.82 (released October 2024).

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

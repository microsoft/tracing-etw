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

Other platforms use a no-op exporter: providers are always disabled and no
events are emitted.

## Standalone subscriber

The default `std` feature enables integration with `tracing-subscriber`'s span
registry through `LayerBuilder::build()` / `build_layer()`. These registry
builders are only available with `std`. `LayerBuilder::build_subscriber()`
constructs a standalone `tracing::Subscriber` with either feature configuration,
and is the builder to use when `std` is disabled. Each builder has the same
signature regardless of features, so enabling `std` elsewhere in a dependency
graph does not change the meaning of existing calls.

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
tracing-etw = { version = "0.4", default-features = false }
tracing = { version = "0.1", default-features = false }
```

Use `LayerBuilder::build_subscriber()` and install the subscriber
with `tracing::subscriber::set_global_default`. A no-std executable must also
provide its panic handler and native startup, in addition to an allocator.
Neither `tracing-subscriber` nor OpenTelemetry is required in this configuration.

Without `std`, span stacks and reusable event builders use Windows FLS slots
or Linux pthread keys. Values are initialized on first access and dropped by
native thread-exit callbacks, releasing references held by entered spans.
Windows FLS also isolates values between fibers and runs cleanup on fiber
deletion. FLS callbacks run outside the loader lock, so cleanup that waits on
span locks does not stall thread creation or DLL loading. Recursive
initialization and access during destruction panic.
Native keys live for the process lifetime; per-thread values are freed at
exit. Because the keys and their cleanup callbacks are never unregistered,
the `no_std` backends must not be used from a DLL or shared object that can
be unloaded (`FreeLibrary` / `dlclose`) while threads that logged are still
running; the OS would later invoke the callback in unmapped code.
With `std`, standard-library thread-local storage is used. On Windows,
standard-library thread-exit destructors may run under the loader lock, so
spans still entered when a thread exits are queued without locking and
released by a later `new_span` or `try_close` call on any thread.
Cleanup follows native thread/fiber teardown; process termination does not
guarantee that destructors run.
Native keys are published atomically after initialization, so subsequent key
lookups do not acquire a lock.

Event string fields and formatted `Debug` values are copied directly into the
native event builder from a 512-byte stack buffer, so they do not allocate
unless longer. Span fields must be owned; they are formatted on the stack and
copied once into an exactly sized allocation. Common Schema timestamps are
formatted without allocating, once per event.

Hashing uses explicitly seeded `ahash::RandomState` values. The dependency's
standard-library and runtime-RNG default features are disabled.

The standalone subscriber stores stable, cache-line-aligned entries in a
generational slab with independent payload locks, and caches owning handles
in entered-span stacks.
Re-entry, recording, nonfinal entered-reference release, and stop
emission can use those handles without hashing or acquiring a shard lock.
Current-span queries and contextual event correlation read immutable cached
IDs and metadata; events without a parent also bypass span storage.

Tracing's logical reference count is tracked separately from allocation
ownership. An entered handle retains one logical reference; temporary memory
borrows do not delay logical closure. Final removal acquires the shard
exclusively, then releases retained parents and destroys payloads outside it.
Standalone start/stop encoding borrows the payload under its own lock, so
unrelated spans in the same shard do not serialize emission. Stop emission uses
a shared guard, preserving standard-library lock poisoning behavior on writer panics.
Recorded values are formatted before borrowing TLS or locking the payload;
replaced values are destroyed after releasing both. An unavailable entered
cache falls back to ID lookup, including during TLS initialization/teardown.
Each shard can reuse up to four fully released entry allocations. Reuse requires
exclusive allocation ownership, clears recorded fields before caching, and
never recycles an entry still referenced by an entered handle or memory borrow.
The registry-backed layer keeps each span payload in a shared allocation with
its own lock. Enter, exit, and record clone the payload handle under a brief
shard read lock, then encode and emit without holding the shard. Like
standalone entries, each shard can reuse up to four uniquely owned released
payload allocations. The two
integrations have separate, statically selected storage and lifetime paths,
without per-access storage-kind dispatch. Both APIs remain available together
when `std` is enabled.

Span strings (trace/span ID text) are written only by Common Schema output.
Normal output skips OpenTelemetry context lookups and ID formatting, and no
output mode looks up OpenTelemetry context for span start events.

Standalone span IDs encode a 5-bit shard, 27-bit slot, and 32-bit generation.
Indexed lookup checks both the generation and subscriber identity without
hashing. Freed slots are recycled with a new generation; exhausted generations
retire their slots instead of wrapping, so stale IDs never address a new span.
Each shard supports up to 2^27 slots and reports exhaustion rather than reusing
an ID. Each thread/fiber selects a preferred shard on its first span. This
replaces per-thread batches of globally allocated span IDs; subscriber IDs
still use a checked global counter.

Entered-span state and the lazily initialized native event builder share one
TLS allocation. Standalone enter, exit, and contextual event emission pass the
already accessed event context to the writer, avoiding a second native TLS
lookup. The entered stack and builder are borrowed independently; user
formatting runs without a borrowed entered stack.

Each storage backend uses 32 independently locked, cache-line-aligned shards.
Standalone IDs select their shard and slot directly. The registry backend
hashes the full `(subscriber_id, span_id)` key once per lookup and reuses that
hash for map access. Its shard selection uses middle hash bits, leaving the
map's bucket and control-byte bits independent.
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

## Breaking Changes

Breaking changes between released versions, and how to resolve them. Add an
entry here for every release that breaks existing consumers.

### 0.3 to 0.4

- **Minimum supported Rust version is now 1.85** (edition 2024, previously 1.82).
  Update the toolchain.
- **New default features: `std` and `opentelemetry`.** 0.3 had no default
  features. The registry-backed builders (`build()`, `build_layer()`,
  `build_with_target()`), `EtwFilter`, and
  `tracing-subscriber` integration now require `std`. Dependents that set
  `default-features = false` must add `features = ["std"]` (and
  `"opentelemetry"` if needed) to keep using `tracing-subscriber` layers, or
  switch to `LayerBuilder::build_subscriber()` for the standalone subscriber.
- **The `global_filter` feature and `LayerBuilder::build_global_filter()` were
  removed.** Remove the feature from `Cargo.toml` and replace
  `build_global_filter()` with `build()` (or `build_with_target()`). These apply
  ETW/user_events enablement as a per-layer filter, so events that are not
  enabled by a collector are skipped by the ETW layer only; other layers in the
  subscriber are no longer filtered by ETW/user_events enablement. Add your own
  filters to those layers if they relied on it.
- **OpenTelemetry integration requires `tracing-opentelemetry` 0.34** (previously
  0.32) and `opentelemetry` 0.33. Update those dependencies to matching versions
  so span contexts are found.
- **Linux requires `eventheader` / `eventheader_dynamic` 0.5 or later.**

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

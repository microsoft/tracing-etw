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

Without `std`, entered spans on Windows are tracked per fiber. Thread or fiber
cleanup is not guaranteed at process termination. The `no_std` backends must
not be used from a DLL or shared object that can be unloaded (`FreeLibrary` /
`dlclose`) while threads that logged are still running, because their cleanup
callbacks can outlive the module.

## OpenTelemetry span IDs

Only Common Schema output writes trace and span IDs as text. It does not look
up OpenTelemetry context for span start events. Layers that are never
registered, such as replacements installed with
`tracing_subscriber::reload::Handle::reload` or layers inside a subscriber
shared through `Arc`, use local span IDs instead of OpenTelemetry IDs.

## MSV

Minimum supported Rust version is 1.85 (edition 2024).

## Breaking Changes

Breaking changes between released versions, and how to resolve them.

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

## Development

See the [development instructions](DEVELOPMENT.md) for testing, benchmarking,
and implementation details.

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

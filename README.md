# tracing-etw-user_events

[ETW][etw] and [EventHeader formatted user_events][eventheader] layer for [`tracing`].

[![Crates.io][crates-badge]][crates-url]
[![Documentation][docs-badge]][docs-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]
![maintenance status][maint-badge]

# Overview

This layer emits tracing events as Windows ETW events or Linux user-mode tracepoints
(user_events with the [EventHeader](https://github.com/microsoft/LinuxTracepoints/tree/main/libeventheader-tracepoint)
encoding; requires a Linux 6.4+ kernel).

*Note*: Linux kernels without user_events support will not log any events.

# MSV

Minimum supported Rust version is 1.80 (released July 2024).

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

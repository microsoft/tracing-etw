//! # A [tracing](https://crates.io/crates/tracing) layer for ETW and Linux user_events
//!
//! ## Overview
//!
//! This layer emits tracing events as Windows ETW events or Linux user-mode tracepoints
//! (user_events with the [EventHeader](https://github.com/microsoft/LinuxTracepoints/tree/main/libeventheader-tracepoint)
//! encoding; requires a Linux 6.4+ kernel).
//! *Note*: Linux kernels without user_events support will not log any events.
//!
//! ### ETW
//!
//! ETW is a Windows-specific system wide, high performance, lossy tracing API built into the
//! Windows kernel. Events can be correlated alongside other system activity, such as disk IO,
//! memory allocations, sample profiling, network activity, or any other event logged by the
//! thousands of ETW providers built into Windows and 3rd party software and drivers.
//!
//! ETW is not designed to be a transport mechanism or message passing interface for
//! forwarding data. These scenarios are better covered by other technologies
//! such as RPC or socket-based transports.
//!
//! Users unfamiliar with the basics of ETW may find the following links helpful.
//! The rest of the documentation for this exporter will assume familiarity
//! with ETW and trace processing tools such as WPA, PerfView, or TraceView.
//! - <https://learn.microsoft.com/windows/win32/etw/about-event-tracing>
//! - <https://learn.microsoft.com/windows-hardware/test/weg/instrumenting-your-code-with-etw>
//!
//! This layer uses [TraceLogging](https://learn.microsoft.com/windows/win32/tracelogging/trace-logging-about)
//! to log events. The ETW provider ID is generated from a hash of the specified provider name.
//!
//! ### Linux user_events
//!
//! User-mode event tracing [(user_events)](https://docs.kernel.org/trace/user_events.html)
//! is new to the Linux kernel starting with version 6.4. For the purposes of this exporter,
//! its functionality is nearly identical to ETW. Any differences between the two will be explicitly
//! called out in these docs.
//!
//! The [perf](https://perf.wiki.kernel.org/index.php/Tutorial) tool can be used on Linux to
//! collect user_events events to a file on disk that can then be processed into a readable
//! format. Because these events are encoded in the new [EventHeader](https://github.com/microsoft/LinuxTracepoints/)
//! format, you will need a tool that understands this encoding to process the perf.dat file.
//! The [decode_perf](https://github.com/microsoft/LinuxTracepoints/tree/main/libeventheader-decode-cpp)
//! sample tool can be used to do this currently; in the future support will be added to additional tools.
//!
//! ## Example
//!
//! ```no_run
//! use tracing::{event, Level};
//! use tracing_subscriber::{self, prelude::*};
//! 
//! tracing_subscriber::registry()
//!     .with(tracing_etw::LayerBuilder::new("SampleProviderName").build().unwrap())
//!     .init();
//!
//! event!(Level::INFO, fieldB = b'x', fieldA = 7, "Event Message!");
//! ```
//! 
//! ## etw_event macro
//! 
//! **Despite the name, this macro works for both ETW and user_events.**
//! 
//! The `etw_event!` macro is an **optional** additional logging macro
//! based on  `event!` that adds keyword, tags, and event-name support.
//! Keywords are a fundamental part of efficient event filtering in ETW,
//! and naming events make them easier to understand in tools like WPA.
//! It is highly recommended that every event have a non-zero keyword;
//! the [LayerBuilder::with_default_keyword] function can set the default keyword assigned
//! to every event logged through the `tracing` macros (e.g. `event!`).
//! 
//! This extra information is stored as static metadata in the final
//! compiled binary, and relies on linker support to work properly.
//! It has been tested with Microsoft's, GCC's, and LLVM's linker.
//!
//! ## Performance Details
//! 
//! Performance will vary from machine to machine, but this crate should be
//! fast enough to log tens of thousands of events per second.
//! 
//! There are benchmarks available in the code, but they currently rely on
//! an unpublished crate to start and stop an ETW tracing session
//! (and rely on the user to manually start collecting events with the
//! `perf` tool on Linux). Future work will make these easier to run locally.
//!
//! ### Disabled Events
//! 
//! When an event is not enabled by a ETW session or Linux tracing session,
//! the cost of logging an event should be effectively zero. On Windows,
//! this is a few instructions to check the process-local enablement mask
//! and skip over performing any further work if the event is not enabled.
//! On Linux, this enablement check involves a syscall to ask the kernel
//! if the event is enabled, though Linux syscalls are significantly faster
//! than performing a syscall on Windows, and the total overhead remains small.
//! 
//! ### Enabled Events
//! 
//! When an event is enabled by a collector, in addition to the unavoidable
//! overhead from the `tracing` crate (which is itself quite minimal), this
//! crate will dynamically convert the `tracing` event into a Tracelogging/EventHeader
//! event. Users of the C macros for these logging APIs may be familiar with
//! how they generate the event metadata statically at compile time and copied
//! directly to the kernel with the event payload values. While the `tracing`
//! logging macros are able to generate metadata statically at compile time
//! too, this metadata is not in the correct format for ETW/user_events.
//! Instead, when an event is logged, this crate will iterate over the
//! fields in the event and dynamically generate an equivalent field in the
//! new event. This is in line with how every other `tracing` layer works,
//! however, it may be unexpected for those coming from Tracelogging for C/C++.
//! For the most part this translation is quite fast and should not be any slower
//! than any other `tracing` layer.
//! 
//! Events are logged synchronously through to the kernel. They are then delivered
//! to consumers asynchronously per the platform design.
//! 
//! ### Heap Allocations
//! 
//! Each `tracing-etw::Layer` that is added will heap allocate the provider name and GUID.
//! 
//! Logging events with the [std::fmt::Debug](debug format specifier (`:?`)) will
//! necessitate a heap allocation to format the value into a string.
//! 
//! Logging strings copies them to the heap first. This is a side-effect of how
//! `tracing` presents the strings to each layer; the lifetime of the string is
//! too short for what this crate currently needs, but it may be possible to improve
//! this in the future.
//! 
//! Logging a span allocates a copy of the span's fields on the heap. This is needed
//! so the values can be updated during execution and the final payload values logged
//! when the span ends. This allocation is freed when the span ends.
//! 
//! The first time an event is logged (the event is enabled at the platform layer and
//! the logging code is run), this crate will scan the binary for any metadata left
//! by the `etw_event!` macro. This information will be cached in a single heap
//! allocation for later use by other logging calls. This cached memory is never freed
//! until the process exits; if this crate is used in a dynamic library that unloads
//! before the process exits, the memory will be leaked.
//! 
//! A thread-local event builder is allocated for each thread that logs an event.
//! This allows for complete thread safety when logging events. This allocation
//! will stay alive until the thread ends. Additionally, the builder itself will allocate
//! scratch space for constructing the event. This scratch space will grow to fit the
//! very largest event that has been logged so far, but will not shrink. Generally,
//! this should not be much more than a few kilobytes per-thread.
//! 
//! ### Miscellaneous
//! 
//! This crate attempts to avoid dynamic dispatch in the critical logging path.
//! This may result in some very large type names, especially for the global
//! subscriber.
//! 
//! ## Alternative Crates
//! 
//! There are a few other crates that also convert `tracing` events to ETW and/or user_events.
//! - [win_etw_tracing](https://crates.io/crates/win_etw_tracing): This crate should be
//!   considered superseded by `tracing-etw` when it comes to utilizing it with `tracing`.
//!   For logging directly to ETW without going through `tracing` it may still be used, though
//!   the ETW team recommends using the [tracelogging] crate for this instead, as it allows the use
//!   of features such as keywords, which are considered best practice for ETW events.
//! - OpenTelemetry ETW / user_events exporters with `tracing-opentelemetry` layer: These crates
//!   have significant performance overhead, and sending events through OpenTelemetry adds additional
//!   overhead and latency. Events that are disabled at the platform layer still appear as
//!   enabled to `tracing`, and will always have the performance cost of being sent through the
//!   OpenTelemetry export pipeline even if they will then be thrown away. Some implementations
//!   of this export pipeline double-buffer events for processing, and the event may not have
//!   been delivered to the kernel by the time the event logging function has returned. Some
//!   export pipelines are also strictly single-threaded when it comes to exporting events in
//!   the queue.

// only enables the `doc_cfg` feature when
// the `docsrs` configuration attribute is defined
#![cfg_attr(docsrs, feature(doc_cfg))]

mod layer_builder;
// Module that abstracts the native ETW and Linux user_events APIs, depending on the target platform.
// Consumers of the crate should not need to use this module directly.
#[doc(hidden)]
pub mod native;
mod values;
mod statics;
// Module holding internal details that need to be public but should not be directly used by consumers of the crate.
#[doc(hidden)]
pub mod _details;
pub mod error;

pub use layer_builder::LayerBuilder;

mod layer;

#[macro_export]
macro_rules! etw_event {
    (target: $target:expr, name: $name:expr, $lvl:expr, $kw:expr, $tags:expr, { $($fields:tt)* } )=> ({
        use tracing::Callsite;
        use paste::paste;

        static CALLSITE: tracing::callsite::DefaultCallsite =
            tracing::callsite::DefaultCallsite::new(
            {
                const EVENT_NAME: &'static str = $name;
                static META: tracing::metadata::Metadata =
                    tracing::metadata::Metadata::new(
                        EVENT_NAME,
                        $target,
                        $lvl,
                        Some(file!()),
                        Some(line!()),
                        Some(module_path!()),
                        tracing::field::FieldSet::new(tracing::fieldset!( $($fields)* ), tracing_core::identify_callsite!(&CALLSITE)),
                        tracing::metadata::Kind::EVENT,
                );
                &META
            }
        );

        static ETW_META: $crate::_details::EventMetadata = $crate::_details::EventMetadata{
            kw: $kw,
            identity: tracing_core::identify_callsite!(&CALLSITE),
            event_tag: $tags as u32
        };

        paste! {
            #[cfg(target_os = "linux")]
            #[link_section = "_etw_kw"]
            #[allow(non_upper_case_globals)]
            static mut [<ETW_META_PTR $name>]: *const $crate::_details::EventMetadata = &ETW_META;
        }

        paste! {
            #[cfg(target_os = "windows")]
            #[link_section = ".rsdata$zRSETW5"]
            #[allow(non_upper_case_globals)]
            static mut [<ETW_META_PTR $name>]: *const $crate::_details::EventMetadata = &ETW_META;
        }

        let enabled = tracing::level_enabled!($lvl) && {
            let interest = CALLSITE.interest();
            !interest.is_never() && tracing::__macro_support::__is_enabled(CALLSITE.metadata(), interest)
        };
        if enabled {
            (|value_set: tracing::field::ValueSet| {
                let meta = CALLSITE.metadata();
                // event with contextual parent
                tracing::Event::dispatch(
                    meta,
                    &value_set
                );
                tracing::__tracing_log!(
                    $lvl,
                    CALLSITE,
                    &value_set
                );
            })(tracing::valueset!(CALLSITE.metadata().fields(), $($fields)*));
        } else {
            tracing::__tracing_log!(
                $lvl,
                CALLSITE,
                &tracing::valueset!(CALLSITE.metadata().fields(), $($fields)*)
            );
        }
    });
    (target: $target:expr, name: $name:expr, $lvl:expr, $kw:expr, { $($fields:tt)* }, $($arg:tt)+ ) => (
        $crate::etw_event!(
            target: $target,
            name: $name,
            $lvl,
            $kw,
            0,
            { message = format_args!($($arg)+), $($fields)* }
        )
    );
    (target: $target:expr, name: $name:expr, $lvl:expr, $kw:expr, $($k:ident).+ = $($fields:tt)* ) => (
        $crate::etw_event!(target: $target, name: $name, $lvl, $kw, 0, { $($k).+ = $($fields)* })
    );
    (target: $target:expr, name: $name:expr, $lvl:expr, $kw:expr, $($arg:tt)+ ) => (
        $crate::etw_event!(target: $target, name: $name, $lvl, $kw, 0, { $($arg)+ })
    );
    (name: $name:expr, $lvl:expr, $kw:expr, { $($fields:tt)* }, $($arg:tt)+ ) => (
        $crate::etw_event!(
            target: module_path!(),
            name: $name,
            $lvl,
            $kw,
            0,
            { message = format_args!($($arg)+), $($fields)* }
        )
    );
    (name: $name:expr, $lvl:expr, $kw:expr, { $($fields:tt)* }, $($arg:tt)+ ) => (
        $crate::etw_event!(
            target: module_path!(),
            name: $name,
            $lvl,
            $kw,
            0,
            { message = format_args!($($arg)+), $($fields)* }
        )
    );
    (name: $name:expr, $lvl:expr, $kw:expr, $($k:ident).+ = $($field:tt)*) => (
        $crate::etw_event!(
            target: module_path!(),
            name: $name,
            $lvl,
            $kw,
            0,
            { $($k).+ = $($field)*}
        )
    );
    (name: $name:expr, $lvl:expr, $kw:expr, $($k:ident).+, $($field:tt)*) => (
        $crate::etw_event!(
            target: module_path!(),
            name: $name,
            $lvl,
            $kw,
            0,
            { $($k).+, $($field)*}
        )
    );
    (name: $name:expr, $lvl:expr, $kw:expr, ?$($k:ident).+, $($field:tt)*) => (
        $crate::etw_event!(
            target: module_path!(),
            name: $name,
            $lvl,
            $kw,
            0,
            { ?$($k).+, $($field)*}
        )
    );
    (name: $name:expr, $lvl:expr, $kw:expr, %$($k:ident).+, $($field:tt)*) => (
        $crate::etw_event!(
            target: module_path!(),
            name: $name,
            $lvl,
            $kw,
            0,
            { %$($k).+, $($field)*}
        )
    );
    (name: $name:expr, $lvl:expr, $kw:expr, ?$($k:ident).+) => (
        $crate::etw_event!(name: $name, $lvl, $kw, 0, ?$($k).+,)
    );
    (name: $name:expr, $lvl:expr, $kw:expr, %$($k:ident).+) => (
        $crate::etw_event!(name: $name, $lvl, $kw, 0, %$($k).+,)
    );
    (name: $name:expr, $lvl:expr, $kw:expr, $($k:ident).+) => (
        $crate::etw_event!(name: $name, $lvl, $kw, 0, $($k).+,)
    );
    (name: $name:expr, $lvl:expr, $kw:expr, $($arg:tt)+ ) => (
        $crate::etw_event!(target: module_path!(), name: $name, $lvl, $kw, 0, { $($arg)+ })
    );
}

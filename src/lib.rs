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
//!     .with(tracing_etw::LayerBuilder::new("SampleProviderName").build())
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
//! the [default_keyword] function can set the default keyword assigned
//! to every event logged through the `tracing` macros (e.g. `event!`).
//! 
//! This extra information is stored as static metadata in the final
//! compiled binary, and relies on linker support to work properly.
//! It has been tested with Microsoft's, GCC's, and LLVM's linker.
//! 

mod layer;
// Module that abstracts the native ETW and Linux user_events APIs, depending on the target platform.
// Consumers of the crate should not need to use this module directly.
#[doc(hidden)]
pub mod native;
mod values;
mod statics;
// Module holding internal details that need to be public but should not be directly used by consumers of the crate.
#[doc(hidden)]
pub mod _details;

pub use layer::LayerBuilder;

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

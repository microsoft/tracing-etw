//! Emit [ETW] events in [tracing]-enabled Rust applications.
//!
//! ```rust
//! use tracing::{info};
//! use tracing_subscriber::{self, layer::SubscriberExt};
//! 
//! #[tracing::instrument]
//! fn example() {
//!     for i in 0..10 {
//!         info!("{}", i);
//!     }
//! }
//! 
//! fn main() {
//!     tracing::subscriber::set_global_default(
//!         tracing_subscriber::registry().with(tracing_etw::EtwLayer::new(true)),
//!     )
//!     .expect("setup the subscriber");
//! 
//!     example();
//! }
//! ```
//! 
//! # Instrumentation
//! 
//! You can configure whether to emit span enter/exit ETW events by passing in a bool into `EtwLayer::new`.
//!
//! [ETW]: https://docs.microsoft.com/en-us/windows/win32/etw/about-event-tracing
//! [tracing]: https://tracing.rs

use std::{fmt::Write};

use tracing_core::{
    Level,
    field::{Field, Visit},
    span::{Attributes, Id},
    Event, Subscriber,
};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry,
};

use win_etw_macros::trace_logging_provider;
use win_etw_provider::EventOptions;

/// GUID of the tracing-etw provider, enable this GUID during collection
pub const PROVIDER_GUID: &'static str = "9c211c60-a6bc-43c3-8d4d-232c121b1852";

#[trace_logging_provider(guid = "9c211c60-a6bc-43c3-8d4d-232c121b1852")]
pub trait TracingProvider {
    fn NewSpan(span_id: u64, name: &str, file: &str, line: u32);
    fn EnterSpan(span_id: u64);
    fn ExitSpan(span_id: u64);
    fn Event(span_id: u64, message: &str);
}

/// A tracing layer that collects data in ETW profiling format.
pub struct EtwLayer {
    enable_instrumentation: bool,
    etw_events: TracingProvider,
}

impl EtwLayer {
    /// Create a new `EtwLayer`.
    /// 
    /// enable_instrumentation configures whether it emits span enter/exit ETW events
    pub fn new(enable_instrumentation: bool) -> Self {
        Self {
            enable_instrumentation,
            etw_events: TracingProvider::new(),
        }
    }
}

impl<S> Layer<S> for EtwLayer
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let metadata = span.metadata();
        let file = metadata.file().unwrap_or("<not available>");
        let line = metadata.line().unwrap_or(0);
        let name = format!("{}{}", metadata.name(), span.fields());
        self.etw_events.NewSpan(make_etw_options(metadata.level()).as_ref(), id.into_u64(), &name, file, line);
    }

    fn on_enter(&self, id: &Id, ctx: Context<S>) {
        if !self.enable_instrumentation {
            return;
        }

        if let Some(span) = ctx.span(id) {
            self.etw_events.EnterSpan(make_etw_options(span.metadata().level()).as_ref(), id.into_u64());
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<S>) {
        if !self.enable_instrumentation {
            return;
        }

        if let Some(span) = ctx.span(id) {
            self.etw_events.ExitSpan(make_etw_options(span.metadata().level()).as_ref(), id.into_u64());
        }
    }

    fn on_event(&self, event: &Event, _: Context<'_, S>) {
        let mut visitor = EtwEventFieldVisitor::new();
        event.record(&mut visitor);
        let span_id = event.parent().map(|id| id.into_u64()).unwrap_or(0);
        self.etw_events.Event(make_etw_options(event.metadata().level()).as_ref(), span_id, &visitor.msg);
    }
}

fn tracing_lvl_to_etw_lvl(tracing_lvl: &Level) -> win_etw_provider::Level {
    match tracing_lvl {
        i if i <= &Level::ERROR => win_etw_provider::Level::CRITICAL,
        i if i <= &Level::WARN => win_etw_provider::Level::WARN,
        i if i <= &Level::INFO => win_etw_provider::Level::INFO,
        i if i <= &Level::DEBUG => win_etw_provider::Level::VERBOSE,
        i if i <= &Level::TRACE => win_etw_provider::Level::VERBOSE,
        _ => win_etw_provider::Level::NONE,
    }
}

fn make_etw_options(lvl: &Level) -> Option<EventOptions> {
    Some(EventOptions {
        level: Some(tracing_lvl_to_etw_lvl(lvl)),
        ..Default::default()
    })

}

struct EtwEventFieldVisitor {
    msg: String,
}

impl EtwEventFieldVisitor {
    fn new() -> Self {
        Self {
            msg: String::new(),
        }
    }
}

impl Visit for EtwEventFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(&mut self.msg, "{} = {:?};", field.name(), value);
    }
}
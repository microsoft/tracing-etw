//! OpenTelemetry integration helpers.
//!
//! Extracts OpenTelemetry trace context through tracing-opentelemetry's public API.

use std::io::{Cursor, Write};

use opentelemetry::trace::TraceContextExt;
use tracing::{Dispatch, span::Id};

use crate::layer::common::OtelSpanStrings;

pub(crate) fn get_otel_span_data(id: &Id, dispatch: &Dispatch) -> Option<OtelSpanStrings> {
    // This API can activate the span, so no registry extension lock may be held.
    let context = tracing_opentelemetry::get_otel_context(id, dispatch)?;
    let span = context.span();
    let span_context = span.span_context();
    if !span_context.is_valid() {
        return None;
    }

    let mut ctx = OtelSpanStrings {
        trace_id: [0u8; 32],
        span_id: [0u8; 16],
        parent_span_id: None,
    };
    write!(
        Cursor::new(&mut ctx.trace_id[..]),
        "{:032x}",
        span_context.trace_id()
    )
    .expect("A trace ID fits in 32 hexadecimal bytes");
    write!(
        Cursor::new(&mut ctx.span_id[..]),
        "{:016x}",
        span_context.span_id()
    )
    .expect("A span ID fits in 16 hexadecimal bytes");
    Some(ctx)
}

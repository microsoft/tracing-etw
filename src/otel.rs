//! OpenTelemetry integration helpers.
//!
//! This module provides utilities for extracting OpenTelemetry trace context
//! from tracing span extensions when the `opentelemetry` feature is enabled.

use std::io::{Cursor, Write};

use tracing_subscriber::registry::LookupSpan;

use crate::layer::common::OtelSpanStrings;

// Attempt to read the `OtelData` span extension that is set by
// `tracing-opentelemetry` and extract the trace_id and span_id from it.
pub(crate) fn get_otel_span_data<'a, R>(span: &tracing_subscriber::registry::SpanRef<'a, R>) -> Option<OtelSpanStrings>
where
    R: LookupSpan<'a>,
{
    let extensions = span.extensions();
    if let Some(otel_data) = extensions.get::<tracing_opentelemetry::OtelData>() {
        let mut ctx = OtelSpanStrings {
            trace_id: [0u8; 32],
            span_id: [0u8; 16],
            parent_span_id: None,
        };

        // Format IDs while avoiding heap allocations

        if let Some(trace_id) = otel_data.trace_id() {
            let mut trace_id_cursor = Cursor::new(&mut ctx.trace_id[..]);
            let _ = write!(trace_id_cursor, "{:032x}", trace_id); // Ignore errors since we're writing to a fixed-size buffer
        }

        if let Some(span_id) = otel_data.span_id() {
            let mut span_id_cursor = Cursor::new(&mut ctx.span_id[..]);
            let _ = write!(span_id_cursor, "{:016x}", span_id); // Ignore errors since we're writing to a fixed-size buffer
        }

        Some(ctx)
    }
    else {
        None
    }
}

/// Extract OpenTelemetry context from a parent span's extensions.
///
/// Similar to `get_otel_span_data` but takes an optional span reference
/// and returns None if the span is None or has no OTel context.
pub(crate) fn extract_parent_otel_context<'a, R>(
    parent: Option<tracing_subscriber::registry::SpanRef<'a, R>>,
) -> Option<OtelSpanStrings>
where
    R: LookupSpan<'a>,
{
    parent.map(|p| get_otel_span_data(&p)).flatten()
}

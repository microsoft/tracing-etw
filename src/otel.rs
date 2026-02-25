//! OpenTelemetry integration helpers.
//!
//! This module provides utilities for extracting OpenTelemetry trace context
//! from tracing span extensions when the `opentelemetry` feature is enabled.

use tracing_subscriber::registry::LookupSpan;

/// Represents OpenTelemetry trace context extracted from a span.
#[derive(Debug, Clone, Copy, Default)]
pub struct OtelContext {
    /// The trace ID as a 32-character lowercase hex string.
    pub trace_id: [u8; 32],
    /// The span ID as a 16-character lowercase hex string.
    pub span_id: [u8; 16],
    /// Whether valid OTel context was found.
    pub is_valid: bool,
}

impl OtelContext {
    /// Create an invalid/empty OTel context
    pub const fn empty() -> Self {
        Self {
            trace_id: [0u8; 32],
            span_id: [0u8; 16],
            is_valid: false,
        }
    }
}

/// Extract OpenTelemetry context from a span's extensions.
///
/// This function attempts to read the `OtelData` extension that is set by
/// `tracing-opentelemetry` and extract the trace_id and span_id from it.
///
/// Returns an `OtelContext` with `is_valid = true` if OTel context was found,
/// otherwise returns a default context with `is_valid = false`.
pub fn extract_otel_context<'a, R>(span: &tracing_subscriber::registry::SpanRef<'a, R>) -> OtelContext
where
    R: LookupSpan<'a>,
{
    let extensions = span.extensions();
    if let Some(otel_data) = extensions.get::<tracing_opentelemetry::OtelData>() {
        // In tracing-opentelemetry 0.32+, OtelData provides trace_id() and span_id() methods
        let trace_id = otel_data.trace_id();
        let span_id = otel_data.span_id();

        if let (Some(tid), Some(sid)) = (trace_id, span_id) {
            return format_otel_context(tid, sid);
        }
    }

    OtelContext::empty()
}

/// Format trace_id and span_id into an OtelContext with lowercase hex string
/// bytes.
fn format_otel_context(
    tid: opentelemetry::trace::TraceId,
    sid: opentelemetry::trace::SpanId,
) -> OtelContext {
    let mut ctx = OtelContext {
        trace_id: [0u8; 32],
        span_id: [0u8; 16],
        is_valid: true,
    };

    // Format trace_id as 32-character lowercase hex
    let tid_bytes = tid.to_bytes();
    for (i, byte) in tid_bytes.iter().enumerate() {
        let hex = format!("{:02x}", byte);
        ctx.trace_id[i * 2] = hex.as_bytes()[0];
        ctx.trace_id[i * 2 + 1] = hex.as_bytes()[1];
    }

    // Format span_id as 16-character lowercase hex
    let sid_bytes = sid.to_bytes();
    for (i, byte) in sid_bytes.iter().enumerate() {
        let hex = format!("{:02x}", byte);
        ctx.span_id[i * 2] = hex.as_bytes()[0];
        ctx.span_id[i * 2 + 1] = hex.as_bytes()[1];
    }

    ctx
}

/// Extract OpenTelemetry context from a parent span's extensions.
///
/// Similar to `extract_otel_context` but takes an optional span reference
/// and returns None if the span is None or has no OTel context.
pub fn extract_parent_otel_context<'a, R>(
    parent: Option<tracing_subscriber::registry::SpanRef<'a, R>>,
) -> Option<OtelContext>
where
    R: LookupSpan<'a>,
{
    parent.map(|p| {
        let ctx = extract_otel_context(&p);
        if ctx.is_valid {
            Some(ctx)
        } else {
            None
        }
    }).flatten()
}

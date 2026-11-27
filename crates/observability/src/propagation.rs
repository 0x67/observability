//! W3C trace context as plain strings, so callers need no `opentelemetry` dependency.
//!
//! Call order matters: a span's OTel context starts the first time it is entered or its
//! headers are read, and its parent is fixed from then on. Set a remote parent with
//! [`set_parent_from_headers`] right after creating the span, then enter it or call
//! [`trace_headers`].

use std::collections::HashMap;

use opentelemetry::{propagation::TextMapPropagator, trace::TraceContextExt};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_opentelemetry::{OpenTelemetrySpanExt, SetParentError};

use crate::error::PropagationError;

/// `traceparent`, plus `tracestate` when set, for `span`, to send on an outgoing request.
/// Empty when the span is not exported.
///
/// This starts the span's OTel context, so set a remote parent before calling it.
pub fn trace_headers(span: &tracing::Span) -> Vec<(String, String)> {
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(&span.context(), &mut carrier);
    carrier.into_iter().collect()
}

/// Make `span` a child of the remote span that the incoming `headers` describe. Header
/// names match without regard to case. Call it before the span is first entered.
///
/// Without a valid `traceparent` header the span keeps its local parent and this returns
/// `Ok`.
///
/// # Errors
///
/// [`PropagationError::NoOtelLayer`] when the pipeline has no tracing,
/// [`PropagationError::AlreadyStarted`] when the span was entered or its headers were read,
/// and [`PropagationError::SpanDisabled`] when filters drop the span.
pub fn set_parent_from_headers<'a>(
    span: &tracing::Span,
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), PropagationError> {
    // the HashMap extractor looks keys up in lowercase
    let carrier: HashMap<String, String> = headers
        .into_iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    let parent = TraceContextPropagator::new().extract(&carrier);

    // an empty context would replace the local parent with none
    if !parent.span().span_context().is_valid() {
        return Ok(());
    }
    span.set_parent(parent).map_err(|error| match error {
        SetParentError::LayerNotFound => PropagationError::NoOtelLayer,
        SetParentError::AlreadyStarted => PropagationError::AlreadyStarted,
        SetParentError::SpanDisabled => PropagationError::SpanDisabled,
    })
}

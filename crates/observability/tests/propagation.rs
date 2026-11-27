//! `traceparent` round-trips through the string API, a sampled remote parent samples its
//! child under a zero ratio, and setting the parent too late is an error.
#![cfg(feature = "otel")]

use std::sync::OnceLock;

use observability::{
    OtlpConfig, PipelineConfig, PipelineGuard, PropagationError, TracingConfig, init_pipeline,
    propagation::{set_parent_from_headers, trace_headers},
};

const REMOTE_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const REMOTE_SPAN_ID: &str = "00f067aa0ba902b7";

// one pipeline per process; nextest runs each test in its own process
fn pipeline() -> &'static PipelineGuard {
    static GUARD: OnceLock<PipelineGuard> = OnceLock::new();
    GUARD.get_or_init(|| {
        // SAFETY: every test here calls this first; OnceLock runs it once while the others wait
        unsafe { std::env::remove_var("RUST_LOG") };
        // nothing is exported: the guard never drops and the batch delay outlasts the test
        init_pipeline(PipelineConfig {
            otlp: Some(OtlpConfig::default()),
            logging: None,
            tracing: Some(TracingConfig {
                sample_ratio: Some(0.0),
                ..TracingConfig::default()
            }),
            ..Default::default()
        })
        .expect("pipeline installs")
    })
}

// @lat: [[tests#Trace context propagation#Remote parent round trips and overrides zero ratio]]
#[test]
fn remote_parent_round_trips_and_overrides_zero_ratio() {
    pipeline();
    let incoming = format!("00-{REMOTE_TRACE_ID}-{REMOTE_SPAN_ID}-01");
    let span = tracing::info_span!("child");

    let set = set_parent_from_headers(&span, [("TraceParent", incoming.as_str())]);
    let headers = trace_headers(&span);

    assert!(set.is_ok(), "{set:?}");
    let traceparent = headers
        .iter()
        .find_map(|(name, value)| (name == "traceparent").then_some(value))
        .unwrap_or_else(|| panic!("no traceparent in {headers:?}"));
    assert!(
        traceparent.starts_with(&format!("00-{REMOTE_TRACE_ID}-")),
        "{traceparent}"
    );
    assert!(!traceparent.contains(REMOTE_SPAN_ID), "{traceparent}");
    assert!(traceparent.ends_with("-01"), "{traceparent}");
}

// @lat: [[tests#Trace context propagation#Set parent after headers returns already started]]
#[test]
fn set_parent_after_headers_returns_already_started() {
    pipeline();
    let incoming = format!("00-{REMOTE_TRACE_ID}-{REMOTE_SPAN_ID}-01");
    let span = tracing::info_span!("started");

    let _ = trace_headers(&span);
    let set = set_parent_from_headers(&span, [("traceparent", incoming.as_str())]);

    assert!(
        matches!(set, Err(PropagationError::AlreadyStarted)),
        "{set:?}"
    );
}

// @lat: [[tests#Trace context propagation#Headers without traceparent keep the local parent]]
#[test]
fn headers_without_traceparent_keep_the_local_parent() {
    pipeline();
    let parent = tracing::info_span!("local-parent");
    let child = parent.in_scope(|| tracing::info_span!("local-child"));

    let set = set_parent_from_headers(&child, [("x-request-id", "r1")]);

    assert!(set.is_ok(), "{set:?}");
    let trace_id = |span: &tracing::Span| {
        let headers = trace_headers(span);
        let traceparent = headers
            .into_iter()
            .find_map(|(name, value)| (name == "traceparent").then_some(value))
            .expect("the span has a traceparent");
        traceparent.split('-').nth(1).map(str::to_owned)
    };
    assert_eq!(trace_id(&child), trace_id(&parent));
}

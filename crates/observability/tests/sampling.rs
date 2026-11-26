//! A zero sample ratio drops every root span from export, while a child of a sampled
//! remote parent is still exported.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use observability::{OtlpConfig, PipelineConfig, TracingConfig, init_pipeline};
use opentelemetry::{
    Context,
    trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
};
use tracing_opentelemetry::OpenTelemetrySpanExt;

// @lat: [[tests#Tracing config#Zero ratio exports no root spans]]
#[test]
fn zero_ratio_exports_no_root_spans() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None);
    let cfg = PipelineConfig {
        otlp: Some(OtlpConfig {
            endpoint: base,
            ..Default::default()
        }),
        logging: None,
        tracing: Some(TracingConfig {
            sample_ratio: Some(0.0),
            ..TracingConfig::default()
        }),
        ..Default::default()
    };
    let sampled_parent = SpanContext::new(
        TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id"),
        SpanId::from_hex("00f067aa0ba902b7").expect("valid span id"),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("root-span").in_scope(|| {});
    // the control: proves spans reach the collector under this sampler
    let child = tracing::info_span!("remote-child-span");
    child
        .set_parent(Context::new().with_remote_span_context(sampled_parent))
        .expect("the OTel span layer is installed");
    child.in_scope(|| {});
    drop(child);
    drop(guard);

    let received = collector.received_text();
    assert!(received.contains("remote-child-span"), "{received}");
    assert!(!received.contains("root-span"), "{received}");
}

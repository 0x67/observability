//! A span sent over HTTP/protobuf reaches the collector with the configured service name and
//! resource attributes.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use observability::{OtlpConfig, OtlpProtocol, PipelineConfig, TracingConfig, init_pipeline};

// @lat: [[tests#Init end to end#Span reaches mock collector with resource]]
#[test]
fn span_reaches_mock_collector_with_resource() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None, 0);
    let cfg = PipelineConfig {
        service_name: "compat-probe".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint: base,
            protocol: OtlpProtocol::HttpProtobuf,
            resource: [("deployment.environment".to_owned(), "compat-ci".to_owned())].into(),
            ..Default::default()
        }),
        tracing: Some(TracingConfig::default()),
        ..Default::default()
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("export-probe").in_scope(|| {});
    drop(guard);

    // protobuf keeps strings as plain UTF-8 runs, so each value shows in the lossy text
    let received = collector.received_text();
    for expected in [
        "POST /v1/traces ",
        "export-probe",
        "service.name",
        "compat-probe",
        "deployment.environment",
        "compat-ci",
    ] {
        assert!(
            received.contains(expected),
            "missing {expected}: {received}"
        );
    }
}

//! The default `OtlpConfig` exports HTTP/protobuf to `{endpoint}/v1/traces`, with no app
//! runtime.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use observability::{OtlpConfig, PipelineConfig, TracingConfig, init_pipeline};

// @lat: [[tests#Config defaults#Default OtlpConfig exports HTTP protobuf to the base URL]]
#[test]
fn default_otlp_config_exports_http_protobuf_to_base_url() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None, 0);
    let cfg = PipelineConfig {
        otlp: Some(OtlpConfig {
            endpoint: base,
            ..Default::default()
        }),
        tracing: Some(TracingConfig::default()),
        ..Default::default()
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("default-span").in_scope(|| {});
    drop(guard);

    // the mock records requests only, so this content-type came from the exporter
    let received = collector.received_text().to_ascii_lowercase();
    assert!(received.contains("post /v1/traces "), "{received}");
    assert!(
        received.contains("content-type: application/x-protobuf"),
        "{received}"
    );
}

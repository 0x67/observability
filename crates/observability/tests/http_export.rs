//! HTTP export sends every configured header from a thread with no tokio runtime.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use std::time::Duration;

use observability::{OtlpConfig, OtlpProtocol, PipelineConfig, TracingConfig, init_pipeline};

// @lat: [[tests#OTLP headers#HTTP export sends configured headers]]
#[test]
fn http_export_sends_configured_headers() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None, 0);
    let cfg = PipelineConfig {
        service_name: "http-export".to_owned(),
        level: "info".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint: base,
            protocol: OtlpProtocol::HttpProtobuf,
            headers: [("x-api-key".to_owned(), "k1".to_owned())].into(),
            timeout: Duration::from_secs(5),
            ..Default::default()
        }),
        logging: None,
        tracing: Some(TracingConfig::default()),
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("http-span").in_scope(|| {});
    drop(guard);

    let received = collector.received_text();
    assert!(received.contains("http-span"), "{received}");
    assert!(received.contains("x-api-key: k1"), "{received}");
}

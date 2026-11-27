//! HTTP export retries a batch the collector answers with 503.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use std::time::Duration;

use observability::{OtlpConfig, OtlpProtocol, PipelineConfig, TracingConfig, init_pipeline};

// @lat: [[tests#Export retry#HTTP export retries after 503]]
#[test]
fn http_export_retries_after_503() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None, 1);
    let cfg = PipelineConfig {
        service_name: "retry-http".to_owned(),
        level: "info".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint: base,
            protocol: OtlpProtocol::HttpProtobuf,
            timeout: Duration::from_secs(5),
            ..Default::default()
        }),
        logging: None,
        tracing: Some(TracingConfig::default()),
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("retried-span").in_scope(|| {});
    drop(guard);

    // the mock records the rejected request too, so a retry shows the span twice
    let received = collector.received_text();
    assert_eq!(
        received.matches("POST /v1/traces ").count(),
        2,
        "{received}"
    );
    assert_eq!(received.matches("retried-span").count(), 2, "{received}");
}

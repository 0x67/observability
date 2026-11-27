//! gRPC export retries a batch the collector answers with `Unavailable`.
#![cfg(feature = "otel")]

#[path = "support/grpc_collector.rs"]
mod grpc_collector;

use std::time::Duration;

use observability::{OtlpConfig, OtlpProtocol, PipelineConfig, TracingConfig, init_pipeline};

// @lat: [[tests#Export retry#gRPC export retries after Unavailable]]
#[test]
fn grpc_export_retries_after_unavailable() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (endpoint, collector) = grpc_collector::start(None, 1);
    let cfg = PipelineConfig {
        service_name: "retry-grpc".to_owned(),
        level: "info".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint,
            protocol: OtlpProtocol::Grpc,
            timeout: Duration::from_secs(5),
            ..Default::default()
        }),
        logging: None,
        tracing: Some(TracingConfig::default()),
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("retried-span").in_scope(|| {});
    drop(guard);

    // the mock records the rejected export too, so a retry shows the span twice
    let received = collector.received_text();
    assert_eq!(
        received.matches("ExportTraceServiceRequest").count(),
        2,
        "{received}"
    );
    assert_eq!(received.matches("retried-span").count(), 2, "{received}");
}

//! gRPC export works from a thread with no tokio runtime and sends every configured header
//! as gRPC metadata.
#![cfg(feature = "otel")]

#[path = "support/grpc_collector.rs"]
mod grpc_collector;

use std::time::Duration;

use observability::{
    OtlpConfig, OtlpProtocol, OtlpTls, PipelineConfig, TraceExporter, TracingConfig, init_pipeline,
};

// @lat: [[tests#OTLP headers#gRPC export without app runtime sends headers as metadata]]
#[test]
fn grpc_export_without_app_runtime_sends_headers_as_metadata() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (endpoint, collector) = grpc_collector::start(None);
    assert!(tokio::runtime::Handle::try_current().is_err());
    let cfg = PipelineConfig {
        service_name: "grpc-export".to_owned(),
        level: "info".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint,
            protocol: OtlpProtocol::Grpc,
            headers: vec![
                ("x-api-key".to_owned(), "k1".to_owned()),
                ("X-Tenant".to_owned(), "t1".to_owned()),
            ],
            resource: Vec::new(),
            timeout: Duration::from_secs(5),
            tls: OtlpTls::default(),
        }),
        logging: None,
        tracing: Some(TracingConfig {
            exporter: TraceExporter::Otlp,
        }),
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("grpc-span").in_scope(|| {});
    drop(guard);

    let received = collector.received_text();
    assert!(received.contains("grpc-span"), "{received}");
    assert!(received.contains("x-api-key: k1"), "{received}");
    assert!(received.contains("x-tenant: t1"), "{received}");
}

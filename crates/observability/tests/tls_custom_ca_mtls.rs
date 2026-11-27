//! With `TlsRoots::CaFileOnly`, the test CA as `ca_file`, and the test client certificate,
//! traces over gRPC and metrics over HTTP reach TLS mocks that require a client certificate.
#![cfg(all(feature = "tls", feature = "metrics-otel"))]

#[path = "support/grpc_collector.rs"]
mod grpc_collector;
#[path = "support/http_collector.rs"]
mod http_collector;
#[path = "support/test_pki.rs"]
mod test_pki;

use std::time::Duration;

use observability::{
    MetricsConfig, MetricsExporter, OtlpConfig, OtlpProtocol, OtlpTls, PipelineConfig,
    TracingConfig, init_metrics, init_pipeline,
};

fn otlp(protocol: OtlpProtocol, endpoint: String, tls: OtlpTls) -> OtlpConfig {
    OtlpConfig {
        endpoint,
        protocol,
        timeout: Duration::from_secs(5),
        tls,
        ..Default::default()
    }
}

// @lat: [[tests#TLS export#Custom CA and client cert export on both transports]]
#[test]
fn custom_ca_and_client_cert_export_on_both_transports() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let pki = test_pki::generate("mtls");
    let (grpc_endpoint, grpc) = grpc_collector::start(Some(pki.grpc_server_tls()), 0);
    let (http_base, http) = http_collector::start(Some(pki.http_server_tls()), 0);

    let pipeline = init_pipeline(PipelineConfig {
        service_name: "tls-mtls".to_owned(),
        level: "info".to_owned(),
        otlp: Some(otlp(OtlpProtocol::Grpc, grpc_endpoint, pki.otlp_tls())),
        logging: None,
        tracing: Some(TracingConfig::default()),
    })
    .expect("pipeline installs");
    let metrics = init_metrics(
        MetricsConfig {
            exporter: MetricsExporter::Otlp,
        },
        Some(&otlp(OtlpProtocol::HttpProtobuf, http_base, pki.otlp_tls())),
        "tls-mtls",
    )
    .expect("metrics install");

    tracing::info_span!("mtls-span").in_scope(|| {});
    metrics::counter!("mtls_counter").increment(1);
    drop(metrics);
    drop(pipeline);

    let traces = grpc.received_text();
    let metrics = http.received_text();
    assert!(traces.contains("mtls-span"), "{traces}");
    assert!(metrics.contains("mtls_counter"), "{metrics}");
}

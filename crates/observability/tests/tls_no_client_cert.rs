//! A TLS collector that requires a client certificate rejects export from a config that has
//! none, on both transports, and dropping the guards still returns.
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
        timeout: Duration::from_secs(2),
        tls,
        ..Default::default()
    }
}

// @lat: [[tests#TLS export#Export without client cert is rejected]]
#[test]
fn export_without_client_cert_is_rejected() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let pki = test_pki::generate("no-client-cert");
    let (grpc_endpoint, grpc) = grpc_collector::start(Some(pki.grpc_server_tls()), 0);
    let (http_base, http) = http_collector::start(Some(pki.http_server_tls()), 0);
    let without_cert = || OtlpTls {
        client_cert: None,
        ..pki.otlp_tls()
    };

    let pipeline = init_pipeline(PipelineConfig {
        service_name: "tls-no-client-cert".to_owned(),
        level: "info".to_owned(),
        otlp: Some(otlp(OtlpProtocol::Grpc, grpc_endpoint, without_cert())),
        logging: None,
        tracing: Some(TracingConfig::default()),
    })
    .expect("pipeline installs");
    let metrics = init_metrics(
        MetricsConfig {
            exporter: MetricsExporter::Otlp,
        },
        Some(&otlp(OtlpProtocol::HttpProtobuf, http_base, without_cert())),
        "tls-no-client-cert",
    )
    .expect("metrics install");

    tracing::info_span!("rejected-span").in_scope(|| {});
    metrics::counter!("rejected_counter").increment(1);
    drop(metrics);
    drop(pipeline);

    assert_eq!(grpc.received_text(), "");
    assert_eq!(http.received_text(), "");
}

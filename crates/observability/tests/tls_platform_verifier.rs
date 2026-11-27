//! With `TlsRoots::Platform` and the test CA as `ca_file` (extra roots for the OS verifier),
//! traces over gRPC and metrics over HTTP reach TLS mocks that require a client certificate.
#![cfg(all(feature = "tls-platform-verifier", feature = "metrics-otel"))]

#[path = "support/grpc_collector.rs"]
mod grpc_collector;
#[path = "support/http_collector.rs"]
mod http_collector;
#[path = "support/test_pki.rs"]
mod test_pki;

use std::time::Duration;

use observability::{
    MetricsConfig, MetricsExporter, OtlpConfig, OtlpProtocol, OtlpTls, PipelineConfig, TlsRoots,
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

// @lat: [[tests#TLS export#Platform verifier with extra root exports on both transports]]
#[test]
fn platform_verifier_with_extra_root_exports_on_both_transports() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let pki = test_pki::generate("platform");
    let (grpc_endpoint, grpc) = grpc_collector::start(Some(pki.grpc_server_tls()), 0);
    let (http_base, http) = http_collector::start(Some(pki.http_server_tls()), 0);
    let platform = || OtlpTls {
        roots: TlsRoots::Platform,
        ..pki.otlp_tls()
    };

    let pipeline = init_pipeline(PipelineConfig {
        service_name: "tls-platform".to_owned(),
        level: "info".to_owned(),
        otlp: Some(otlp(OtlpProtocol::Grpc, grpc_endpoint, platform())),
        logging: None,
        tracing: Some(TracingConfig::default()),
    })
    .expect("pipeline installs");
    let metrics = init_metrics(
        MetricsConfig {
            exporter: MetricsExporter::Otlp,
        },
        Some(&otlp(OtlpProtocol::HttpProtobuf, http_base, platform())),
        "tls-platform",
    )
    .expect("metrics install");

    tracing::info_span!("platform-span").in_scope(|| {});
    metrics::counter!("platform_counter").increment(1);
    drop(metrics);
    drop(pipeline);

    let traces = grpc.received_text();
    let metrics = http.received_text();
    assert!(traces.contains("platform-span"), "{traces}");
    assert!(metrics.contains("platform_counter"), "{metrics}");
}

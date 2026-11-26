//! Dropping `ObsGuard` inside a multi-thread runtime's async context does not panic and
//! delivers the final span and metric over HTTP.
#![cfg(all(feature = "otel", feature = "metrics-otel"))]

#[path = "support/http_collector.rs"]
mod http_collector;

use std::time::Duration;

use observability::{
    MetricsConfig, MetricsExporter, ObsConfig, OtlpConfig, OtlpProtocol, OtlpTls, PipelineConfig,
    TraceExporter, TracingConfig, init,
};

// @lat: [[tests#Guard drop#ObsGuard drop inside async context delivers over HTTP]]
#[test]
fn obs_guard_drop_inside_async_context_delivers_over_http() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None);
    // `init` shares one OtlpConfig, so metrics post to this path too; the mock takes any path
    let cfg = ObsConfig {
        pipeline: PipelineConfig {
            service_name: "drop-multi-thread".to_owned(),
            level: "info".to_owned(),
            otlp: Some(OtlpConfig {
                endpoint: format!("{base}/v1/traces"),
                protocol: OtlpProtocol::HttpProtobuf,
                headers: Vec::new(),
                resource: Vec::new(),
                timeout: Duration::from_secs(5),
                tls: OtlpTls::default(),
            }),
            logging: None,
            tracing: Some(TracingConfig {
                exporter: TraceExporter::Otlp,
            }),
        },
        metrics: Some(MetricsConfig {
            exporter: MetricsExporter::Otlp,
        }),
    };
    let app = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("app runtime starts");

    app.block_on(async {
        let guard = init(cfg).expect("observability installs");
        assert!(guard.metrics.is_some(), "metrics recorder installs");
        tracing::info_span!("final-span").in_scope(|| {});
        metrics::counter!("final_counter").increment(1);
        drop(guard);
    });

    let received = collector.received_text();
    assert!(received.contains("final-span"), "{received}");
    assert!(received.contains("final_counter"), "{received}");
}

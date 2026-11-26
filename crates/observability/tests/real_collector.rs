//! Export over TLS to a real collector, checked by hand:
//!
//! ```sh
//! OBS_TEST_OTLP_HTTPS_ENDPOINT=https://collector.example \
//!     cargo test -p observability --all-features --test real_collector -- --ignored
//! ```
//!
//! The endpoint must serve OTLP over gRPC at its root and over HTTP at `/v1/metrics` under
//! it, as managed collectors on port 443 do. Set `OTEL_EXPORTER_OTLP_HEADERS` for collectors
//! that need an API key; opentelemetry-otlp adds those headers on both transports.
#![cfg(all(feature = "tls", feature = "metrics-otel"))]

use std::{fs, time::Duration};

use observability::{
    LogFormat, LogSink, LogSinkKind, LoggingConfig, MetricsConfig, MetricsExporter, OtlpConfig,
    OtlpProtocol, PipelineConfig, Rotation, TracingConfig, init_metrics, init_pipeline,
};

#[test]
#[ignore = "needs OBS_TEST_OTLP_HTTPS_ENDPOINT, a collector reachable over TLS"]
fn https_export_reaches_real_collector() {
    let Ok(endpoint) = std::env::var("OBS_TEST_OTLP_HTTPS_ENDPOINT") else {
        eprintln!("OBS_TEST_OTLP_HTTPS_ENDPOINT is unset, skipping");
        return;
    };
    let log_dir = std::env::temp_dir().join(format!("obs-real-collector-{}", std::process::id()));
    let otlp = |protocol, endpoint: String| OtlpConfig {
        endpoint,
        protocol,
        timeout: Duration::from_secs(10),
        ..Default::default()
    };

    // export errors from the SDK land in this file at warn or error level
    let pipeline = init_pipeline(PipelineConfig {
        service_name: "real-collector-test".to_owned(),
        level: "warn".to_owned(),
        otlp: Some(otlp(OtlpProtocol::Grpc, endpoint.clone())),
        logging: Some(LoggingConfig {
            sinks: vec![LogSink {
                kind: LogSinkKind::RollingFile {
                    dir: log_dir.clone(),
                    prefix: "export".to_owned(),
                    rotation: Rotation::Never,
                    max_files: None,
                },
                format: LogFormat::Json,
                level: None,
            }],
        }),
        tracing: Some(TracingConfig::default()),
    })
    .expect("pipeline installs");
    let metrics = init_metrics(
        MetricsConfig {
            exporter: MetricsExporter::Otlp,
        },
        Some(&otlp(OtlpProtocol::HttpProtobuf, endpoint)),
        "real-collector-test",
    )
    .expect("metrics install");

    tracing::warn_span!("real-collector-span").in_scope(|| {});
    metrics::counter!("real_collector_counter").increment(1);
    drop(metrics);
    drop(pipeline);

    let logged: String = fs::read_dir(&log_dir)
        .expect("log dir exists")
        .flatten()
        .map(|entry| fs::read_to_string(entry.path()).expect("log file reads"))
        .collect();
    assert!(!logged.contains("\"level\":\"ERROR\""), "{logged}");
}

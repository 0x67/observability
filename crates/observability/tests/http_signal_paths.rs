//! One OTLP/HTTP endpoint is the collector base URL: spans, log records, and metrics each
//! go to their own path under it.
#![cfg(all(feature = "otel-logs", feature = "metrics-otel"))]

#[path = "support/http_collector.rs"]
mod http_collector;

use std::time::Duration;

use observability::{
    LogFormat, LogSink, LogSinkKind, LoggingConfig, MetricsConfig, MetricsExporter, ObsConfig,
    OtlpConfig, OtlpProtocol, PipelineConfig, TracingConfig, init,
};

const SERVICE: &str = "signal-paths-probe";

// @lat: [[tests#OTLP signal paths#One endpoint sends each signal to its path]]
#[test]
fn one_endpoint_sends_each_signal_to_its_path() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None);
    let cfg = ObsConfig {
        pipeline: PipelineConfig {
            service_name: SERVICE.to_owned(),
            level: "info".to_owned(),
            otlp: Some(OtlpConfig {
                endpoint: base,
                protocol: OtlpProtocol::HttpProtobuf,
                timeout: Duration::from_secs(5),
                ..Default::default()
            }),
            logging: Some(LoggingConfig {
                sinks: vec![LogSink {
                    kind: LogSinkKind::Otlp,
                    format: LogFormat::Json,
                    level: None,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            tracing: Some(TracingConfig::default()),
        },
        metrics: Some(MetricsConfig {
            exporter: MetricsExporter::Otlp,
        }),
    };

    let guard = init(cfg).expect("observability installs");
    assert!(guard.metrics.is_some(), "metrics recorder installs");
    tracing::info_span!("path-span").in_scope(|| {});
    tracing::info!("path-log");
    metrics::counter!("path_counter").increment(1);
    drop(guard);

    let received = collector.received_text();
    let traces = posted_to(&received, "/v1/traces");
    assert!(traces.iter().any(|r| r.contains("path-span")), "{received}");
    assert!(traces.iter().all(|r| r.contains(SERVICE)), "{received}");
    let logs = posted_to(&received, "/v1/logs");
    assert!(logs.iter().any(|r| r.contains("path-log")), "{received}");
    let metrics = posted_to(&received, "/v1/metrics");
    assert!(
        metrics.iter().any(|r| r.contains("path_counter")),
        "{received}"
    );
}

// the mock records each request from its request line on, one after another
fn posted_to<'a>(received: &'a str, path: &str) -> Vec<&'a str> {
    let line_start = format!("{path} ");
    received
        .split("POST ")
        .filter(|request| request.starts_with(&line_start))
        .collect()
}

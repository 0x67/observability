//! The OTLP log bridge drops events created on exporter threads and keeps the same events
//! from app threads, whatever their target.
#![cfg(feature = "otel-logs")]

#[path = "support/http_collector.rs"]
mod http_collector;

use std::time::Duration;

use observability::{
    LogFormat, LogSink, LogSinkKind, LoggingConfig, OtlpConfig, OtlpProtocol, PipelineConfig,
    init_pipeline,
};

// @lat: [[tests#OTLP layer filters#Log bridge drops exporter thread events]]
#[test]
fn log_bridge_drops_exporter_thread_events() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start();
    let cfg = PipelineConfig {
        service_name: "http-logs".to_owned(),
        level: "info".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint: format!("{base}/v1/logs"),
            protocol: OtlpProtocol::HttpProtobuf,
            headers: Vec::new(),
            resource: Vec::new(),
            timeout: Duration::from_secs(5),
        }),
        logging: Some(LoggingConfig {
            sinks: vec![LogSink {
                kind: LogSinkKind::Otlp,
                format: LogFormat::Json,
                level: Some("info".to_owned()),
            }],
        }),
        tracing: None,
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::warn!(target: "tonic::transport", "app-tonic-log");
    std::thread::Builder::new()
        .name("OpenTelemetry.Logs.BatchProcessor".to_owned())
        .spawn(|| tracing::warn!("exporter-thread-log"))
        .expect("spawn the exporter-named thread")
        .join()
        .expect("exporter-named thread panicked");
    drop(guard);

    let received = collector.received_text();
    assert!(received.contains("app-tonic-log"), "{received}");
    assert!(!received.contains("exporter-thread-log"), "{received}");
}

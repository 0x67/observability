//! With tracing on, the OTel span layer keeps the global max level at the configured level,
//! and spans below it never reach the collector.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use std::time::Duration;

use observability::{OtlpConfig, OtlpProtocol, PipelineConfig, TracingConfig, init_pipeline};
use tracing::level_filters::LevelFilter;

// @lat: [[tests#OTLP layer filters#Tracing on keeps global max level and drops debug spans]]
#[test]
fn tracing_on_keeps_global_max_level_and_drops_debug_spans() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None);
    let cfg = PipelineConfig {
        service_name: "level-hint".to_owned(),
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
    let max_level = LevelFilter::current();
    tracing::debug_span!("below").in_scope(|| {});
    tracing::info_span!("kept").in_scope(|| {});
    drop(guard);

    assert_eq!(max_level, LevelFilter::INFO);
    let received = collector.received_text();
    assert!(received.contains("kept"), "{received}");
    assert!(!received.contains("below"), "{received}");
}

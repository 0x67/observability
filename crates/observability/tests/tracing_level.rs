//! `TracingConfig::level` filters exported spans apart from the pipeline level.
#![cfg(feature = "otel")]

#[path = "support/http_collector.rs"]
mod http_collector;

use observability::{OtlpConfig, PipelineConfig, TracingConfig, init_pipeline};

// @lat: [[tests#Tracing config#Tracing level filters exported spans]]
#[test]
fn tracing_level_filters_exported_spans() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (base, collector) = http_collector::start(None, 0);
    // the default stdout sink keeps the pipeline at info, so info spans are created
    let cfg = PipelineConfig {
        level: "info".to_owned(),
        otlp: Some(OtlpConfig {
            endpoint: base,
            ..Default::default()
        }),
        tracing: Some(TracingConfig {
            level: Some("warn".to_owned()),
            ..TracingConfig::default()
        }),
        ..Default::default()
    };

    let guard = init_pipeline(cfg).expect("pipeline installs");
    tracing::info_span!("info-span").in_scope(|| {});
    tracing::warn_span!("warn-span").in_scope(|| {});
    drop(guard);

    let received = collector.received_text();
    assert!(received.contains("warn-span"), "{received}");
    assert!(!received.contains("info-span"), "{received}");
}

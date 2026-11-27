//! Dropping `ObsGuard` inside a current_thread runtime returns quickly and delivers the
//! final span and metric over gRPC.
#![cfg(all(feature = "otel", feature = "metrics-otel"))]

#[path = "support/grpc_collector.rs"]
mod grpc_collector;

use std::time::{Duration, Instant};

use observability::{
    MetricsConfig, MetricsExporter, ObsConfig, OtlpConfig, OtlpProtocol, PipelineConfig,
    TracingConfig, init,
};

// @lat: [[tests#Guard drop#ObsGuard drop inside current_thread runtime delivers final batch]]
#[test]
fn obs_guard_drop_inside_current_thread_runtime_delivers_final_batch() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let (endpoint, collector) = grpc_collector::start(None, 0);
    let cfg = ObsConfig {
        pipeline: PipelineConfig {
            service_name: "drop-current-thread".to_owned(),
            level: "info".to_owned(),
            otlp: Some(OtlpConfig {
                endpoint,
                protocol: OtlpProtocol::Grpc,
                timeout: Duration::from_secs(5),
                ..Default::default()
            }),
            logging: None,
            tracing: Some(TracingConfig::default()),
        },
        metrics: Some(MetricsConfig {
            exporter: MetricsExporter::Otlp,
        }),
    };
    let app = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("app runtime starts");

    let drop_took = app.block_on(async {
        let guard = init(cfg).expect("observability installs");
        assert!(guard.metrics.is_some(), "metrics recorder installs");
        tracing::info_span!("final-span").in_scope(|| {});
        metrics::counter!("final_counter").increment(1);

        let started = Instant::now();
        drop(guard);
        started.elapsed()
    });

    assert!(
        drop_took < Duration::from_secs(2),
        "drop took {drop_took:?}"
    );
    let received = collector.received_text();
    assert!(received.contains("final-span"), "{received}");
    assert!(received.contains("final_counter"), "{received}");
}

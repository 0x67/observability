//! Logging, OTLP tracing, and Prometheus metrics in a tokio app.
//!
//! `cargo run -p observability --example quickstart` logs JSON to stdout, sends spans over
//! gRPC to a collector on `localhost:4317`, and serves Prometheus metrics on
//! `127.0.0.1:9000` until it exits. With no collector listening, it logs the failed span
//! export and still exits 0.

use observability::{
    MetricsConfig, ObsConfig, OtlpConfig, OtlpProtocol, PipelineConfig, PipelineError,
    TracingConfig, init,
};

#[tokio::main]
async fn main() -> Result<(), PipelineError> {
    let guard = init(ObsConfig {
        pipeline: PipelineConfig {
            service_name: "quickstart".into(),
            otlp: Some(OtlpConfig {
                endpoint: "http://localhost:4317".into(),
                protocol: OtlpProtocol::Grpc,
                ..Default::default()
            }),
            tracing: Some(TracingConfig::default()),
            ..Default::default()
        },
        metrics: Some(MetricsConfig::default()),
    })?;

    // the span closes when in_scope returns, so the guard drop below exports it
    tracing::info_span!("quickstart").in_scope(|| tracing::info!("observability up"));

    // Export runs on the crate's own runtime, so the guard can drop inside this async fn.
    // Drop shuts the metrics recorder down, then exports the final span batch and flushes
    // the log writers.
    drop(guard);
    Ok(())
}

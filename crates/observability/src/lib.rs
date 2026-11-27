#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]
//! Tracing + metrics backend wiring over the `observability-core` gate.
//!
//! Two independent subsystems that share nothing at runtime except one `OtlpConfig`:
//!
//! - subscriber pipeline: one `tracing_subscriber::Registry`, logging fan-out over N sinks
//!   plus a single tracing OTLP sink (`pipeline`).
//! - metrics recorder: a separate `metrics` global recorder, Prometheus pull or OTLP bridge,
//!   flipping the `observability-core` gate on install (`metrics`).
//!
//! `init` is the thin convenience that runs both. See module docs for each subsystem.

mod config;
mod error;
mod metrics;
mod otlp;
mod pipeline;
mod redact;

pub use config::{
    LogFormat, LogSink, LogSinkKind, LoggingConfig, MetricsConfig, MetricsExporter, PipelineConfig,
    Rotation, TraceExporter, TracingConfig,
};
pub use error::{MetricsError, OtlpError, PipelineError, Signal};
pub use metrics::{MetricsGuard, init_metrics};
// hot-path gate lives in the leaf crate; re-export so a binary drives it through `observability`.
pub use observability_core;
pub use observability_core::{metrics_enabled, set_metrics_enabled};
pub use otlp::{ClientCert, OtlpConfig, OtlpProtocol, OtlpTls, TlsRoots};
pub use pipeline::{PipelineGuard, init_pipeline};

#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
pub struct RootReadmeDoctests;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

/// Both subsystems in one value. `metrics` is optional so a build can run logging + tracing
/// without a recorder. The default is the default pipeline with no metrics recorder.
#[derive(Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct ObsConfig {
    pub pipeline: PipelineConfig,
    pub metrics: Option<MetricsConfig>,
}

/// Held for the process lifetime. Drop shuts the metrics recorder down first (gate off,
/// final metric export), then the pipeline (final span and log export, file writer flush).
#[must_use = "dropping the guard stops the log writers and flushes the exporters; keep it alive for the process lifetime"]
pub struct ObsGuard {
    // declared first so it drops first: the log sinks still print metric shutdown warnings
    pub metrics: Option<MetricsGuard>,
    pub pipeline: PipelineGuard,
}

/// Bring up both subsystems: pipeline first (so a metrics warning is itself logged), then
/// metrics sharing the pipeline's `OtlpConfig` and service name.
///
/// A pipeline failure is fatal and returned. A metrics failure is downgraded to a
/// `tracing::warn!` and `metrics: None`, never taking down logging or tracing.
pub fn init(cfg: ObsConfig) -> Result<ObsGuard, PipelineError> {
    // clone the two values metrics needs before the pipeline config is moved into init.
    let service_name = cfg.pipeline.service_name.clone();
    let otlp = cfg.pipeline.otlp.clone();

    let pipeline = init_pipeline(cfg.pipeline)?;

    let metrics = match cfg.metrics {
        Some(mcfg) => match init_metrics(mcfg, otlp.as_ref(), &service_name) {
            Ok(guard) => Some(guard),
            Err(e) => {
                tracing::warn!(error = %e, "metrics init failed, running without metrics");
                None
            }
        },
        None => None,
    };

    Ok(ObsGuard { metrics, pipeline })
}

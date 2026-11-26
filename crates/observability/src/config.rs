//! Plain-data pipeline config. No `opentelemetry` type in any field; `otlp.rs` is the
//! only module allowed to know about the SDK.

use std::{
    net::{Ipv4Addr, SocketAddr},
    num::NonZeroUsize,
    path::PathBuf,
};

use crate::otlp::OtlpConfig;

/// Top-level pipeline config: service identity, default log level, per-signal wiring.
///
/// The default logs JSON to stdout at `info` for service `unknown_service`, with no OTLP
/// and no tracing.
#[derive(Clone, Debug)]
pub struct PipelineConfig {
    pub service_name: String,
    pub level: String,
    pub otlp: Option<OtlpConfig>,
    pub logging: Option<LoggingConfig>,
    pub tracing: Option<TracingConfig>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            // the name the OTel SDK falls back to when none is set
            service_name: "unknown_service".to_owned(),
            level: "info".to_owned(),
            otlp: None,
            logging: Some(LoggingConfig::default()),
            tracing: None,
        }
    }
}

/// Log fan-out across N sinks. The default is one stdout JSON sink.
#[derive(Clone, Debug)]
pub struct LoggingConfig {
    pub sinks: Vec<LogSink>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            sinks: vec![LogSink::default()],
        }
    }
}

/// One logging sink: destination, wire format, optional per-sink level override.
#[derive(Clone, Debug, Default)]
pub struct LogSink {
    pub kind: LogSinkKind,
    pub format: LogFormat,
    pub level: Option<String>,
}

/// Sink destination.
#[derive(Clone, Debug, Default)]
pub enum LogSinkKind {
    #[default]
    Stdout,
    RollingFile {
        dir: PathBuf,
        prefix: String,
        rotation: Rotation,
        /// Most files with this prefix kept in `dir`, pruned when the sink opens and on
        /// each rotation; `None` keeps every file.
        max_files: Option<NonZeroUsize>,
    },
    Otlp,
}

/// Log line wire format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    #[default]
    Json,
    Bunyan,
    Pretty,
}

/// Rolling-file rotation cadence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    Minutely,
    Hourly,
    #[default]
    Daily,
    Never,
}

/// Tracing subsystem config: which exporter backs the span pipeline.
#[derive(Clone, Debug, Default)]
pub struct TracingConfig {
    pub exporter: TraceExporter,
}

/// Trace exporter choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceExporter {
    #[default]
    Otlp,
}

/// Metrics subsystem config: which recorder backs the `metrics` facade.
#[derive(Clone, Debug, Default)]
pub struct MetricsConfig {
    pub exporter: MetricsExporter,
}

/// Metrics exporter choice: Prometheus pull endpoint or OTLP push bridge. The default is
/// Prometheus on `127.0.0.1:9000`.
#[derive(Clone, Debug)]
pub enum MetricsExporter {
    Prometheus { bind: SocketAddr },
    Otlp,
}

impl Default for MetricsExporter {
    fn default() -> Self {
        // loopback, so a default build does not serve metrics on every interface
        Self::Prometheus {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 9000)),
        }
    }
}

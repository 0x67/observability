//! Error types for pipeline init, metrics recorder install, and OTLP exporter setup.

use std::{fmt, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("global tracing subscriber already initialized")]
    AlreadyInit,
    #[error("{0} export needs `otlp` set in PipelineConfig")]
    MissingOtlp(Signal),
    #[error("the configuration needs the `{feature}` cargo feature")]
    FeatureDisabled { feature: &'static str },
    #[error("cannot open a rolling log file in {}", dir.display())]
    RollingFile {
        dir: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("cannot build the OTLP {signal} exporter")]
    Exporter {
        signal: Signal,
        #[source]
        source: OtlpError,
    },
    #[error("trace sample ratio {0} is outside 0.0..=1.0")]
    SampleRatio(f64),
}

/// Why [`crate::propagation::set_parent_from_headers`] could not set a span's parent.
#[cfg(feature = "otel")]
#[derive(Debug, Error)]
pub enum PropagationError {
    #[error("no OTel span layer is installed; enable tracing in PipelineConfig")]
    NoOtelLayer,
    #[error("the span already started; set its parent before entering it or reading its headers")]
    AlreadyStarted,
    #[error("the span is filtered out and is not exported")]
    SpanDisabled,
}

/// OTLP signal a pipeline error refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Traces,
    Logs,
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Traces => "traces",
            Self::Logs => "logs",
        })
    }
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("failed to bind metrics exporter socket")]
    Bind(#[source] std::io::Error),
    // boxed so error.rs stays decoupled from prometheus/otel recorder error types;
    // metrics.rs picks the concrete source at the install site
    #[error("failed to install metrics recorder")]
    Install(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("cannot build the OTLP metrics exporter")]
    Exporter(#[source] OtlpError),
}

/// Why an OTLP exporter could not be set up. Upstream errors are boxed, so no
/// `opentelemetry`, `reqwest`, or `tonic` type appears in this crate's public API.
#[derive(Debug, Error)]
pub enum OtlpError {
    #[error("cannot start the observability-otlp export runtime")]
    Runtime(#[source] std::io::Error),
    #[error("the OTLP config needs the `{feature}` cargo feature")]
    FeatureDisabled { feature: &'static str },
    #[error(
        "TlsRoots::Native found no certificate in the OS store; use TlsRoots::Webpki or a ca_file"
    )]
    NoNativeRoots,
    #[error("TlsRoots::CaFileOnly needs OtlpTls::ca_file")]
    NoTrustAnchors,
    #[error("cannot read TLS file {}", path.display())]
    TlsFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("TLS file {} does not contain a valid PEM certificate or key", path.display())]
    InvalidPem {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("cannot set up the OS certificate verifier")]
    PlatformVerifier(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    // the value is left out on purpose: header values are usually credentials
    #[error("header {name:?} is not a valid header name and value")]
    InvalidHeader { name: String },
    #[error("exporter build failed")]
    Build(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

#[cfg(feature = "otel")]
impl OtlpError {
    pub(crate) fn build(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Build(Box::new(source))
    }
}

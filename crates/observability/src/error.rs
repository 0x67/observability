//! Error types for pipeline init, metrics recorder install, and OTLP exporter setup.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("global tracing subscriber already initialized")]
    AlreadyInit,
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("failed to bind metrics exporter socket")]
    Bind(#[source] std::io::Error),
    // boxed so error.rs stays decoupled from prometheus/otel recorder error types;
    // metrics.rs picks the concrete source at the install site
    #[error("failed to install metrics recorder")]
    Install(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
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

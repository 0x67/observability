//! OTLP export target shared by span, metric, and log exporters.
//!
//! One `OtlpConfig` builds three sibling exporters off the same endpoint/timeout/headers.
//! Exporter builders stay `pub(crate)` and otel-gated so no `opentelemetry` type leaks
//! into a public signature; `pipeline/otel.rs` and `metrics.rs` call these to wire
//! providers. Every exporter's I/O runs on the crate-owned export runtime (`runtime.rs`).
//!
//! Under `--no-default-features` the otel impl is gated out. `otel_resource` folds
//! `service.name` plus the configured `resource` attrs onto every signal provider, so one
//! `OtlpConfig` drives identical resource attributes across span, metric, and log.

#[cfg(feature = "otel")]
mod http_client;
#[cfg(feature = "otel")]
mod runtime;

use std::{fmt, time::Duration};

#[cfg(feature = "otel")]
use opentelemetry::KeyValue;
#[cfg(feature = "otel")]
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};

#[cfg(feature = "otel")]
use self::http_client::RuntimeHttpClient;
#[cfg(feature = "otel")]
pub(crate) use self::runtime::ExportRuntime;
#[cfg(feature = "otel")]
use crate::error::OtlpError;
use crate::redact::REDACTED;

/// Endpoint, transport, headers, resource attrs, and timeout for the OTLP collector.
///
/// `Debug` prints header names with `[REDACTED]` in place of each value.
#[derive(Clone)]
pub struct OtlpConfig {
    pub endpoint: String,
    pub protocol: OtlpProtocol,
    pub headers: Vec<(String, String)>,
    pub resource: Vec<(String, String)>,
    pub timeout: Duration,
}

impl fmt::Debug for OtlpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // header values are usually credentials; print the names only
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, _)| (name.as_str(), REDACTED))
            .collect();

        f.debug_struct("OtlpConfig")
            .field("endpoint", &self.endpoint)
            .field("protocol", &self.protocol)
            .field("headers", &headers)
            .field("resource", &self.resource)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Wire transport for the OTLP exporters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
}

#[cfg(feature = "otel")]
impl OtlpConfig {
    /// Span exporter for `protocol`, with its I/O on `runtime`.
    pub(crate) fn span_exporter(
        &self,
        runtime: &ExportRuntime,
    ) -> Result<opentelemetry_otlp::SpanExporter, OtlpError> {
        use opentelemetry_otlp::SpanExporter;
        match self.protocol {
            OtlpProtocol::Grpc => {
                self.grpc(SpanExporter::builder().with_tonic(), runtime, |b| b.build())
            }
            OtlpProtocol::HttpProtobuf => self
                .http(SpanExporter::builder().with_http(), runtime)?
                .build()
                .map_err(OtlpError::build),
        }
    }

    /// Metric exporter for `protocol`, with its I/O on `runtime`. Cumulative temporality
    /// (SDK default).
    #[cfg(feature = "metrics-otel")]
    pub(crate) fn metric_exporter(
        &self,
        runtime: &ExportRuntime,
    ) -> Result<opentelemetry_otlp::MetricExporter, OtlpError> {
        use opentelemetry_otlp::MetricExporter;
        use opentelemetry_sdk::metrics::Temporality;
        match self.protocol {
            OtlpProtocol::Grpc => self.grpc(
                MetricExporter::builder()
                    .with_tonic()
                    .with_temporality(Temporality::default()),
                runtime,
                |b| b.build(),
            ),
            OtlpProtocol::HttpProtobuf => self
                .http(
                    MetricExporter::builder()
                        .with_http()
                        .with_temporality(Temporality::default()),
                    runtime,
                )?
                .build()
                .map_err(OtlpError::build),
        }
    }

    /// Log exporter for `protocol`, with its I/O on `runtime`.
    #[cfg(feature = "otel-logs")]
    pub(crate) fn log_exporter(
        &self,
        runtime: &ExportRuntime,
    ) -> Result<opentelemetry_otlp::LogExporter, OtlpError> {
        use opentelemetry_otlp::LogExporter;
        match self.protocol {
            OtlpProtocol::Grpc => {
                self.grpc(LogExporter::builder().with_tonic(), runtime, |b| b.build())
            }
            OtlpProtocol::HttpProtobuf => self
                .http(LogExporter::builder().with_http(), runtime)?
                .build()
                .map_err(OtlpError::build),
        }
    }

    /// Setup shared by every gRPC exporter: endpoint, timeout, and the runtime context
    /// tonic spawns its channel worker into.
    // NOTE: headers wired on http only; tonic's MetadataKey isn't re-exported by
    // opentelemetry-otlp (only MetadataMap is), so a dynamic key needs `tonic` as a
    // direct dep, which this crate does not take.
    fn grpc<B, E>(
        &self,
        builder: B,
        runtime: &ExportRuntime,
        build: impl FnOnce(B) -> Result<E, opentelemetry_otlp::ExporterBuildError>,
    ) -> Result<E, OtlpError>
    where
        B: WithExportConfig,
    {
        let builder = builder
            .with_endpoint(&self.endpoint)
            .with_timeout(self.timeout);

        // connect_lazy spawns the channel worker onto the runtime current at build time
        let _entered = runtime.handle().enter();
        build(builder).map_err(OtlpError::build)
    }

    /// Setup shared by every HTTP exporter: endpoint, timeout, headers, and a client whose
    /// requests run on `runtime`.
    fn http<B: WithExportConfig + WithHttpConfig>(
        &self,
        builder: B,
        runtime: &ExportRuntime,
    ) -> Result<B, OtlpError> {
        let client = RuntimeHttpClient::new(self.reqwest_client()?, runtime.handle().clone());

        Ok(builder
            .with_http_client(client)
            .with_endpoint(&self.endpoint)
            .with_timeout(self.timeout)
            .with_headers(self.http_headers()))
    }

    // opentelemetry-otlp applies its export timeout only to a client it builds itself
    fn reqwest_client(&self) -> Result<reqwest::Client, OtlpError> {
        reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(OtlpError::build)
    }

    /// Headers as a map for the http transport's `with_headers`.
    fn http_headers(&self) -> std::collections::HashMap<String, String> {
        self.headers.iter().cloned().collect()
    }

    /// Resource shared by all three signal providers: `service.name` plus every configured
    /// `resource` pair. One config, identical attributes on span/metric/log.
    pub(crate) fn otel_resource(&self, service_name: &str) -> opentelemetry_sdk::Resource {
        let mut builder =
            opentelemetry_sdk::Resource::builder().with_service_name(service_name.to_string());
        for (key, value) in &self.resource {
            builder = builder.with_attribute(KeyValue::new(key.clone(), value.clone()));
        }
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{OtlpConfig, OtlpProtocol};

    #[cfg(feature = "otel")]
    #[test]
    fn exporters_build_without_app_runtime() {
        use std::sync::PoisonError;

        use super::{ExportRuntime, runtime::tests::SLOT_LOCK};

        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(tokio::runtime::Handle::try_current().is_err());
        let runtime = ExportRuntime::shared().expect("export runtime starts");

        for (protocol, endpoint) in [
            (OtlpProtocol::Grpc, "http://127.0.0.1:4317"),
            (OtlpProtocol::HttpProtobuf, "http://127.0.0.1:4318"),
        ] {
            let cfg = OtlpConfig {
                endpoint: endpoint.to_string(),
                protocol,
                headers: Vec::new(),
                resource: Vec::new(),
                timeout: Duration::from_secs(1),
            };

            if let Err(e) = cfg.span_exporter(&runtime) {
                panic!("{protocol:?} span exporter: {e}");
            }
            #[cfg(feature = "otel-logs")]
            if let Err(e) = cfg.log_exporter(&runtime) {
                panic!("{protocol:?} log exporter: {e}");
            }
            #[cfg(feature = "metrics-otel")]
            if let Err(e) = cfg.metric_exporter(&runtime) {
                panic!("{protocol:?} metric exporter: {e}");
            }
        }
    }

    // @lat: [[tests#OTLP config debug output#Debug prints header names and hides values]]
    #[test]
    fn debug_prints_header_names_and_hides_values() {
        let cfg = OtlpConfig {
            endpoint: "http://collector:4318".to_string(),
            protocol: OtlpProtocol::HttpProtobuf,
            headers: vec![("x-api-key".to_string(), "s3cr3t-value".to_string())],
            resource: vec![("deployment.environment".to_string(), "prod".to_string())],
            timeout: Duration::from_secs(5),
        };

        let out = format!("{cfg:?}");

        assert!(out.contains("\"x-api-key\", \"[REDACTED]\""), "{out}");
        assert!(!out.contains("s3cr3t-value"), "{out}");
        for shown in [
            "http://collector:4318",
            "HttpProtobuf",
            "deployment.environment",
            "prod",
            "5s",
        ] {
            assert!(out.contains(shown), "missing {shown}: {out}");
        }
    }
}

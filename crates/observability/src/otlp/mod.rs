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
mod tls;

use std::{collections::BTreeMap, fmt, time::Duration};

#[cfg(feature = "otel")]
use opentelemetry::KeyValue;
#[cfg(feature = "otel")]
use opentelemetry_otlp::{
    Protocol, WithExportConfig, WithHttpConfig, WithTonicConfig, tonic_types::metadata::MetadataMap,
};

#[cfg(feature = "otel")]
use self::http_client::RuntimeHttpClient;
#[cfg(feature = "otel")]
pub(crate) use self::runtime::{EXPORT_THREAD_NAME, ExportRuntime};
#[cfg(feature = "tls")]
use self::tls::TlsMaterial;
pub use self::tls::{ClientCert, OtlpTls, TlsRoots};
#[cfg(feature = "otel")]
use crate::error::OtlpError;
use crate::redact::REDACTED;

/// Endpoint, transport, headers, resource attrs, timeout, and TLS for the OTLP collector.
///
/// The default is HTTP/protobuf to `http://localhost:4318` with a 10 s timeout, the OTLP
/// defaults. `Debug` prints header names with `[REDACTED]` in place of each value.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct OtlpConfig {
    /// Collector base URL. HTTP export appends `/v1/traces`, `/v1/logs`, or `/v1/metrics`
    /// after dropping one trailing `/`; gRPC uses it as given.
    pub endpoint: String,
    pub protocol: OtlpProtocol,
    pub headers: BTreeMap<String, String>,
    pub resource: BTreeMap<String, String>,
    #[cfg_attr(
        feature = "serde",
        serde(rename = "timeout_ms", with = "crate::config::duration_ms")
    )]
    pub timeout: Duration,
    /// Used only for `https://` endpoints.
    pub tls: OtlpTls,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4318".to_owned(),
            protocol: OtlpProtocol::HttpProtobuf,
            headers: BTreeMap::new(),
            resource: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            tls: OtlpTls::default(),
        }
    }
}

impl fmt::Debug for OtlpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // header values are usually credentials; print the names only
        let headers: Vec<(&str, &str)> = self
            .headers
            .keys()
            .map(|name| (name.as_str(), REDACTED))
            .collect();

        f.debug_struct("OtlpConfig")
            .field("endpoint", &self.endpoint)
            .field("protocol", &self.protocol)
            .field("headers", &headers)
            .field("resource", &self.resource)
            .field("timeout", &self.timeout)
            .field("tls", &self.tls)
            .finish()
    }
}

#[cfg(feature = "otel")]
const TRACES_PATH: &str = "/v1/traces";
#[cfg(feature = "otel-logs")]
const LOGS_PATH: &str = "/v1/logs";
#[cfg(feature = "metrics-otel")]
const METRICS_PATH: &str = "/v1/metrics";

/// Wire transport for the OTLP exporters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum OtlpProtocol {
    Grpc,
    #[default]
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
                .http(SpanExporter::builder().with_http(), runtime, TRACES_PATH)?
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
                    METRICS_PATH,
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
                .http(LogExporter::builder().with_http(), runtime, LOGS_PATH)?
                .build()
                .map_err(OtlpError::build),
        }
    }

    /// Setup shared by every gRPC exporter: endpoint, timeout, headers as metadata, TLS,
    /// and the runtime context tonic spawns its channel worker into.
    fn grpc<B, E>(
        &self,
        builder: B,
        runtime: &ExportRuntime,
        build: impl FnOnce(B) -> Result<E, opentelemetry_otlp::ExporterBuildError>,
    ) -> Result<E, OtlpError>
    where
        B: WithExportConfig + WithTonicConfig,
    {
        self.check_endpoint()?;
        self.check_scheme()?;

        // connect_lazy spawns the channel worker onto the runtime current at build time,
        // whether opentelemetry-otlp builds the channel or `platform_channel` does
        let _entered = runtime.handle().enter();

        // set in code, so an OTEL_EXPORTER_OTLP_PROTOCOL meant for other services is ignored
        let builder = builder
            .with_protocol(Protocol::Grpc)
            .with_endpoint(&self.endpoint)
            .with_timeout(self.timeout)
            .with_metadata(MetadataMap::from_headers(self.header_map()?));
        let builder = self.with_grpc_tls(builder)?;
        build(builder).map_err(OtlpError::build)
    }

    /// Setup shared by every HTTP exporter: the signal's URL under the endpoint, timeout,
    /// headers, and a client whose requests run on `runtime`.
    fn http<B: WithExportConfig + WithHttpConfig>(
        &self,
        builder: B,
        runtime: &ExportRuntime,
        path: &str,
    ) -> Result<B, OtlpError> {
        self.check_endpoint()?;
        self.check_scheme()?;

        // opentelemetry-otlp drops an invalid header without a word, so reject it here
        self.header_map()?;
        let client = RuntimeHttpClient::new(self.reqwest_client()?, runtime.handle().clone());

        // set in code, so an OTEL_EXPORTER_OTLP_PROTOCOL meant for other services is ignored
        Ok(builder
            .with_protocol(Protocol::HttpBinary)
            .with_http_client(client)
            .with_endpoint(self.signal_url(path))
            .with_timeout(self.timeout)
            .with_headers(self.http_headers()))
    }

    /// Rejects a user name or password in `endpoint`. Every exporter setup runs this first,
    /// because later errors, upstream ones included, may print the endpoint.
    fn check_endpoint(&self) -> Result<(), OtlpError> {
        if endpoint_has_credentials(&self.endpoint) {
            return Err(OtlpError::EndpointCredentials);
        }
        Ok(())
    }

    /// Rejects an endpoint without `http://` or `https://`. opentelemetry-otlp can turn a
    /// scheme-less gRPC endpoint into `https://` with no trusted roots, and `is_https` would
    /// skip the crate's TLS setup for it. Runs after `check_endpoint`, since the error prints
    /// the endpoint.
    fn check_scheme(&self) -> Result<(), OtlpError> {
        if self.has_scheme("http://") || self.has_scheme("https://") {
            return Ok(());
        }
        Err(OtlpError::EndpointScheme {
            endpoint: self.endpoint.clone(),
        })
    }

    // opentelemetry-otlp sends to a programmatic HTTP endpoint as given, with no signal path
    fn signal_url(&self, path: &str) -> String {
        let base = self.endpoint.strip_suffix('/').unwrap_or(&self.endpoint);
        format!("{base}{path}")
    }

    // opentelemetry-otlp applies its export timeout only to a client it builds itself
    fn reqwest_client(&self) -> Result<reqwest::Client, OtlpError> {
        let builder = reqwest::Client::builder().timeout(self.timeout);
        #[cfg(feature = "tls")]
        let builder = match self.tls_material()? {
            Some(tls) => tls.apply_to_reqwest(builder)?,
            None => builder,
        };
        #[cfg(not(feature = "tls"))]
        self.reject_https()?;

        builder.build().map_err(OtlpError::build)
    }

    fn with_grpc_tls<B: WithTonicConfig>(&self, builder: B) -> Result<B, OtlpError> {
        #[cfg(feature = "tls")]
        if let Some(tls) = self.tls_material()? {
            #[cfg(feature = "tls-platform-verifier")]
            if tls.uses_platform_verifier() {
                return Ok(builder.with_channel(self.platform_channel(&tls)?));
            }
            return Ok(builder.with_tls_config(tls.tonic_config()));
        }
        #[cfg(not(feature = "tls"))]
        self.reject_https()?;

        Ok(builder)
    }

    /// A channel whose TLS goes through the OS verifier. tonic's `ClientTlsConfig` has no
    /// verifier hook but `Endpoint` does, and opentelemetry-otlp still adds the configured
    /// metadata to a channel it is given.
    #[cfg(feature = "tls-platform-verifier")]
    fn platform_channel(&self, tls: &TlsMaterial) -> Result<tonic::transport::Channel, OtlpError> {
        let (config, verifier) = tls.platform_tonic()?;
        let endpoint = tonic::transport::Channel::from_shared(self.endpoint.clone())
            .map_err(OtlpError::build)?
            .timeout(self.timeout)
            .tls_config_with_verifier(config, verifier)
            .map_err(OtlpError::build)?;
        Ok(endpoint.connect_lazy())
    }

    /// TLS material for an `https://` endpoint; `None` keeps the connection plain text.
    #[cfg(feature = "tls")]
    fn tls_material(&self) -> Result<Option<TlsMaterial>, OtlpError> {
        self.is_https()
            .then(|| TlsMaterial::load(&self.tls))
            .transpose()
    }

    #[cfg(not(feature = "tls"))]
    fn reject_https(&self) -> Result<(), OtlpError> {
        if self.is_https() {
            return Err(OtlpError::FeatureDisabled { feature: "tls" });
        }
        Ok(())
    }

    fn is_https(&self) -> bool {
        self.has_scheme("https://")
    }

    /// True when `endpoint` starts with `scheme`, compared without case.
    fn has_scheme(&self, scheme: &str) -> bool {
        self.endpoint
            .get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    }

    /// Validated headers. The error names the header and never carries its value.
    fn header_map(&self) -> Result<http::HeaderMap, OtlpError> {
        self.headers
            .iter()
            .map(|(name, value)| {
                let invalid = || OtlpError::InvalidHeader { name: name.clone() };
                let name = http::HeaderName::try_from(name.as_str()).map_err(|_| invalid())?;
                let value = http::HeaderValue::try_from(value.as_str()).map_err(|_| invalid())?;
                Ok((name, value))
            })
            .collect()
    }

    /// Headers as a map for the http transport's `with_headers`.
    fn http_headers(&self) -> std::collections::HashMap<String, String> {
        self.headers.clone().into_iter().collect()
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

/// True when the authority part of `endpoint` has a `user@` or `user:password@` prefix.
#[cfg(feature = "otel")]
fn endpoint_has_credentials(endpoint: &str) -> bool {
    let after_scheme = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    // the authority ends at the first `/` only: stopping at `?` or `#` as well would let a
    // password containing one of them pass and reach upstream URI errors
    let authority = after_scheme
        .split_once('/')
        .map_or(after_scheme, |(authority, _)| authority);
    authority.contains('@')
}

#[cfg(test)]
mod tests;

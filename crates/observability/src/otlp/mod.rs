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
    WithExportConfig, WithHttpConfig, WithTonicConfig, tonic_types::metadata::MetadataMap,
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
        // connect_lazy spawns the channel worker onto the runtime current at build time,
        // whether opentelemetry-otlp builds the channel or `platform_channel` does
        let _entered = runtime.handle().enter();

        let builder = builder
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
        // opentelemetry-otlp drops an invalid header without a word, so reject it here
        self.header_map()?;
        let client = RuntimeHttpClient::new(self.reqwest_client()?, runtime.handle().clone());

        Ok(builder
            .with_http_client(client)
            .with_endpoint(self.signal_url(path))
            .with_timeout(self.timeout)
            .with_headers(self.http_headers()))
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
        self.endpoint
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{OtlpConfig, OtlpProtocol, OtlpTls};

    #[cfg(feature = "otel")]
    fn exporter_config(protocol: OtlpProtocol, endpoint: &str) -> OtlpConfig {
        OtlpConfig {
            endpoint: endpoint.to_owned(),
            protocol,
            timeout: Duration::from_secs(1),
            ..Default::default()
        }
    }

    #[cfg(all(feature = "otel", not(feature = "tls")))]
    #[test]
    fn https_without_tls_feature_returns_feature_disabled() {
        use std::sync::PoisonError;

        use super::{ExportRuntime, runtime::tests::SLOT_LOCK};
        use crate::error::OtlpError;

        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let runtime = ExportRuntime::shared().expect("export runtime starts");

        for (protocol, endpoint) in [
            (OtlpProtocol::Grpc, "https://collector:4317"),
            (OtlpProtocol::HttpProtobuf, "https://collector:4318"),
            (OtlpProtocol::Grpc, "HTTPS://collector:4317"),
        ] {
            let built = exporter_config(protocol, endpoint).span_exporter(&runtime);

            assert!(
                matches!(built, Err(OtlpError::FeatureDisabled { feature: "tls" })),
                "{protocol:?} {endpoint}: {:?}",
                built.err()
            );
        }
    }

    // @lat: [[tests#OTLP signal paths#Signal URL appends the signal path]]
    #[cfg(feature = "otel")]
    #[test]
    fn signal_url_appends_signal_path() {
        for (endpoint, path, want) in [
            ("http://c:4318", "/v1/traces", "http://c:4318/v1/traces"),
            ("http://c:4318/", "/v1/logs", "http://c:4318/v1/logs"),
            ("https://c/otlp", "/v1/metrics", "https://c/otlp/v1/metrics"),
        ] {
            let cfg = exporter_config(OtlpProtocol::HttpProtobuf, endpoint);

            assert_eq!(cfg.signal_url(path), want, "{endpoint} + {path}");
        }
    }

    // @lat: [[tests#OTLP headers#Invalid header is rejected on both protocols]]
    #[cfg(feature = "otel")]
    #[test]
    fn invalid_header_is_rejected_on_both_protocols() {
        use std::sync::PoisonError;

        use super::{ExportRuntime, runtime::tests::SLOT_LOCK};
        use crate::error::OtlpError;

        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let runtime = ExportRuntime::shared().expect("export runtime starts");

        for (name, value) in [("x api", "k1"), ("x-api-key", "k1\n")] {
            for protocol in [OtlpProtocol::Grpc, OtlpProtocol::HttpProtobuf] {
                let cfg = OtlpConfig {
                    headers: [(name.to_owned(), value.to_owned())].into(),
                    ..exporter_config(protocol, "http://127.0.0.1:4317")
                };

                let err = cfg.span_exporter(&runtime).err();

                let Some(err @ OtlpError::InvalidHeader { name: named }) = &err else {
                    panic!("{protocol:?} {name:?}: {err:?}");
                };
                assert_eq!(named, name);
                let text = err.to_string();
                assert!(text.contains(name) && !text.contains("k1"), "{text}");
            }
        }
    }

    // needs an OS certificate store with at least one root, as on every CI image
    #[cfg(feature = "tls")]
    #[test]
    fn https_endpoints_build_with_tls() {
        use std::sync::PoisonError;

        use super::{ExportRuntime, runtime::tests::SLOT_LOCK};

        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let runtime = ExportRuntime::shared().expect("export runtime starts");

        for (protocol, endpoint) in [
            (OtlpProtocol::Grpc, "https://collector:4317"),
            (OtlpProtocol::HttpProtobuf, "https://collector:4318"),
        ] {
            if let Err(e) = exporter_config(protocol, endpoint).span_exporter(&runtime) {
                panic!("{protocol:?} {endpoint}: {e}");
            }
        }
    }

    #[cfg(feature = "tls-platform-verifier")]
    #[test]
    fn platform_roots_build_both_transports() {
        use std::sync::PoisonError;

        use super::{ExportRuntime, TlsRoots, runtime::tests::SLOT_LOCK};

        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let runtime = ExportRuntime::shared().expect("export runtime starts");

        for (protocol, endpoint) in [
            (OtlpProtocol::Grpc, "https://collector:4317"),
            (OtlpProtocol::HttpProtobuf, "https://collector:4318"),
        ] {
            let cfg = OtlpConfig {
                tls: OtlpTls {
                    roots: TlsRoots::Platform,
                    ..OtlpTls::default()
                },
                ..exporter_config(protocol, endpoint)
            };

            if let Err(e) = cfg.span_exporter(&runtime) {
                panic!("{protocol:?} {endpoint}: {e}");
            }
        }
    }

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
                timeout: Duration::from_secs(1),
                ..Default::default()
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
            headers: [("x-api-key".to_string(), "s3cr3t-value".to_string())].into(),
            resource: [("deployment.environment".to_string(), "prod".to_string())].into(),
            timeout: Duration::from_secs(5),
            ..Default::default()
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

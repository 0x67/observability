//! Plain-data pipeline config. No `opentelemetry` type in any field; `otlp.rs` is the
//! only module allowed to know about the SDK.
//!
//! With the `serde` feature every config type implements `Deserialize`: snake_case names,
//! unknown fields rejected, omitted fields taken from the type's `Default`, and each
//! `Duration` as integer milliseconds in a field ending in `_ms`.

use std::{
    net::{Ipv4Addr, SocketAddr},
    num::NonZeroUsize,
    path::PathBuf,
    time::Duration,
};

use crate::otlp::OtlpConfig;

/// Top-level pipeline config: service identity, default log level, per-signal wiring.
///
/// The default logs JSON to stdout at `info` for service `unknown_service`, with no OTLP
/// and no tracing.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
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

/// Log fan-out across N sinks, at most one of them `LogSinkKind::Otlp`. The default is one
/// stdout JSON sink.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct LoggingConfig {
    pub sinks: Vec<LogSink>,
    /// Batching and queue bound of the `LogSinkKind::Otlp` sink. The default exports a
    /// partial batch after 1 s, the SDK log default; a partial table in a config file takes
    /// the rest from `ExportBatch::default()`, whose delay is 5 s.
    pub otlp_batch: ExportBatch,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            sinks: vec![LogSink::default()],
            otlp_batch: ExportBatch {
                delay: Duration::from_secs(1),
                ..ExportBatch::default()
            },
        }
    }
}

/// One logging sink: destination, wire format, optional per-sink level override.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct LogSink {
    pub kind: LogSinkKind,
    /// Line format of a stdout or rolling-file sink.
    pub format: LogFormat,
    pub level: Option<String>,
    /// Line queue of a stdout or rolling-file sink.
    pub queue: LineQueue,
}

/// Lines a stdout or rolling-file sink buffers before its writer thread takes them, and
/// what a full buffer does. The default is 128,000 lines and [`Overflow::Drop`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct LineQueue {
    pub lines: NonZeroUsize,
    pub overflow: Overflow,
}

impl Default for LineQueue {
    fn default() -> Self {
        Self {
            lines: DEFAULT_QUEUE_LINES,
            overflow: Overflow::Drop,
        }
    }
}

// tracing-appender's own default; the unwrap runs at compile time
const DEFAULT_QUEUE_LINES: NonZeroUsize =
    NonZeroUsize::new(tracing_appender::non_blocking::DEFAULT_BUFFERED_LINES_LIMIT).unwrap();

/// What a writer sink does with a line when its queue is full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Overflow {
    /// Drop the line and count it in `PipelineGuard::dropped` and `observability.dropped`.
    #[default]
    Drop,
    /// Block the logging thread until the writer catches up, so no line is lost. Never use
    /// it on a latency-sensitive thread: a slow disk or stdout then stalls that thread.
    Block,
}

/// Sink destination.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "snake_case", deny_unknown_fields)
)]
pub enum LogSinkKind {
    #[default]
    Stdout,
    RollingFile {
        dir: PathBuf,
        prefix: String,
        #[cfg_attr(feature = "serde", serde(default))]
        rotation: Rotation,
        /// Most files with this prefix kept in `dir`, pruned when the sink opens and on
        /// each rotation; `None` keeps every file.
        #[cfg_attr(feature = "serde", serde(default))]
        max_files: Option<NonZeroUsize>,
    },
    Otlp,
}

/// Log line wire format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LogFormat {
    #[default]
    Json,
    Bunyan,
    Pretty,
}

/// Rolling-file rotation cadence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Rotation {
    Minutely,
    Hourly,
    #[default]
    Daily,
    Never,
}

/// OTLP span export. `TracingConfig::default()` exports every span the pipeline level
/// lets through, with the SDK's default sampler.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct TracingConfig {
    /// Level filter for exported spans; `None` uses `PipelineConfig::level`. A set
    /// `RUST_LOG` overrides it, as it does for every sink.
    pub level: Option<String>,
    /// Probability that a root span is sampled, `0.0..=1.0`; child spans follow their
    /// parent's decision. `None` keeps the SDK default, which reads `OTEL_TRACES_SAMPLER`
    /// and samples every span when it is unset.
    pub sample_ratio: Option<f64>,
    pub batch: ExportBatch,
}

/// Export batching and the queue bound for one OTLP signal. These values replace the SDK's
/// `OTEL_BSP_*` and `OTEL_BLRP_*` env vars, which are not read.
///
/// The default holds 2048 items, sends up to 512 per request, and exports a partial batch
/// after 5 s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct ExportBatch {
    /// Most items held from span end (or log emit) to the end of their export. Items past
    /// it are dropped and counted.
    pub queue_size: NonZeroUsize,
    /// Most items per export request, capped at `queue_size`.
    pub max_batch_size: NonZeroUsize,
    /// Longest wait before a partial batch is exported.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "delay_ms", with = "crate::config::duration_ms")
    )]
    pub delay: Duration,
}

impl Default for ExportBatch {
    fn default() -> Self {
        Self {
            queue_size: DEFAULT_EXPORT_QUEUE,
            max_batch_size: DEFAULT_EXPORT_BATCH,
            delay: Duration::from_secs(5),
        }
    }
}

// the SDK's defaults; the unwraps run at compile time
const DEFAULT_EXPORT_QUEUE: NonZeroUsize = NonZeroUsize::new(2048).unwrap();
const DEFAULT_EXPORT_BATCH: NonZeroUsize = NonZeroUsize::new(512).unwrap();

/// Metrics subsystem config: which recorder backs the `metrics` facade.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct MetricsConfig {
    pub exporter: MetricsExporter,
}

/// Metrics exporter choice: Prometheus pull endpoint or OTLP push bridge. The default is
/// Prometheus on `127.0.0.1:9000`.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "snake_case", deny_unknown_fields)
)]
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

/// `Duration` fields in a config file are integer milliseconds.
#[cfg(feature = "serde")]
pub(crate) mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer};

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_millis)
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use std::{collections::BTreeMap, num::NonZeroUsize, path::Path, time::Duration};

    use super::{LogFormat, LogSinkKind, Rotation};
    use crate::{ClientCert, ObsConfig, OtlpProtocol, OtlpTls, TlsRoots};

    fn parse(json: &str) -> Result<ObsConfig, serde_json::Error> {
        serde_json::from_str(json)
    }

    // @lat: [[tests#Config files#Config file parses into expected values]]
    #[test]
    fn config_file_parses_into_expected_values() {
        let cfg = parse(
            r#"{"pipeline":{"service_name":"svc","otlp":{"endpoint":"http://c:4318","protocol":"grpc","timeout_ms":2500,"headers":{"x-key":"k"},"resource":{"env":"ci"}},"logging":{"sinks":[{"kind":"stdout"},{"kind":{"rolling_file":{"dir":"/tmp/x","prefix":"p","max_files":3}},"format":"pretty"}]}}}"#,
        )
        .expect("the document parses");

        let pipeline = &cfg.pipeline;
        assert_eq!(pipeline.service_name, "svc");
        assert_eq!(pipeline.level, "info");
        let otlp = pipeline.otlp.as_ref().expect("otlp is set");
        assert_eq!(otlp.protocol, OtlpProtocol::Grpc);
        assert_eq!(otlp.timeout, Duration::from_millis(2500));
        let map = |key: &str, value: &str| BTreeMap::from([(key.to_owned(), value.to_owned())]);
        assert_eq!(otlp.headers, map("x-key", "k"));
        assert_eq!(otlp.resource, map("env", "ci"));
        let sinks = &pipeline.logging.as_ref().expect("logging is set").sinks;
        assert!(matches!(sinks[0].kind, LogSinkKind::Stdout), "{sinks:?}");
        let LogSinkKind::RollingFile {
            dir,
            prefix,
            rotation,
            max_files,
        } = &sinks[1].kind
        else {
            panic!("second sink is a rolling file: {sinks:?}");
        };
        assert_eq!(dir, Path::new("/tmp/x"));
        assert_eq!(prefix, "p");
        assert_eq!(*rotation, Rotation::Daily);
        assert_eq!(max_files.map(NonZeroUsize::get), Some(3));
        assert_eq!(sinks[1].format, LogFormat::Pretty);
        assert!(cfg.metrics.is_none());
    }

    // @lat: [[tests#Config files#TLS settings parse from file]]
    #[test]
    fn tls_settings_parse_from_file() {
        let cfg = parse(
            r#"{"pipeline":{"otlp":{"tls":{"roots":"webpki","ca_file":"/etc/ca.pem","client_cert":{"cert_file":"c.pem","key_file":"k.pem"}}}}}"#,
        )
        .expect("the document parses");

        let otlp = cfg.pipeline.otlp.expect("otlp is set");
        let want = OtlpTls {
            roots: TlsRoots::Webpki,
            ca_file: Some("/etc/ca.pem".into()),
            client_cert: Some(ClientCert {
                cert_file: "c.pem".into(),
                key_file: "k.pem".into(),
            }),
        };
        assert_eq!(otlp.tls, want);

        let no_key =
            parse(r#"{"pipeline":{"otlp":{"tls":{"client_cert":{"cert_file":"c.pem"}}}}}"#);
        let err = no_key.err().expect("a client_cert without key_file fails");
        assert!(err.to_string().contains("`key_file`"), "{err}");
    }

    // @lat: [[tests#Config files#Logging otlp batch defaults to one second delay when omitted]]
    #[test]
    fn logging_otlp_batch_defaults_to_one_second_delay_when_omitted() {
        let cfg = parse(r#"{"pipeline":{"logging":{"sinks":[]}}}"#).expect("the document parses");

        let logging = cfg.pipeline.logging.expect("logging is set");
        assert_eq!(logging.otlp_batch.delay, Duration::from_secs(1));
    }

    // @lat: [[tests#Config files#Unknown field is rejected]]
    #[test]
    fn unknown_field_is_rejected() {
        for (json, field) in [
            (r#"{"pipeline":{"servce_name":"x"}}"#, "servce_name"),
            (
                r#"{"pipeline":{"logging":{"sinks":[{"kind":{"rolling_file":{"dir":"/x","prefix":"p","max_file":3}}}]}}}"#,
                "max_file",
            ),
        ] {
            let err = parse(json)
                .err()
                .unwrap_or_else(|| panic!("{field} is rejected"));

            assert!(
                err.to_string()
                    .contains(&format!("unknown field `{field}`")),
                "{err}"
            );
        }
    }
}

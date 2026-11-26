//! Config the build cannot honor returns an error from `init_pipeline` before any
//! subscriber is installed. No test here installs one, so they share this binary.

use observability::{
    LogFormat, LogSink, LogSinkKind, LoggingConfig, PipelineConfig, PipelineError, TraceExporter,
    TracingConfig, init_pipeline,
};

fn config() -> PipelineConfig {
    PipelineConfig {
        service_name: "init-errors".to_owned(),
        level: "info".to_owned(),
        otlp: None,
        logging: None,
        tracing: None,
    }
}

fn tracing_on() -> Option<TracingConfig> {
    Some(TracingConfig {
        exporter: TraceExporter::Otlp,
    })
}

fn one_sink(kind: LogSinkKind, format: LogFormat) -> Option<LoggingConfig> {
    Some(LoggingConfig {
        sinks: vec![LogSink {
            kind,
            format,
            level: None,
        }],
    })
}

// init_pipeline consumes the config, so the error is all that comes back
fn init_error(cfg: PipelineConfig) -> PipelineError {
    let err = init_pipeline(cfg).err().expect("init must fail");
    assert!(!tracing::dispatcher::has_been_set(), "a subscriber was set");
    err
}

// @lat: [[tests#Init contract#Tracing without otlp returns missing otlp traces]]
#[cfg(feature = "otel")]
#[test]
fn tracing_without_otlp_returns_missing_otlp_traces() {
    use observability::Signal;

    let err = init_error(PipelineConfig {
        tracing: tracing_on(),
        ..config()
    });

    assert!(
        matches!(err, PipelineError::MissingOtlp(Signal::Traces)),
        "{err:?}"
    );
}

// @lat: [[tests#Init contract#OTLP log sink without otlp returns missing otlp logs]]
#[cfg(feature = "otel-logs")]
#[test]
fn otlp_log_sink_without_otlp_returns_missing_otlp_logs() {
    use observability::Signal;

    let err = init_error(PipelineConfig {
        logging: one_sink(LogSinkKind::Otlp, LogFormat::Json),
        ..config()
    });

    assert!(
        matches!(err, PipelineError::MissingOtlp(Signal::Logs)),
        "{err:?}"
    );
}

// @lat: [[tests#Init contract#Tracing without otel feature returns feature disabled]]
#[cfg(not(feature = "otel"))]
#[test]
fn tracing_without_otel_feature_returns_feature_disabled() {
    let err = init_error(PipelineConfig {
        tracing: tracing_on(),
        ..config()
    });

    assert!(
        matches!(err, PipelineError::FeatureDisabled { feature: "otel" }),
        "{err:?}"
    );
}

// @lat: [[tests#Init contract#OTLP log sink without otel-logs returns feature disabled]]
#[cfg(not(feature = "otel-logs"))]
#[test]
fn otlp_log_sink_without_otel_logs_returns_feature_disabled() {
    let err = init_error(PipelineConfig {
        logging: one_sink(LogSinkKind::Otlp, LogFormat::Json),
        ..config()
    });

    assert!(
        matches!(
            err,
            PipelineError::FeatureDisabled {
                feature: "otel-logs"
            }
        ),
        "{err:?}"
    );
}

// @lat: [[tests#Init contract#Bunyan sink without feature returns feature disabled]]
#[cfg(not(feature = "bunyan"))]
#[test]
fn bunyan_sink_without_feature_returns_feature_disabled() {
    let err = init_error(PipelineConfig {
        logging: one_sink(LogSinkKind::Stdout, LogFormat::Bunyan),
        ..config()
    });

    assert!(
        matches!(err, PipelineError::FeatureDisabled { feature: "bunyan" }),
        "{err:?}"
    );
}

// @lat: [[tests#Init contract#Invalid endpoint returns exporter error]]
#[cfg(feature = "otel")]
#[test]
fn invalid_endpoint_returns_exporter_error() {
    use std::time::Duration;

    use observability::{OtlpConfig, OtlpError, OtlpProtocol, OtlpTls, Signal};

    let err = init_error(PipelineConfig {
        tracing: tracing_on(),
        otlp: Some(OtlpConfig {
            endpoint: "http://exa mple:4318".to_owned(),
            protocol: OtlpProtocol::HttpProtobuf,
            headers: Vec::new(),
            resource: Vec::new(),
            timeout: Duration::from_secs(1),
            tls: OtlpTls::default(),
        }),
        ..config()
    });

    assert!(
        matches!(
            err,
            PipelineError::Exporter {
                signal: Signal::Traces,
                source: OtlpError::Build(_),
            }
        ),
        "{err:?}"
    );
}

// @lat: [[tests#Rolling file sink#Rolling file in unwritable dir returns err]]
#[test]
fn rolling_file_in_unwritable_dir_returns_err() {
    use observability::Rotation;

    // a directory below a regular file cannot be created on any OS, root or not
    let file = std::env::temp_dir().join(format!("obs-not-a-dir-{}", std::process::id()));
    std::fs::write(&file, "").expect("write the blocking file");
    let dir = file.join("logs");

    let err = init_error(PipelineConfig {
        logging: one_sink(
            LogSinkKind::RollingFile {
                dir: dir.clone(),
                prefix: "app".to_owned(),
                rotation: Rotation::Daily,
                max_files: None,
            },
            LogFormat::Json,
        ),
        ..config()
    });

    std::fs::remove_file(&file).expect("remove the blocking file");
    assert!(
        matches!(&err, PipelineError::RollingFile { dir: got, .. } if *got == dir),
        "{err:?}"
    );
}

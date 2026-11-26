use std::sync::Arc;

use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel-logs")]
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
#[cfg(feature = "otel-logs")]
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::Layer;

use super::{BoxedLayer, filter::otlp_filter};
use crate::{
    config::{LogSink, LogSinkKind, PipelineConfig},
    error::{OtlpError, PipelineError, Signal},
    otlp::{ExportRuntime, OtlpConfig},
};

/// The OTel providers a pipeline built, plus the export runtime their exporters run on.
pub(super) struct OtelProviders {
    tracer: Option<SdkTracerProvider>,
    #[cfg(feature = "otel-logs")]
    logger: Option<SdkLoggerProvider>,
    // declared last so the runtime outlives the providers' final flush
    _runtime: Arc<ExportRuntime>,
}

impl OtelProviders {
    pub(super) fn force_flush(&self) {
        if let Some(p) = &self.tracer {
            let _ = p.force_flush();
        }
        #[cfg(feature = "otel-logs")]
        if let Some(p) = &self.logger {
            let _ = p.force_flush();
        }
    }
}

/// Push the OTel span layer and one OTLP log bridge per OTLP log sink onto `layers`, and
/// return the providers behind them. `None` when the config asks for no OTLP signal.
pub(super) fn otel_layers(
    cfg: &PipelineConfig,
    sinks: &[LogSink],
    layers: &mut Vec<BoxedLayer>,
) -> Result<Option<OtelProviders>, PipelineError> {
    let log_levels: Vec<&str> = sinks
        .iter()
        .filter(|sink| matches!(sink.kind, LogSinkKind::Otlp))
        .map(|sink| sink.level.as_deref().unwrap_or(&cfg.level))
        .collect();
    let first = match (&cfg.tracing, log_levels.is_empty()) {
        (Some(_), _) => Signal::Traces,
        (None, false) => Signal::Logs,
        (None, true) => return Ok(None),
    };
    #[cfg(not(feature = "otel-logs"))]
    if !log_levels.is_empty() {
        return Err(PipelineError::FeatureDisabled {
            feature: "otel-logs",
        });
    }

    let otlp = cfg.otlp.as_ref().ok_or(PipelineError::MissingOtlp(first))?;
    let runtime = ExportRuntime::shared().map_err(|source| PipelineError::Exporter {
        signal: first,
        source,
    })?;

    let tracer = match cfg.tracing {
        Some(_) => {
            let provider =
                build_tracer_provider(otlp, &cfg.service_name, &runtime).map_err(|source| {
                    PipelineError::Exporter {
                        signal: Signal::Traces,
                        source,
                    }
                })?;
            layers.push(trace_layer(&provider, &cfg.level));
            Some(provider)
        }
        None => None,
    };

    // one provider serves every OTLP log sink; each sink keeps its own level
    #[cfg(feature = "otel-logs")]
    let logger = if log_levels.is_empty() {
        None
    } else {
        let provider =
            build_logger_provider(otlp, &cfg.service_name, &runtime).map_err(|source| {
                PipelineError::Exporter {
                    signal: Signal::Logs,
                    source,
                }
            })?;
        layers.extend(
            log_levels
                .iter()
                .map(|level| log_bridge_layer(&provider, level)),
        );
        Some(provider)
    };

    Ok(Some(OtelProviders {
        tracer,
        #[cfg(feature = "otel-logs")]
        logger,
        _runtime: runtime,
    }))
}

/// The OTel span layer, filtered by `level` and with exporter threads dropped.
fn trace_layer(provider: &SdkTracerProvider, level: &str) -> BoxedLayer {
    let tracer = provider.tracer("observability");
    tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(otlp_filter(level))
        .boxed()
}

/// The OTLP log bridge, filtered by `level` and with exporter threads dropped.
#[cfg(feature = "otel-logs")]
fn log_bridge_layer(provider: &SdkLoggerProvider, level: &str) -> BoxedLayer {
    OpenTelemetryTracingBridge::new(provider)
        .with_filter(otlp_filter(level))
        .boxed()
}

fn build_tracer_provider(
    otlp: &OtlpConfig,
    service: &str,
    runtime: &ExportRuntime,
) -> Result<SdkTracerProvider, OtlpError> {
    let exporter = otlp.span_exporter(runtime)?;
    Ok(SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(otlp.otel_resource(service))
        .build())
}

#[cfg(feature = "otel-logs")]
fn build_logger_provider(
    otlp: &OtlpConfig,
    service: &str,
    runtime: &ExportRuntime,
) -> Result<SdkLoggerProvider, OtlpError> {
    let exporter = otlp.log_exporter(runtime)?;
    Ok(SdkLoggerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(otlp.otel_resource(service))
        .build())
}

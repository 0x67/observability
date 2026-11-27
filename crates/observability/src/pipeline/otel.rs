use std::sync::Arc;

use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel-logs")]
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
#[cfg(feature = "otel-logs")]
use opentelemetry_sdk::logs::{BatchLogProcessor, SdkLoggerProvider};
use opentelemetry_sdk::trace::{BatchSpanProcessor, Sampler, SdkTracerProvider};
use tracing_subscriber::Layer;

#[cfg(feature = "otel-logs")]
use super::bounded::{BoundedLogProcessor, bounded_log_chain};
use super::{
    BoxedLayer,
    bounded::{BoundedSpanProcessor, bounded_span_chain},
    dropped::DropCounters,
    filter::otlp_filter,
};
#[cfg(feature = "otel-logs")]
use crate::config::ExportBatch;
use crate::{
    config::{LogSinkKind, PipelineConfig, TracingConfig},
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
    /// Shut each provider down, which exports its final batch, then release the runtime.
    // the global subscriber keeps the tracer, and with it the provider, alive until exit,
    // so the provider's own Drop never runs its shutdown
    pub(super) fn shutdown(self) {
        if let Some(tracer) = &self.tracer
            && let Err(error) = tracer.shutdown()
        {
            tracing::warn!(signal = %Signal::Traces, %error, "OTLP provider shutdown failed");
        }
        #[cfg(feature = "otel-logs")]
        if let Some(logger) = &self.logger
            && let Err(error) = logger.shutdown()
        {
            tracing::warn!(signal = %Signal::Logs, %error, "OTLP provider shutdown failed");
        }
    }
}

/// Push the OTel span layer and the OTLP log bridge onto `layers`, and return the providers
/// behind them. `None` when the config asks for no OTLP signal.
pub(super) fn otel_layers(
    cfg: &PipelineConfig,
    layers: &mut Vec<BoxedLayer>,
    dropped: &DropCounters,
) -> Result<Option<OtelProviders>, PipelineError> {
    // the level and batch of the one OTLP log sink; `build_layers` rejected a second one
    let otlp_log = cfg.logging.as_ref().and_then(|logging| {
        logging
            .sinks
            .iter()
            .find(|sink| matches!(sink.kind, LogSinkKind::Otlp))
            .map(|sink| {
                let level = sink.level.as_deref().unwrap_or(&cfg.level);
                (level, &logging.otlp_batch)
            })
    });
    let first = match (&cfg.tracing, otlp_log) {
        (Some(_), _) => Signal::Traces,
        (None, Some(_)) => Signal::Logs,
        (None, None) => return Ok(None),
    };
    #[cfg(not(feature = "otel-logs"))]
    if otlp_log.is_some() {
        return Err(PipelineError::FeatureDisabled {
            feature: "otel-logs",
        });
    }

    let otlp = cfg.otlp.as_ref().ok_or(PipelineError::MissingOtlp(first))?;
    let runtime = ExportRuntime::shared().map_err(|source| PipelineError::Exporter {
        signal: first,
        source,
    })?;

    let inputs = ProviderInputs {
        otlp,
        service: &cfg.service_name,
        runtime: &runtime,
        dropped,
    };

    let tracer = match &cfg.tracing {
        Some(tracing) => {
            let provider = build_tracer_provider(&inputs, tracing).map_err(|source| {
                PipelineError::Exporter {
                    signal: Signal::Traces,
                    source,
                }
            })?;
            let level = tracing.level.as_deref().unwrap_or(&cfg.level);
            layers.push(trace_layer(&provider, level));
            Some(provider)
        }
        None => None,
    };

    #[cfg(feature = "otel-logs")]
    let logger = match otlp_log {
        Some((level, batch)) => {
            let provider = build_logger_provider(&inputs, batch).map_err(|source| {
                PipelineError::Exporter {
                    signal: Signal::Logs,
                    source,
                }
            })?;
            layers.push(log_bridge_layer(&provider, level));
            Some(provider)
        }
        None => None,
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

/// What both provider builders share.
struct ProviderInputs<'a> {
    otlp: &'a OtlpConfig,
    service: &'a str,
    runtime: &'a ExportRuntime,
    dropped: &'a DropCounters,
}

/// The bounded span chain over the OTLP span exporter.
fn bounded_span_processor(
    inputs: &ProviderInputs<'_>,
    tracing: &TracingConfig,
) -> Result<BoundedSpanProcessor<BatchSpanProcessor>, OtlpError> {
    let exporter = inputs.otlp.span_exporter(inputs.runtime)?;
    Ok(bounded_span_chain(exporter, &tracing.batch, inputs.dropped))
}

fn build_tracer_provider(
    inputs: &ProviderInputs<'_>,
    tracing: &TracingConfig,
) -> Result<SdkTracerProvider, OtlpError> {
    let builder = SdkTracerProvider::builder()
        .with_span_processor(bounded_span_processor(inputs, tracing)?)
        .with_resource(inputs.otlp.otel_resource(inputs.service));
    // without a sampler set, the SDK reads OTEL_TRACES_SAMPLER
    let builder = match tracing.sample_ratio {
        Some(ratio) => builder.with_sampler(Sampler::ParentBased(Box::new(
            Sampler::TraceIdRatioBased(ratio),
        ))),
        None => builder,
    };

    Ok(builder.build())
}

/// The bounded log chain over the OTLP log exporter.
#[cfg(feature = "otel-logs")]
fn bounded_log_processor(
    inputs: &ProviderInputs<'_>,
    batch: &ExportBatch,
) -> Result<BoundedLogProcessor<BatchLogProcessor>, OtlpError> {
    let exporter = inputs.otlp.log_exporter(inputs.runtime)?;
    Ok(bounded_log_chain(exporter, batch, inputs.dropped))
}

#[cfg(feature = "otel-logs")]
fn build_logger_provider(
    inputs: &ProviderInputs<'_>,
    batch: &ExportBatch,
) -> Result<SdkLoggerProvider, OtlpError> {
    Ok(SdkLoggerProvider::builder()
        .with_log_processor(bounded_log_processor(inputs, batch)?)
        .with_resource(inputs.otlp.otel_resource(inputs.service))
        .build())
}

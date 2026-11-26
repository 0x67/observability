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
    error::OtlpError,
    otlp::{ExportRuntime, OtlpConfig},
};

/// The OTel providers a pipeline built, plus the export runtime their exporters run on.
pub(super) struct OtelProviders {
    pub(super) tracer: Option<SdkTracerProvider>,
    #[cfg(feature = "otel-logs")]
    pub(super) logger: Option<SdkLoggerProvider>,
    // declared last so the runtime outlives the providers' final flush
    pub(super) _runtime: Arc<ExportRuntime>,
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

/// The OTel span layer, filtered by `level` and with exporter threads dropped.
pub(super) fn trace_layer(provider: &SdkTracerProvider, level: &str) -> BoxedLayer {
    let tracer = provider.tracer("observability");
    tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(otlp_filter(level))
        .boxed()
}

/// The OTLP log bridge, filtered by `level` and with exporter threads dropped.
#[cfg(feature = "otel-logs")]
pub(super) fn log_bridge_layer(provider: &SdkLoggerProvider, level: &str) -> BoxedLayer {
    OpenTelemetryTracingBridge::new(provider)
        .with_filter(otlp_filter(level))
        .boxed()
}

pub(super) fn build_tracer_provider(
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
pub(super) fn build_logger_provider(
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

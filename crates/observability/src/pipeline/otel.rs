use std::sync::Arc;

#[cfg(feature = "otel-logs")]
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

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

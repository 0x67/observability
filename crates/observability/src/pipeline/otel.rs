use crate::otlp::OtlpConfig;

pub(super) fn build_tracer_provider(
    otlp: &OtlpConfig,
    service: &str,
) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, opentelemetry_otlp::ExporterBuildError> {
    let exporter = otlp.span_exporter()?;
    Ok(opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(otlp.otel_resource(service))
        .build())
}

#[cfg(feature = "otel-logs")]
pub(super) fn build_logger_provider(
    otlp: &OtlpConfig,
    service: &str,
) -> Result<opentelemetry_sdk::logs::SdkLoggerProvider, opentelemetry_otlp::ExporterBuildError> {
    let exporter = otlp.log_exporter()?;
    Ok(opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(otlp.otel_resource(service))
        .build())
}

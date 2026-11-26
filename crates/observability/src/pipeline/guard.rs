use tracing_appender::non_blocking::WorkerGuard;

/// Holds everything whose drop must outlive the process: non-blocking file writer guards
/// and the OTel providers (dropping a provider flushes its pending batch).
pub struct PipelineGuard {
    // held only so the non-blocking writer threads flush their tail on drop; never read.
    #[allow(dead_code)]
    pub(super) workers: Vec<WorkerGuard>,
    #[cfg(feature = "otel")]
    pub(super) tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(feature = "otel-logs")]
    pub(super) logger_provider: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
}

impl Drop for PipelineGuard {
    fn drop(&mut self) {
        // final flush before worker guards drop; provider drop also shuts down.
        #[cfg(feature = "otel")]
        if let Some(p) = &self.tracer_provider {
            let _ = p.force_flush();
        }
        #[cfg(feature = "otel-logs")]
        if let Some(p) = &self.logger_provider {
            let _ = p.force_flush();
        }
    }
}

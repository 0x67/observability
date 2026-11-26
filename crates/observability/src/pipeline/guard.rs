use tracing_appender::non_blocking::WorkerGuard;

#[cfg(feature = "otel")]
use super::otel::OtelProviders;

/// Holds everything whose drop must outlive the process: non-blocking file writer guards,
/// the OTel providers, and the export runtime they run on.
pub struct PipelineGuard {
    // held only so the non-blocking writer threads flush their tail on drop; never read.
    #[allow(dead_code)]
    pub(super) workers: Vec<WorkerGuard>,
    #[cfg(feature = "otel")]
    pub(super) otel: Option<OtelProviders>,
}

impl Drop for PipelineGuard {
    fn drop(&mut self) {
        // final flush before worker guards drop; provider drop also shuts down.
        #[cfg(feature = "otel")]
        if let Some(otel) = &self.otel {
            otel.force_flush();
        }
    }
}

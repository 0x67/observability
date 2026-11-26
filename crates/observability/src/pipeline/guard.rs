use tracing_appender::non_blocking::WorkerGuard;

#[cfg(feature = "otel")]
use super::otel::OtelProviders;

/// Holds everything whose drop must outlive the process: non-blocking file writer guards,
/// the OTel providers, and the export runtime they run on.
///
/// Bind it for the process lifetime. Discarding it is a compile-time warning, because the
/// log writers stop and the exporters flush the moment it drops:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// # use observability::{PipelineConfig, PipelineError, init_pipeline};
/// # fn main() -> Result<(), PipelineError> {
/// # let cfg = PipelineConfig { service_name: "app".into(), level: "info".into(), otlp: None, logging: None, tracing: None };
/// init_pipeline(cfg)?;
/// # Ok(())
/// # }
/// ```
///
/// ```no_run
/// #![deny(unused_must_use)]
/// # use observability::{PipelineConfig, PipelineError, init_pipeline};
/// # fn main() -> Result<(), PipelineError> {
/// # let cfg = PipelineConfig { service_name: "app".into(), level: "info".into(), otlp: None, logging: None, tracing: None };
/// let _guard = init_pipeline(cfg)?;
/// # Ok(())
/// # }
/// ```
#[must_use = "dropping the guard stops the log writers and flushes the exporters; keep it alive for the process lifetime"]
pub struct PipelineGuard {
    // held only so the non-blocking writer threads flush their tail on drop
    pub(super) _workers: Vec<WorkerGuard>,
    #[cfg(feature = "otel")]
    pub(super) otel: Option<OtelProviders>,
}

impl Drop for PipelineGuard {
    fn drop(&mut self) {
        // providers shut down while the log writers can still print their warnings
        #[cfg(feature = "otel")]
        if let Some(otel) = self.otel.take() {
            otel.shutdown();
        }
    }
}

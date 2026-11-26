//! The crate-owned tokio runtime that drives every OTLP exporter's I/O, so export works
//! with or without an app runtime and never waits on the app's executor.

use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::runtime::{Handle, Runtime};

use crate::error::OtlpError;

/// Name of every thread the export runtime starts.
pub(crate) const EXPORT_THREAD_NAME: &str = "observability-otlp";

/// The tokio runtime every OTLP exporter in the process runs on.
pub(crate) struct ExportRuntime {
    handle: Handle,
    // taken in Drop, because dropping a Runtime inside an async context panics
    runtime: Option<Runtime>,
}

// the runtime a live guard holds; empty once every holder has dropped its Arc
static SHARED: Mutex<Weak<ExportRuntime>> = Mutex::new(Weak::new());

impl ExportRuntime {
    /// The runtime a live guard holds, or a new one when no guard holds any.
    pub(crate) fn shared() -> Result<Arc<Self>, OtlpError> {
        let mut slot = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(live) = slot.upgrade() {
            return Ok(live);
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name(EXPORT_THREAD_NAME)
            .enable_all()
            .build()
            .map_err(OtlpError::Runtime)?;
        let shared = Arc::new(Self {
            handle: runtime.handle().clone(),
            runtime: Some(runtime),
        });
        *slot = Arc::downgrade(&shared);
        Ok(shared)
    }

    pub(crate) fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl Drop for ExportRuntime {
    fn drop(&mut self) {
        // shutdown_background does not wait, so it is safe inside an async context
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use super::{EXPORT_THREAD_NAME, ExportRuntime};

    // every test that reads or replaces the process-wide SHARED slot takes this first
    pub(in crate::otlp) static SLOT_LOCK: Mutex<()> = Mutex::new(());

    // a runtime that cannot run the task drops it, and with it the sender
    fn spawned_thread_name(runtime: &ExportRuntime) -> Option<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        runtime.handle().spawn(async move {
            let _ = tx.send(std::thread::current().name().map(str::to_owned));
        });
        rx.recv().expect("export runtime dropped the task")
    }

    #[test]
    fn shared_runtime_is_reused_and_runs_on_named_thread() {
        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

        let first = ExportRuntime::shared().expect("runtime starts");
        let second = ExportRuntime::shared().expect("runtime starts");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            spawned_thread_name(&first).as_deref(),
            Some(EXPORT_THREAD_NAME)
        );
    }

    #[test]
    fn shared_runtime_is_replaced_after_release() {
        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        drop(ExportRuntime::shared().expect("runtime starts"));

        let next = ExportRuntime::shared().expect("runtime starts again");

        assert_eq!(
            spawned_thread_name(&next).as_deref(),
            Some(EXPORT_THREAD_NAME)
        );
    }

    #[test]
    fn releasing_last_holder_inside_async_context_does_not_panic() {
        let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let app = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("app runtime starts");

        app.block_on(async {
            drop(ExportRuntime::shared().expect("runtime starts"));
        });
    }
}

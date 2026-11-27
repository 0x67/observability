//! Exporters for the bounded processor tests, with a probe that reads what they received.

use std::sync::{
    Arc, Mutex, PoisonError, RwLock, RwLockWriteGuard,
    atomic::{AtomicUsize, Ordering},
};

use opentelemetry_sdk::{
    Resource,
    error::{OTelSdkError, OTelSdkResult},
    trace::{SpanData, SpanExporter},
};

/// What an exporter's first export does; every later export succeeds.
#[derive(Clone, Copy, Debug)]
pub(super) enum FirstExport {
    Succeed,
    /// Waits while the test holds [`Probe::close_gate`].
    WaitForGate,
    Fail,
}

/// The test's view of an exporter: items in successful exports and the last resource set.
#[derive(Clone, Debug, Default)]
pub(super) struct Probe {
    gate: Arc<RwLock<()>>,
    exported: Arc<AtomicUsize>,
    resource: Arc<Mutex<Option<Resource>>>,
}

impl Probe {
    pub(super) fn close_gate(&self) -> RwLockWriteGuard<'_, ()> {
        self.gate.write().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn exported(&self) -> usize {
        self.exported.load(Ordering::Relaxed)
    }

    pub(super) fn resource(&self) -> Option<Resource> {
        self.resource
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // what every exporter does per export call; `Err` fails that export
    fn export(&self, first: FirstExport, calls: &AtomicUsize, items: usize) -> OTelSdkResult {
        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
            match first {
                FirstExport::Succeed => {}
                FirstExport::WaitForGate => {
                    drop(self.gate.read().unwrap_or_else(PoisonError::into_inner));
                }
                FirstExport::Fail => {
                    return Err(OTelSdkError::InternalFailure("test export failure".into()));
                }
            }
        }
        self.exported.fetch_add(items, Ordering::Relaxed);
        Ok(())
    }

    fn set_resource(&self, resource: &Resource) {
        *self.resource.lock().unwrap_or_else(PoisonError::into_inner) = Some(resource.clone());
    }
}

#[derive(Debug)]
pub(super) struct TestSpanExporter {
    first: FirstExport,
    calls: AtomicUsize,
    probe: Probe,
}

impl TestSpanExporter {
    pub(super) fn new(first: FirstExport, probe: &Probe) -> Self {
        Self {
            first,
            calls: AtomicUsize::new(0),
            probe: probe.clone(),
        }
    }
}

impl SpanExporter for TestSpanExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.probe.export(self.first, &self.calls, batch.len())
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.probe.set_resource(resource);
    }
}

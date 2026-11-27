use std::time::Duration;

use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    logs::{
        BatchConfig, BatchConfigBuilder, BatchLogProcessor, LogBatch, LogExporter, LogProcessor,
        SdkLogRecord,
    },
};

use super::InFlight;
use crate::{
    config::ExportBatch,
    error::Signal,
    pipeline::dropped::{DropCounters, DropReason},
};

#[cfg(test)]
mod tests;

/// `BoundedLogProcessor` over a `BatchLogProcessor` set from `batch`, whose exporter is
/// `exporter` behind a counter of failed batches. Every dropped record goes to `dropped`.
pub(in crate::pipeline) fn bounded_log_chain<E: LogExporter + 'static>(
    exporter: E,
    batch: &ExportBatch,
    dropped: &DropCounters,
) -> BoundedLogProcessor<BatchLogProcessor> {
    let in_flight = InFlight::new(batch.queue_size);
    let exporter = CountingLogExporter {
        inner: exporter,
        in_flight: in_flight.clone(),
        dropped: dropped.clone(),
    };
    let inner = BatchLogProcessor::builder(exporter)
        .with_batch_config(log_batch_config(batch))
        .build();

    BoundedLogProcessor {
        inner,
        in_flight,
        dropped: dropped.clone(),
    }
}

// set explicitly, so the OTEL_BLRP_* queue, batch, and delay values are not read
fn log_batch_config(batch: &ExportBatch) -> BatchConfig {
    BatchConfigBuilder::default()
        .with_max_queue_size(batch.queue_size.get())
        .with_max_export_batch_size(batch.max_batch_size.get())
        .with_scheduled_delay(batch.delay)
        .build()
}

/// Passes a record on only while it can take an in-flight slot; otherwise drops and counts
/// it.
#[derive(Debug)]
pub(in crate::pipeline) struct BoundedLogProcessor<P> {
    inner: P,
    in_flight: InFlight,
    dropped: DropCounters,
}

impl<P: LogProcessor> LogProcessor for BoundedLogProcessor<P> {
    fn emit(&self, record: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
        if self.in_flight.try_acquire() {
            self.inner.emit(record, instrumentation);
        } else {
            self.dropped.add(Signal::Logs, DropReason::QueueFull, 1);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// Frees a batch's in-flight slots when its export returns, and counts a failed batch.
#[derive(Debug)]
struct CountingLogExporter<E> {
    inner: E,
    in_flight: InFlight,
    dropped: DropCounters,
}

impl<E: LogExporter> LogExporter for CountingLogExporter<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        // LogBatch has no len()
        let len = batch.iter().count();
        let result = self.inner.export(batch).await;

        self.in_flight.release(len);
        if result.is_err() {
            self.dropped
                .add(Signal::Logs, DropReason::ExportFailed, len as u64);
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

use std::time::Duration;

use opentelemetry::Context;
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    trace::{
        BatchConfig, BatchConfigBuilder, BatchSpanProcessor, Span, SpanData, SpanExporter,
        SpanProcessor,
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

/// `BoundedSpanProcessor` over a `BatchSpanProcessor` set from `batch`, whose exporter is
/// `exporter` behind a counter of failed batches. Every dropped span goes to `dropped`.
pub(in crate::pipeline) fn bounded_span_chain<E: SpanExporter + 'static>(
    exporter: E,
    batch: &ExportBatch,
    dropped: &DropCounters,
) -> BoundedSpanProcessor<BatchSpanProcessor> {
    let in_flight = InFlight::new(batch.queue_size);
    let exporter = CountingSpanExporter {
        inner: exporter,
        in_flight: in_flight.clone(),
        dropped: dropped.clone(),
    };
    let inner = BatchSpanProcessor::builder(exporter)
        .with_batch_config(span_batch_config(batch))
        .build();

    BoundedSpanProcessor {
        inner,
        in_flight,
        dropped: dropped.clone(),
    }
}

// set explicitly, so the OTEL_BSP_* queue, batch, and delay values are not read
fn span_batch_config(batch: &ExportBatch) -> BatchConfig {
    BatchConfigBuilder::default()
        .with_max_queue_size(batch.queue_size.get())
        .with_max_export_batch_size(batch.max_batch_size.get())
        .with_scheduled_delay(batch.delay)
        .build()
}

/// Passes a span on only while it can take an in-flight slot; otherwise drops and counts it.
#[derive(Debug)]
pub(in crate::pipeline) struct BoundedSpanProcessor<P> {
    inner: P,
    in_flight: InFlight,
    dropped: DropCounters,
}

impl<P: SpanProcessor> SpanProcessor for BoundedSpanProcessor<P> {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        if self.in_flight.try_acquire() {
            self.inner.on_end(span);
        } else {
            self.dropped.add(Signal::Traces, DropReason::QueueFull, 1);
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

    // the provider's resource reaches the exporter only through this call
    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// Frees a batch's in-flight slots when its export returns, and counts a failed batch.
#[derive(Debug)]
struct CountingSpanExporter<E> {
    inner: E,
    in_flight: InFlight,
    dropped: DropCounters,
}

impl<E: SpanExporter> SpanExporter for CountingSpanExporter<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let len = batch.len();
        let result = self.inner.export(batch).await;

        self.in_flight.release(len);
        if result.is_err() {
            self.dropped
                .add(Signal::Traces, DropReason::ExportFailed, len as u64);
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

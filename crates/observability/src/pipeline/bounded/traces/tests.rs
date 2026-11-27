use std::{num::NonZeroUsize, time::Duration};

use opentelemetry::{
    Key, Value,
    trace::{Span as _, Tracer as _, TracerProvider as _},
};
use opentelemetry_sdk::{
    Resource,
    trace::{Sampler, SdkTracerProvider},
};

use super::{
    super::test_exporters::{FirstExport, Probe, TestSpanExporter},
    bounded_span_chain,
};
use crate::{config::ExportBatch, pipeline::dropped::DropCounters};

fn batch(queue_size: usize, max_batch_size: usize, delay: Duration) -> ExportBatch {
    let non_zero = |n| NonZeroUsize::new(n).expect("test sizes are nonzero");
    ExportBatch {
        queue_size: non_zero(queue_size),
        max_batch_size: non_zero(max_batch_size),
        delay,
    }
}

fn provider(
    first: FirstExport,
    batch: &ExportBatch,
    probe: &Probe,
    dropped: &DropCounters,
) -> SdkTracerProvider {
    let exporter = TestSpanExporter::new(first, probe);
    SdkTracerProvider::builder()
        .with_span_processor(bounded_span_chain(exporter, batch, dropped))
        .with_sampler(Sampler::AlwaysOn)
        .with_resource(
            Resource::builder_empty()
                .with_service_name("res-probe")
                .build(),
        )
        .build()
}

fn end_spans(provider: &SdkTracerProvider, n: usize) {
    let tracer = provider.tracer("bounded-test");
    for _ in 0..n {
        tracer.start("span").end();
    }
}

// @lat: [[tests#Bounded export queues#Bounded span processor drops and counts past queue size]]
#[test]
fn bounded_span_processor_drops_and_counts_past_queue_size() {
    let probe = Probe::default();
    let dropped = DropCounters::default();
    let provider = provider(
        FirstExport::WaitForGate,
        &batch(4, 1, Duration::from_millis(10)),
        &probe,
        &dropped,
    );
    let closed = probe.close_gate();

    end_spans(&provider, 10);
    let dropped_while_stalled = dropped.snapshot().traces;
    drop(closed);
    provider.shutdown().expect("the provider shuts down");

    assert_eq!(dropped_while_stalled, 6);
    assert_eq!(probe.exported(), 4);
    assert_eq!(dropped.snapshot().traces, 6);
}

// @lat: [[tests#Bounded export queues#Failed export frees slots and counts the batch]]
#[test]
fn failed_export_frees_slots_and_counts_the_batch() {
    let probe = Probe::default();
    let dropped = DropCounters::default();
    // no timed export runs before each flush
    let provider = provider(
        FirstExport::Fail,
        &batch(4, 4, Duration::from_secs(60)),
        &probe,
        &dropped,
    );

    end_spans(&provider, 3);
    assert!(provider.force_flush().is_err(), "the first export fails");
    end_spans(&provider, 4);
    provider.force_flush().expect("the second export succeeds");

    assert_eq!(dropped.snapshot().traces, 3);
    assert_eq!(probe.exported(), 4);
}

// @lat: [[tests#Bounded export queues#Wrappers forward resource to exporter]]
#[test]
fn wrappers_forward_resource_to_exporter() {
    let probe = Probe::default();
    let provider = provider(
        FirstExport::Succeed,
        &ExportBatch::default(),
        &probe,
        &DropCounters::default(),
    );

    end_spans(&provider, 1);
    provider.force_flush().expect("the export succeeds");

    let resource = probe.resource().expect("the exporter got a resource");
    assert_eq!(
        resource.get(&Key::new("service.name")),
        Some(Value::from("res-probe"))
    );
}

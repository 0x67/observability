use std::{num::NonZeroUsize, time::Duration};

use opentelemetry::logs::{LogRecord as _, Logger as _, LoggerProvider as _};
use opentelemetry_sdk::logs::SdkLoggerProvider;

use super::{
    super::test_exporters::{FirstExport, Probe, TestLogExporter},
    bounded_log_chain,
};
use crate::{config::ExportBatch, pipeline::dropped::DropCounters};

// @lat: [[tests#Bounded export queues#Bounded log processor drops and counts past queue size]]
#[test]
fn bounded_log_processor_drops_and_counts_past_queue_size() {
    let probe = Probe::default();
    let dropped = DropCounters::default();
    let batch = ExportBatch {
        queue_size: NonZeroUsize::new(4).expect("nonzero"),
        max_batch_size: NonZeroUsize::MIN,
        delay: Duration::from_millis(10),
    };
    let exporter = TestLogExporter::new(FirstExport::WaitForGate, &probe);
    let provider = SdkLoggerProvider::builder()
        .with_log_processor(bounded_log_chain(exporter, &batch, &dropped))
        .build();
    let logger = provider.logger("bounded-test");
    let closed = probe.close_gate();

    for _ in 0..10 {
        let mut record = logger.create_log_record();
        record.set_body("record".into());
        logger.emit(record);
    }
    let dropped_while_stalled = dropped.snapshot().logs;
    drop(closed);
    provider.shutdown().expect("the provider shuts down");

    assert_eq!(dropped_while_stalled, 6);
    assert_eq!(probe.exported(), 4);
}

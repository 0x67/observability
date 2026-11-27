use std::{
    io::{self, Write},
    num::NonZeroUsize,
    sync::{Arc, Mutex, PoisonError, RwLock},
    thread,
};

use super::{CountingWriter, sink_writer};
use crate::{
    config::{LineQueue, Overflow},
    pipeline::dropped::DropCounters,
};

const LINES: usize = 10;

/// Output that blocks every write while the test holds the gate's write lock.
#[derive(Clone, Default)]
struct GatedOutput {
    gate: Arc<RwLock<()>>,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Write for GatedOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _open = self.gate.read().unwrap_or_else(PoisonError::into_inner);
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(String::from_utf8_lossy(buf).into_owned());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl GatedOutput {
    fn delivered(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn one_line_queue(overflow: Overflow) -> LineQueue {
    LineQueue {
        lines: NonZeroUsize::MIN,
        overflow,
    }
}

fn write_lines(mut writer: CountingWriter) {
    for i in 0..LINES {
        writer
            .write_all(format!("line {i}\n").as_bytes())
            .expect("the writer accepts the line");
    }
}

// @lat: [[tests#Writer sink queue#Drop overflow counts lost lines]]
#[test]
fn drop_overflow_counts_lost_lines() {
    let output = GatedOutput::default();
    let dropped = DropCounters::default();
    let (writer, worker) = sink_writer(output.clone(), one_line_queue(Overflow::Drop), &dropped);
    let closed = output.gate.write().unwrap_or_else(PoisonError::into_inner);

    write_lines(writer);
    drop(closed);
    drop(worker);

    let delivered = output.delivered().len() as u64;
    let lost = dropped.snapshot().logs;
    assert_eq!(delivered + lost, LINES as u64, "{delivered} delivered");
    // one line can sit in the queue and one in the blocked write
    assert!(lost >= LINES as u64 - 2, "{lost} lost");
}

// @lat: [[tests#Writer sink queue#Block overflow delivers every line]]
#[test]
fn block_overflow_delivers_every_line() {
    let output = GatedOutput::default();
    let dropped = DropCounters::default();
    let (writer, worker) = sink_writer(output.clone(), one_line_queue(Overflow::Block), &dropped);
    let closed = output.gate.write().unwrap_or_else(PoisonError::into_inner);

    let logger = thread::spawn(move || write_lines(writer));
    drop(closed);
    logger.join().expect("the logging thread finishes");
    drop(worker);

    let want: Vec<String> = (0..LINES).map(|i| format!("line {i}\n")).collect();
    assert_eq!(output.delivered(), want);
    assert_eq!(dropped.snapshot().logs, 0);
}

//! The non-blocking writer behind each stdout and rolling-file sink, with its line queue
//! bounded and its dropped lines counted.

use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::fmt::MakeWriter;

use super::dropped::{DropCounters, DropReason};
use crate::{
    config::{LineQueue, Overflow},
    error::Signal,
};

#[cfg(test)]
mod tests;

/// A writer thread for `out` with `queue`'s bound and overflow policy, and the guard that
/// flushes it on drop.
pub(super) fn sink_writer<W: Write + Send + 'static>(
    out: W,
    queue: LineQueue,
    dropped: &DropCounters,
) -> (CountingWriter, WorkerGuard) {
    let (writer, guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(queue.lines.get())
        .lossy(queue.overflow == Overflow::Drop)
        .finish(out);
    let counts = Arc::new(LineDrops {
        errors: writer.error_counter(),
        seen: AtomicUsize::new(0),
        dropped: dropped.clone(),
    });

    (CountingWriter { writer, counts }, guard)
}

/// `NonBlocking` that reports newly dropped lines to the pipeline's drop counters.
#[derive(Clone)]
pub(super) struct CountingWriter {
    writer: NonBlocking,
    counts: Arc<LineDrops>,
}

struct LineDrops {
    errors: ErrorCounter,
    // dropped lines already added to `dropped`
    seen: AtomicUsize,
    dropped: DropCounters,
}

impl LineDrops {
    // a lossy NonBlocking returns Ok for a dropped line and only bumps its error counter,
    // so the write that lost a line reports it here
    fn report_new(&self) {
        let now = self.errors.dropped_lines();
        if now <= self.seen.load(Ordering::Relaxed) {
            return;
        }
        let before = self.seen.fetch_max(now, Ordering::Relaxed);
        if now > before {
            self.dropped
                .add(Signal::Logs, DropReason::BufferFull, (now - before) as u64);
        }
    }
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(buf)?;
        self.counts.report_new();
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

// one clone per event, as tracing-appender's NonBlocking does
impl<'a> MakeWriter<'a> for CountingWriter {
    type Writer = CountingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

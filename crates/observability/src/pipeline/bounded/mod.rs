//! Span and log processors that hold at most `ExportBatch::queue_size` items from span end
//! (or log emit) to the end of their export, and count every item they drop.
//!
//! The SDK batch processors drop on a full channel with only a log line, and keep their
//! queue size private. The bounded processor decides first: [`InFlight`] counts every item
//! passed on until its export returns, which is never less than what sits in the SDK
//! channel of the same capacity, so the SDK never finds its channel full.

mod traces;

#[cfg(test)]
mod test_exporters;

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(in crate::pipeline) use self::traces::{BoundedSpanProcessor, bounded_span_chain};

/// Items a bounded processor accepted whose export has not finished.
#[derive(Clone, Debug)]
struct InFlight {
    count: Arc<AtomicUsize>,
    limit: usize,
}

impl InFlight {
    fn new(limit: NonZeroUsize) -> Self {
        Self {
            count: Arc::default(),
            limit: limit.get(),
        }
    }

    /// Take one slot; `false` when all `limit` are taken.
    fn try_acquire(&self) -> bool {
        if self.count.fetch_add(1, Ordering::Relaxed) < self.limit {
            return true;
        }
        self.count.fetch_sub(1, Ordering::Relaxed);
        false
    }

    /// Free the slots of an export that returned, whatever its result.
    fn release(&self, n: usize) {
        self.count.fetch_sub(n, Ordering::Relaxed);
    }
}

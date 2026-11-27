//! `metrics` recorder that exports through an OpenTelemetry meter.
//!
//! Each `metrics::Key` gets one OTel instrument with the key's labels as attributes:
//! an observable `u64` counter, an observable `f64` gauge, or an `f64` histogram.
//! `Counter::absolute` keeps the larger of the stored and the new value, so a counter
//! never reports a drop.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use ::metrics::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit};
use metrics_util::{MetricKind, registry::Registry};
use opentelemetry::metrics::Meter;

use self::storage::OtelStorage;

mod histogram;
mod storage;
#[cfg(test)]
mod tests;

/// Registers one OTel instrument per key on the meter it was built with.
pub(crate) struct OtelRecorder {
    registry: Registry<Key, OtelStorage>,
    descriptions: Arc<Descriptions>,
}

impl OtelRecorder {
    pub(crate) fn new(meter: Meter) -> Self {
        let descriptions = Arc::new(Descriptions::default());
        let storage = OtelStorage::new(meter, Arc::clone(&descriptions));
        Self {
            registry: Registry::new(storage),
            descriptions,
        }
    }
}

impl Recorder for OtelRecorder {
    fn describe_counter(&self, name: KeyName, unit: Option<Unit>, text: SharedString) {
        self.descriptions
            .insert(name, MetricKind::Counter, unit, text);
    }

    fn describe_gauge(&self, name: KeyName, unit: Option<Unit>, text: SharedString) {
        self.descriptions
            .insert(name, MetricKind::Gauge, unit, text);
    }

    fn describe_histogram(&self, name: KeyName, unit: Option<Unit>, text: SharedString) {
        self.descriptions
            .insert(name, MetricKind::Histogram, unit, text);
    }

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        self.registry
            .get_or_create_counter(key, |counter| Counter::from_arc(Arc::clone(counter)))
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        self.registry
            .get_or_create_gauge(key, |gauge| Gauge::from_arc(Arc::clone(gauge)))
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        self.registry
            .get_or_create_histogram(key, |histogram| Histogram::from_arc(Arc::clone(histogram)))
    }
}

/// Unit and text from `describe_*`, read once when a key's instrument is created.
#[derive(Default)]
struct Descriptions(Mutex<HashMap<(KeyName, MetricKind), Description>>);

impl Descriptions {
    /// Keeps the first description of a name, as the Prometheus recorder does.
    fn insert(&self, name: KeyName, kind: MetricKind, unit: Option<Unit>, text: SharedString) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((name, kind))
            .or_insert(Description { unit, text });
    }

    fn get(&self, name: KeyName, kind: MetricKind) -> Option<Description> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(name, kind))
            .cloned()
    }
}

#[derive(Clone)]
struct Description {
    unit: Option<Unit>,
    text: SharedString,
}

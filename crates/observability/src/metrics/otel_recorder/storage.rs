use std::sync::{Arc, atomic::Ordering};

use ::metrics::{Key, Unit, atomics::AtomicU64};
use metrics_util::{MetricKind, registry::Storage};
use opentelemetry::{
    KeyValue,
    metrics::{AsyncInstrumentBuilder, HistogramBuilder, Meter},
};

use super::{Description, Descriptions, histogram::OtelHistogram};

/// Creates the OTel instrument for a key on its first registration.
pub(super) struct OtelStorage {
    meter: Meter,
    descriptions: Arc<Descriptions>,
}

impl OtelStorage {
    pub(super) fn new(meter: Meter, descriptions: Arc<Descriptions>) -> Self {
        Self {
            meter,
            descriptions,
        }
    }
}

// `metrics` implements `CounterFn` for `AtomicU64` and `GaugeFn` for it as f64 bits, so
// the callbacks read the same atomic the handles write.
impl Storage<Key> for OtelStorage {
    type Counter = Arc<AtomicU64>;
    type Gauge = Arc<AtomicU64>;
    type Histogram = Arc<OtelHistogram>;

    fn counter(&self, key: &Key) -> Self::Counter {
        let value = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&value);
        let attributes = attributes(key);

        let builder = self
            .meter
            .u64_observable_counter(key.name_shared().into_inner());
        // the SDK keeps the callback after the returned handle drops
        described(
            builder,
            self.descriptions
                .get(key.name_shared(), MetricKind::Counter),
        )
        .with_callback(move |observer| {
            observer.observe(observed.load(Ordering::Acquire), &attributes);
        })
        .build();
        value
    }

    fn gauge(&self, key: &Key) -> Self::Gauge {
        let value = Arc::new(AtomicU64::new(0.0_f64.to_bits()));
        let observed = Arc::clone(&value);
        let attributes = attributes(key);

        let builder = self
            .meter
            .f64_observable_gauge(key.name_shared().into_inner());
        described(
            builder,
            self.descriptions.get(key.name_shared(), MetricKind::Gauge),
        )
        .with_callback(move |observer| {
            observer.observe(
                f64::from_bits(observed.load(Ordering::Acquire)),
                &attributes,
            );
        })
        .build();
        value
    }

    fn histogram(&self, key: &Key) -> Self::Histogram {
        let builder = self.meter.f64_histogram(key.name_shared().into_inner());
        let description = self
            .descriptions
            .get(key.name_shared(), MetricKind::Histogram);
        let histogram = described_histogram(builder, description).build();
        Arc::new(OtelHistogram::new(histogram, attributes(key)))
    }
}

fn attributes(key: &Key) -> Vec<KeyValue> {
    key.labels()
        .map(|label| KeyValue::new(label.key().to_owned(), label.value().to_owned()))
        .collect()
}

fn described<'a, I, M>(
    builder: AsyncInstrumentBuilder<'a, I, M>,
    description: Option<Description>,
) -> AsyncInstrumentBuilder<'a, I, M> {
    let Some(Description { unit, text }) = description else {
        return builder;
    };
    let builder = builder.with_description(text);
    match unit {
        Some(unit) => builder.with_unit(ucum(unit)),
        None => builder,
    }
}

fn described_histogram<'a, T>(
    builder: HistogramBuilder<'a, T>,
    description: Option<Description>,
) -> HistogramBuilder<'a, T> {
    let Some(Description { unit, text }) = description else {
        return builder;
    };
    let builder = builder.with_description(text);
    match unit {
        Some(unit) => builder.with_unit(ucum(unit)),
        None => builder,
    }
}

/// UCUM unit string, the form OTel expects in `unit`.
fn ucum(unit: Unit) -> &'static str {
    match unit {
        Unit::Count => "1",
        Unit::Percent => "%",
        Unit::Seconds => "s",
        Unit::Milliseconds => "ms",
        Unit::Microseconds => "us",
        Unit::Nanoseconds => "ns",
        Unit::Tebibytes => "TiBy",
        Unit::Gibibytes => "GiBy",
        Unit::Mebibytes => "MiBy",
        Unit::Kibibytes => "KiBy",
        Unit::Bytes => "By",
        Unit::TerabitsPerSecond => "Tbit/s",
        Unit::GigabitsPerSecond => "Gbit/s",
        Unit::MegabitsPerSecond => "Mbit/s",
        Unit::KilobitsPerSecond => "kbit/s",
        Unit::BitsPerSecond => "bit/s",
        Unit::CountPerSecond => "1/s",
    }
}

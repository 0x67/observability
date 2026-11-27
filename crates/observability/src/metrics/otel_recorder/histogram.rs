use ::metrics::HistogramFn;
use opentelemetry::{KeyValue, metrics::Histogram};

/// One histogram key: the OTel instrument plus the key's labels as attributes.
pub(super) struct OtelHistogram {
    histogram: Histogram<f64>,
    attributes: Vec<KeyValue>,
}

impl OtelHistogram {
    pub(super) fn new(histogram: Histogram<f64>, attributes: Vec<KeyValue>) -> Self {
        Self {
            histogram,
            attributes,
        }
    }
}

impl HistogramFn for OtelHistogram {
    fn record(&self, value: f64) {
        self.histogram.record(value, &self.attributes);
    }
}

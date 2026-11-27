use ::metrics::{
    Unit, counter, describe_gauge, describe_histogram, gauge, histogram, with_local_recorder,
};
use opentelemetry::{KeyValue, metrics::MeterProvider as _};
use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, SdkMeterProvider,
    data::{AggregatedMetrics, Metric, MetricData, ResourceMetrics, ScopeMetrics},
};

use super::OtelRecorder;

/// Records through an `OtelRecorder` on a fresh provider and returns one flush's export.
fn export(record: impl FnOnce()) -> Vec<ResourceMetrics> {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let recorder = OtelRecorder::new(provider.meter("observability"));

    with_local_recorder(&recorder, record);
    provider.force_flush().expect("in-memory flush succeeds");

    exporter
        .get_finished_metrics()
        .expect("in-memory exporter holds the flush")
}

fn metrics_named<'a>(
    exported: &'a [ResourceMetrics],
    name: &'a str,
) -> impl Iterator<Item = &'a Metric> {
    exported
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .filter(move |metric| metric.name() == name)
}

fn only_metric<'a>(exported: &'a [ResourceMetrics], name: &'a str) -> &'a Metric {
    let mut found = metrics_named(exported, name);
    let metric = found
        .next()
        .unwrap_or_else(|| panic!("no metric named {name}"));
    assert!(found.next().is_none(), "{name} exported more than once");
    metric
}

// @lat: [[tests#OTLP metrics recorder#Counter labels become attributes]]
#[test]
fn counter_labels_become_attributes() {
    let exported = export(|| {
        counter!("feed.msgs", "venue" => "x").increment(3);
        counter!("feed.msgs", "venue" => "y").increment(5);
    });

    let AggregatedMetrics::U64(MetricData::Sum(sum)) = only_metric(&exported, "feed.msgs").data()
    else {
        panic!("feed.msgs is not a u64 sum");
    };
    assert!(sum.is_monotonic());
    let mut points: Vec<_> = sum
        .data_points()
        .map(|point| {
            (
                point.value(),
                point.attributes().cloned().collect::<Vec<_>>(),
            )
        })
        .collect();
    points.sort_by_key(|(value, _)| *value);
    assert_eq!(
        points,
        [
            (3, vec![KeyValue::new("venue", "x")]),
            (5, vec![KeyValue::new("venue", "y")]),
        ]
    );
}

// @lat: [[tests#OTLP metrics recorder#Gauge reports last value after set and increment]]
#[test]
fn gauge_reports_last_value_after_set_and_increment() {
    let exported = export(|| {
        gauge!("feed.depth").set(2.5);
        gauge!("feed.depth").increment(0.5);
    });

    let AggregatedMetrics::F64(MetricData::Gauge(gauge)) =
        only_metric(&exported, "feed.depth").data()
    else {
        panic!("feed.depth is not an f64 gauge");
    };
    let values: Vec<f64> = gauge.data_points().map(|point| point.value()).collect();
    assert_eq!(values, [3.0]);
}

// @lat: [[tests#OTLP metrics recorder#Histogram reports count and sum]]
#[test]
fn histogram_reports_count_and_sum() {
    let exported = export(|| {
        let latency = histogram!("feed.latency", "venue" => "x");
        latency.record(0.25);
        latency.record(0.75);
    });

    let AggregatedMetrics::F64(MetricData::Histogram(histogram)) =
        only_metric(&exported, "feed.latency").data()
    else {
        panic!("feed.latency is not an f64 histogram");
    };
    let points: Vec<_> = histogram
        .data_points()
        .map(|point| {
            (
                point.count(),
                point.sum(),
                point.attributes().cloned().collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(points, [(2, 1.0, vec![KeyValue::new("venue", "x")])]);
}

// @lat: [[tests#OTLP metrics recorder#Described gauge exports UCUM unit and text]]
#[test]
fn described_gauge_exports_ucum_unit_and_text() {
    let exported = export(|| {
        describe_gauge!("feed.p99", Unit::Seconds, "p99 window");
        gauge!("feed.p99").set(0.004);
    });

    let metric = only_metric(&exported, "feed.p99");
    assert_eq!((metric.unit(), metric.description()), ("s", "p99 window"));
}

// @lat: [[tests#OTLP metrics recorder#Described histogram exports UCUM unit and text]]
#[test]
fn described_histogram_exports_ucum_unit_and_text() {
    let exported = export(|| {
        describe_histogram!("payload", Unit::Bytes, "payload size");
        histogram!("payload").record(512.0);
    });

    let metric = only_metric(&exported, "payload");
    assert_eq!(
        (metric.unit(), metric.description()),
        ("By", "payload size")
    );
}

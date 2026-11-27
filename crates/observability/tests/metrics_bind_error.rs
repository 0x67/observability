//! A Prometheus listen address the host cannot bind returns `MetricsError::Bind` and leaves
//! the metrics gate off.
#![cfg(feature = "metrics-prom")]

use observability::{
    MetricsConfig, MetricsError, MetricsExporter, PrometheusConfig, init_metrics, metrics_enabled,
};

// @lat: [[tests#Init end to end#Unbindable Prometheus address maps to Bind and leaves gate off]]
#[test]
fn unbindable_prometheus_address_maps_to_bind_and_leaves_gate_off() {
    // 192.0.2.1 is TEST-NET-1 (RFC 5737): no CI host owns it, so binding it fails with
    // "address not available"
    let cfg = MetricsConfig {
        exporter: MetricsExporter::Prometheus(PrometheusConfig {
            bind: "192.0.2.1:0".parse().expect("valid socket address"),
            ..Default::default()
        }),
    };

    let result = init_metrics(cfg, None, "bind-error");

    assert!(
        matches!(result, Err(MetricsError::Bind(_))),
        "{:?}",
        result.err()
    );
    assert!(!metrics_enabled());
}

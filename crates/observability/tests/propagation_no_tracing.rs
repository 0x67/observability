//! Without tracing there is no OTel span layer to take a remote parent.
#![cfg(feature = "otel")]

use observability::{
    PipelineConfig, PropagationError, init_pipeline, propagation::set_parent_from_headers,
};

// @lat: [[tests#Trace context propagation#Set parent without tracing returns no OTel layer]]
#[test]
fn set_parent_without_tracing_returns_no_otel_layer() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::remove_var("RUST_LOG") };
    let _guard = init_pipeline(PipelineConfig::default()).expect("pipeline installs");
    let span = tracing::info_span!("local");

    let set = set_parent_from_headers(
        &span,
        [(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )],
    );

    assert!(matches!(set, Err(PropagationError::NoOtelLayer)), "{set:?}");
}

//! A second `init_pipeline` in one process returns `AlreadyInit` instead of panicking.

use observability::{PipelineConfig, PipelineError, init_pipeline};

// @lat: [[tests#Init end to end#Second init pipeline returns already init]]
#[test]
fn second_init_pipeline_returns_already_init() {
    let cfg = || PipelineConfig {
        logging: None,
        tracing: None,
        otlp: None,
        ..Default::default()
    };

    let first = init_pipeline(cfg()).expect("first init installs the subscriber");
    let second = init_pipeline(cfg());

    assert!(
        matches!(second, Err(PipelineError::AlreadyInit)),
        "{:?}",
        second.err()
    );
    drop(first);
}

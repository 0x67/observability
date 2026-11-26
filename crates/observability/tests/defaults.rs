//! The default config brings up stdout JSON logging with no tokio runtime and no recorder.

use observability::{ObsConfig, init};

// @lat: [[tests#Config defaults#Default ObsConfig inits without a runtime]]
#[test]
fn default_obs_config_inits_without_a_runtime() {
    let guard = init(ObsConfig::default()).expect("the default config installs");

    assert!(guard.metrics.is_none(), "the default installs no recorder");
}

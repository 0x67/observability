use tracing_subscriber::EnvFilter;

/// Build a sink `EnvFilter`. Precedence: valid `RUST_LOG` > `level` arg > `"info"`.
///
/// `try_from_default_env` errors when `RUST_LOG` is unset or holds bad directives, so the
/// `level` arg (per-sink or config) takes effect only then. WARNING: a set `RUST_LOG`
/// overrides every sink uniformly, since each layer builds its filter from the same env;
/// per-sink `level` applies only while `RUST_LOG` is absent. Empty `RUST_LOG` counts as
/// set and yields the builder default directive, not `level`.
pub(super) fn env_filter(level: &str) -> EnvFilter {
    EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap_or_else(|_| EnvFilter::new("info"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::env_filter;

    // RUST_LOG is process-global; serialize mutation so a threaded runner cannot race.
    // nextest forks one process per test, but this keeps the tests correct under any runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // Set (or clear) RUST_LOG, read the resulting filter, then leave env clean.
    fn filter_string(rust_log: Option<&str>, config: &str) -> String {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: ENV_LOCK serializes every RUST_LOG access in this module.
        unsafe {
            match rust_log {
                Some(v) => std::env::set_var("RUST_LOG", v),
                None => std::env::remove_var("RUST_LOG"),
            }
        }
        let out = env_filter(config).to_string();
        // SAFETY: same lock still held; restore for the next test.
        unsafe { std::env::remove_var("RUST_LOG") };
        out
    }

    #[test]
    fn valid_rust_log_beats_config_level() {
        assert_eq!(filter_string(Some("debug"), "warn"), "debug");
    }

    #[test]
    fn unset_rust_log_uses_config_level() {
        assert_eq!(filter_string(None, "warn"), "warn");
    }

    #[test]
    fn invalid_rust_log_falls_to_config_level() {
        // bad level after `=` fails to parse, so config wins. NOTE a bare word like
        // "not_a_level" is a valid target directive, not an error, so it would win.
        assert_eq!(filter_string(Some("app=notalevel"), "debug"), "debug");
    }

    #[test]
    fn invalid_config_level_falls_to_info() {
        assert_eq!(filter_string(None, "app=notalevel"), "info");
    }

    #[test]
    fn empty_rust_log_counts_as_set() {
        // empty string IS set, so it beats config yet yields the empty default filter.
        assert_eq!(filter_string(Some(""), "warn"), "");
    }
}

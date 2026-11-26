#[cfg(feature = "otel")]
use tracing::Metadata;
use tracing_subscriber::EnvFilter;
#[cfg(feature = "otel")]
use tracing_subscriber::{
    Registry,
    filter::{FilterExt, LevelFilter, combinator::And},
    layer,
};

#[cfg(feature = "otel")]
use crate::otlp::EXPORT_THREAD_NAME;

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

/// Threads that run only export work: this crate's export runtime and the SDK's batch
/// and periodic threads. Events created there describe the export itself.
#[cfg(feature = "otel")]
const EXPORTER_THREADS: [&str; 4] = [
    EXPORT_THREAD_NAME,
    "OpenTelemetry.Traces.BatchProcessor",
    "OpenTelemetry.Logs.BatchProcessor",
    "OpenTelemetry.Metrics.PeriodicReader",
];

#[cfg(feature = "otel")]
thread_local! {
    static ON_EXPORTER_THREAD: bool = std::thread::current()
        .name()
        .is_some_and(|name| EXPORTER_THREADS.contains(&name));
}

/// Drops events and spans created on an exporter thread; every other thread passes.
#[cfg(feature = "otel")]
pub(super) struct ExporterThreadFilter;

#[cfg(feature = "otel")]
impl<S> layer::Filter<S> for ExporterThreadFilter {
    fn enabled(&self, _: &Metadata<'_>, _: &layer::Context<'_, S>) -> bool {
        // a thread-local already destroyed at thread exit counts as an app thread
        !ON_EXPORTER_THREAD.try_with(|on| *on).unwrap_or(false)
    }

    // the trait default is None, which And passes on and which lifts the global max
    // level to TRACE
    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }
}

/// Filter for the OTLP span and log layers: the sink level, minus exporter threads.
#[cfg(feature = "otel")]
pub(super) fn otlp_filter(level: &str) -> And<EnvFilter, ExporterThreadFilter, Registry> {
    env_filter(level).and(ExporterThreadFilter)
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::Mutex;

    use super::env_filter;

    // RUST_LOG is process-global; serialize mutation so a threaded runner cannot race.
    // nextest forks one process per test, but this keeps the tests correct under any runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Set (or clear) RUST_LOG, run `build`, then leave env clean. Every pipeline test that
    /// builds an `EnvFilter` goes through here.
    pub(in crate::pipeline) fn with_rust_log<T>(
        rust_log: Option<&str>,
        build: impl FnOnce() -> T,
    ) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: ENV_LOCK serializes every RUST_LOG access in this crate's tests.
        unsafe {
            match rust_log {
                Some(v) => std::env::set_var("RUST_LOG", v),
                None => std::env::remove_var("RUST_LOG"),
            }
        }
        let out = build();
        // SAFETY: same lock still held; restore for the next test.
        unsafe { std::env::remove_var("RUST_LOG") };
        out
    }

    fn filter_string(rust_log: Option<&str>, config: &str) -> String {
        with_rust_log(rust_log, || env_filter(config).to_string())
    }

    // @lat: [[tests#OTLP layer filters#Exporter thread filter drops only exporter thread events]]
    #[cfg(feature = "otel")]
    #[test]
    fn exporter_thread_filter_drops_only_exporter_thread_events() {
        use std::sync::{Arc, PoisonError};

        use tracing::{Dispatch, Event, Subscriber, dispatcher};
        use tracing_subscriber::{
            Layer, Registry,
            layer::{Context, SubscriberExt},
        };

        use super::otlp_filter;

        struct ThreadNames(Arc<Mutex<Vec<String>>>);

        impl<S: Subscriber> Layer<S> for ThreadNames {
            fn on_event(&self, _: &Event<'_>, _: Context<'_, S>) {
                let name = std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .to_owned();
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(name);
            }
        }

        fn probe(dispatch: &Dispatch) {
            dispatcher::with_default(dispatch, || {
                tracing::warn!(target: "tonic::transport", "probe");
            });
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let filter = with_rust_log(None, || otlp_filter("info"));
        let dispatch = Dispatch::new(
            Registry::default().with(ThreadNames(Arc::clone(&seen)).with_filter(filter)),
        );

        probe(&dispatch);
        for name in [
            "app-worker",
            "observability-otlp",
            "OpenTelemetry.Traces.BatchProcessor",
            "OpenTelemetry.Logs.BatchProcessor",
            "OpenTelemetry.Metrics.PeriodicReader",
        ] {
            let dispatch = dispatch.clone();
            std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || probe(&dispatch))
                .expect("spawn probe thread")
                .join()
                .expect("probe thread panicked");
        }

        let test_thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        let seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(*seen, [test_thread, "app-worker".to_owned()]);
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

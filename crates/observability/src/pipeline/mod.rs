//! Subscriber subsystem: one `tracing_subscriber::Registry` fusing a logging fan-out
//! (N sinks at once) with a single tracing OTLP sink.
//!
//! Logging fans out to every configured sink (stdout, rolling file, OTLP-log bridge),
//! each with its own format and level filter; every non-blocking file sink hands back a
//! `WorkerGuard` retained in `PipelineGuard` so tail lines flush on shutdown. Tracing is a
//! single OTLP span sink by design, never fanned out, never on disk. Both read transport
//! from the shared `OtlpConfig`; that is the only runtime coupling with the metrics recorder.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{Layer, Registry, layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(feature = "otel")]
use self::otel::{OtelProviders, build_tracer_provider, trace_layer};
#[cfg(feature = "otel-logs")]
use self::otel::{build_logger_provider, log_bridge_layer};
#[cfg(feature = "bunyan")]
use self::sinks::bunyan_storage_layer;
use self::sinks::{log_layer, rolling_appender};
#[cfg(feature = "otel")]
use crate::otlp::ExportRuntime;
use crate::{
    config::{LogSinkKind, PipelineConfig},
    error::PipelineError,
};

mod filter;
mod guard;
#[cfg(feature = "otel")]
mod otel;
mod sinks;

pub use guard::PipelineGuard;

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Build the one Registry and install it as the global default via `try_init`.
///
/// A second call returns `PipelineError::AlreadyInit` instead of panicking. An OTLP
/// exporter that fails to build degrades to logging-only (warn on stderr, still `Ok`).
pub fn init_pipeline(cfg: PipelineConfig) -> Result<PipelineGuard, PipelineError> {
    let mut workers: Vec<WorkerGuard> = Vec::new();
    let mut layers: Vec<BoxedLayer> = Vec::new();

    // bunyan needs one JsonStorageLayer in the registry, added once ahead of any bunyan sink.
    #[cfg(feature = "bunyan")]
    if let Some(logging) = &cfg.logging
        && let Some(storage) = bunyan_storage_layer(&logging.sinks, &cfg.level)
    {
        layers.push(storage);
    }

    // every OTLP provider here runs on one export runtime; `shared` hands back the live one
    #[cfg(feature = "otel")]
    let mut export_runtime = None;
    #[cfg(feature = "otel-logs")]
    let mut logger_provider = None;

    if let Some(logging) = &cfg.logging {
        for sink in &logging.sinks {
            let level = sink.level.as_deref().unwrap_or(&cfg.level);
            match &sink.kind {
                LogSinkKind::Stdout => {
                    let (writer, guard) = tracing_appender::non_blocking(std::io::stdout());
                    workers.push(guard);
                    layers.push(log_layer(&sink.format, &cfg.service_name, writer, level));
                }
                LogSinkKind::RollingFile {
                    dir,
                    prefix,
                    rotation,
                } => {
                    let appender = rolling_appender(dir, prefix, rotation);
                    let (writer, guard) = tracing_appender::non_blocking(appender);
                    workers.push(guard);
                    layers.push(log_layer(&sink.format, &cfg.service_name, writer, level));
                }
                LogSinkKind::Otlp => {
                    #[cfg(feature = "otel-logs")]
                    match cfg.otlp.as_ref() {
                        Some(otlp) => match ExportRuntime::shared().and_then(|runtime| {
                            build_logger_provider(otlp, &cfg.service_name, &runtime)
                                .map(|provider| (provider, runtime))
                        }) {
                            Ok((provider, runtime)) => {
                                layers.push(log_bridge_layer(&provider, level));
                                logger_provider = Some(provider);
                                export_runtime = Some(runtime);
                            }
                            Err(e) => pre_init_notice(format_args!(
                                "otlp log exporter build failed, log sink skipped: {e}"
                            )),
                        },
                        None => pre_init_notice(format_args!(
                            "otlp log sink needs an otlp config, skipping"
                        )),
                    }
                    #[cfg(not(feature = "otel-logs"))]
                    pre_init_notice(format_args!(
                        "otlp log sink configured but `otel-logs` feature off, skipping"
                    ));
                }
            }
        }
    }

    // tracing: a single OTLP span sink, never fanned out.
    #[cfg(feature = "otel")]
    let mut tracer_provider = None;
    #[cfg(feature = "otel")]
    if let (Some(_tracing), Some(otlp)) = (&cfg.tracing, &cfg.otlp) {
        match ExportRuntime::shared().and_then(|runtime| {
            build_tracer_provider(otlp, &cfg.service_name, &runtime)
                .map(|provider| (provider, runtime))
        }) {
            Ok((provider, runtime)) => {
                layers.push(trace_layer(&provider, &cfg.level));
                tracer_provider = Some(provider);
                export_runtime = Some(runtime);
            }
            Err(e) => pre_init_notice(format_args!(
                "otlp trace exporter build failed, tracing disabled: {e}"
            )),
        }
    }

    Registry::default()
        .with(layers)
        .try_init()
        .map_err(|_| PipelineError::AlreadyInit)?;

    Ok(PipelineGuard {
        workers,
        #[cfg(feature = "otel")]
        otel: export_runtime.map(|runtime| OtelProviders {
            tracer: tracer_provider,
            #[cfg(feature = "otel-logs")]
            logger: logger_provider,
            _runtime: runtime,
        }),
    })
}

#[expect(
    clippy::print_stderr,
    reason = "notices print before this crate's subscriber exists, so stderr is the only sink"
)]
fn pre_init_notice(msg: std::fmt::Arguments<'_>) {
    eprintln!("observability: {msg}");
}

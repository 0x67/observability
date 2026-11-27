//! Subscriber subsystem: one `tracing_subscriber::Registry` fusing a logging fan-out
//! (N sinks at once) with a single tracing OTLP sink.
//!
//! Logging fans out to every configured sink (stdout, rolling file, OTLP-log bridge),
//! each with its own format and level filter; every non-blocking file sink hands back a
//! `WorkerGuard` retained in `PipelineGuard` so tail lines flush on shutdown. Tracing is a
//! single OTLP span sink by design, never fanned out, never on disk. Both read transport
//! from the shared `OtlpConfig`; that is the only runtime coupling with the metrics recorder.

use tracing_subscriber::{Layer, Registry, layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(feature = "otel")]
use self::otel::otel_layers;
#[cfg(feature = "bunyan")]
use self::sinks::bunyan_storage_layer;
use self::sinks::{log_layer, rolling_appender};
#[cfg(not(feature = "otel"))]
use crate::config::LogSink;
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

/// Build every layer, then install them as the global default via `try_init`.
///
/// With tracing on, a successful install also sets the W3C trace context propagator as the
/// process-wide OTel text map propagator, replacing one set earlier; one set after this
/// call replaces it in turn.
///
/// # Errors
///
/// Every check runs before install, so an error leaves no subscriber set.
/// [`PipelineError::SampleRatio`] for a trace sample ratio that is NaN or outside
/// `0.0..=1.0`, [`PipelineError::FeatureDisabled`] when the config needs a cargo feature
/// this build lacks, [`PipelineError::MissingOtlp`] when tracing or an OTLP log sink has no
/// `otlp`, and [`PipelineError::Exporter`] when the export runtime or an exporter fails to
/// build.
/// A second call returns [`PipelineError::AlreadyInit`].
pub fn init_pipeline(cfg: PipelineConfig) -> Result<PipelineGuard, PipelineError> {
    let (layers, guard) = build_layers(&cfg)?;

    Registry::default()
        .with(layers)
        .try_init()
        .map_err(|_| PipelineError::AlreadyInit)?;

    // other OTel-aware libraries read the global propagator; a failed init leaves it alone
    #[cfg(feature = "otel")]
    if cfg.tracing.is_some() {
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
    }
    Ok(guard)
}

/// Every layer and the guard that owns their writers, providers, and export runtime.
fn build_layers(cfg: &PipelineConfig) -> Result<(Vec<BoxedLayer>, PipelineGuard), PipelineError> {
    // the SDK would clamp a bad ratio; in a config file it is a typo, so reject it
    if let Some(ratio) = cfg
        .tracing
        .as_ref()
        .and_then(|tracing| tracing.sample_ratio)
        .filter(|ratio| !(0.0..=1.0).contains(ratio))
    {
        return Err(PipelineError::SampleRatio(ratio));
    }

    let sinks = cfg
        .logging
        .as_ref()
        .map_or(&[][..], |logging| logging.sinks.as_slice());
    let mut layers: Vec<BoxedLayer> = Vec::new();
    let mut workers = Vec::new();

    // bunyan needs one JsonStorageLayer in the registry, added once ahead of any bunyan sink.
    #[cfg(feature = "bunyan")]
    layers.extend(bunyan_storage_layer(sinks, &cfg.level));

    for sink in sinks {
        let (writer, worker) = match &sink.kind {
            LogSinkKind::Stdout => tracing_appender::non_blocking(std::io::stdout()),
            LogSinkKind::RollingFile {
                dir,
                prefix,
                rotation,
                max_files,
            } => tracing_appender::non_blocking(rolling_appender(
                dir, prefix, *rotation, *max_files,
            )?),
            // the OTLP log bridge is built with the other OTel layers below
            LogSinkKind::Otlp => continue,
        };
        let level = sink.level.as_deref().unwrap_or(&cfg.level);
        layers.push(log_layer(&sink.format, &cfg.service_name, writer, level)?);
        workers.push(worker);
    }

    #[cfg(feature = "otel")]
    let otel = otel_layers(cfg, sinks, &mut layers)?;
    #[cfg(not(feature = "otel"))]
    reject_otlp_signals(cfg, sinks)?;

    let guard = PipelineGuard {
        _workers: workers,
        #[cfg(feature = "otel")]
        otel,
    };
    Ok((layers, guard))
}

/// Without `otel`, every OTLP signal in the config needs a feature this build lacks.
#[cfg(not(feature = "otel"))]
fn reject_otlp_signals(cfg: &PipelineConfig, sinks: &[LogSink]) -> Result<(), PipelineError> {
    if cfg.tracing.is_some() {
        return Err(PipelineError::FeatureDisabled { feature: "otel" });
    }
    if sinks
        .iter()
        .any(|sink| matches!(sink.kind, LogSinkKind::Otlp))
    {
        return Err(PipelineError::FeatureDisabled {
            feature: "otel-logs",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_layers;
    use crate::{
        config::{PipelineConfig, TracingConfig},
        error::PipelineError,
    };

    fn with_ratio(ratio: f64) -> PipelineConfig {
        PipelineConfig {
            logging: None,
            tracing: Some(TracingConfig {
                sample_ratio: Some(ratio),
                ..TracingConfig::default()
            }),
            ..PipelineConfig::default()
        }
    }

    // @lat: [[tests#Tracing config#Sample ratio outside unit range is rejected]]
    #[test]
    fn sample_ratio_outside_unit_range_is_rejected() {
        for ratio in [-0.1, 1.5, f64::NAN] {
            let result = build_layers(&with_ratio(ratio));

            assert!(
                matches!(result, Err(PipelineError::SampleRatio(r)) if r.to_bits() == ratio.to_bits()),
                "{ratio}: {:?}",
                result.err()
            );
        }

        // otlp is unset, so a ratio that passes this check fails the next one
        for ratio in [0.0, 1.0] {
            let result = build_layers(&with_ratio(ratio));

            assert!(
                matches!(result, Err(ref e) if !matches!(e, PipelineError::SampleRatio(_))),
                "{ratio}: {:?}",
                result.err()
            );
        }
    }
}

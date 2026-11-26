#[cfg(feature = "bunyan")]
use tracing_bunyan_formatter::JsonStorageLayer;
use tracing_subscriber::Layer;
#[cfg(feature = "bunyan")]
use tracing_subscriber::filter::LevelFilter;

#[cfg(not(feature = "bunyan"))]
use super::pre_init_notice;
use super::{BoxedLayer, filter::env_filter};
#[cfg(feature = "bunyan")]
use crate::config::LogSink;
use crate::config::{LogFormat, Rotation};

pub(super) fn rolling_appender(
    dir: &std::path::Path,
    prefix: &str,
    rotation: &Rotation,
) -> tracing_appender::rolling::RollingFileAppender {
    let rot = match rotation {
        Rotation::Minutely => tracing_appender::rolling::Rotation::MINUTELY,
        Rotation::Hourly => tracing_appender::rolling::Rotation::HOURLY,
        Rotation::Daily => tracing_appender::rolling::Rotation::DAILY,
        Rotation::Never => tracing_appender::rolling::Rotation::NEVER,
    };
    tracing_appender::rolling::RollingFileAppender::new(rot, dir, prefix)
}

// `service` feeds the bunyan formatter's app-name field; unused when that feature is off.
#[cfg_attr(not(feature = "bunyan"), allow(unused_variables))]
pub(super) fn log_layer<W>(format: &LogFormat, service: &str, writer: W, level: &str) -> BoxedLayer
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    match format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_writer(writer)
            .with_filter(env_filter(level))
            .boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer()
            .pretty()
            .with_writer(writer)
            .with_filter(env_filter(level))
            .boxed(),
        LogFormat::Bunyan => {
            #[cfg(feature = "bunyan")]
            {
                tracing_bunyan_formatter::BunyanFormattingLayer::new(service.to_string(), writer)
                    .with_filter(env_filter(level))
                    .boxed()
            }
            #[cfg(not(feature = "bunyan"))]
            {
                pre_init_notice(format_args!(
                    "bunyan format needs `bunyan` feature, using json"
                ));
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(writer)
                    .with_filter(env_filter(level))
                    .boxed()
            }
        }
    }
}

/// The span storage every bunyan sink reads, filtered like the most verbose bunyan sink so
/// it never raises the process-wide max level above what a bunyan sink prints.
#[cfg(feature = "bunyan")]
pub(super) fn bunyan_storage_layer(sinks: &[LogSink], fallback_level: &str) -> Option<BoxedLayer> {
    sinks
        .iter()
        .filter(|sink| sink.format == LogFormat::Bunyan)
        .map(|sink| env_filter(sink.level.as_deref().unwrap_or(fallback_level)))
        // an unknown hint counts as TRACE, the most verbose
        .max_by_key(|filter| filter.max_level_hint().unwrap_or(LevelFilter::TRACE))
        .map(|filter| JsonStorageLayer.with_filter(filter).boxed())
}

#[cfg(all(test, feature = "bunyan"))]
mod tests {
    use tracing_subscriber::{Layer, filter::LevelFilter};

    use super::{super::filter::tests::with_rust_log, bunyan_storage_layer};
    use crate::config::{LogFormat, LogSink, LogSinkKind};

    // @lat: [[tests#OTLP layer filters#Bunyan storage layer hint follows most verbose bunyan sink]]
    #[test]
    fn bunyan_storage_layer_hint_follows_most_verbose_bunyan_sink() {
        let sink = |format, level: &str| LogSink {
            kind: LogSinkKind::Stdout,
            format,
            level: Some(level.to_owned()),
        };
        let sinks = [
            sink(LogFormat::Bunyan, "info"),
            sink(LogFormat::Bunyan, "debug"),
            sink(LogFormat::Json, "trace"),
        ];

        let layer = with_rust_log(None, || bunyan_storage_layer(&sinks, "warn"))
            .expect("bunyan sinks give a storage layer");

        assert_eq!(layer.max_level_hint(), Some(LevelFilter::DEBUG));
    }
}

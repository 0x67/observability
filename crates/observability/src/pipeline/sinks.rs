use std::{num::NonZeroUsize, path::Path};

use tracing_appender::rolling::{self, RollingFileAppender};
#[cfg(feature = "bunyan")]
use tracing_bunyan_formatter::JsonStorageLayer;
use tracing_subscriber::Layer;
#[cfg(feature = "bunyan")]
use tracing_subscriber::filter::LevelFilter;

use super::{BoxedLayer, filter::env_filter};
#[cfg(feature = "bunyan")]
use crate::config::LogSink;
use crate::{
    config::{LogFormat, Rotation},
    error::PipelineError,
};

/// The file appender for a rolling sink. A directory that cannot be created or opened is
/// an error, not a panic.
pub(super) fn rolling_appender(
    dir: &Path,
    prefix: &str,
    rotation: Rotation,
    max_files: Option<NonZeroUsize>,
) -> Result<RollingFileAppender, PipelineError> {
    let builder = RollingFileAppender::builder()
        .rotation(appender_rotation(rotation))
        .filename_prefix(prefix);
    let builder = match max_files {
        Some(n) => builder.max_log_files(n.get()),
        None => builder,
    };

    builder
        .build(dir)
        .map_err(|source| PipelineError::RollingFile {
            dir: dir.to_path_buf(),
            source: Box::new(source),
        })
}

fn appender_rotation(rotation: Rotation) -> rolling::Rotation {
    match rotation {
        Rotation::Minutely => rolling::Rotation::MINUTELY,
        Rotation::Hourly => rolling::Rotation::HOURLY,
        Rotation::Daily => rolling::Rotation::DAILY,
        Rotation::Never => rolling::Rotation::NEVER,
    }
}

// `service` feeds the bunyan formatter's app-name field; unused when that feature is off.
#[cfg_attr(not(feature = "bunyan"), allow(unused_variables))]
pub(super) fn log_layer<W>(
    format: &LogFormat,
    service: &str,
    writer: W,
    level: &str,
) -> Result<BoxedLayer, PipelineError>
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = match format {
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
        #[cfg(feature = "bunyan")]
        LogFormat::Bunyan => {
            tracing_bunyan_formatter::BunyanFormattingLayer::new(service.to_string(), writer)
                .with_filter(env_filter(level))
                .boxed()
        }
        #[cfg(not(feature = "bunyan"))]
        LogFormat::Bunyan => return Err(PipelineError::FeatureDisabled { feature: "bunyan" }),
    };
    Ok(layer)
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

#[cfg(test)]
mod tests {
    use std::{fs, num::NonZeroUsize};

    use super::rolling_appender;
    use crate::config::Rotation;

    // @lat: [[tests#Rolling file sink#Rolling appender prunes to max files]]
    #[test]
    fn rolling_appender_prunes_to_max_files() {
        let dir = std::env::temp_dir().join(format!("obs-prune-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create the temp log dir");
        for day in 1..=4 {
            fs::write(dir.join(format!("app.2020-01-0{day}")), "old").expect("write an old log");
        }

        let appender = rolling_appender(&dir, "app", Rotation::Daily, NonZeroUsize::new(2));

        let kept = fs::read_dir(&dir)
            .expect("read the temp log dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("app"))
            .count();
        drop(appender);
        fs::remove_dir_all(&dir).expect("remove the temp log dir");
        assert!(kept <= 2, "{kept} files kept");
    }

    // @lat: [[tests#OTLP layer filters#Bunyan storage layer hint follows most verbose bunyan sink]]
    #[cfg(feature = "bunyan")]
    #[test]
    fn bunyan_storage_layer_hint_follows_most_verbose_bunyan_sink() {
        use tracing_subscriber::{Layer, filter::LevelFilter};

        use super::{super::filter::tests::with_rust_log, bunyan_storage_layer};
        use crate::config::{LogFormat, LogSink, LogSinkKind};

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

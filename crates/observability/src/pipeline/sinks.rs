use tracing_subscriber::{Layer, Registry};

use super::filter::env_filter;
#[cfg(not(feature = "bunyan"))]
use super::pre_init_notice;
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
pub(super) fn log_layer<W>(
    format: &LogFormat,
    service: &str,
    writer: W,
    level: &str,
) -> Box<dyn Layer<Registry> + Send + Sync>
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

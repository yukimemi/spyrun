// =============================================================================
// File        : logger.rs
// Author      : yukimemi
// Last Change : 2023/09/17 21:48:12.
// =============================================================================

use std::{collections::HashMap, fs::create_dir_all, path::Path};

use anyhow::Result;
use text_placeholder::Template;
use tracing_appender::non_blocking;
use tracing_log::LogTracer;
use tracing_subscriber::{
    fmt::{time::LocalTime, writer::BoxMakeWriter, Layer},
    prelude::*,
    EnvFilter, Registry,
};

use super::settings::Settings;

pub fn init(
    settings: Settings,
    rep_map: &HashMap<String, String>,
) -> Result<(
    tracing_appender::non_blocking::WorkerGuard,
    tracing_appender::non_blocking::WorkerGuard,
)> {
    LogTracer::init()?;

    let hashmap: HashMap<&str, &str> = rep_map
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let log_tpl = Template::new(&settings.log.path);
    let log_path = log_tpl.fill_with_hashmap(&hashmap);
    let log_dir = Path::new(&log_path).parent().unwrap();
    let log_name = Path::new(&log_path).file_name().unwrap();

    create_dir_all(log_dir)?;

    let time_format = time::format_description::well_known::Iso8601::DEFAULT;
    let timer = LocalTime::new(time_format);

    let file_appender = non_blocking(tracing_appender::rolling::daily(log_dir, log_name));
    let stdout_appender = non_blocking(std::io::stdout());

    let file_writer = BoxMakeWriter::new(file_appender.0);
    let stdout_writer = BoxMakeWriter::new(stdout_appender.0);

    let file_layer = Layer::default()
        .with_writer(file_writer)
        .with_timer(timer.clone())
        .json()
        .with_filter(EnvFilter::from_default_env())
        .boxed();
    let stdout_layer = Layer::default()
        .with_writer(stdout_writer)
        .with_timer(timer.clone())
        .pretty()
        .with_file(false)
        .with_filter(EnvFilter::from_default_env())
        .boxed();

    let registry = Registry::default().with(file_layer).with(stdout_layer);

    tracing::subscriber::set_global_default(registry)?;

    Ok((file_appender.1, stdout_appender.1))
}

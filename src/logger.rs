// =============================================================================
// File        : logger.rs
// Author      : yukimemi
// Last Change : 2025/03/09 00:32:25.
// =============================================================================

use std::{
    env, fs,
    fs::create_dir_all,
    path::{Path, PathBuf},
};

use anyhow::Result;
use chrono::Local;
use tera::Context;
use tracing_appender::non_blocking;
use tracing_log::LogTracer;
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{Layer, time::ChronoLocal, writer::BoxMakeWriter},
    prelude::*,
};

use super::{settings::Settings, util::insert_file_context};

pub fn init(
    settings: Settings,
    context: &mut Context,
) -> Result<(
    tracing_appender::non_blocking::WorkerGuard,
    tracing_appender::non_blocking::WorkerGuard,
)> {
    LogTracer::init()?;

    insert_file_context(&settings.log.path, "log", context)?;

    let log_dir = context.get("log_dir").unwrap().as_str().unwrap();
    let log_name = context.get("log_name").unwrap().as_str().unwrap();
    create_dir_all(log_dir)?;

    if settings.log.switch {
        let old_log_path = Path::join(
            &PathBuf::from(log_dir),
            format!(
                "{}.{}",
                &log_name,
                &Local::now().format("%Y-%m-%d").to_string()
            ),
        );
        let rename_log_path = Path::join(
            &PathBuf::from(log_dir),
            format!(
                "{}_{}.{}",
                context.get("log_stem").unwrap().as_str().unwrap(),
                context.get("now").unwrap().as_str().unwrap(),
                context.get("log_ext").unwrap().as_str().unwrap()
            ),
        );

        if old_log_path.is_file() {
            fs::rename(old_log_path, rename_log_path)?;
        }
    }

    let timer = ChronoLocal::new("%+".to_string());
    let file_appender = non_blocking(tracing_appender::rolling::daily(log_dir, log_name));
    let stdout_appender = non_blocking(std::io::stdout());

    let file_writer = BoxMakeWriter::new(file_appender.0);
    let stdout_writer = BoxMakeWriter::new(stdout_appender.0);

    let file_layer = Layer::default()
        .with_writer(file_writer)
        .with_timer(timer.clone())
        // .json()
        .with_ansi(false)
        .with_filter(EnvFilter::new(
            env::var("SPYRUN_LOG_FILE").unwrap_or(settings.log.level),
        ))
        .boxed();
    let stdout_layer = Layer::default()
        .with_writer(stdout_writer)
        .with_timer(timer.clone())
        .pretty()
        .with_file(false)
        .with_filter(EnvFilter::new(
            env::var("SPYRUN_LOG_STDOUT").unwrap_or_else(|_| "info".to_string()),
        ))
        .boxed();

    let registry = Registry::default().with(file_layer).with(stdout_layer);
    tracing::subscriber::set_global_default(registry)?;

    Ok((file_appender.1, stdout_appender.1))
}

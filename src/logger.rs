use super::settings::Settings;
use anyhow::Result;
use std::collections::HashMap;
use std::fs::create_dir_all;
use std::path::Path;
use text_placeholder::Template;

use tracing_subscriber::fmt::time::LocalTime;

const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub fn init(
    settings: Settings,
    rep_map: &HashMap<String, String>,
) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let hashmap: HashMap<&str, &str> = rep_map
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let log_tpl = Template::new(&settings.log.path);
    let log_path = log_tpl.fill_with_hashmap(&hashmap);
    let log_dir = Path::new(&log_path).parent().unwrap();
    let log_name = Path::new(&log_path).file_name().unwrap();

    create_dir_all(log_dir)?;

    // let time_format = time::format_description::parse(&settings.log.time_format)
    let time_format =
        time::format_description::parse(TIME_FORMAT).expect("format string should be valid !");
    let timer = LocalTime::new(time_format);

    let file_appender = tracing_appender::rolling::hourly(log_dir, log_name);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let subscriber = tracing_subscriber::fmt()
        .with_timer(timer)
        .with_writer(non_blocking)
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    Ok(guard)
}


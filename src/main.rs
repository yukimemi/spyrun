// =============================================================================
// File        : main.rs
// Author      : yukimemi
// Last Change : 2023/09/16 21:38:30.
// =============================================================================

// #![windows_subsystem = "windows"]

mod logger;
mod settings;

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use notify::{EventKind, RecursiveMode, Watcher};
use settings::Settings;
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use tracing::info;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE", default_value = "spyrun.toml")]
    config: PathBuf,

    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,
}

fn build_cmd_map() -> Result<HashMap<String, String>> {
    let cmd_file = env::current_exe()?;
    dbg!(&cmd_file);

    let mut m: HashMap<String, String> = HashMap::new();

    let cmd = cmd_file.to_string_lossy().to_string();
    m.insert("cmd_file".to_string(), cmd);
    let cmd_dir = cmd_file.parent().unwrap().to_string_lossy().to_string();
    m.insert("cmd_dir".to_string(), cmd_dir);
    let cmd_name = cmd_file.file_name().unwrap().to_string_lossy().to_string();
    m.insert("cmd_name".to_string(), cmd_name);
    let cmd_stem = cmd_file.file_stem().unwrap().to_string_lossy().to_string();
    m.insert("cmd_stem".to_string(), cmd_stem);
    let now = Local::now().format("%Y%m%d%H%M%S%3f").to_string();
    m.insert("now".to_string(), now);
    let cwd = env::current_dir()?.to_string_lossy().to_string();
    m.insert("cwd".to_string(), cwd);

    Ok(m)
}

fn main() -> Result<()> {
    let m = build_cmd_map()?;
    dbg!(&m);

    let cli = Cli::parse();
    dbg!(&cli);

    let settings = Settings::new(cli.config)?;
    dbg!(&settings);

    let (guard1, guard2) = logger::init(settings.clone(), &m)?;

    info!("start !");

    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(tx)?;

    watcher.watch(Path::new("."), RecursiveMode::Recursive)?;

    thread::spawn(move || {
        for res in rx.into_iter() {
            match res {
                Ok(event) => match event.kind {
                    EventKind::Create(_) => {
                        info!("A file was created: {:?}", event.paths);
                    }
                    EventKind::Remove(_) => {
                        info!("A file was removed: {:?}", event.paths);
                    }
                    EventKind::Modify(_) => {
                        info!("A file was modified: {:?}", event.paths);
                    }
                    EventKind::Access(_) => {
                        info!("A file was accessed: {:?}", event.paths);
                    }
                    EventKind::Other => {
                        info!("Other event: {:?}", event);
                    }
                    EventKind::Any => {
                        info!("Unknown or unsupported event: {:?}", event);
                    }
                },
                Err(e) => info!("watch error: {:?}", e),
            }
        }
        info!("channel closed");
    });

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();

    drop(guard1);
    drop(guard2);

    Ok(())
}

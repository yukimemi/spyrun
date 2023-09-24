// =============================================================================
// File        : main.rs
// Author      : yukimemi
// Last Change : 2023/09/24 02:25:22.
// =============================================================================

// #![windows_subsystem = "windows"]

mod logger;
mod settings;

use std::{
    collections::HashMap,
    env,
    fs::canonicalize,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use go_defer::defer;
use notify::{EventKind, RecursiveMode, Watcher};
use rayon::prelude::*;
use settings::{Settings, Spy};
use tracing::{error, info};

enum Message {
    Event(notify::Event),
    Stop,
}

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

fn watcher(
    spy: Spy,
) -> Result<(
    std::thread::JoinHandle<()>,
    mpsc::Sender<Message>,
    notify::RecommendedWatcher,
)> {
    let (tx, rx) = mpsc::channel();
    let tx_clone = tx.clone();
    let mut watcher = notify::recommended_watcher(move |res| match res {
        Ok(event) => {
            tx_clone.send(Message::Event(event)).unwrap();
        }
        Err(e) => {
            error!("watch error: {:?}", e);
        }
    })?;
    let input = spy.input.expect("spy.input is None");
    let input = canonicalize(input)?;
    info!("watching {}", &input.display());
    watcher.watch(Path::new(&input), RecursiveMode::Recursive)?;

    let handle = thread::spawn(move || {
        for msg in rx.into_iter() {
            match msg {
                Message::Event(event) => match event.kind {
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
                    EventKind::Other | EventKind::Any => {
                        info!("Other or Any event: {:?}", event);
                    }
                },
                Message::Stop => {
                    info!("watch stop");
                    break;
                }
            }
        }
        info!("channel closed");
    });

    Ok((handle, tx, watcher))
}

fn main() -> Result<()> {
    let m = build_cmd_map()?;
    dbg!(&m);

    let cli = Cli::parse();
    dbg!(&cli);

    let settings = Settings::new(cli.config)?.rebuild();
    dbg!(&settings);

    let (guard1, guard2) = logger::init(settings.clone(), &m)?;
    defer!({
        drop(guard1);
        drop(guard2);
    });

    info!("start !");

    let results = settings
        .spys
        .iter()
        .map(|spy| watcher(spy.clone()))
        .collect::<Result<Vec<_>>>()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();

    results.into_par_iter().for_each(|result| {
        let (handle, tx, _) = result;
        tx.send(Message::Stop).unwrap();
        match handle.join() {
            Ok(_) => {
                info!("watch thread joined");
            }
            Err(e) => {
                error!("watch thread error: {:?}", e);
            }
        }
    });

    Ok(())
}

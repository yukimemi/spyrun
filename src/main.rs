// =============================================================================
// File        : main.rs
// Author      : yukimemi
// Last Change : 2023/09/24 22:48:53.
// =============================================================================

// #![windows_subsystem = "windows"]

mod logger;
mod settings;

use std::{
    collections::HashMap,
    env,
    fs::{canonicalize, create_dir_all, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::mpsc,
    thread,
};

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use go_defer::defer;
use log_derive::logfn;
use notify::{EventKind, RecursiveMode, Watcher};
use rayon::prelude::*;
use settings::{Pattern, Settings, Spy};
use tracing::{debug, error, info};

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

#[tracing::instrument]
#[logfn(Info)]
fn build_cmd_map() -> Result<HashMap<String, String>> {
    let cmd_file = env::current_exe()?;
    debug!("{:?}", &cmd_file);

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

#[tracing::instrument]
#[logfn(Info)]
fn event_kind_to_string(kind: EventKind) -> String {
    match kind {
        EventKind::Create(_) => "Create".to_string(),
        EventKind::Remove(_) => "Remove".to_string(),
        EventKind::Modify(_) => "Modify".to_string(),
        EventKind::Access(_) => "Access".to_string(),
        _ => "Other".to_string(),
    }
}

#[tracing::instrument]
#[logfn(Info)]
fn find_pattern(event: &notify::Event, spy: &Spy) -> Option<Pattern> {
    let event_kind = event_kind_to_string(event.kind);
    let event_path = event.paths.last().unwrap();
    let event_ext = event_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let event_match = spy
        .events
        .as_ref()
        .unwrap()
        .iter()
        .any(|e| e == &event_kind);
    let match_pattern = spy
        .patterns
        .as_ref()
        .unwrap()
        .iter()
        .find(|p| p.extension == "*" || p.extension == event_ext);
    if event_match {
        match_pattern.cloned()
    } else {
        None
    }
}

#[tracing::instrument]
#[logfn(Info)]
fn execute_command(event: &notify::Event, spy: &Spy, pattern: &Pattern) -> Result<ExitStatus> {
    create_dir_all(spy.output.as_ref().unwrap())?;
    let now = Local::now().format("%Y%m%d_%H%M%S%3f").to_string();
    let stdout_path = PathBuf::from(&spy.output.as_ref().unwrap())
        .join(format!("{}_stdout_{}.log", &spy.name, now));
    create_dir_all(spy.output.as_ref().unwrap())?;
    let stdout_file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&stdout_path)?;
    let stderr_path = PathBuf::from(&spy.output.as_ref().unwrap())
        .join(format!("{}_stderr_{}.log", &spy.name, now));
    let stderr_file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&stderr_path)?;
    let event_path = event.paths.last().unwrap();
    let cmd = &pattern.cmd;
    let arg = &pattern
        .arg
        .iter()
        .map(|s| {
            if s.contains("{{input}}") {
                s.replace("{{input}}", event_path.to_string_lossy().as_ref())
                    .to_string()
            } else {
                s.to_string()
            }
        })
        .collect::<Vec<_>>();
    info!(
        "cmd: {}, arg: {}, stdout: {}, stderr: {}",
        cmd,
        arg.join(" "),
        stdout_path.display(),
        stderr_path.display()
    );
    Ok(Command::new(cmd)
        .args(arg)
        .stdout(stdout_file)
        .stderr(stderr_file)
        .spawn()?
        .wait()?)
}

#[tracing::instrument]
#[logfn(Info)]
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
    let input = spy.clone().input.expect("spy.input is None");
    let input = canonicalize(input)?;
    info!("watching {}", &input.display());
    watcher.watch(Path::new(&input), RecursiveMode::Recursive)?;

    let handle = thread::spawn(move || {
        for msg in rx.into_iter() {
            match msg {
                Message::Event(event) => {
                    match event.kind {
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
                    }
                    if let Some(pattern) = find_pattern(&event, &spy) {
                        info!("pattern: {:?}", pattern);
                        let status = execute_command(&event, &spy, &pattern);
                        match status {
                            Ok(s) => info!("Success status: {:?}", s),
                            Err(e) => error!("Error status: {:?}", e),
                        }
                    }
                }
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

#[tracing::instrument]
#[logfn(Info)]
fn main() -> Result<()> {
    let m = build_cmd_map()?;
    debug!("{:?}", &m);

    let cli = Cli::parse();
    debug!("{:?}", &cli);

    let settings = Settings::new(cli.config)?.rebuild();
    debug!("{:?}", &settings);

    let (guard1, guard2) = logger::init(settings.clone(), &m)?;
    defer!({
        drop(guard1);
        drop(guard2);
    });

    debug!("start !");

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

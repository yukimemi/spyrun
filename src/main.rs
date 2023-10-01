// =============================================================================
// File        : main.rs
// Author      : yukimemi
// Last Change : 2023/10/01 17:16:43.
// =============================================================================

// #![windows_subsystem = "windows"]

mod logger;
mod settings;
mod util;

use std::{
    collections::HashMap,
    env,
    fs::{create_dir_all, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::{bail, Result};
use chrono::Local;
use clap::Parser;
use crypto_hash::{hex_digest, Algorithm};
use go_defer::defer;
use log_derive::logfn;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use rayon::prelude::*;
use regex::Regex;
use settings::{Pattern, Settings, Spy};
use single_instance::SingleInstance;
use tera::{Context, Tera, Value};
use tracing::{debug, error, info, warn};
use util::insert_file_context;

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
#[logfn(Debug)]
fn build_cmd_map() -> Result<Context> {
    let cmd_file = env::current_exe()?;
    debug!("{:?}", &cmd_file);

    let mut context = Context::new();

    context.insert("cmd_line", &env::args().collect::<Vec<String>>().join(" "));
    context.insert("now", &Local::now().format("%Y%m%d%H%M%S%3f").to_string());
    context.insert("cwd", &env::current_dir()?.to_string_lossy());

    insert_file_context(&cmd_file, "cmd", &mut context)?;

    Ok(context)
}

#[tracing::instrument]
#[logfn(Debug)]
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
#[logfn(Debug)]
fn find_pattern(event: &notify::Event, spy: &Spy) -> Option<Pattern> {
    let event_kind = event_kind_to_string(event.kind);
    let event_path = event.paths.last().unwrap();
    info!(
        "event_kind: {}, event_path: {}",
        &event_kind,
        &event_path.to_string_lossy()
    );
    let event_match = spy
        .events
        .as_ref()
        .unwrap()
        .iter()
        .any(|e| e == &event_kind);
    let match_pattern = spy.patterns.as_ref().unwrap().iter().find(|p| {
        let re = Regex::new(&p.pattern).unwrap();
        re.is_match(&event_path.to_string_lossy())
    });
    if event_match {
        match_pattern.cloned()
    } else {
        None
    }
}

#[tracing::instrument]
#[logfn(Debug)]
fn execute_command(
    event_path: &PathBuf,
    name: &str,
    input: &str,
    output: &str,
    cmd: &str,
    arg: Vec<String>,
    context: Context,
) -> Result<ExitStatus> {
    let mut context = context.clone();
    create_dir_all(output)?;
    let now = Local::now().format("%Y%m%d_%H%M%S%3f").to_string();
    let stdout_path = PathBuf::from(&output).join(format!("{}_stdout_{}.log", &name, now));
    let stdout_file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&stdout_path)?;
    let stderr_path = PathBuf::from(&output).join(format!("{}_stderr_{}.log", &name, now));
    let stderr_file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&stderr_path)?;
    let arg = &arg
        .iter()
        .map(|s| {
            let mut tera = Tera::default();
            let event_path = event_path.to_string_lossy();
            tera.add_raw_template("arg", s).unwrap();
            tera.register_function("env", |args: &HashMap<String, Value>| {
                let name = match args.get("name") {
                    Some(val) => val.as_str().unwrap(),
                    None => return Err("name is required".into()),
                };
                Ok(Value::String(env::var(name).unwrap_or_default()))
            });
            context.insert("input", &input);
            context.insert("output", &output);
            context.insert("event_path", &event_path);
            tera.render("arg", &context).unwrap()
        })
        .collect::<Vec<_>>();
    info!(
        "Execute cmd: {}, arg: {}, stdout: {}, stderr: {}",
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
#[logfn(Debug)]
fn watcher(
    spy: Spy,
    context: Context,
) -> Result<(
    std::thread::JoinHandle<()>,
    mpsc::Sender<Message>,
    Debouncer<RecommendedWatcher, FileIdMap>,
)> {
    let (tx, rx) = mpsc::channel();
    let tx_clone = tx.clone();
    let mut debouncer = new_debouncer(
        Duration::from_secs(1),
        None,
        move |res: DebounceEventResult| match res {
            Ok(events) => events.into_iter().for_each(|event| {
                tx_clone.send(Message::Event(event.event)).unwrap();
            }),
            Err(e) => {
                error!("watch error: {:?}", e);
            }
        },
    )?;
    let input = spy.clone().input.expect("spy.input is None");
    info!("watching {}", &input);
    debouncer
        .watcher()
        .watch(Path::new(&input), RecursiveMode::Recursive)?;

    let (tx2, rx2) = mpsc::channel();
    let handle = thread::spawn(move || {
        let handle2 = thread::spawn(|| {
            rx2.into_iter().for_each(|status| {
                debug!("rx2 received: {:?}", status);
                match status {
                    Ok(s) => info!("Command success status: {:?}", s),
                    Err(e) => error!("Command error status: {:?}", e),
                }
            });
        });
        rayon::scope(|s| {
            for msg in rx {
                match msg {
                    Message::Event(event) => {
                        if let Some(pattern) = find_pattern(&event, &spy) {
                            let tx2 = tx2.clone();
                            let spy = spy.clone();
                            let event = event.clone();
                            let context = context.clone();
                            info!("pattern: {:?}", pattern);
                            s.spawn(move |_| {
                                let status = execute_command(
                                    event.paths.last().unwrap(),
                                    &spy.name,
                                    &spy.input.unwrap(),
                                    &spy.output.unwrap(),
                                    &pattern.cmd,
                                    pattern.arg,
                                    context,
                                );
                                tx2.send(status).unwrap();
                            });
                        }
                    }
                    Message::Stop => {
                        info!("watch stop !");
                        break;
                    }
                }
            }
            info!("channel closed");
        });
        drop(tx2);
        handle2.join().unwrap();
    });

    Ok((handle, tx, debouncer))
}

#[tracing::instrument]
#[logfn(Debug)]
fn main() -> Result<()> {
    let mut context = build_cmd_map()?;
    debug!("{:?}", &context);

    let cli = Cli::parse();
    debug!("{:?}", &cli);

    let settings = Settings::new(cli.config, &mut context)?.rebuild();
    debug!("{:?}", &settings);

    let (guard1, guard2) = logger::init(settings.clone(), &mut context)?;
    defer!({
        drop(guard1);
        drop(guard2);
    });

    let cmd_line = context.get("cmd_line").unwrap().as_str().unwrap();
    debug!("cmd_line: {}", &cmd_line);
    let hash = hex_digest(Algorithm::SHA256, cmd_line.as_bytes());
    #[cfg(not(target_os = "windows"))]
    let hash = env::temp_dir().join(hash);
    #[cfg(not(target_os = "windows"))]
    let hash = hash.to_string_lossy();

    debug!("hash: {}", &hash);
    let instance = SingleInstance::new(&hash)?;
    if !instance.is_single() {
        let warn_msg = format!("Another instance is already running. [{}]", &cmd_line);
        warn!("{}", &warn_msg);
        bail!(warn_msg);
    }

    debug!("start !");

    let results = settings
        .spys
        .iter()
        .map(|spy| watcher(spy.clone(), context.clone()))
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

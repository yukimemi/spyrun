// =============================================================================
// File        : main.rs
// Author      : yukimemi
// Last Change : 2023/10/10 21:32:11.
// =============================================================================

// #![windows_subsystem = "windows"]

mod logger;
mod message;
mod settings;
mod spy;
mod util;

use std::{
    env,
    fs::{create_dir_all, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::mpsc,
    thread,
};

use anyhow::{bail, Result};
use chrono::Local;
use clap::Parser;
use crypto_hash::{hex_digest, Algorithm};
use go_defer::defer;
use log_derive::logfn;
use message::Message;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use path_slash::PathBufExt as _;
use rayon::prelude::*;
use regex::Regex;
use settings::{Pattern, Settings, Spy};
use single_instance::SingleInstance;
use tera::Context;
use tracing::{debug, error, info, warn};
use util::{insert_file_context, new_tera};

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
    context.insert("cwd", &env::current_dir()?.to_slash_lossy());

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
    let now = Local::now().format("%Y%m%d_%H%M%S%3f").to_string();
    insert_file_context(event_path, "event", &mut context).unwrap();
    let arg = &arg
        .iter()
        .map(|s| {
            let tera = new_tera("arg", s).unwrap();
            tera.render("arg", &context).unwrap()
        })
        .collect::<Vec<_>>();
    let tera = new_tera("input", input)?;
    let input = tera.render("input", &context)?;
    context.insert("input", &input);
    let tera = new_tera("output", output)?;
    let output = tera.render("output", &context)?;
    context.insert("output", &output);
    create_dir_all(&output)?;
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
) -> Result<(std::thread::JoinHandle<()>, mpsc::Sender<Message>)> {
    let (tx, rx) = mpsc::channel();
    let (tx_execute, rx_execute) = mpsc::channel();
    let tx_clone = tx.clone();
    let handle = thread::spawn(move || {
        if let Some(ref _walk) = spy.walk {
            let handle = spy.walk(tx_clone.clone()).unwrap();
            handle.join().unwrap();
        }
        let _watcher = spy.watch(tx_clone);
        let handle_execute_wait = thread::spawn(|| {
            rx_execute.into_iter().for_each(|status| {
                debug!("rx_execute received: {:?}", status);
                match status {
                    Ok(s) => info!("Command success status: {:?}", s),
                    Err(e) => error!("Command error status: {:?}", e),
                }
            });
        });
        for msg in rx {
            match msg {
                Message::Event(event) => {
                    if let Some(pattern) = find_pattern(&event, &spy) {
                        let tx_exec_clone = tx_execute.clone();
                        let spy = spy.clone();
                        let event = event.clone();
                        let context = context.clone();
                        info!("pattern: {:?}", pattern);
                        rayon::spawn(move || {
                            let status = execute_command(
                                event.paths.last().unwrap(),
                                &spy.name,
                                &spy.input.unwrap(),
                                &spy.output.unwrap(),
                                &pattern.cmd,
                                pattern.arg,
                                context,
                            );
                            tx_exec_clone.send(status).unwrap();
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
        drop(tx_execute);
        handle_execute_wait.join().unwrap();
    });

    Ok((handle, tx))
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
    info!("==================== start ! ====================");
    defer!({
        info!("==================== end ! ====================");
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

    let (tx_stop, rx_stop) = mpsc::channel();
    let stop_flg = if Path::new(&settings.cfg.stop_flg).is_relative() {
        Path::join(env::current_dir()?.as_path(), &settings.cfg.stop_flg)
    } else {
        Path::new(&settings.cfg.stop_flg).to_path_buf()
    };
    insert_file_context(&stop_flg, "stop", &mut context)?;

    if let Some(init) = &settings.init {
        let status = execute_command(
            &(env::current_exe()?),
            "init",
            "input",
            "output",
            &init.cmd,
            init.arg.clone(),
            context.clone(),
        );
        match status {
            Ok(s) => info!("Init command success status: {:?}", s),
            Err(e) => {
                error!("Init command error status: {:?}", e);
                if init.error_stop {
                    bail!(e);
                }
            }
        }
    }

    let results = settings
        .spys
        .iter()
        .map(|spy| {
            watcher(spy.clone(), context.clone())
                .map_err(|e| error!("watcher error: {:?}", e))
                .ok()
        })
        .collect::<Vec<_>>();

    let mut stop_watcher =
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
            Ok(event) => {
                let event_str = event_kind_to_string(event.kind);
                if vec!["Create", "Modify"].into_iter().any(|e| e == event_str)
                    && event.paths.last().unwrap() == Path::new(&stop_flg)
                {
                    tx_stop.send("stop").unwrap();
                }
            }
            Err(e) => error!("stop watch error: {:?}", e),
        })?;
    stop_watcher.watch(
        Path::new(&settings.cfg.stop_flg).parent().unwrap(),
        RecursiveMode::NonRecursive,
    )?;
    info!("watching stop flg {}", &settings.cfg.stop_flg);
    loop {
        match rx_stop.recv() {
            Ok("stop") => break,
            Err(e) => error!("stop watch error: {:?}", e),
            _ => unreachable!(),
        }
    }

    results.into_par_iter().for_each(|result| {
        if let Some((handle, tx)) = result {
            tx.send(Message::Stop).unwrap();
            match handle.join() {
                Ok(_) => {
                    info!("watch thread joined");
                }
                Err(e) => {
                    error!("watch thread error: {:?}", e);
                }
            }
        }
    });

    Ok(())
}

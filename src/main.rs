// =============================================================================
// File        : main.rs
// Author      : yukimemi
// Last Change : 2024/06/22 20:58:32.
// =============================================================================

// #![windows_subsystem = "windows"]

mod command;
mod logger;
mod message;
mod settings;
mod spy;
mod util;

use std::{
    collections::HashMap,
    env,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{bail, Result};
use chrono::Local;
use clap::Parser;
use command::execute_command;
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
use tracing::{debug, error, info, trace, warn};
use util::insert_file_context;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE", default_value = "spyrun.toml")]
    config: PathBuf,
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
#[logfn(Trace)]
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
#[logfn(Trace)]
fn find_pattern(event: &notify::Event, spy: &Spy) -> Option<Pattern> {
    let event_kind = event_kind_to_string(event.kind);
    let event_path = event.paths.last().unwrap();
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
        trace!(
            "event_kind: {}, event_path: {}",
            &event_kind,
            &event_path.to_string_lossy()
        );
        match_pattern.cloned()
    } else {
        None
    }
}

#[tracing::instrument]
#[logfn(Debug)]
fn watcher(
    spy: Spy,
    context: Context,
) -> Result<(std::thread::JoinHandle<String>, mpsc::Sender<Message>)> {
    let (tx, rx) = mpsc::channel();
    let (tx_execute, rx_execute) = mpsc::channel();
    let tx_clone = tx.clone();
    info!("[watcher] watch start: {}", &spy.name);
    let handle = thread::spawn(move || -> String {
        if let Some(ref _walk) = spy.walk {
            let handle = spy.walk(tx_clone.clone()).unwrap();
            handle.join().unwrap();
        }
        let _watcher = spy.watch(tx_clone);
        let spy_clone = spy.clone();
        let handle_execute_wait = thread::spawn(move || {
            rx_execute.into_iter().for_each(|status| {
                debug!("[{}] rx_execute received: {:?}", &spy_clone.name, status);
                match status {
                    Ok(s) => debug!("[{}] Command success status: {:?}", &spy_clone.name, s),
                    Err(e) => error!("[{}] Command error status: {:?}", &spy_clone.name, e),
                }
            });
        });
        let cache = HashMap::new();
        let cache = Arc::new(Mutex::new(cache));
        for msg in rx {
            match msg {
                Message::Event(event) => {
                    if let Some(pattern) = find_pattern(&event, &spy) {
                        let event_kind = event_kind_to_string(event.kind);
                        let tx_exec_clone = tx_execute.clone();
                        let spy = spy.clone();
                        let event = event.clone();
                        let cache = cache.clone();
                        let mut context = context.clone();
                        context.insert("event_kind", &event_kind);
                        debug!("[{}] pattern: {:?}", &spy.name, pattern);
                        rayon::spawn(move || {
                            let status = execute_command(
                                event.paths.last().unwrap(),
                                &spy.name,
                                &spy.input.unwrap(),
                                &spy.output.unwrap(),
                                &pattern.cmd,
                                pattern.arg,
                                Duration::from_millis(spy.debounce.unwrap()),
                                Duration::from_millis(spy.throttle.unwrap()),
                                context,
                                &cache,
                            );
                            tx_exec_clone.send(status).unwrap();
                        });
                    }
                }
                Message::Stop => {
                    info!("[{}] watch stop !", &spy.name);
                    break;
                }
            }
        }
        info!("[{}] channel closed", &spy.name);
        drop(tx_execute);
        handle_execute_wait.join().unwrap();
        spy.name
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

    let error_log_path =
        Path::new(context.get("cmd_dir").unwrap().as_str().unwrap()).join("error.log");

    let mut load_error = String::new();
    let settings = Settings::new(&cli.config, true, &mut context);
    let settings = match settings {
        Ok(s) => s.rebuild(),
        Err(e) => {
            load_error = format!("Failed to load toml. so use backup file. {}", e);
            let mut error_file = File::create(error_log_path)?;
            writeln!(error_file, "{}", load_error)?;
            error_file.flush()?;
            let backup_cfg_path = Settings::backup_path(&cli.config);
            Settings::new(backup_cfg_path, false, &mut context)?.rebuild()
        }
    };

    debug!("{:?}", &settings);

    if let Some(max_threads) = &settings.cfg.max_threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(*max_threads)
            .build_global()?;
    }

    let (guard1, guard2) = logger::init(settings.clone(), &mut context)?;
    info!("==================== start ! ====================");
    if !load_error.is_empty() {
        error!(load_error);
    }
    defer!({
        info!("==================== end ! ====================");
        drop(guard1);
        drop(guard2);
    });

    let cmd_line = context.get("cmd_line").unwrap().as_str().unwrap();
    debug!("cmd_line: {}", &cmd_line);
    let toml_str = std::fs::read_to_string(&cli.config)?;
    let hash = hex_digest(Algorithm::SHA256, toml_str.as_bytes());
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
    let stop_force_flg = if let Some(s) = &settings.cfg.stop_force_flg {
        if Path::new(s).is_relative() {
            Path::join(env::current_dir()?.as_path(), s)
        } else {
            Path::new(s).to_path_buf()
        }
    } else {
        Path::join(
            stop_flg.parent().unwrap(),
            format!(
                "{}_force.{}",
                stop_flg.file_stem().unwrap().to_string_lossy(),
                stop_flg.extension().unwrap().to_string_lossy()
            ),
        )
    };
    insert_file_context(&stop_force_flg, "stop_force", &mut context)?;

    let tx_stop_clone = tx_stop.clone();
    let stop_flg_clone = stop_flg.clone();
    let mut stop_watcher =
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
            Ok(event) => {
                let event_str = event_kind_to_string(event.kind);
                if vec!["Create", "Modify"].into_iter().any(|e| e == event_str)
                    && event.paths.last().unwrap() == Path::new(&stop_flg_clone)
                {
                    tx_stop_clone.send("stop".to_string()).unwrap();
                }
            }
            Err(e) => error!("stop watch error: {:?}", e),
        })?;
    stop_watcher.watch(
        stop_flg.clone().parent().unwrap(),
        RecursiveMode::NonRecursive,
    )?;
    info!("watching stop flg {}", &settings.cfg.stop_flg);

    let tx_stop_force_clone = tx_stop.clone();
    let stop_force_flg_clone = stop_force_flg.clone();
    let mut stop_force_watcher =
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
            Ok(event) => {
                let event_str = event_kind_to_string(event.kind);
                if vec!["Create", "Modify"].into_iter().any(|e| e == event_str)
                    && event.paths.last().unwrap() == Path::new(&stop_force_flg_clone)
                {
                    tx_stop_force_clone.send("stop_force".to_string()).unwrap();
                }
            }
            Err(e) => error!("stop force watch error: {:?}", e),
        })?;
    stop_force_watcher.watch(
        stop_force_flg.parent().unwrap(),
        RecursiveMode::NonRecursive,
    )?;
    info!(
        "watching stop force flg {}",
        &stop_force_flg.to_string_lossy()
    );

    if let Some(init) = &settings.init {
        let status = execute_command(
            &(env::current_exe()?),
            "init",
            "input",
            context.get("log_dir").unwrap().as_str().unwrap(),
            &init.cmd,
            init.arg.clone(),
            Duration::from_secs(0),
            Duration::from_secs(1),
            context.clone(),
            &Arc::new(Mutex::new(HashMap::new())),
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

    // Wait stop...
    loop {
        match rx_stop.recv() {
            Ok(s) if s == "stop" => {
                info!("Received stop");
                break;
            }
            Ok(s) if s == "stop_force" => {
                info!("Received stop_force");
                info!("==================== end ! ====================");
                std::process::exit(1);
            }
            Err(e) => error!("stop watch error: {:?}", e),
            _ => unreachable!(),
        }
    }

    // Recv stop_force
    thread::spawn(move || match rx_stop.recv() {
        Ok(s) if s == "stop" || s == "stop_force" => {
            info!("Received stop or stop_force");
            info!("==================== end ! ====================");
            std::process::exit(1);
        }
        Err(e) => error!("stop watch error: {:?}", e),
        _ => unreachable!(),
    });

    results.into_par_iter().for_each(|result| {
        if let Some((handle, tx)) = result {
            tx.send(Message::Stop).unwrap();
            match handle.join() {
                Ok(name) => {
                    info!("[{}] watch thread joined", name);
                }
                Err(e) => {
                    error!("watch thread error: {:?}", e);
                }
            }
        }
    });

    Ok(())
}

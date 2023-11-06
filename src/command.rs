// =============================================================================
// File        : command.rs
// Author      : yukimemi
// Last Change : 2023/11/06 14:17:39.
// =============================================================================

use std::{
    collections::HashMap,
    fs::{create_dir_all, OpenOptions},
    path::PathBuf,
    process::{Command, ExitStatus},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::Local;
use log_derive::logfn;
use tera::Context;
use tracing::{info, warn};

use crate::util::{insert_file_context, new_tera};

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct CommandKey {
    name: String,
    event_path: PathBuf,
    cmd: String,
    arg: Vec<String>,
    input: String,
    output: String,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CommandResult {
    status: ExitStatus,
    stdout: PathBuf,
    stderr: PathBuf,
    skipped: bool,
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn render_command(key: CommandKey, context: Context) -> Result<CommandKey> {
    let mut context = context.clone();
    insert_file_context(&key.event_path, "event", &mut context).unwrap();
    let tera = new_tera("cmd", &key.cmd)?;
    let cmd = tera.render("cmd", &context)?;
    context.insert("cmd", &cmd);
    let arg = &key
        .arg
        .iter()
        .map(|s| {
            let tera = new_tera("arg", s).unwrap();
            tera.render("arg", &context).unwrap()
        })
        .collect::<Vec<_>>();
    context.insert("arg", &arg.join(" "));
    let tera = new_tera("input", &key.input)?;
    let input = tera.render("input", &context)?;
    context.insert("input", &input);
    let tera = new_tera("output", &key.output)?;
    let output = tera.render("output", &context)?;
    context.insert("output", &output);
    create_dir_all(&output)?;

    Ok(CommandKey {
        name: key.name,
        event_path: key.event_path,
        cmd,
        arg: arg.to_vec(),
        input,
        output,
    })
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn debounce_command(
    key: CommandKey,
    threshold: Duration,
    context: Context,
    cache: &Arc<Mutex<HashMap<CommandKey, Instant>>>,
) -> Result<CommandResult> {
    let key = render_command(key, context)?;
    let now = Instant::now();
    let mut lock = cache.lock().unwrap();
    lock.insert(key.clone(), now);
    drop(lock);

    thread::sleep(threshold);

    let lock = cache.lock().unwrap();
    let executed = lock.get(&key).unwrap();
    if executed > &now {
        info!(
            "Debounce ! Skip execute cmd: {}, arg: {}",
            key.cmd,
            key.arg.join(" ")
        );
        return Ok(CommandResult {
            status: ExitStatus::default(),
            stdout: PathBuf::new(),
            stderr: PathBuf::new(),
            skipped: true,
        });
    }
    drop(lock);

    exec(key)
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn throttle_command(
    key: CommandKey,
    threshold: Duration,
    context: Context,
    cache: &Arc<Mutex<HashMap<CommandKey, Instant>>>,
) -> Result<CommandResult> {
    let key = render_command(key, context)?;
    let now = Instant::now();
    let mut lock = cache.lock().unwrap();
    let executed = lock.get(&key);
    if let Some(executed) = executed {
        if now.duration_since(*executed) < threshold {
            drop(lock);
            info!(
                "Throttle ! Skip execute cmd: {}, arg: {}",
                key.cmd,
                key.arg.join(" ")
            );
            return Ok(CommandResult {
                status: ExitStatus::default(),
                stdout: PathBuf::default(),
                stderr: PathBuf::default(),
                skipped: true,
            });
        }
    }
    lock.insert(key.clone(), now);
    drop(lock);

    exec(key)
}

#[tracing::instrument]
#[logfn(Debug)]
pub fn exec(key: CommandKey) -> Result<CommandResult> {
    let now = Local::now().format("%Y%m%d_%H%M%S%3f").to_string();
    let stdout_path = PathBuf::from(&key.output).join(format!("{}_stdout_{}.log", &key.name, now));
    let stderr_path = PathBuf::from(&key.output).join(format!("{}_stderr_{}.log", &key.name, now));
    let stdout_file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&stdout_path)?;
    let stderr_file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&stderr_path)?;
    info!(
        "Execute cmd: {}, arg: {}, stdout: {}, stderr: {}",
        &key.cmd,
        &key.arg.join(" "),
        stdout_path.display(),
        stderr_path.display()
    );
    Ok(CommandResult {
        status: Command::new(&key.cmd)
            .args(&key.arg)
            .stdout(stdout_file)
            .stderr(stderr_file)
            .spawn()?
            .wait()?,
        stdout: stdout_path,
        stderr: stderr_path,
        skipped: false,
    })
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn execute_command(
    event_path: &PathBuf,
    name: &str,
    input: &str,
    output: &str,
    cmd: &str,
    arg: Vec<String>,
    debounce: Duration,
    throttle: Duration,
    context: Context,
    cache: &Arc<Mutex<HashMap<CommandKey, Instant>>>,
) -> Result<CommandResult> {
    if debounce > Duration::from_millis(0) {
        return debounce_command(
            CommandKey {
                name: name.to_string(),
                event_path: event_path.clone(),
                cmd: cmd.to_string(),
                arg: arg.clone(),
                input: input.to_string(),
                output: output.to_string(),
            },
            debounce,
            context.clone(),
            cache,
        );
    }
    if throttle > Duration::from_millis(0) {
        return throttle_command(
            CommandKey {
                name: name.to_string(),
                event_path: event_path.clone(),
                cmd: cmd.to_string(),
                arg: arg.clone(),
                input: input.to_string(),
                output: output.to_string(),
            },
            throttle,
            context.clone(),
            cache,
        );
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use std::{env, thread, time::Duration};

    use anyhow::Result;

    use super::*;

    #[test]
    fn test_execute_command_with_throttle() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let name = "test";
        let input = "input";
        let output = tmp.join("test_execute_command");
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "test_execute_command"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "test_execute_command"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let throttle = Duration::from_secs(10);
        let context = Context::new();
        let cache = Arc::new(Mutex::new(HashMap::new()));

        let mut handles = vec![];
        for i in 0..3 {
            let cache = cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();
            handles.push(thread::spawn(move || {
                let result = execute_command(
                    &event_path,
                    name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    Duration::from_millis(0),
                    throttle,
                    context,
                    &cache,
                )
                .unwrap();
                if i == 0 {
                    assert_eq!(result.status.code(), Some(0));
                    assert_ne!(result.stdout.to_str().unwrap(), "");
                    assert_ne!(result.stderr.to_str().unwrap(), "");
                    assert!(!result.skipped);
                } else {
                    assert_eq!(result.status.code(), Some(0));
                    assert_eq!(result.stdout.to_str().unwrap(), "");
                    assert_eq!(result.stderr.to_str().unwrap(), "");
                    assert!(result.skipped);
                }
            }));
            thread::sleep(Duration::from_millis(100));
        }

        handles.into_iter().for_each(|h| h.join().unwrap());

        Ok(())
    }

    #[test]
    fn test_execute_long_command_with_throttle() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let name = "test";
        let input = "input";
        let output = tmp.join("test_execute_command");
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sleep";
        #[cfg(windows)]
        let arg = vec!["/c", "timeout", "/t", "3"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["3"].into_iter().map(String::from).collect::<Vec<_>>();
        let throttle = Duration::from_millis(100);
        let context = Context::new();
        let cache = Arc::new(Mutex::new(HashMap::new()));

        let mut handles = vec![];
        let start = Instant::now();
        for _ in 0..3 {
            let cache = cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();
            handles.push(thread::spawn(move || {
                let result = execute_command(
                    &event_path,
                    name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    Duration::from_millis(0),
                    throttle,
                    context,
                    &cache,
                )
                .unwrap();
                assert_eq!(result.status.code(), Some(0));
                assert_ne!(result.stdout.to_str().unwrap(), "");
                assert_ne!(result.stderr.to_str().unwrap(), "");
                assert!(!result.skipped);
            }));
            thread::sleep(Duration::from_millis(200));
        }
        handles.into_iter().for_each(|h| h.join().unwrap());

        let end = Instant::now();
        let duration = end.duration_since(start);
        assert!(duration < Duration::from_secs(6));

        Ok(())
    }
}

// =============================================================================
// File        : command.rs
// Author      : yukimemi
// Last Change : 2023/10/24 11:55:53.
// =============================================================================

use std::{
    collections::HashMap,
    fs::{create_dir_all, OpenOptions},
    path::PathBuf,
    process::{Command, ExitStatus},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::Local;
use log_derive::logfn;
use tera::Context;
use tracing::{info, warn};

use crate::util::{insert_file_context, new_tera};

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct ThrottleKey {
    name: String,
    event_path: PathBuf,
    command: String,
    args: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CommandResult {
    status: ExitStatus,
    stdout: PathBuf,
    stderr: PathBuf,
}

#[tracing::instrument]
#[logfn(Debug)]
pub fn execute_command(
    event_path: &PathBuf,
    name: &str,
    input: &str,
    output: &str,
    cmd: &str,
    arg: Vec<String>,
    threshold: Duration,
    context: Context,
    cache: &Arc<Mutex<HashMap<ThrottleKey, Instant>>>,
) -> Result<CommandResult> {
    let mut context = context.clone();
    insert_file_context(event_path, "event", &mut context).unwrap();
    let tera = new_tera("cmd", cmd)?;
    let cmd = tera.render("cmd", &context)?;
    context.insert("cmd", &cmd);
    let arg = &arg
        .iter()
        .map(|s| {
            let tera = new_tera("arg", s).unwrap();
            tera.render("arg", &context).unwrap()
        })
        .collect::<Vec<_>>();
    context.insert("arg", &arg.join(" "));
    let tera = new_tera("input", input)?;
    let input = tera.render("input", &context)?;
    context.insert("input", &input);
    let tera = new_tera("output", output)?;
    let output = tera.render("output", &context)?;
    context.insert("output", &output);
    create_dir_all(&output)?;
    let key = ThrottleKey {
        name: name.to_string(),
        event_path: event_path.clone(),
        command: cmd.clone(),
        args: arg.clone(),
    };
    let now = Instant::now();
    let mut cache = cache.lock().unwrap();
    if let Some(executed) = cache.get(&key) {
        if now.duration_since(*executed) < threshold {
            info!("Skip execute cmd: {}, arg: {}", cmd, arg.join(" "));
            return Ok(CommandResult {
                status: ExitStatus::default(),
                stdout: PathBuf::default(),
                stderr: PathBuf::default(),
            });
        }
    }
    cache.insert(key, now);

    let now = Local::now().format("%Y%m%d_%H%M%S%3f").to_string();
    let stdout_path = PathBuf::from(&output).join(format!("{}_stdout_{}.log", &name, now));
    let stderr_path = PathBuf::from(&output).join(format!("{}_stderr_{}.log", &name, now));
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
        cmd,
        arg.join(" "),
        stdout_path.display(),
        stderr_path.display()
    );
    Ok(CommandResult {
        status: Command::new(cmd)
            .args(arg)
            .stdout(stdout_file)
            .stderr(stderr_file)
            .spawn()?
            .wait()?,
        stdout: stdout_path,
        stderr: stderr_path,
    })
}

#[cfg(test)]
mod tests {
    use std::{env, thread, time::Duration};

    use anyhow::Result;

    use super::*;

    #[test]
    fn test_execute_command() -> Result<()> {
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
        let threshold = Duration::from_secs(10);
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
                    threshold,
                    context,
                    &cache,
                )
                .unwrap();
                if i == 0 {
                    assert_eq!(result.status.code(), Some(0));
                    assert_ne!(result.stdout.to_str().unwrap(), "");
                    assert_ne!(result.stderr.to_str().unwrap(), "");
                } else {
                    assert_eq!(result.status.code(), Some(0));
                    assert_eq!(result.stdout.to_str().unwrap(), "");
                    assert_eq!(result.stderr.to_str().unwrap(), "");
                }
            }));
            thread::sleep(Duration::from_millis(100));
        }

        handles.into_iter().for_each(|h| h.join().unwrap());

        Ok(())
    }
}

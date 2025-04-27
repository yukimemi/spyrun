// =============================================================================
// File        : command.rs
// Author      : yukimemi
// Last Change : 2025/04/27 17:02:58.
// =============================================================================

use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::{OpenOptions, create_dir_all},
    path::PathBuf,
    process::{Command, ExitStatus},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::Local;
use go_defer::defer;
use log_derive::logfn;
use tera::Context;
use tracing::{debug, info, warn};

use crate::util::{insert_file_context, new_tera};

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct CommandInfo {
    name: String,
    event_path: PathBuf,
    event_kind: String,
    cmd: String,
    arg: Vec<String>,
    input: String,
    output: String,
}

impl fmt::Display for CommandInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "CommandInfo {{ name: {}, event_path: {:?}, event_kind: {}, cmd: {}, arg: {:?}, input: {}, output: {} }}",
            self.name,
            self.event_path,
            self.event_kind,
            self.cmd,
            self.arg,
            self.input,
            self.output
        )
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CommandResult {
    status: ExitStatus,
    stdout: PathBuf,
    stderr: PathBuf,
    skipped: bool,
}

// Helper function to separate debounce logic
fn apply_debounce(
    limitkey: &str,
    threshold: Duration,
    dt_cache: &Arc<Mutex<HashMap<String, Instant>>>,
) -> bool /* true if skipped */ {
    if threshold == Duration::from_millis(0) {
        return false; // Debounce disabled
    }
    let now = Instant::now();
    let mut lock = dt_cache.lock().unwrap();
    lock.insert(limitkey.to_string(), now);
    drop(lock);

    // Wait for the specified threshold (blocking)
    thread::sleep(threshold);

    let lock = dt_cache.lock().unwrap();
    let executed = lock.get(limitkey).unwrap(); // Should exist as it was just inserted
    if executed > &now {
        debug!("Debounce! Skip execute limitkey: {}", limitkey);
        true // Skip
    } else {
        debug!("Debounce passed for limitkey: {}", limitkey);
        false // Do not skip
    }
}

// Helper function to separate throttle logic
fn apply_throttle(
    limitkey: &str,
    threshold: Duration,
    dt_cache: &Arc<Mutex<HashMap<String, Instant>>>,
) -> bool /* true if skipped */ {
    if threshold == Duration::from_millis(0) {
        return false; // Throttle disabled
    }
    let now = Instant::now();
    let mut lock = dt_cache.lock().unwrap();
    let executed = lock.get(limitkey);
    if let Some(executed) = executed {
        if now.duration_since(*executed) < threshold {
            drop(lock);
            debug!("Throttle! Skip execute limitkey: {}", limitkey);
            return true; // Skip
        }
    }
    // Update the cache if not skipped
    lock.insert(limitkey.to_string(), now);
    drop(lock);
    debug!("Throttle passed for limitkey: {}", limitkey);
    false // Do not skip
}

// Helper function to attempt acquiring a mutex lock
fn acquire_mutex(mutexkey: &str, mutex_cache: &Arc<Mutex<HashSet<String>>>) -> bool /* true if acquired, false if skipped */
{
    if mutexkey.is_empty() {
        // If mutexkey is empty, always consider acquisition successful (mutex disabled)
        return true;
    }
    let mut lock = mutex_cache.lock().unwrap();
    if lock.contains(mutexkey) {
        debug!("Mutex held! Skip execute mutexkey: {}", mutexkey);
        false // Failed to acquire lock, skip
    } else {
        lock.insert(mutexkey.to_string());
        debug!("Mutex acquired for mutexkey: {}", mutexkey);
        true // Acquired lock successfully
    }
}

// Helper function to release a mutex lock
fn release_mutex(mutexkey: &str, mutex_cache: &Arc<Mutex<HashSet<String>>>) {
    if mutexkey.is_empty() {
        // If mutexkey is empty, do nothing
        return;
    }
    let mut lock = mutex_cache.lock().unwrap();
    lock.remove(mutexkey);
    debug!("Mutex released for mutexkey: {}", mutexkey);
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn render_command(cmd_info: CommandInfo, context: Context) -> Result<CommandInfo> {
    let mut context = context.clone();
    insert_file_context(&cmd_info.event_path, "event", &mut context).unwrap();
    let tera = new_tera("spy_name", &cmd_info.name)?;
    let spy_name = tera.render("spy_name", &context)?;
    context.insert("spy_name", &spy_name);
    let tera = new_tera("cmd", &cmd_info.cmd)?;
    let cmd = tera.render("cmd", &context)?;
    context.insert("cmd", &cmd);
    let arg = &cmd_info
        .arg
        .iter()
        .map(|s| {
            let tera = new_tera("arg", s).unwrap();
            tera.render("arg", &context).unwrap()
        })
        .collect::<Vec<_>>();
    context.insert("arg", &arg.join(" "));
    let tera = new_tera("input", &cmd_info.input)?;
    let input = tera.render("input", &context)?;
    context.insert("input", &input);
    let tera = new_tera("output", &cmd_info.output)?;
    let output = tera.render("output", &context)?;
    context.insert("output", &output);
    create_dir_all(&output)?;

    Ok(CommandInfo {
        name: cmd_info.name,
        event_path: cmd_info.event_path,
        event_kind: cmd_info.event_kind,
        cmd,
        arg: arg.to_vec(),
        input,
        output,
    })
}

#[tracing::instrument]
#[logfn(Debug)]
pub fn exec(cmd_info: CommandInfo) -> Result<CommandResult> {
    let now = Local::now().format("%Y%m%d_%H%M%S%3f").to_string();
    let output_dir = PathBuf::from(&cmd_info.output);
    std::fs::create_dir_all(&output_dir)?;
    let stdout_path = output_dir.join(format!("{}_stdout_{}.log", &cmd_info.name, now));
    let stderr_path = output_dir.join(format!("{}_stderr_{}.log", &cmd_info.name, now));
    let stdout_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&stdout_path)?;
    let stderr_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&stderr_path)?;
    warn!(
        "[exec] Running command: '{} {}' > {} 2> {}",
        &cmd_info.cmd,
        &cmd_info.arg.join(" "),
        stdout_path.display(),
        stderr_path.display()
    );
    let status = Command::new(&cmd_info.cmd)
        .args(&cmd_info.arg)
        .stdout(stdout_file)
        .stderr(stderr_file)
        .spawn()?
        .wait()?;
    warn!(
        "[exec] Finished command: '{} {}' with status: {}",
        &cmd_info.cmd,
        &cmd_info.arg.join(" "),
        status
    );
    Ok(CommandResult {
        status,
        stdout: stdout_path,
        stderr: stderr_path,
        skipped: false,
    })
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn execute_command(
    event_path: &PathBuf,
    event_kind: &str,
    name: &str,
    input: &str,
    output: &str,
    cmd: &str,
    arg: Vec<String>,
    debounce: Duration,
    throttle: Duration,
    limitkey_tmpl: &str, // Key template for debounce/throttle
    mutexkey_tmpl: &str, // Add key template for mutex
    mut context: Context,
    dt_cache: &Arc<Mutex<HashMap<String, Instant>>>, // Renamed to debounce/throttle cache
    mutex_cache: &Arc<Mutex<HashSet<String>>>,       // Add mutex cache
) -> Result<CommandResult> {
    // 1. Render CommandInfo
    let cmd_info = render_command(
        CommandInfo {
            name: name.to_string(),
            event_path: event_path.clone(),
            event_kind: event_kind.to_string(),
            cmd: cmd.to_string(),
            arg: arg.clone(),
            input: input.to_string(),
            output: output.to_string(),
        },
        context.clone(), // Clone Context for rendering
    )?;

    // 2. Render limitkey and mutexkey templates
    let limitkey = if limitkey_tmpl.is_empty() {
        cmd_info.to_string() // Use CommandInfo as default key if template is empty
    } else {
        let tera = new_tera("limitkey", limitkey_tmpl)?;
        // Context includes event and info added by render_command
        tera.render("limitkey", &context)?
    };
    context.insert("limitkey", &limitkey); // Add rendered limitkey to context

    let mutexkey = if mutexkey_tmpl.is_empty() {
        cmd_info.to_string() // Use CommandInfo as default key if template is empty
    } else {
        let tera = new_tera("mutexkey", mutexkey_tmpl)?;
        // Context also includes limitkey
        tera.render("mutexkey", &context)?
    };
    context.insert("mutexkey", &mutexkey); // Add rendered mutexkey to context

    warn!(
        "[execute_command] limitkey: [{}], mutexkey: [{}], cmd_info: [{}]",
        &limitkey,
        &mutexkey,
        cmd_info.to_string()
    );

    // 3. Apply Debounce logic (if enabled)
    if debounce > Duration::from_millis(0) && apply_debounce(&limitkey, debounce, dt_cache) {
        return Ok(CommandResult {
            status: ExitStatus::default(), // Default value when skipped
            stdout: PathBuf::new(),
            stderr: PathBuf::new(),
            skipped: true,
        });
    }

    // 4. Apply Throttle logic (if enabled and Debounce is disabled)
    // Note: Debounce and Throttle are intended to be mutually exclusive
    if throttle > Duration::from_millis(0) && apply_throttle(&limitkey, throttle, dt_cache) {
        return Ok(CommandResult {
            status: ExitStatus::default(), // Default value when skipped
            stdout: PathBuf::default(),
            stderr: PathBuf::default(),
            skipped: true,
        });
    }

    // 5. Apply Mutex logic
    // acquire_mutex checks if mutexkey is empty internally, so just calling it is enough
    if acquire_mutex(&mutexkey, mutex_cache) {
        // Mutex acquired successfully (or mutex disabled if mutexkey is empty)
        // Set up defer to ensure release_mutex is called when leaving the scope
        defer! {
            release_mutex(&mutexkey, mutex_cache);
        }
        // Execute the command and return the result
        exec(cmd_info)
    } else {
        // Failed to acquire Mutex (another thread is executing)
        Ok(CommandResult {
            status: ExitStatus::default(), // Default value when skipped
            stdout: PathBuf::new(),        // Empty path when skipped
            stderr: PathBuf::new(),
            skipped: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{env, time::Duration};

    use super::*;

    // Modify existing tests to match the new execute_command arguments
    #[test]
    fn test_execute_command_with_throttle() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let name = "test_throttle"; // Changed name
        let input = "input";
        let event_kind = "Create";
        let output = tmp.join(name); // Changed output directory name
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "test_execute_command_throttle"] // Changed message
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "test_execute_command_throttle"] // Changed message
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let throttle = Duration::from_secs(1); // Shorten threshold for easier testing
        let debounce = Duration::from_millis(0);
        let limitkey_tmpl = ""; // Use default limitkey
        let mutexkey_tmpl = ""; // Do not use mutex
        let context = Context::new();
        let dt_cache = Arc::new(Mutex::new(HashMap::new()));
        let mutex_cache = Arc::new(Mutex::new(HashSet::new())); // dummy mutex cache

        let mut handles = vec![];
        let num_threads = 3;

        for _i in 0..num_threads {
            let dt_cache = dt_cache.clone();
            let mutex_cache = mutex_cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();

            handles.push(thread::spawn(move || {
                execute_command(
                    &event_path,
                    event_kind,
                    name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutexkey_tmpl, // New argument
                    context,
                    &dt_cache,    // Renamed argument
                    &mutex_cache, // New argument
                )
                .unwrap()
            }));
            // Start threads in rapid succession to make throttle more effective, wait a bit
            thread::sleep(Duration::from_millis(100));
        }

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Attempting to execute 3 times within the throttle threshold (1 second)
        // The first execution should happen
        // Subsequent executions should be skipped if they occur within 1 second of the first
        // In this test, threads 2 and 3 start 100ms apart after the first, so they are expected
        // to be skipped by throttle.
        let executed_count = results.iter().filter(|r| !r.skipped).count();
        let skipped_count = results.iter().filter(|r| r.skipped).count();

        assert_eq!(
            executed_count, 1,
            "Exactly one command should have been executed"
        );
        assert_eq!(
            skipped_count,
            num_threads - 1,
            "Remaining commands should have been skipped by throttle"
        );

        // Verify the executed command
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // Verify the skipped commands
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            // status is default value when skipped, stdout/stderr are empty paths
            // original test checked status.code() == Some(0), keeping for consistency but it's brittle
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        Ok(())
    }

    // This test assumes command execution time is shorter than the throttle threshold
    #[test]
    fn test_execute_short_command_with_throttle() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_short_throttle"; // Changed name
        let input = "input";
        let output = tmp.join(name); // Changed output directory name
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "short_throttle"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "short_throttle"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let throttle = Duration::from_millis(500); // throttle threshold
        let debounce = Duration::from_millis(0);
        let limitkey_tmpl = "";
        let mutexkey_tmpl = ""; // Do not use mutex
        let context = Context::new();
        let dt_cache = Arc::new(Mutex::new(HashMap::new()));
        let mutex_cache = Arc::new(Mutex::new(HashSet::new())); // dummy mutex cache

        let mut handles = vec![];
        let start = Instant::now();
        let num_threads = 3;

        for _ in 0..num_threads {
            let dt_cache = dt_cache.clone();
            let mutex_cache = mutex_cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();
            handles.push(thread::spawn(move || {
                execute_command(
                    &event_path,
                    event_kind,
                    name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutexkey_tmpl, // New argument
                    context,
                    &dt_cache,    // Renamed argument
                    &mutex_cache, // New argument
                )
                .unwrap()
            }));
            // Start threads at intervals shorter than the throttle threshold
            thread::sleep(Duration::from_millis(100));
        }
        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let end = Instant::now();
        let duration = end.duration_since(start);

        // Attempting to execute 3 times at intervals (100ms) shorter than the throttle threshold (500ms)
        // The first execution should happen
        // The second execution comes 100ms after the first, skipped by throttle (500ms)
        // The third execution comes 100ms after the second, skipped by throttle (500ms)
        let executed_count = results.iter().filter(|r| !r.skipped).count();
        let skipped_count = results.iter().filter(|r| r.skipped).count();

        assert_eq!(
            executed_count, 1,
            "Exactly one command should have been executed"
        );
        assert_eq!(
            skipped_count,
            num_threads - 1,
            "Remaining commands should have been skipped by throttle"
        );

        // Verify the executed command
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // Verify the skipped commands
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        // Verify the execution duration
        // Total time is roughly the time until the last thread starts + the command execution time (almost zero) + overhead
        // With 3 threads starting at 100ms intervals, the last thread starts at t=200ms.
        // The total duration should be roughly 200ms + alpha.
        assert!(duration >= Duration::from_millis(200));
        assert!(duration < Duration::from_millis(1000)); // Should finish within 1 second

        Ok(())
    }

    #[test]
    fn test_execute_command_with_debounce() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_debounce"; // Changed name
        let input = "input";
        let output = tmp.join(name); // Changed output directory name
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "test_execute_command_debounce"] // Changed message
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "test_execute_command_debounce"] // Changed message
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let debounce = Duration::from_millis(500); // debounce threshold
        let throttle = Duration::from_millis(0);
        let limitkey_tmpl = "";
        let mutexkey_tmpl = "";
        let context = Context::new();
        let dt_cache = Arc::new(Mutex::new(HashMap::new()));
        let mutex_cache = Arc::new(Mutex::new(HashSet::new()));

        let mut handles = vec![];
        let num_threads = 3;

        for _i in 0..num_threads {
            let dt_cache = dt_cache.clone();
            let mutex_cache = mutex_cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();

            handles.push(thread::spawn(move || {
                execute_command(
                    &event_path,
                    event_kind,
                    name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutexkey_tmpl, // New argument
                    context,
                    &dt_cache,    // Renamed argument
                    &mutex_cache, // New argument
                )
                .unwrap()
            }));
            // Start threads at intervals (100ms) shorter than the debounce threshold (500ms)
            thread::sleep(Duration::from_millis(100));
        }

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Attempting to execute 3 times within the debounce threshold (500ms)
        // Should wait for 500ms after the last request. The first 2 should be skipped, only the last one executed.
        let executed_count = results.iter().filter(|r| !r.skipped).count();
        let skipped_count = results.iter().filter(|r| r.skipped).count();

        assert_eq!(
            executed_count, 1,
            "Exactly one command should have been executed"
        );
        assert_eq!(
            skipped_count,
            num_threads - 1,
            "Remaining commands should have been skipped by debounce"
        );

        // Verify the executed command
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // Verify the skipped commands
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        Ok(())
    }

    // This test assumes command execution time is shorter than the debounce threshold
    #[test]
    fn test_execute_short_command_with_debounce() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_short_debounce"; // Changed name
        let input = "input";
        let output = tmp.join(name); // Changed output directory name
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "short_debounce"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "short_debounce"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let debounce = Duration::from_millis(100); // debounce threshold
        let throttle = Duration::from_millis(0);
        let limitkey_tmpl = "";
        let mutexkey_tmpl = ""; // Do not use mutex
        let context = Context::new();
        let dt_cache = Arc::new(Mutex::new(HashMap::new()));
        let mutex_cache = Arc::new(Mutex::new(HashSet::new())); // dummy mutex cache

        let mut handles = vec![];
        let start = Instant::now();
        let num_threads = 3;

        for _ in 0..num_threads {
            let dt_cache = dt_cache.clone();
            let mutex_cache = mutex_cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();
            handles.push(thread::spawn(move || {
                execute_command(
                    &event_path,
                    event_kind,
                    name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutexkey_tmpl, // New argument
                    context,
                    &dt_cache,    // Renamed argument
                    &mutex_cache, // New argument
                )
                .unwrap()
            }));
            // Start threads at intervals (50ms) shorter than the debounce threshold (100ms)
            thread::sleep(Duration::from_millis(50));
        }
        // Need to wait for the debounce threshold after the last request
        // thread::sleep(debounce); // This wait is handled inside execute_command

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let end = Instant::now();
        let duration = end.duration_since(start);

        // Attempting to execute 3 times within the debounce threshold (100ms)
        // Should wait for 100ms after the last request. The first 2 should be skipped, only the last one executed.
        let executed_count = results.iter().filter(|r| !r.skipped).count();
        let skipped_count = results.iter().filter(|r| r.skipped).count();

        assert_eq!(
            executed_count, 1,
            "Exactly one command should have been executed"
        );
        assert_eq!(
            skipped_count,
            num_threads - 1,
            "Remaining commands should have been skipped by debounce"
        );

        // Verify the executed command
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // Verify the skipped commands
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        // Verify the execution duration
        // Total time is roughly the time from the first thread start until the last thread finishes its debounce wait + command execution time (almost zero) + overhead.
        // Threads start at t=0, t=50, t=100ms. The last thread finishes its debounce wait 100ms after it started, i.e., at t=200ms.
        // The total duration should be roughly 200ms + alpha.
        assert!(duration >= Duration::from_millis(200));
        assert!(duration < Duration::from_millis(500)); // Should finish within 500ms

        Ok(())
    }

    // Test case for mutex functionality
    #[test]
    fn test_execute_command_with_mutex() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_mutex"; // Name
        let input = "input";
        let output = tmp.join(name); // Output directory
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sleep"; // Use sleep to make command execution take time
        #[cfg(windows)]
        let arg = vec!["/c", "timeout", "/t", "2"] // sleep for 2 seconds
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["1"].into_iter().map(String::from).collect::<Vec<_>>(); // sleep for 1 second
        let debounce = Duration::from_millis(0); // Debounce disabled
        let throttle = Duration::from_millis(0); // Throttle disabled
        let limitkey_tmpl = ""; // Do not use limitkey
        let mutexkey_tmpl = "my_shared_mutexkey"; // Common mutex key
        let context = Context::new();
        let dt_cache = Arc::new(Mutex::new(HashMap::new())); // dummy dt cache
        let mutex_cache = Arc::new(Mutex::new(HashSet::new())); // mutex cache

        let num_threads = 5;
        let mut handles = vec![];
        let start = Instant::now();

        for i in 0..num_threads {
            let dt_cache = dt_cache.clone();
            let mutex_cache = mutex_cache.clone();
            let event_path = event_path.clone();
            let arg = arg.clone();
            let context = context.clone();
            let output = output.clone();
            // Change name slightly per thread (for easier distinction in logs)
            let thread_name = format!("{name}_{i}");
            let mutexkey_tmpl = mutexkey_tmpl.to_string(); // clone for the thread
            let limitkey_tmpl = limitkey_tmpl.to_string(); // clone for the thread

            handles.push(thread::spawn(move || {
                info!("Thread {} trying to execute...", i);
                let result = execute_command(
                    &event_path,
                    event_kind,
                    &thread_name, // Thread-specific name
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    &limitkey_tmpl,
                    &mutexkey_tmpl, // Specify mutex key template
                    context,
                    &dt_cache,    // dummy cache
                    &mutex_cache, // mutex cache
                )
                .unwrap();
                info!(
                    "Thread {} finished execution. Skipped: {}",
                    i, result.skipped
                );
                result
            }));
            // Start threads almost simultaneously to increase mutex contention, wait a bit
            thread::sleep(Duration::from_millis(50));
        }

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let end = Instant::now();
        let duration = end.duration_since(start);
        dbg!(&duration);

        // Using the same mutexkey, only one can execute at a time.
        // Therefore, out of num_threads, only one should execute, and the rest should be skipped.
        let executed_count = results.iter().filter(|r| !r.skipped).count();
        let skipped_count = results.iter().filter(|r| r.skipped).count();

        assert_eq!(
            executed_count, 1,
            "Exactly one command should have been executed"
        );
        assert_eq!(
            skipped_count,
            num_threads - 1,
            "Remaining commands should have been skipped by mutex"
        );

        // Verify the executed command
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        // The sleep command doesn't output anything to stdout/stderr, but the exec function creates files, so paths should exist
        assert!(!executed_result.skipped);

        // Verify the skipped commands
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0)); // Default value when skipped
        }

        // Verify the total execution duration
        // The command sleeps for 1 second, so the total duration should be 1 second + overhead.
        // Although 5 threads attempt to start simultaneously, they don't execute one by one due to the Mutex.
        // Instead, the other 4 are skipped while the first one is executing.
        // Thus, the total time won't be N * command_time.
        assert!(duration >= Duration::from_secs(1)); // Should wait for the 1-second sleep
        assert!(duration < Duration::from_secs(3)); // Should be less than 5 seconds

        Ok(())
    }
}

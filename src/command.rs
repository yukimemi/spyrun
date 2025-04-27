// =============================================================================
// File        : command.rs
// Author      : yukimemi
// Last Change : 2025/04/27 16:34:19.
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

// debounce のロジックを分離したヘルパー関数
fn apply_debounce(
    limitkey: &str,
    threshold: Duration,
    dt_cache: &Arc<Mutex<HashMap<String, Instant>>>,
) -> bool /* true if skipped */ {
    if threshold == Duration::from_millis(0) {
        return false; // Debounce 無効
    }
    let now = Instant::now();
    let mut lock = dt_cache.lock().unwrap();
    lock.insert(limitkey.to_string(), now);
    drop(lock);

    // 指定された閾値だけ待つ (ブロッキング)
    thread::sleep(threshold);

    let lock = dt_cache.lock().unwrap();
    let executed = lock.get(limitkey).unwrap(); // 直前に挿入したので存在するはず
    if executed > &now {
        debug!("Debounce ! Skip execute limitkey: {}", limitkey);
        true // スキップ
    } else {
        debug!("Debounce passed for limitkey: {}", limitkey);
        false // スキップしない
    }
}

// throttle のロジックを分離したヘルパー関数
fn apply_throttle(
    limitkey: &str,
    threshold: Duration,
    dt_cache: &Arc<Mutex<HashMap<String, Instant>>>,
) -> bool /* true if skipped */ {
    if threshold == Duration::from_millis(0) {
        return false; // Throttle 無効
    }
    let now = Instant::now();
    let mut lock = dt_cache.lock().unwrap();
    let executed = lock.get(limitkey);
    if let Some(executed) = executed {
        if now.duration_since(*executed) < threshold {
            drop(lock);
            debug!("Throttle ! Skip execute limitkey: {}", limitkey);
            return true; // スキップ
        }
    }
    // スキップしなかった場合はキャッシュを更新
    lock.insert(limitkey.to_string(), now);
    drop(lock);
    debug!("Throttle passed for limitkey: {}", limitkey);
    false // スキップしない
}

// mutex のロック取得を試みるヘルパー関数
fn acquire_mutex(mutex_key: &str, mutex_cache: &Arc<Mutex<HashSet<String>>>) -> bool /* true if acquired, false if skipped */
{
    if mutex_key.is_empty() {
        // mutex_key が空の場合は常に取得成功とみなす（mutex 無効）
        return true;
    }
    let mut lock = mutex_cache.lock().unwrap();
    if lock.contains(mutex_key) {
        debug!("Mutex held ! Skip execute mutex_key: {}", mutex_key);
        false // ロック取得失敗、スキップ
    } else {
        lock.insert(mutex_key.to_string());
        debug!("Mutex acquired for mutex_key: {}", mutex_key);
        true // ロック取得成功
    }
}

// mutex のロックを解除するヘルパー関数
fn release_mutex(mutex_key: &str, mutex_cache: &Arc<Mutex<HashSet<String>>>) {
    if mutex_key.is_empty() {
        // mutex_key が空の場合は何もしない
        return;
    }
    let mut lock = mutex_cache.lock().unwrap();
    lock.remove(mutex_key);
    debug!("Mutex released for mutex_key: {}", mutex_key);
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
#[logfn(Trace)]
pub fn debounce_command(
    cmd_info: CommandInfo,
    threshold: Duration,
    limitkey: &str,
    context: Context,
    cache: &Arc<Mutex<HashMap<String, Instant>>>,
) -> Result<CommandResult> {
    let now = Instant::now();
    let mut lock = cache.lock().unwrap();
    lock.insert(limitkey.to_string(), now);
    drop(lock);

    thread::sleep(threshold);

    let lock = cache.lock().unwrap();
    let executed = lock.get(limitkey).unwrap();
    if executed > &now {
        debug!(
            "Debounce ! Skip execute limitkey: {}",
            &limitkey.to_string(),
        );
        return Ok(CommandResult {
            status: ExitStatus::default(),
            stdout: PathBuf::new(),
            stderr: PathBuf::new(),
            skipped: true,
        });
    }
    drop(lock);

    exec(cmd_info)
}

#[tracing::instrument]
#[logfn(Trace)]
pub fn throttle_command(
    cmd_info: CommandInfo,
    threshold: Duration,
    limitkey: &str,
    context: Context,
    cache: &Arc<Mutex<HashMap<String, Instant>>>,
) -> Result<CommandResult> {
    let now = Instant::now();
    let mut lock = cache.lock().unwrap();
    let executed = lock.get(limitkey);
    if let Some(executed) = executed {
        if now.duration_since(*executed) < threshold {
            drop(lock);
            debug!(
                "Throttle ! Skip execute limitkey: {}",
                &limitkey.to_string(),
            );
            return Ok(CommandResult {
                status: ExitStatus::default(),
                stdout: PathBuf::default(),
                stderr: PathBuf::default(),
                skipped: true,
            });
        }
    }
    lock.insert(limitkey.to_string(), now);
    drop(lock);

    exec(cmd_info)
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
    limitkey_tmpl: &str,  // debounce/throttle 用のキーテンプレート
    mutex_key_tmpl: &str, // mutex 用のキーテンプレートを追加
    mut context: Context,
    dt_cache: &Arc<Mutex<HashMap<String, Instant>>>, // debounce/throttle 用キャッシュにリネーム
    mutex_cache: &Arc<Mutex<HashSet<String>>>,       // mutex 用キャッシュを追加
) -> Result<CommandResult> {
    // 1. CommandInfo をレンダリング
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
        context.clone(), // レンダリング用に Context をクローン
    )?;

    // 2. limitkey および mutex_key テンプレートをレンダリング
    let limitkey = if limitkey_tmpl.is_empty() {
        cmd_info.to_string() // テンプレートが空なら CommandInfo をデフォルトキーとする
    } else {
        let tera = new_tera("limitkey", limitkey_tmpl)?;
        // context には event や render_command で追加された情報が含まれている
        tera.render("limitkey", &context)?
    };
    context.insert("limitkey", &limitkey); // レンダリングした limitkey を context に追加

    let mutex_key = if mutex_key_tmpl.is_empty() {
        cmd_info.to_string() // テンプレートが空なら CommandInfo をデフォルトキーとする
    } else {
        let tera = new_tera("mutex_key", mutex_key_tmpl)?;
        // context には limitkey も追加されている
        tera.render("mutex_key", &context)?
    };
    context.insert("mutex_key", &mutex_key); // レンダリングした mutex_key を context に追加

    info!(
        "[execute_command] limitkey: [{}], mutex_key: [{}], cmd_info: [{}]",
        &limitkey,
        &mutex_key,
        cmd_info.to_string()
    );

    // 3. Debounce ロジック適用 (有効な場合)
    if debounce > Duration::from_millis(0) {
        if apply_debounce(&limitkey, debounce, dt_cache) {
            return Ok(CommandResult {
                status: ExitStatus::default(), // スキップ時はデフォルト値
                stdout: PathBuf::new(),
                stderr: PathBuf::new(),
                skipped: true,
            });
        }
    }

    // 4. Throttle ロジック適用 (有効かつ Debounce が無効な場合)
    // Note: DebounceとThrottleは排他利用を想定
    if throttle > Duration::from_millis(0) && debounce == Duration::from_millis(0) {
        if apply_throttle(&limitkey, throttle, dt_cache) {
            return Ok(CommandResult {
                status: ExitStatus::default(), // スキップ時はデフォルト値
                stdout: PathBuf::default(),
                stderr: PathBuf::default(),
                skipped: true,
            });
        }
    }

    // 5. Mutex ロジック適用
    // acquire_mutex の中で mutex_key が空かチェックしているので、ここではacquire_mutexを呼ぶだけでOK
    if acquire_mutex(&mutex_key, mutex_cache) {
        // Mutex の取得に成功（または mutex_key が空で mutex 無効の場合）
        // スコープを抜けるときに必ず release_mutex を呼ぶように設定
        defer! {
            release_mutex(&mutex_key, mutex_cache);
        }
        // コマンドを実行し、その結果を返す
        exec(cmd_info)
    } else {
        // Mutex の取得に失敗（他のスレッドが実行中）
        Ok(CommandResult {
            status: ExitStatus::default(), // スキップ時はデフォルト値
            stdout: PathBuf::new(),        // スキップ時は空のパス
            stderr: PathBuf::new(),
            skipped: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{env, time::Duration};

    use super::*;

    // 既存のテストを新しい execute_command の引数に合わせて修正
    #[test]
    fn test_execute_command_with_throttle() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let name = "test_throttle"; // 名前を変更
        let input = "input";
        let event_kind = "Create";
        let output = tmp.join(name); // 出力ディレクトリ名も変更
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "test_execute_command_throttle"] // メッセージ変更
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "test_execute_command_throttle"] // メッセージ変更
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let throttle = Duration::from_secs(1); // 閾値を短くしてテストしやすくする
        let debounce = Duration::from_millis(0);
        let limitkey_tmpl = ""; // デフォルトの limitkey を使う
        let mutex_key_tmpl = ""; // mutex は使用しない
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
                let result = execute_command(
                    &event_path,
                    event_kind,
                    &name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutex_key_tmpl, // 新しい引数
                    context,
                    &dt_cache,    // リネーム後の引数
                    &mutex_cache, // 新しい引数
                )
                .unwrap();
                result
            }));
            // スレッドを立て続けに開始して、throttle の影響が出やすいように少し待つ
            thread::sleep(Duration::from_millis(100));
        }

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // throttle の閾値内 (1秒) に3回実行しようとしている
        // 1回目は実行されるはず
        // 2回目以降は、1回目の実行から1秒経ってなければスキップされるはず
        // このテストでは、1回目の実行開始後100ms間隔で2,3回目を開始するので、
        // 2,3回目はthrottleによってスキップされることを期待する
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

        // 実行されたコマンドの確認
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // スキップされたコマンドの確認
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            // スキップされた場合は status はデフォルト値、stdout/stderr は空パス
            // original test checked status.code() == Some(0), keeping for consistency but it's brittle
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        Ok(())
    }

    // このテストは throttle の閾値よりコマンド実行時間が短い場合を想定
    #[test]
    fn test_execute_short_command_with_throttle() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_short_throttle"; // 名前変更
        let input = "input";
        let output = tmp.join(name); // 出力ディレクトリ名変更
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
        let throttle = Duration::from_millis(500); // throttle 閾値
        let debounce = Duration::from_millis(0);
        let limitkey_tmpl = "";
        let mutex_key_tmpl = ""; // mutex は使用しない
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
                let result = execute_command(
                    &event_path,
                    event_kind,
                    &name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutex_key_tmpl, // 新しい引数
                    context,
                    &dt_cache,    // リネーム後の引数
                    &mutex_cache, // 新しい引数
                )
                .unwrap();
                result
            }));
            // throttle 閾値より短い間隔でスレッドを開始
            thread::sleep(Duration::from_millis(100));
        }
        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let end = Instant::now();
        let duration = end.duration_since(start);

        // throttle 閾値 (500ms) より短い間隔 (100ms) で実行しようとしている
        // 1回目は実行される
        // 2回目は1回目から100ms後に来るので、throttle(500ms)によりスキップ
        // 3回目は2回目から100ms後に来るので、throttle(500ms)によりスキップ
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

        // 実行されたコマンドの確認
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // スキップされたコマンドの確認
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        // 実行時間は最初のコマンド実行時間(ほぼゼロ) + thread.sleep + オーバーヘッド
        // 3回のthread.sleep(100ms)があるので、合計時間はおおよそ 300ms + α
        assert!(duration >= Duration::from_millis(300));
        assert!(duration < Duration::from_millis(1000)); // 1秒以内には終わるはず

        Ok(())
    }

    #[test]
    fn test_execute_command_with_debounce() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_debounce"; // 名前変更
        let input = "input";
        let output = tmp.join(name); // 出力ディレクトリ名変更
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "/bin/sh";
        #[cfg(windows)]
        let arg = vec!["/c", "echo", "test_execute_command_debounce"] // メッセージ変更
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["-c", "echo", "test_execute_command_debounce"] // メッセージ変更
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let debounce = Duration::from_millis(500); // debounce 閾値
        let throttle = Duration::from_millis(0);
        let limitkey_tmpl = "";
        let mutex_key_tmpl = "";
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
                let result = execute_command(
                    &event_path,
                    event_kind,
                    &name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutex_key_tmpl, // 新しい引数
                    context,
                    &dt_cache,    // リネーム後の引数
                    &mutex_cache, // 新しい引数
                )
                .unwrap();
                result
            }));
            // debounce 閾値 (500ms) より短い間隔 (100ms) でスレッドを開始
            thread::sleep(Duration::from_millis(100));
        }

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // debounce 閾値 (500ms) 内に3回実行しようとしている
        // 最後のリクエストから500ms待つので、最初の2回はスキップされ、最後の1回だけ実行されるはず
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

        // 実行されたコマンドの確認
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // スキップされたコマンドの確認
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        Ok(())
    }

    // このテストは debounce の閾値よりコマンド実行時間が短い場合を想定
    #[test]
    fn test_execute_short_command_with_debounce() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_short_debounce"; // 名前変更
        let input = "input";
        let output = tmp.join(name); // 出力ディレクトリ名変更
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
        let debounce = Duration::from_millis(100); // debounce 閾値
        let throttle = Duration::from_millis(0);
        let limitkey_tmpl = "";
        let mutex_key_tmpl = ""; // mutex は使用しない
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
                let result = execute_command(
                    &event_path,
                    event_kind,
                    &name,
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    limitkey_tmpl,
                    mutex_key_tmpl, // 新しい引数
                    context,
                    &dt_cache,    // リネーム後の引数
                    &mutex_cache, // 新しい引数
                )
                .unwrap();
                result
            }));
            // debounce 閾値より短い間隔 (50ms) でスレッドを開始
            thread::sleep(Duration::from_millis(50));
        }
        // 最後のリクエストの後、debounce 閾値分待つ時間が必要
        // thread::sleep(debounce); // execute_command の中で sleep するので不要

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let end = Instant::now();
        let duration = end.duration_since(start);

        // debounce 閾値 (100ms) 内に3回実行しようとしている
        // 最後のリクエストから100ms待つので、最初の2回はスキップされ、最後の1回だけ実行されるはず
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

        // 実行されたコマンドの確認
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        assert!(!executed_result.skipped);

        // スキップされたコマンドの確認
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0));
        }

        // 実行時間は最後のリクエストが来てから debounce 閾値分待つ時間 + コマンド実行時間(ほぼゼロ) + オーバーヘッド
        // 最後のスレッドが始まってから結果が返るまでにおおよそ debounce 閾値(100ms)がかかるはず。
        // 全体としては、最初のスレッド開始から最後のスレッドが debounce wait を終えるまで。
        // 最初のスレッド開始(t=0), 次(t=50), 次(t=100). 最後のスレッドがwaitを終えるのは t=100+100=200ms.
        // 全体時間はおおよそ 200ms + α
        assert!(duration >= Duration::from_millis(200));
        assert!(duration < Duration::from_millis(500)); // 500ms 以内には終わるはず

        Ok(())
    }

    // mutex 機能のテストケース
    #[test]
    fn test_execute_command_with_mutex() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let event_path = PathBuf::from("event");
        let event_kind = "Create";
        let name = "test_mutex"; // 名前
        let input = "input";
        let output = tmp.join(name); // 出力ディレクトリ
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sleep"; // コマンド実行に時間がかかるように sleep を使う
        #[cfg(windows)]
        let arg = vec!["/c", "timeout", "/t", "2"] // 2秒 sleep
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        #[cfg(not(windows))]
        let arg = vec!["1"].into_iter().map(String::from).collect::<Vec<_>>(); // 1秒 sleep
        let debounce = Duration::from_millis(0); // debounce 無効
        let throttle = Duration::from_millis(0); // throttle 無効
        let limitkey_tmpl = ""; // limitkey は使用しない
        let mutex_key_tmpl = "my_shared_mutex_key"; // 共通の mutex キー
        let context = Context::new();
        let dt_cache = Arc::new(Mutex::new(HashMap::new())); // dummy dt cache
        let mutex_cache = Arc::new(Mutex::new(HashSet::new())); // mutex 用キャッシュ

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
            // thread ごとに名前を少し変える（ログ出力などで区別しやすくするため）
            let thread_name = format!("{}_{}", name, i);
            let mutex_key_tmpl = mutex_key_tmpl.to_string(); // clone for the thread
            let limitkey_tmpl = limitkey_tmpl.to_string(); // clone for the thread

            handles.push(thread::spawn(move || {
                info!("Thread {} trying to execute...", i);
                let result = execute_command(
                    &event_path,
                    event_kind,
                    &thread_name, // スレッドごとの名前
                    input,
                    output.to_str().unwrap(),
                    cmd,
                    arg,
                    debounce,
                    throttle,
                    &limitkey_tmpl,
                    &mutex_key_tmpl, // mutex キーテンプレートを指定
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
            // スレッドをほぼ同時に開始して、Mutex 競合を起こしやすくするために少し待つ
            thread::sleep(Duration::from_millis(50));
        }

        let results: Vec<CommandResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let end = Instant::now();
        let duration = end.duration_since(start);
        dbg!(&duration);

        // 同じ mutex_key を使っているので、同時に実行できるのは1つだけ
        // したがって、num_threads のうち1つだけが実行され、残りはスキップされるはず
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

        // 実行されたコマンドの確認
        let executed_result = results.iter().find(|r| !r.skipped).unwrap();
        assert_eq!(executed_result.status.code(), Some(0));
        // sleep コマンドは stdout/stderr には何も出さないが、exec 関数はファイルを作成するのでパスは存在するはず
        assert!(!executed_result.skipped);

        // スキップされたコマンドの確認
        for skipped_result in results.iter().filter(|r| r.skipped) {
            assert!(skipped_result.skipped);
            assert_eq!(skipped_result.status.code(), Some(0)); // スキップ時はデフォルト値
        }

        // 全体実行時間の確認
        // コマンドは1秒 sleep するので、全体時間は1秒 + オーバーヘッドになるはず
        // 5スレッドが同時に開始を試みるが、Mutex により1つずつ実行されるわけではなく、
        // 最初の1つが実行中に他の4つはスキップされるため、
        // 全体時間は N * コマンド時間 にはならない。
        assert!(duration >= Duration::from_secs(1)); // 1秒の sleep は待つ
        assert!(duration < Duration::from_secs(3)); // 5秒には満たないはず

        Ok(())
    }
}

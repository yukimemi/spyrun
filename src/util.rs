// =============================================================================
// File        : util.rs
// Author      : yukimemi
// Last Change : 2025/03/02 22:24:00.
// =============================================================================

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Command,
};

use aead::generic_array::GenericArray;
use aes_gcm_siv::{
    Aes256GcmSiv, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use log_derive::logfn;
#[cfg(windows)]
use normpath::PathExt;
use path_slash::{PathBufExt as _, PathExt as _};
use tera::{Context, Tera, Value};
use tracing::{debug, trace};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const KEY: &[u8; 32] = b"an example very very secret key.";
const NONCE: &[u8; 12] = b"unique nonce";

#[logfn(Debug)]
pub fn powershell(script: &str) -> Result<String, String> {
    let script = format!(
        "& {{ chcp 65001 | Out-Null; [Console]::OutputEncoding = [System.Text.Encoding]::GetEncoding('utf-8'); {} }}",
        &script
    );
    debug!("{:?}", &script);

    #[cfg(windows)]
    let output = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .arg("-NoProfile")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-ExecutionPolicy")
        .arg("ByPass")
        .arg("-Command")
        .arg(&script)
        .output()
        .expect("failed to execute process !");

    #[cfg(not(windows))]
    let output = Command::new("pwsh")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("ByPass")
        .arg("-Command")
        .arg(&script)
        .output()
        .expect("failed to execute process !");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    debug!(
        "status: {:?}, stdout: {:?}, stderr: {:?}",
        &output.status, &stdout, &stderr
    );
    Ok(stdout.trim().to_string())
}

#[logfn(Debug)]
pub fn powershell_file(script_path: &str) -> Result<String, String> {
    #[cfg(windows)]
    let output = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .arg("-NoProfile")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-ExecutionPolicy")
        .arg("ByPass")
        .arg("-File")
        .arg(script_path)
        .output()
        .expect("failed to execute process !");

    #[cfg(not(windows))]
    let output = Command::new("pwsh")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("ByPass")
        .arg("-File")
        .arg(script_path)
        .output()
        .expect("failed to execute process !");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    debug!(
        "status: {:?}, stdout: {:?}, stderr: {:?}",
        &output.status, &stdout, &stderr
    );
    Ok(stdout.trim().to_string())
}

#[logfn(Debug)]
pub fn insert_file_context<P: AsRef<Path>>(
    p: P,
    prefix: &str,
    context: &mut Context,
) -> Result<()> {
    let mut p = PathBuf::from(p.as_ref());
    trace!("p: {:?}", p);
    if p.is_relative() {
        p = std::env::current_dir()?.join(p);
    }
    #[cfg(windows)]
    let normpath = p.normalize_virtually()?;
    #[cfg(windows)]
    trace!("normpath: {:?}", normpath);
    #[cfg(windows)]
    let p = PathBuf::from(normpath);
    context.insert(format!("{}_path", &prefix), &p.to_slash_lossy());
    // context.insert(format!("{}_path", &prefix), &p.to_string_lossy());
    context.insert(
        format!("{}_dir", &prefix),
        &p.parent().unwrap().to_slash_lossy(),
        // &p.parent().unwrap().to_string_lossy(),
    );
    context.insert(
        format!("{}_dirname", &prefix),
        &p.parent().unwrap().file_name().unwrap().to_string_lossy(),
    );
    context.insert(
        format!("{}_name", &prefix),
        &p.file_name().unwrap().to_string_lossy(),
    );
    context.insert(
        format!("{}_stem", &prefix),
        &p.file_stem().unwrap().to_string_lossy(),
    );
    context.insert(
        format!("{}_ext", &prefix),
        &p.extension().unwrap_or_default().to_string_lossy(),
    );
    Ok(())
}

#[logfn(Debug)]
pub fn insert_default_context(context: &mut Context) {
    context.insert("spy_name", "{{ spy_name }}");
    context.insert("input", "{{ input }}");
    context.insert("output", "{{ output }}");
    context.insert("event_kind", "{{ event_kind }}");
    context.insert("event_path", "{{ event_path }}");
    context.insert("event_dir", "{{ event_dir }}");
    context.insert("event_dirname", "{{ event_dirname }}");
    context.insert("event_name", "{{ event_name }}");
    context.insert("event_stem", "{{ event_stem }}");
    context.insert("event_ext", "{{ event_ext }}");
    context.insert("stop_path", "{{ stop_path }}");
    context.insert("stop_dir", "{{ stop_dir }}");
    context.insert("stop_dirname", "{{ stop_dirname }}");
    context.insert("stop_name", "{{ stop_name }}");
    context.insert("stop_stem", "{{ stop_stem }}");
    context.insert("stop_ext", "{{ stop_ext }}");
    context.insert("stop_force_path", "{{ stop_force_path }}");
    context.insert("stop_force_dir", "{{ stop_force_dir }}");
    context.insert("stop_force_dirname", "{{ stop_force_dirname }}");
    context.insert("stop_force_name", "{{ stop_force_name }}");
    context.insert("stop_force_stem", "{{ stop_force_stem }}");
    context.insert("stop_force_ext", "{{ stop_force_ext }}");
    context.insert("log_path", "{{ log_path }}");
    context.insert("log_dir", "{{ log_dir }}");
    context.insert("log_dirname", "{{ log_dirname }}");
    context.insert("log_name", "{{ log_name }}");
    context.insert("log_stem", "{{ log_stem }}");
    context.insert("log_ext", "{{ log_ext }}");
}

#[logfn(Trace)]
pub fn render_vars(context: &mut Context, toml_str: &str) -> Result<()> {
    let toml_value: toml::Value = toml::from_str(toml_str)?;
    if let Some(vars) = toml_value.get("vars") {
        let table = vars
            .as_table()
            .ok_or_else(|| anyhow::Error::msg("Expected a table for 'vars'"))?;
        for (k, v) in table.iter() {
            let mut tera_key = new_tera("key", k)?;
            let rendered_key = tera_key.render_str(k, context)?;
            let v_str = v
                .as_str()
                .ok_or_else(|| anyhow::Error::msg("Expected a string for 'value'"))?;
            let mut tera_value = new_tera("value", v_str)?;
            let rendered_value = tera_value.render_str(v_str, context)?;
            context.insert(rendered_key, &rendered_value);
        }
    }
    Ok(())
}

#[logfn(Trace)]
pub fn new_tera(name: &str, content: &str) -> Result<Tera> {
    let mut tera = Tera::default();
    tera.add_raw_template(name, content)?;
    tera.register_function("env", env_function);
    tera.register_function("setenv", setenv_function);
    tera.register_function("enc", enc_function);
    tera.register_function("dec", dec_function);
    tera.register_function("ps", powershell_function);
    tera.register_function("psf", powershell_file_function);
    Ok(tera)
}

#[logfn(Trace)]
fn env_function(args: &HashMap<String, Value>) -> tera::Result<Value> {
    let arg = args
        .get("arg")
        .ok_or_else(|| tera::Error::msg("arg is required"))?
        .as_str()
        .unwrap();
    Ok(Value::String(env::var(arg).unwrap_or_default()))
}

fn setenv_function(args: &HashMap<String, Value>) -> tera::Result<Value> {
    if let (Some(key), Some(value)) = (args.get("key"), args.get("value")) {
        if let (Some(key_str), Some(value_str)) = (key.as_str(), value.as_str()) {
            unsafe {
                env::set_var(key_str, value_str);
            }
            return Ok(Value::String(format!("Set {} to {}", key_str, value_str)));
        }
    }
    Err("Invalid arguments".into())
}

#[logfn(Trace)]
fn enc_function(args: &HashMap<String, Value>) -> tera::Result<Value> {
    let arg = args
        .get("arg")
        .ok_or_else(|| tera::Error::msg("arg is required"))?
        .as_str()
        .unwrap();

    let bytes = arg.as_bytes();
    let key = GenericArray::from_slice(KEY);
    let cipher = Aes256GcmSiv::new(key);
    let nonce = Nonce::from_slice(NONCE);
    let ciphertext = cipher.encrypt(nonce, bytes.as_ref()).unwrap();

    Ok(Value::String(general_purpose::STANDARD.encode(ciphertext)))
}

#[logfn(Trace)]
fn dec_function(args: &HashMap<String, Value>) -> tera::Result<Value> {
    let arg = args
        .get("arg")
        .ok_or_else(|| tera::Error::msg("arg is required"))?
        .as_str()
        .unwrap();

    let bytes = general_purpose::STANDARD.decode(arg).unwrap();
    let key = GenericArray::from_slice(KEY);
    let cipher = Aes256GcmSiv::new(key);
    let nonce = Nonce::from_slice(NONCE);
    let plaintext = cipher.decrypt(nonce, bytes.as_ref()).unwrap();

    Ok(Value::String(String::from_utf8(plaintext).unwrap()))
}

#[logfn(Trace)]
fn powershell_function(args: &HashMap<String, Value>) -> tera::Result<Value> {
    let arg = args
        .get("arg")
        .ok_or_else(|| tera::Error::msg("arg is required"))?
        .as_str()
        .unwrap();

    let stdout = powershell(arg)?;

    Ok(Value::String(stdout))
}

#[logfn(Trace)]
fn powershell_file_function(args: &HashMap<String, Value>) -> tera::Result<Value> {
    let arg = args
        .get("arg")
        .ok_or_else(|| tera::Error::msg("arg is required"))?
        .as_str()
        .unwrap();

    let stdout = powershell_file(arg)?;

    Ok(Value::String(stdout))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tera::Context;

    use crate::util::new_tera;

    #[test]
    fn test_enc_dec() -> Result<()> {
        let tera = new_tera(
            "template",
            "The encrypted text of {{ name }} is {{ enc(arg='Alice') }}\nThe decrypted text of {{ enc(arg='Alice') }} is {{ dec(arg=enc(arg='Alice')) }}",
        )?;
        let mut context = Context::new();
        context.insert("name", "Alice");
        let result = tera.render("template", &context).unwrap();

        assert_eq!(
            result,
            "The encrypted text of Alice is EzB4qO+2K66gKXPBNRl7owf4EGpo\nThe decrypted text of EzB4qO+2K66gKXPBNRl7owf4EGpo is Alice"
        );
        Ok(())
    }
}

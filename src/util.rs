// =============================================================================
// File        : util.rs
// Author      : yukimemi
// Last Change : 2024/06/22 21:09:24.
// =============================================================================

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use aead::generic_array::GenericArray;
use aes_gcm_siv::{
    aead::{Aead, KeyInit},
    Aes256GcmSiv, Nonce,
};
use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use log_derive::logfn;
use path_slash::{PathBufExt as _, PathExt as _};
use tera::{Context, Tera, Value};
use tracing::trace;

const KEY: &[u8; 32] = b"an example very very secret key.";
const NONCE: &[u8; 12] = b"unique nonce";

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
    context.insert(format!("{}_path", &prefix), &p.to_slash_lossy());
    context.insert(
        format!("{}_dir", &prefix),
        &p.parent().unwrap().to_slash_lossy(),
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
    tera.register_function("enc", enc_function);
    tera.register_function("dec", dec_function);
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tera::Context;

    use crate::util::new_tera;

    #[test]
    fn test_enc_dec() -> Result<()> {
        let tera = new_tera("template", "The encrypted text of {{ name }} is {{ enc(arg='Alice') }}\nThe decrypted text of {{ enc(arg='Alice') }} is {{ dec(arg=enc(arg='Alice')) }}")?;
        let mut context = Context::new();
        context.insert("name", "Alice");
        let result = tera.render("template", &context).unwrap();

        assert_eq!(result, "The encrypted text of Alice is EzB4qO+2K66gKXPBNRl7owf4EGpo\nThe decrypted text of EzB4qO+2K66gKXPBNRl7owf4EGpo is Alice");
        Ok(())
    }
}

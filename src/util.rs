// =============================================================================
// File        : util.rs
// Author      : yukimemi
// Last Change : 2023/10/11 23:54:34.
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
use tracing::debug;

const KEY: &[u8; 32] = b"an example very very secret key.";
const NONCE: &[u8; 12] = b"unique nonce";

#[logfn(Debug)]
pub fn insert_file_context<P: AsRef<Path>>(
    p: P,
    prefix: &str,
    context: &mut Context,
) -> Result<()> {
    let mut p = PathBuf::from(p.as_ref());
    debug!("p: {:?}", p);
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

    use crate::new_tera;

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

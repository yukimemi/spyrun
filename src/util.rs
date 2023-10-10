// =============================================================================
// File        : util.rs
// Author      : yukimemi
// Last Change : 2023/10/10 17:25:44.
// =============================================================================

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::Result;
use log_derive::logfn;
use path_slash::{PathBufExt as _, PathExt as _};
use tera::{Context, Tera, Value};
use tracing::debug;

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
    tera.register_function("env", |args: &HashMap<String, Value>| {
        let name = match args.get("name") {
            Some(val) => val.as_str().unwrap(),
            None => return Err("name is required".into()),
        };
        Ok(Value::String(env::var(name).unwrap_or_default()))
    });
    Ok(tera)
}

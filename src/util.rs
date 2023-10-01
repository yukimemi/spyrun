// =============================================================================
// File        : util.rs
// Author      : yukimemi
// Last Change : 2023/10/01 17:04:57.
// =============================================================================

use std::path::{Path, PathBuf};

use anyhow::Result;
use log_derive::logfn;
use tera::Context;
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
    context.insert(format!("{}_path", &prefix), &p.to_string_lossy());
    context.insert(
        format!("{}_dir", &prefix),
        &p.parent().unwrap().to_string_lossy(),
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

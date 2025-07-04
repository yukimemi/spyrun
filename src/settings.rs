// =============================================================================
// File        : settings.rs
// Author      : yukimemi
// Last Change : 2025/04/27 16:41:48.
// =============================================================================

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use log_derive::logfn;
use notify::RecursiveMode;
use serde::{Deserialize, Deserializer};
use tera::Context;
use tracing::error;

use crate::util::{insert_default_context, insert_file_context, new_tera, render_vars};

#[derive(Debug, Deserialize, Clone)]
pub struct Poll {
    pub interval: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Walk {
    pub min_depth: Option<usize>,
    pub max_depth: Option<usize>,
    pub follow_symlinks: Option<bool>,
    pub pattern: Option<String>,
    pub delay: Option<(u64, Option<u64>)>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Init {
    pub cmd: String,
    pub arg: Vec<String>,
    #[serde(default)]
    pub error_stop: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Spy {
    pub name: String,
    #[serde(default, deserialize_with = "is_valid_event_kind")]
    pub events: Option<Vec<String>>,
    pub input: Option<String>,
    pub output: Option<String>,
    #[serde(
        default = "default_recursive",
        deserialize_with = "deserialize_recursive_mode"
    )]
    pub recursive: RecursiveMode,
    pub throttle: Option<u64>,
    pub debounce: Option<u64>,
    pub limitkey: Option<String>,
    pub mutexkey: Option<String>,
    pub patterns: Option<Vec<Pattern>>,
    pub delay: Option<(u64, Option<u64>)>,
    pub poll: Option<Poll>,
    pub walk: Option<Walk>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Log {
    pub path: String,
    #[serde(default = "default_loglevel")]
    pub level: String,
    #[serde(default)]
    pub switch: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Cfg {
    pub stop_flg: String,
    pub stop_force_flg: Option<String>,
    pub max_threads: Option<usize>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Pattern {
    pub pattern: String,
    pub cmd: String,
    pub arg: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub log: Log,
    pub cfg: Cfg,
    pub init: Option<Init>,
    pub spys: Vec<Spy>,
}

impl Settings {
    #[logfn(Debug)]
    pub fn new<P: AsRef<Path>>(cfg: P, backup: bool, context: &mut Context) -> Result<Self> {
        insert_file_context(&cfg, "cfg", context)?;
        insert_default_context(context);

        let toml_str = std::fs::read_to_string(&cfg)?;
        let tera = new_tera(&cfg.as_ref().to_string_lossy(), &toml_str)?;
        render_vars(context, &toml_str)?;
        let toml_str = tera.render(&cfg.as_ref().to_string_lossy(), context)?;
        match toml::from_str(&toml_str) {
            Ok(s) => {
                if backup {
                    Settings::backup(&cfg)?;
                }
                Ok(s)
            }
            Err(e) => Err(anyhow!("Failed to parse settings.toml. {:?}", e)),
        }
    }

    #[tracing::instrument]
    #[logfn(Debug)]
    pub fn rebuild(&self) -> Settings {
        let default_spy = Spy::default();
        let default_spy = self
            .spys
            .iter()
            .find(|spy| spy.name == "default")
            .unwrap_or(&default_spy);

        let spys = self
            .spys
            .iter()
            .map(|spy| {
                if spy.name == "default" {
                    spy.clone()
                } else {
                    Spy {
                        name: spy.name.clone(),
                        events: spy.events.clone().or(default_spy.events.clone()),
                        input: spy.input.clone().or(default_spy.input.clone()),
                        output: spy.output.clone().or(default_spy.output.clone()),
                        recursive: spy.recursive,
                        throttle: spy.throttle.or(default_spy.throttle),
                        debounce: spy.debounce.or(default_spy.debounce),
                        limitkey: spy.limitkey.clone().or(default_spy.limitkey.clone()),
                        mutexkey: spy.mutexkey.clone().or(default_spy.mutexkey.clone()),
                        patterns: spy.patterns.clone().or(default_spy.patterns.clone()),
                        delay: spy.delay.or(default_spy.delay),
                        poll: spy.poll.clone().or(default_spy.poll.clone()),
                        walk: spy.walk.clone().or(default_spy.walk.clone()),
                    }
                }
            })
            .collect();

        Settings {
            log: self.log.clone(),
            cfg: self.cfg.clone(),
            init: self.init.clone(),
            spys,
        }
    }

    #[logfn(Debug)]
    pub fn backup_path<P: AsRef<Path>>(cfg: P) -> PathBuf {
        let cfg_path = PathBuf::from(cfg.as_ref());
        let new_basename = cfg_path.file_stem().unwrap().to_string_lossy().to_string() + "_backup";

        cfg_path
            .with_file_name(new_basename)
            .with_extension(cfg_path.extension().unwrap())
    }

    #[logfn(Debug)]
    pub fn backup<P: AsRef<Path>>(cfg: P) -> Result<()> {
        let backup_path = Settings::backup_path(&cfg);
        fs::copy(Path::new(cfg.as_ref()), backup_path).unwrap_or_else(|e| {
            error!("{}", e);
            1
        });
        Ok(())
    }
}

impl Default for Spy {
    #[tracing::instrument]
    #[logfn(Debug)]
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            events: Some(vec!["Create".to_string(), "Modify".to_string()]),
            input: Some("input".to_string()),
            output: Some("output".to_string()),
            recursive: RecursiveMode::Recursive,
            throttle: Some(0),
            debounce: Some(0),
            limitkey: Some("".to_string()),
            mutexkey: Some("".to_string()),
            patterns: Some(vec![
                Pattern {
                    pattern: "\\.ps1$".to_string(),
                    cmd: "powershell".to_string(),
                    arg: [
                        "-NoProfile",
                        "-ExecutionPolicy",
                        "ByPass",
                        "-File",
                        "{{event_path}}",
                    ]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                },
                Pattern {
                    pattern: "\\.cmd$".to_string(),
                    cmd: "{{event_path}}".to_string(),
                    arg: vec![],
                },
                Pattern {
                    pattern: "\\.bat$".to_string(),
                    cmd: "{{event_path}}".to_string(),
                    arg: vec![],
                },
                Pattern {
                    pattern: "\\.sh$".to_string(),
                    cmd: "bash".to_string(),
                    arg: ["-c", "{{event_path}}"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                },
            ]),
            delay: None,
            poll: None,
            walk: None,
        }
    }
}

#[logfn(Debug)]
fn is_valid_event_kind<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    let opt = Option::<Vec<String>>::deserialize(d)?;
    if let Some(v) = opt {
        let valid = v.iter().all(|s| {
            matches!(
                s.as_str(),
                "Access" | "Create" | "Modify" | "Remove" | "Any"
            )
        });
        if valid {
            Ok(Some(v))
        } else {
            Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Seq,
                &"events must be Access, Create, Modify, Remove or Any",
            ))
        }
    } else {
        Ok(None)
    }
}

#[logfn(Debug)]
fn deserialize_recursive_mode<'de, D: Deserializer<'de>>(d: D) -> Result<RecursiveMode, D::Error> {
    let recurse = bool::deserialize(d)?;
    if recurse {
        Ok(RecursiveMode::Recursive)
    } else {
        Ok(RecursiveMode::NonRecursive)
    }
}

#[logfn(Debug)]
fn default_recursive() -> RecursiveMode {
    RecursiveMode::NonRecursive
}

#[logfn(Debug)]
fn default_loglevel() -> String {
    "info".to_string()
}

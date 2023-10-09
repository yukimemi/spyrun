// =============================================================================
// File        : settings.rs
// Author      : yukimemi
// Last Change : 2023/10/09 20:32:23.
// =============================================================================

use std::{collections::HashMap, path::Path};

use anyhow::Result;
use log_derive::logfn;
use notify::RecursiveMode;
use serde::{Deserialize, Deserializer};
use tera::Context;

use crate::util::{insert_file_context, new_tera};

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
    pub debounce: Option<u64>,
    pub patterns: Option<Vec<Pattern>>,
    pub delay: Option<(u64, Option<u64>)>,
    pub poll: Option<Poll>,
    pub walk: Option<Walk>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Vars {
    pub vars: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Log {
    pub path: String,
    pub level: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Cfg {
    pub stop_flg: String,
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
    pub spys: Vec<Spy>,
}

impl Settings {
    #[logfn(Debug)]
    pub fn new<P: AsRef<Path>>(cfg: P, context: &mut Context) -> Result<Self> {
        insert_file_context(&cfg, "cfg", context)?;

        let toml_str = std::fs::read_to_string(&cfg)?;
        let tera = new_tera(&cfg.as_ref().to_string_lossy(), &toml_str)?;
        context.insert("input", "{{ input }}");
        context.insert("output", "{{ output }}");
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

        let toml_value: toml::Value = toml::from_str(&toml_str)?;
        if let Some(vars) = toml_value.get("vars") {
            vars.as_table().unwrap().iter().for_each(|(k, v)| {
                let mut tera = new_tera("key", k).unwrap();
                let k = tera.render_str(k, context).unwrap();
                let v_str = v.as_str().unwrap();
                let mut tera = new_tera("value", v_str).unwrap();
                let v = tera.render_str(v_str, context).unwrap();
                context.insert(k, &v);
            })
        }

        let toml_str = tera.render(&cfg.as_ref().to_string_lossy(), context)?;

        Ok(toml::from_str(&toml_str)?)
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
                        debounce: spy.debounce.or(default_spy.debounce),
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
            spys,
        }
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
            debounce: Some(500),
            patterns: Some(vec![
                Pattern {
                    pattern: "\\.ps1$".to_string(),
                    cmd: "powershell".to_string(),
                    arg: ["-NoProfile", "-File", "{{event_path}}"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                },
                Pattern {
                    pattern: "\\.cmd$".to_string(),
                    cmd: "cmd".to_string(),
                    arg: ["/c", "{{event_path}}"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                },
                Pattern {
                    pattern: "\\.bat$".to_string(),
                    cmd: "cmd".to_string(),
                    arg: ["/c", "{{event_path}}"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
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

// =============================================================================
// File        : settings.rs
// Author      : yukimemi
// Last Change : 2023/10/01 16:46:04.
// =============================================================================

use std::{collections::HashMap, env, path::Path};

use anyhow::Result;
use log_derive::logfn;
use serde::{Deserialize, Deserializer};
use tera::{Context, Tera, Value};

use super::util::insert_file_context;

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
pub struct Pattern {
    pub pattern: String,
    pub cmd: String,
    pub arg: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Spy {
    pub name: String,
    #[serde(default, deserialize_with = "is_valid_event_kind")]
    pub events: Option<Vec<String>>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub patterns: Option<Vec<Pattern>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub log: Log,
    pub spys: Vec<Spy>,
}

impl Settings {
    #[logfn(Debug)]
    pub fn new<P: AsRef<Path>>(cfg: P, context: &mut Context) -> Result<Self> {
        insert_file_context(&cfg, "cfg", context)?;

        let mut tera = Tera::default();
        let toml_str = std::fs::read_to_string(&cfg)?;
        tera.add_raw_template(&cfg.as_ref().to_string_lossy(), &toml_str)?;
        tera.register_function("env", |args: &HashMap<String, Value>| {
            let name = match args.get("name") {
                Some(val) => val.as_str().unwrap(),
                None => return Err("name is required".into()),
            };
            Ok(Value::String(env::var(name).unwrap_or_default()))
        });
        context.insert("input", "{{ input }}");
        context.insert("output", "{{ output }}");
        context.insert("event_path", "{{ event_path }}");

        let toml_value: toml::Value = toml::from_str(&toml_str)?;
        if let Some(vars) = toml_value.get("vars") {
            vars.as_table().unwrap().iter().for_each(|(k, v)| {
                context.insert(k, v);
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
                        patterns: spy
                            .patterns
                            .clone()
                            .or_else(|| default_spy.patterns.clone()),
                    }
                }
            })
            .collect();

        Settings {
            log: self.log.clone(),
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
            patterns: Some(vec![
                Pattern {
                    pattern: "\\.ps1$".to_string(),
                    cmd: "powershell".to_string(),
                    arg: ["-NoProfile", "-File", "{{input}}"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                },
                Pattern {
                    pattern: "\\.cmd$".to_string(),
                    cmd: "cmd".to_string(),
                    arg: ["/c", "{{input}}"].iter().map(|s| s.to_string()).collect(),
                },
                Pattern {
                    pattern: "\\.bat$".to_string(),
                    cmd: "cmd".to_string(),
                    arg: ["/c", "{{input}}"].iter().map(|s| s.to_string()).collect(),
                },
                Pattern {
                    pattern: "\\.sh$".to_string(),
                    cmd: "bash".to_string(),
                    arg: ["-c", "{{input}}"].iter().map(|s| s.to_string()).collect(),
                },
            ]),
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

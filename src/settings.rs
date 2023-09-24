// =============================================================================
// File        : settings.rs
// Author      : yukimemi
// Last Change : 2023/09/24 22:20:34.
// =============================================================================

use std::path::Path;

use config::{Config, ConfigError, Environment, File};
use log_derive::logfn;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize, Clone)]
pub struct Log {
    pub path: String,
    pub level: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Pattern {
    pub extension: String,
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
    #[logfn(Info)]
    pub fn new<P: AsRef<Path>>(cfg: P) -> Result<Self, ConfigError> {
        let s = Config::builder()
            .add_source(File::with_name(cfg.as_ref().to_str().unwrap()))
            .add_source(File::with_name("local").required(false))
            .add_source(Environment::with_prefix("app"))
            .build()?;

        s.try_deserialize()
    }

    #[tracing::instrument]
    #[logfn(Info)]
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
    #[logfn(Info)]
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            events: Some(vec!["Create".to_string(), "Modify".to_string()]),
            input: Some("input".to_string()),
            output: Some("output".to_string()),
            patterns: Some(vec![
                Pattern {
                    extension: "ps1".to_string(),
                    cmd: "powershell".to_string(),
                    arg: ["-NoProfile", "-File", "{{input}}"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                },
                Pattern {
                    extension: "cmd".to_string(),
                    cmd: "cmd".to_string(),
                    arg: ["/c", "{{input}}"].iter().map(|s| s.to_string()).collect(),
                },
                Pattern {
                    extension: "bat".to_string(),
                    cmd: "cmd".to_string(),
                    arg: ["/c", "{{input}}"].iter().map(|s| s.to_string()).collect(),
                },
                Pattern {
                    extension: "sh".to_string(),
                    cmd: "bash".to_string(),
                    arg: ["-c", "{{input}}"].iter().map(|s| s.to_string()).collect(),
                },
            ]),
        }
    }
}

#[logfn(Info)]
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

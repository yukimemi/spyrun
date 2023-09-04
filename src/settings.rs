use config::{Config, ConfigError, Environment, File};
use serde_derive::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct Log {
    pub path: String,
    pub time_format: String,
    pub level: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Pattern {
    pub extension: String,
    pub cmd: String,
    pub arg: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct All {
    pub patterns: Option<Vec<Pattern>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Spy {
    pub input: String,
    pub output: String,
    pub patterns: Option<Vec<Pattern>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub log: Log,
    pub all: Option<All>,
    pub spys: Vec<Spy>,
}

impl Settings {
    pub fn new<P: AsRef<Path>>(cfg: P) -> Result<Self, ConfigError> {
        let s = Config::builder()
            .add_source(File::with_name(cfg.as_ref().to_str().unwrap()))
            .add_source(File::with_name("local").required(false))
            .add_source(Environment::with_prefix("app"))
            .build()?;

        s.try_deserialize()
    }
}

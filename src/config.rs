use std::{collections::BTreeMap, fs::create_dir_all, path::PathBuf};

use directories::ProjectDirs;
use tracing::debug;
use serde::{Deserialize, Serialize};

use crate::{errors::Error, utils::abort};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WindowMatcher {
    Process,
    Class,
}

impl Default for WindowMatcher {
    fn default() -> Self {
        Self::Process
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Instance {
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) matcher: WindowMatcher,
    #[serde(default)]
    pub(crate) class_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct QuropConfig {
    pub(crate) instances: BTreeMap<String, Instance>,
}

fn get_config_path() -> Result<PathBuf, Error> {
    let dir = ProjectDirs::from("net", "arusahni", "qurop")
        .expect("could not find project dirs")
        .config_dir()
        .to_path_buf();
    debug!("Config dir: {:?}", dir);
    create_dir_all(&dir)?;
    Ok(dir.join("config.toml"))
}

pub fn get_config() -> Result<QuropConfig, Error> {
    let file_path = get_config_path()?;
    let cfg: QuropConfig = confy::load_path(file_path)?;
    Ok(cfg)
}

pub fn add_instance(
    name: &str,
    command: &str,
    matcher: WindowMatcher,
    class_name: Option<String>,
) -> Result<(), Error> {
    let mut cfg = get_config()?;
    if cfg.instances.contains_key(name) {
        abort(&format!("{name} already exists"));
    }
    cfg.instances.insert(
        name.into(),
        Instance {
            command: command.into(),
            matcher,
            class_name,
        },
    );
    let file_path = get_config_path()?;
    confy::store_path(file_path, cfg)?;
    Ok(())
}

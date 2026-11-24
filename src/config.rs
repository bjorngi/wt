//! Optional `$WT_ROOT/.wt.toml`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::root::Root;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Preset name -> repo names.
    #[serde(default)]
    pub presets: BTreeMap<String, Vec<String>>,
}

impl Config {
    pub fn load(root: &Root) -> Result<Config> {
        let path = root.path.join(".wt.toml");
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("invalid {}", crate::ui::tilde(&path)))
    }
}

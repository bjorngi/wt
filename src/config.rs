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
    #[serde(default)]
    pub github: GithubConfig,
}

/// `[github]`: pull request and checks status via the `gh` CLI.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubConfig {
    /// `None` = on when `gh` is installed.
    pub enabled: Option<bool>,
    /// Hosts handled by this provider (add GitHub Enterprise hosts here).
    #[serde(default = "default_github_hosts")]
    pub hosts: Vec<String>,
}

impl Default for GithubConfig {
    fn default() -> Self {
        GithubConfig { enabled: None, hosts: default_github_hosts() }
    }
}

fn default_github_hosts() -> Vec<String> {
    vec!["github.com".to_string()]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_section() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.github.enabled, None);
        assert_eq!(c.github.hosts, vec!["github.com"]);
        let c: Config = toml::from_str("[github]\nenabled = false\n").unwrap();
        assert_eq!(c.github.enabled, Some(false));
        assert_eq!(c.github.hosts, vec!["github.com"]);
        let c: Config = toml::from_str("[github]\nhosts = [\"github.com\", \"git.corp\"]\n").unwrap();
        assert_eq!(c.github.hosts, vec!["github.com", "git.corp"]);
        assert!(toml::from_str::<Config>("[github]\ntypo = 1\n").is_err());
    }
}

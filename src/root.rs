//! `$WT_ROOT` and the paths inside it.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::error::hinted;
use crate::ui::tilde;

pub struct Root {
    pub path: PathBuf,
}

impl Root {
    pub fn from_env() -> Result<Root> {
        let raw = std::env::var_os("WT_ROOT")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| hinted("WT_ROOT is not set", "export WT_ROOT=~/wt (in your .zshrc)"))?;
        let path = expand_tilde(PathBuf::from(raw));
        if !path.is_dir() {
            return Err(hinted(
                format!("WT_ROOT ({}) does not exist", tilde(&path)),
                format!("mkdir -p {}", tilde(&path)),
            ));
        }
        // Canonical, so cwd comparisons work (e.g. /var vs /private/var on macOS).
        let path = path.canonicalize()?;
        Ok(Root { path })
    }

    pub fn base(&self) -> PathBuf {
        self.path.join("base")
    }

    pub fn worktrees(&self) -> PathBuf {
        self.path.join("worktrees")
    }

    pub fn repo_path(&self, name: &str) -> PathBuf {
        self.base().join(name)
    }

    /// Repos = dirs in `base/` that contain a `.git` dir. Sorted by name.
    pub fn repos(&self) -> Result<Vec<String>> {
        let mut repos = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.base()) else {
            return Ok(repos);
        };
        for entry in entries {
            let entry = entry?;
            if entry.path().join(".git").is_dir() {
                repos.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        repos.sort();
        Ok(repos)
    }

    /// Like `repos()`, but an empty list is an error with a hint on how to add one.
    pub fn require_repos(&self) -> Result<Vec<String>> {
        let repos = self.repos()?;
        if repos.is_empty() {
            return Err(hinted(
                format!("no repos in {}", tilde(&self.base())),
                format!("git clone <url> {}/<name>", tilde(&self.base())),
            ));
        }
        Ok(repos)
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    path
}

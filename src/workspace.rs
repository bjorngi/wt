//! Workspaces: `$WT_ROOT/worktrees/<feature>/`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::error::{hinted, user};
use crate::root::Root;

pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
}

/// Flat dir name for a feature: `/` and anything outside `[A-Za-z0-9._-]` become `-`.
pub fn sanitize(feature: &str) -> Result<String> {
    let name: String = feature
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    if name.is_empty() || name.starts_with('.') || name.chars().all(|c| c == '-') {
        return Err(user(format!("invalid feature name '{feature}'")));
    }
    Ok(name)
}

pub fn list(root: &Root) -> Result<Vec<Workspace>> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.worktrees()) else {
        return Ok(out);
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.path().is_dir() {
            continue;
        }
        out.push(Workspace { name, path: entry.path() });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The workspace for `feature` (either form: `feature/login` or `feature-login`), if it exists.
pub fn find(root: &Root, feature: &str) -> Result<Option<Workspace>> {
    let name = sanitize(feature)?;
    let path = root.worktrees().join(&name);
    Ok(path.is_dir().then_some(Workspace { name, path }))
}

/// Like `find`, but a missing workspace is an error.
pub fn require(root: &Root, feature: &str) -> Result<Workspace> {
    find(root, feature)?.ok_or_else(|| hinted(format!("no workspace '{feature}'"), "see existing ones with: wt ls"))
}

pub fn is_worktree(path: &Path) -> bool {
    path.join(".git").is_file()
}

/// Repo names (dir names) of the worktrees in a workspace. Sorted.
pub fn worktrees(ws: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(ws)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| is_worktree(&e.path()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// Entries in a workspace that aren't worktrees (e.g. notes you put there). Ignores `.DS_Store`.
pub fn other_entries(ws: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(ws)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !is_worktree(&e.path()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".DS_Store")
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("feature/login").unwrap(), "feature-login");
        assert_eq!(sanitize("fix npe#2").unwrap(), "fix-npe-2");
        assert_eq!(sanitize("v1.2_x").unwrap(), "v1.2_x");
        assert_eq!(sanitize("æøå").unwrap_err().to_string(), "invalid feature name 'æøå'");
        assert!(sanitize("").is_err());
        assert!(sanitize(".hidden").is_err());
        assert!(sanitize("..").is_err());
        assert!(sanitize("/").is_err());
    }
}

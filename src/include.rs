//! Copy files listed in `base/<repo>/.worktreeinclude` into a worktree.

use std::path::Path;

use anyhow::Result;

use crate::git;

#[derive(Default)]
pub struct Copied {
    pub copied: usize,
    /// Matched `.worktreeinclude` but aren't gitignored, so they were not copied.
    pub not_ignored: Vec<String>,
}

impl Copied {
    pub fn describe(&self) -> Option<String> {
        match self.copied {
            0 => None,
            1 => Some("copied 1 file".to_string()),
            n => Some(format!("copied {n} files")),
        }
    }
}

pub fn copy(base: &Path, worktree: &Path) -> Result<Copied> {
    let include = base.join(".worktreeinclude");
    if !include.is_file() {
        return Ok(Copied::default());
    }
    let exclude_from = format!("--exclude-from={}", include.display());
    // Untracked files matching the patterns. (Not `--directory`: that also returns untracked
    // parent dirs that merely contain a match.)
    let matched = git::split_nul(&git::run_raw(
        base,
        &["ls-files", "-z", "--others", "--ignored", &exclude_from],
    )?);
    if matched.is_empty() {
        return Ok(Copied::default());
    }

    // Only copy files that are also gitignored, so a secret can't end up committed.
    let mut input = Vec::new();
    for f in &matched {
        input.extend_from_slice(f.as_bytes());
        input.push(0);
    }
    let ignored = git::split_nul(&git::run_stdin(
        base,
        &["check-ignore", "-z", "--stdin"],
        &input,
        &[0, 1],
    )?);

    let mut result = Copied::default();
    for rel in matched {
        if !ignored.contains(&rel) {
            result.not_ignored.push(rel);
            continue;
        }
        let dest = worktree.join(&rel);
        if dest.symlink_metadata().is_ok() {
            continue; // never overwrite
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(base.join(&rel), &dest)?;
        result.copied += 1;
    }
    Ok(result)
}

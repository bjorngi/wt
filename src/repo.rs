//! Base repos and worktree creation/removal.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::error::hinted;
use crate::git;
use crate::ui::tilde;

/// Default branch, from `origin/HEAD` (not `HEAD`, which is whatever base has checked out).
pub fn default_branch(repo: &Path) -> String {
    if let Some(head) = git::opt(repo, &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]) {
        if let Some(name) = head.strip_prefix("origin/") {
            return name.to_string();
        }
    }
    if !ref_exists(repo, "refs/heads/main") && ref_exists(repo, "refs/heads/master") {
        return "master".to_string();
    }
    "main".to_string()
}

pub fn current_branch(dir: &Path) -> Option<String> {
    git::opt(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
}

pub fn ref_exists(repo: &Path, refname: &str) -> bool {
    git::ok(repo, &["rev-parse", "--verify", "--quiet", refname])
}

/// The base repo a worktree belongs to (parent of its common git dir).
pub fn base_of(worktree: &Path) -> Option<PathBuf> {
    let common = git::opt(worktree, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    PathBuf::from(common).parent().map(Path::to_path_buf)
}

pub fn fetch(repo: &Path) -> Result<()> {
    git::run(repo, &["fetch", "--prune", "--quiet", "origin"])?;
    Ok(())
}

/// Fetch and fast-forward base's default branch. Fails (with a hint) if base isn't a clean default branch.
pub fn update_base(repo: &Path) -> Result<()> {
    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    fetch(repo)?;
    let default = default_branch(repo);
    let fix = format!("cd {}", tilde(repo));
    match current_branch(repo) {
        Some(b) if b == default => {}
        Some(b) => {
            return Err(hinted(
                format!("base/{name} is on branch '{b}', expected '{default}'"),
                format!("{fix} && git switch {default}"),
            ));
        }
        None => {
            return Err(hinted(
                format!("base/{name} has a detached HEAD, expected '{default}'"),
                format!("{fix} && git switch {default}"),
            ));
        }
    }
    let changes = git::run(repo, &["status", "--porcelain", "--untracked-files=no"])?;
    if !changes.is_empty() {
        return Err(hinted(
            format!("base/{name} has uncommitted changes to tracked files"),
            format!("{fix} && git status"),
        ));
    }
    let upstream = format!("origin/{default}");
    if !ref_exists(repo, &upstream) {
        return Err(hinted(format!("base/{name}: {upstream} not found"), format!("{fix} && git remote -v")));
    }
    if !git::ok(repo, &["merge", "--ff-only", "--quiet", &upstream]) {
        return Err(hinted(
            format!("base/{name}: local {default} has diverged from {upstream}"),
            format!("{fix} && git log --oneline {upstream}..{default}"),
        ));
    }
    Ok(())
}

/// Branches checked out in any worktree of `repo` (base included): (path, branch).
pub fn checked_out(repo: &Path) -> Result<Vec<(PathBuf, String)>> {
    let out = git::run(repo, &["worktree", "list", "--porcelain"])?;
    let mut result = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            if let Some(p) = &path {
                result.push((p.clone(), b.to_string()));
            }
        }
    }
    Ok(result)
}

pub fn valid_branch_name(repo: &Path, branch: &str) -> bool {
    git::ok(repo, &["check-ref-format", "--branch", branch])
}

pub enum Created {
    /// New branch from the (freshly updated) default branch.
    NewBranch { from: String },
    /// New local branch tracking `origin/<b>`.
    Tracking,
    /// Existing local branch.
    Existing,
}

impl Created {
    pub fn describe(&self, branch: &str) -> String {
        match self {
            Created::NewBranch { from } => format!("new branch {branch} from {from}"),
            Created::Tracking => format!("tracking origin/{branch}"),
            Created::Existing => format!("existing branch {branch}"),
        }
    }
}

/// `git worktree add` with the right strategy for `branch`.
pub fn create_worktree(repo: &Path, path: &Path, branch: &str) -> Result<Created> {
    let path_str = path.to_string_lossy();
    let remote = format!("refs/remotes/origin/{branch}");
    let created = if ref_exists(repo, &format!("refs/heads/{branch}")) {
        git::run(repo, &["worktree", "add", "--quiet", &path_str, branch])?;
        Created::Existing
    } else if ref_exists(repo, &remote) {
        let start = format!("origin/{branch}");
        git::run(repo, &["worktree", "add", "--quiet", "--track", "-b", branch, &path_str, &start])?;
        Created::Tracking
    } else {
        let default = default_branch(repo);
        git::run(repo, &["worktree", "add", "--quiet", "--no-track", "-b", branch, &path_str, &default])?;
        Created::NewBranch { from: default }
    };
    let has_upstream = git::ok(path, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]);
    if !has_upstream && ref_exists(repo, &remote) {
        let upstream = format!("origin/{branch}");
        git::run(path, &["branch", "--quiet", &format!("--set-upstream-to={upstream}")])?;
    }
    Ok(created)
}

/// Remove a worktree. Falls back to deleting the dir and pruning git's metadata.
pub fn remove_worktree(base: Option<&Path>, path: &Path) -> Result<()> {
    if let Some(base) = base {
        let path_str = path.to_string_lossy();
        if git::ok(base, &["worktree", "remove", "--force", &path_str]) {
            return Ok(());
        }
    }
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    if let Some(base) = base {
        let _ = git::run(base, &["worktree", "prune"]);
    }
    Ok(())
}

/// Fetch several repos in parallel (`Fetching a, b… ✓`). Returns the ones that failed, after warning about them.
pub fn fetch_all(repos: &std::collections::BTreeMap<String, PathBuf>) -> Vec<PathBuf> {
    if repos.is_empty() {
        return Vec::new();
    }
    let names: Vec<&str> = repos.keys().map(String::as_str).collect();
    crate::ui::step(&format!("Fetching {}", names.join(", ")));
    let results: Vec<(&String, &PathBuf, Result<()>)> = std::thread::scope(|s| {
        let handles: Vec<_> = repos.iter().map(|(n, p)| (n, p, s.spawn(move || fetch(p)))).collect();
        handles.into_iter().map(|(n, p, h)| (n, p, h.join().expect("fetch thread panicked"))).collect()
    });
    let failed: Vec<_> = results.iter().filter(|r| r.2.is_err()).collect();
    crate::ui::step_done(failed.is_empty());
    let width = crate::ui::width(failed.iter().map(|r| r.0.as_str()));
    for (name, _, res) in &failed {
        if let Err(e) = res {
            crate::ui::warn(name, width, &format!("fetch failed: {}", crate::error_text(e)));
        }
    }
    failed.into_iter().map(|r| r.1.clone()).collect()
}

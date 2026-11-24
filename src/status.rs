//! Worktree status: safety checks (rm, cleanup) and the `ls` row.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::git;
use crate::repo::{base_of, current_branch, default_branch, ref_exists};

/// What would be lost by deleting the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Commits {
    /// Everything is on the upstream.
    Pushed,
    /// The upstream was configured but the remote branch is deleted (typically merged).
    Gone,
    /// Never pushed, and no commits beyond the default branch.
    Unchanged,
    /// Commits not on the (existing) upstream.
    Unpushed(u32),
    /// Never pushed, with commits beyond the default branch.
    NeverPushed(u32),
    /// Detached HEAD with commits not on any remote branch.
    Detached(u32),
}

impl Commits {
    /// The branch can be deleted without losing commits.
    pub fn safe(&self) -> bool {
        matches!(self, Commits::Pushed | Commits::Gone | Commits::Unchanged | Commits::Detached(0))
    }

    /// Warning text, if this needs attention.
    pub fn warning(&self) -> Option<String> {
        match self {
            Commits::Unpushed(n) => Some(format!("{} not pushed", plural(*n, "commit"))),
            Commits::NeverPushed(n) => Some(format!("{} never pushed", plural(*n, "commit"))),
            Commits::Detached(n) if *n > 0 => Some(format!("detached HEAD with {} not on any remote", plural(*n, "commit"))),
            _ => None,
        }
    }
}

pub struct Check {
    pub repo: String,
    pub path: PathBuf,
    pub base: Option<PathBuf>,
    pub branch: Option<String>,
    pub uncommitted: u32,
    pub commits: Commits,
}

impl Check {
    /// Warnings that should make `rm` ask before removing.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.uncommitted > 0 {
            out.push(format!("{} uncommitted", plural(self.uncommitted, "change")));
        }
        out.extend(self.commits.warning());
        out
    }

    /// Finished: nothing to lose, and (for cleanup) the remote branch is gone or there was nothing to push.
    pub fn done(&self) -> bool {
        self.uncommitted == 0 && matches!(self.commits, Commits::Gone | Commits::Unchanged)
    }
}

pub fn plural(n: u32, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

fn count(dir: &Path, range: &str) -> u32 {
    git::opt(dir, &["rev-list", "--count", range]).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// `refs/remotes/origin/<x>` for the configured upstream of `branch`, if one is configured on origin.
fn configured_upstream(dir: &Path, branch: &str) -> Option<String> {
    let remote = git::opt(dir, &["config", "--get", &format!("branch.{branch}.remote")])?;
    let merge = git::opt(dir, &["config", "--get", &format!("branch.{branch}.merge")])?;
    let name = merge.strip_prefix("refs/heads/")?;
    (remote == "origin").then(|| format!("refs/remotes/origin/{name}"))
}

pub fn check(repo: &str, path: &Path) -> Check {
    let base = base_of(path);
    let branch = current_branch(path);
    let uncommitted = git::opt(path, &["status", "--porcelain"]).map(|s| s.lines().count() as u32).unwrap_or(0);

    let default = base.as_deref().map(default_branch).unwrap_or_else(|| "main".to_string());
    let main_ref = if ref_exists(path, &format!("refs/remotes/origin/{default}")) {
        format!("origin/{default}")
    } else {
        default.clone()
    };

    let commits = match &branch {
        None => Commits::Detached(
            git::opt(path, &["rev-list", "--count", "HEAD", "--not", "--remotes"])
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        ),
        Some(b) => {
            let remote_ref = format!("refs/remotes/origin/{b}");
            match configured_upstream(path, b) {
                Some(up) if ref_exists(path, &up) => match count(path, &format!("{up}..HEAD")) {
                    0 => Commits::Pushed,
                    n => Commits::Unpushed(n),
                },
                Some(_) => Commits::Gone,
                None if ref_exists(path, &remote_ref) => match count(path, &format!("{remote_ref}..HEAD")) {
                    0 => Commits::Pushed,
                    n => Commits::Unpushed(n),
                },
                None => match count(path, &format!("{main_ref}..HEAD")) {
                    0 => Commits::Unchanged,
                    n => Commits::NeverPushed(n),
                },
            }
        }
    };

    Check { repo: repo.to_string(), path: path.to_path_buf(), base, branch, uncommitted, commits }
}

/// One `wt ls` row.
pub struct Row {
    pub repo: String,
    pub branch: String,
    pub is_default: bool,
    pub age: String,
    pub subject: String,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
}

pub fn row(repo: &str, path: &Path) -> Row {
    let base = base_of(path);
    let default = base.as_deref().map(default_branch).unwrap_or_else(|| "main".to_string());
    let branch = current_branch(path);
    let is_default = branch.as_deref() == Some(default.as_str());
    let branch = branch.unwrap_or_else(|| "(detached)".to_string());

    let (age, subject) = match git::opt(path, &["log", "-1", "--format=%ct%x00%s"]) {
        Some(out) => {
            let (ts, subject) = out.split_once('\0').unwrap_or((&out, ""));
            (ts.parse().map(relative_age).unwrap_or_default(), subject.to_string())
        }
        None => (String::new(), String::new()),
    };

    let (mut ahead, mut behind) = (0, 0);
    let main_ref = format!("origin/{default}");
    if ref_exists(path, &main_ref) {
        if let Some(out) = git::opt(path, &["rev-list", "--left-right", "--count", &format!("{main_ref}...HEAD")]) {
            let mut parts = out.split_whitespace().map(|p| p.parse().unwrap_or(0));
            behind = parts.next().unwrap_or(0);
            ahead = parts.next().unwrap_or(0);
        }
    }

    let (mut staged, mut modified, mut untracked) = (0, 0, 0);
    for line in git::opt(path, &["status", "--porcelain"]).unwrap_or_default().lines() {
        let mut chars = line.chars();
        let (x, y) = (chars.next().unwrap_or(' '), chars.next().unwrap_or(' '));
        if x == '?' {
            untracked += 1;
            continue;
        }
        if x != ' ' {
            staged += 1;
        }
        if y != ' ' {
            modified += 1;
        }
    }

    Row { repo: repo.to_string(), branch, is_default, age, subject, ahead, behind, staged, modified, untracked }
}

fn relative_age(timestamp: i64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let secs = (now - timestamp).max(0);
    match secs {
        s if s < 60 => "now".to_string(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 7 * 86_400 => format!("{}d", s / 86_400),
        s if s < 30 * 86_400 => format!("{}w", s / (7 * 86_400)),
        s if s < 365 * 86_400 => format!("{}mo", s / (30 * 86_400)),
        s => format!("{}y", s / (365 * 86_400)),
    }
}

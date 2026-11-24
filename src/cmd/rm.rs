//! `wt rm <feature> [repo]`

use std::path::Path;

use anstream::eprintln;
use anyhow::Result;

use crate::error::hinted;
use crate::root::Root;
use crate::status::{self, Check, Commits};
use crate::ui::{self, paint};
use crate::workspace;
use crate::{git, repo};

pub fn run(root: &Root, feature: &str, only: Option<&str>, yes: bool) -> Result<()> {
    let ws = workspace::require(root, feature)?;
    let present = workspace::worktrees(&ws.path);
    let targets: Vec<String> = match only {
        Some(r) if present.iter().any(|p| p == r) => vec![r.to_string()],
        Some(r) => {
            return Err(hinted(
                format!("{r} is not in {}", ws.name),
                format!("repos in {}: {}", ws.name, present.join(", ")),
            ));
        }
        None => present.clone(),
    };
    let removes_workspace = targets.len() == present.len();

    let checks = check_all(&ws.path, &targets);
    let width = ui::width(targets.iter().map(String::as_str));
    let mut needs_confirm = false;
    for c in &checks {
        let warnings = c.warnings();
        if warnings.is_empty() {
            if c.commits == Commits::Gone {
                ui::skip(&c.repo, width, "remote branch deleted (merged)");
            }
        } else {
            needs_confirm = true;
            ui::warn(&c.repo, width, &warnings.join(", "));
        }
    }
    if removes_workspace {
        let others = workspace::other_entries(&ws.path);
        if !others.is_empty() {
            needs_confirm = true;
            ui::warn(&ws.name, width, &format!("workspace also contains: {}", others.join(", ")));
        }
    }
    if needs_confirm && !ui::confirm("Remove anyway?", yes)? {
        eprintln!("Cancelled.");
        return Ok(());
    }

    // Decide before removing: afterwards the dirs are gone and can't be compared.
    let cwd_removed = (removes_workspace && ui::cwd_inside(&ws.path))
        || checks.iter().any(|c| ui::cwd_inside(&c.path));
    for c in &checks {
        remove(c, width)?;
    }
    if removes_workspace {
        std::fs::remove_dir_all(&ws.path)?;
        eprintln!("Removed {}", paint(ui::BOLD, &ws.name));
    }
    if cwd_removed {
        let target = if ws.path.exists() { ws.path.clone() } else { root.worktrees() };
        ui::request_cd(&target)?;
    }
    Ok(())
}

/// Safety checks for the given repos in a workspace, in parallel.
pub fn check_all(ws: &Path, repos: &[String]) -> Vec<Check> {
    std::thread::scope(|s| {
        let handles: Vec<_> = repos.iter().map(|r| s.spawn(move || status::check(r, &ws.join(r)))).collect();
        handles.into_iter().map(|h| h.join().expect("status thread panicked")).collect()
    })
}

/// Remove a worktree, then delete its branch if no commits would be lost.
pub fn remove(c: &Check, width: usize) -> Result<()> {
    repo::remove_worktree(c.base.as_deref(), &c.path)?;
    let mut msg = "removed".to_string();
    if let (Some(base), Some(branch)) = (&c.base, &c.branch) {
        if c.commits.safe() {
            match git::run(base, &["branch", "-D", "--quiet", branch]) {
                Ok(_) => msg.push_str(&format!(", deleted branch {branch}")),
                Err(_) => msg.push_str(&format!(", couldn't delete branch {branch}")),
            }
        } else {
            msg.push_str(&format!(", kept branch {branch} (unpushed commits)"));
        }
    }
    ui::done(&c.repo, width, &msg);
    Ok(())
}

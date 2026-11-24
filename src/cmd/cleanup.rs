//! `wt cleanup`: remove workspaces whose remote branches are all deleted (merged).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anstream::eprintln;
use anyhow::Result;

use crate::cmd::rm;
use crate::repo::fetch_all;
use crate::root::Root;
use crate::status::{Check, Commits};
use crate::ui::{self, paint};
use crate::workspace::{self, Workspace};

struct Candidate {
    ws: Workspace,
    checks: Vec<Check>,
    others: Vec<String>,
}

pub fn run(root: &Root, yes: bool) -> Result<()> {
    let workspaces = workspace::list(root)?;
    if workspaces.is_empty() {
        eprintln!("Nothing to clean up.");
        return Ok(());
    }

    // Fetch every base repo that has worktrees, so deleted remote branches show up as "gone".
    let mut bases: BTreeMap<String, PathBuf> = BTreeMap::new();
    for ws in &workspaces {
        for r in workspace::worktrees(&ws.path) {
            if let Some(base) = crate::repo::base_of(&ws.path.join(&r)) {
                let name = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(r);
                bases.insert(name, base);
            }
        }
    }
    let failed = fetch_all(&bases);

    let mut candidates = Vec::new();
    let mut skipped = Vec::new();
    for ws in workspaces {
        let repos = workspace::worktrees(&ws.path);
        let checks = rm::check_all(&ws.path, &repos);
        // A repo whose fetch failed can't be judged.
        let unknown: Vec<&str> = checks
            .iter()
            .filter(|c| c.base.as_ref().is_some_and(|b| failed.contains(b)))
            .map(|c| c.repo.as_str())
            .collect();
        let all_done = checks.iter().all(Check::done);
        let any_gone = checks.iter().any(|c| c.commits == Commits::Gone);
        if unknown.is_empty() && (checks.is_empty() || (all_done && any_gone)) {
            let others = workspace::other_entries(&ws.path);
            candidates.push(Candidate { ws, checks, others });
        } else if any_gone {
            let reasons: Vec<String> = checks
                .iter()
                .filter(|c| !c.done() || unknown.contains(&c.repo.as_str()))
                .map(|c| {
                    let why = if unknown.contains(&c.repo.as_str()) {
                        "fetch failed".to_string()
                    } else {
                        let w = c.warnings();
                        if w.is_empty() { "remote branch still exists".to_string() } else { w.join(", ") }
                    };
                    format!("{} {why}", c.repo)
                })
                .collect();
            skipped.push((ws.name, reasons.join("; ")));
        }
    }

    if candidates.is_empty() {
        for (name, why) in &skipped {
            ui::skip(name, 0, &format!("skipped: {why}"));
        }
        eprintln!("Nothing to clean up.");
        return Ok(());
    }

    let width = ui::width(candidates.iter().map(|c| c.ws.name.as_str()).chain(skipped.iter().map(|s| s.0.as_str())));
    for c in &candidates {
        let what = if c.checks.is_empty() {
            "empty".to_string()
        } else {
            let repos: Vec<&str> = c.checks.iter().map(|c| c.repo.as_str()).collect();
            let n = c.checks.iter().filter(|c| c.commits == Commits::Gone).count();
            let deleted = if n == 1 { "remote branch deleted" } else { "remote branches deleted" };
            format!("{}  {}", repos.join(", "), paint(ui::DIM, deleted))
        };
        let others = if c.others.is_empty() {
            String::new()
        } else {
            paint(ui::YELLOW, format!("  (also contains: {})", c.others.join(", ")))
        };
        eprintln!("  {:<width$}  {what}{others}", c.ws.name);
    }
    for (name, why) in &skipped {
        ui::skip(name, width, &format!("skipped: {why}"));
    }

    let n = candidates.len();
    let prompt = if n == 1 { "Remove 1 workspace?".to_string() } else { format!("Remove {n} workspaces?") };
    if !ui::confirm(&prompt, yes)? {
        eprintln!("Cancelled.");
        return Ok(());
    }

    let mut cwd_removed = false;
    for c in &candidates {
        cwd_removed |= ui::cwd_inside(&c.ws.path);
        let width = ui::width(c.checks.iter().map(|c| c.repo.as_str()));
        for check in &c.checks {
            rm::remove(check, width)?;
        }
        std::fs::remove_dir_all(&c.ws.path)?;
        eprintln!("Removed {}", paint(ui::BOLD, &c.ws.name));
    }
    if cwd_removed {
        ui::request_cd(&root.worktrees())?;
    }
    Ok(())
}

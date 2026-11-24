//! `wt add <feature> <spec...>`: create a workspace, or add repos to an existing one.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anstream::eprintln;
use anyhow::Result;

use crate::config::Config;
use crate::error::{hinted, user};
use crate::repo::{self, Created};
use crate::root::Root;
use crate::spec::{self, Spec};
use crate::ui::{self, paint, tilde};
use crate::workspace::{self, is_worktree};
use crate::{git, include};

struct Planned {
    repo: String,
    branch: String,
    base: PathBuf,
    path: PathBuf,
    exists: bool,
}

pub fn run(root: &Root, feature: &str, args: &[String]) -> Result<()> {
    let config = Config::load(root)?;
    let repos = root.require_repos()?;
    let specs = spec::expand(args, &config, &repos)?;

    let ws_name = workspace::sanitize(feature)?;
    let ws_path = root.worktrees().join(&ws_name);
    let ws_existed = ws_path.is_dir();

    // Branch for plain `repo` specs: the branch the workspace's worktrees share, else the feature name.
    let default_branch = if ws_existed { shared_branch(&ws_path) } else { None }.unwrap_or_else(|| feature.to_string());

    let plan: Vec<Planned> = specs
        .into_iter()
        .map(|Spec { repo, branch }| {
            let path = ws_path.join(&repo);
            Planned {
                branch: branch.unwrap_or_else(|| default_branch.clone()),
                base: root.repo_path(&repo),
                exists: path.exists(),
                path,
                repo,
            }
        })
        .collect();

    // Up-front checks, before anything is fetched or created.
    for p in &plan {
        if p.exists && !is_worktree(&p.path) {
            return Err(user(format!("{} exists but isn't a git worktree", tilde(&p.path))));
        }
        if !p.exists && !repo::valid_branch_name(&p.base, &p.branch) {
            return Err(user(format!("invalid branch name '{}'", p.branch)));
        }
    }

    let to_create: Vec<&Planned> = plan.iter().filter(|p| !p.exists).collect();
    if !to_create.is_empty() {
        update_bases(&to_create)?;
        check_not_checked_out(&to_create)?;
        std::fs::create_dir_all(&ws_path)?;
    }

    let width = ui::width(plan.iter().map(|p| p.repo.as_str()));
    let mut added = Vec::new();
    let mut new_branch = false;
    let mut failed = None;
    for p in &plan {
        if p.exists {
            let copied = include::copy(&p.base, &p.path)?;
            let msg = match copied.describe() {
                Some(c) => format!("already there, {c}"),
                None => "already there".to_string(),
            };
            ui::skip(&p.repo, width, &msg);
            warn_not_ignored(&p.repo, width, &copied.not_ignored);
            continue;
        }
        match repo::create_worktree(&p.base, &p.path, &p.branch) {
            Ok(created) => {
                new_branch |= matches!(created, Created::NewBranch { .. });
                let copied = include::copy(&p.base, &p.path)?;
                let mut msg = created.describe(&p.branch);
                if let Some(c) = copied.describe() {
                    msg = format!("{msg}, {c}");
                }
                ui::done(&p.repo, width, &msg);
                warn_not_ignored(&p.repo, width, &copied.not_ignored);
                added.push(p.repo.clone());
            }
            Err(e) => {
                ui::fail(&p.repo, width, &e.to_string());
                failed = Some(e);
                break;
            }
        }
    }

    if !ws_existed && workspace::worktrees(&ws_path).is_empty() {
        let _ = std::fs::remove_dir(&ws_path);
    }
    if let Some(e) = failed {
        return Err(e);
    }

    let all = workspace::worktrees(&ws_path).join(", ");
    if !ws_existed {
        eprintln!("Created {} ({all})", paint(ui::BOLD, tilde(&ws_path)));
    } else if added.is_empty() {
        eprintln!("{} is up to date ({all})", paint(ui::BOLD, &ws_name));
    } else {
        eprintln!("Added {} to {}", added.join(", "), paint(ui::BOLD, &ws_name));
    }

    if new_branch {
        if let Some(p) = to_create.first() {
            if git::opt(&p.base, &["config", "--get", "push.autoSetupRemote"]).as_deref() != Some("true") {
                ui::hint("git config --global push.autoSetupRemote true (so `git push` sets the upstream, and wt can tell when a branch is merged)");
            }
        }
    }

    if !ui::cwd_inside(&ws_path) && !ui::request_cd(&ws_path)? {
        ui::hint(&format!("cd {}", tilde(&ws_path)));
    }
    Ok(())
}

/// The branch all worktrees in the workspace are on, if they agree.
fn shared_branch(ws: &std::path::Path) -> Option<String> {
    let branches: BTreeSet<Option<String>> =
        workspace::worktrees(ws).iter().map(|r| repo::current_branch(&ws.join(r))).collect();
    match branches.into_iter().collect::<Vec<_>>().as_slice() {
        [Some(b)] => Some(b.clone()),
        _ => None,
    }
}

/// Fetch + fast-forward base for each repo, in parallel. All-or-nothing: any failure aborts.
fn update_bases(plan: &[&Planned]) -> Result<()> {
    let names: Vec<&str> = plan.iter().map(|p| p.repo.as_str()).collect();
    ui::step(&format!("Updating {}", names.join(", ")));
    let results: Vec<(String, Result<()>)> = std::thread::scope(|s| {
        let handles: Vec<_> = plan
            .iter()
            .map(|p| (p.repo.clone(), s.spawn(move || repo::update_base(&p.base))))
            .collect();
        handles.into_iter().map(|(r, h)| (r, h.join().expect("update thread panicked"))).collect()
    });
    let errors: Vec<(String, anyhow::Error)> =
        results.into_iter().filter_map(|(r, res)| res.err().map(|e| (r, e))).collect();
    ui::step_done(errors.is_empty());
    let mut errors = errors.into_iter();
    match errors.next() {
        None => Ok(()),
        Some((_, first)) => {
            // Report all but the first here; main prints the first as the error.
            for (repo, e) in errors {
                crate::print_error(&e.context(format!("{repo}: base update failed")));
            }
            Err(first)
        }
    }
}

/// Git refuses to check out a branch twice; give a clearer message than git's, before creating anything.
fn check_not_checked_out(plan: &[&Planned]) -> Result<()> {
    for p in plan {
        for (path, branch) in repo::checked_out(&p.base)? {
            if branch == p.branch {
                return Err(hinted(
                    format!("{}: branch '{}' is already checked out at {}", p.repo, p.branch, tilde(&path)),
                    if path == p.base {
                        "base keeps the default branch; work on a feature branch instead".to_string()
                    } else {
                        "use that worktree, or remove it first (wt rm)".to_string()
                    },
                ));
            }
        }
    }
    Ok(())
}

fn warn_not_ignored(repo: &str, width: usize, files: &[String]) {
    for f in files {
        ui::warn(repo, width, &format!("not copied (not gitignored): {f}"));
    }
}

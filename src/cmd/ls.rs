//! `wt ls`

use std::collections::BTreeMap;
use std::io::IsTerminal;

use anstream::{eprintln, println};
use anyhow::Result;

use crate::config::Config;
use crate::git;
use crate::provider::{CacheMode, Item, Registry, RepoInfo, Shown, Statuses, Tone};
use crate::root::Root;
use crate::status::{self, Row};
use crate::ui::{self, paint};
use crate::workspace;

pub fn run(root: &Root, mode: CacheMode) -> Result<()> {
    let workspaces = workspace::list(root)?;
    if workspaces.is_empty() {
        eprintln!("No workspaces. Create one with: wt add <feature> <repo>");
        return Ok(());
    }
    let registry = Registry::from_config(&Config::load(root)?);

    // Gather status for every worktree in parallel.
    let groups: Vec<(String, Vec<Row>)> = std::thread::scope(|s| {
        let handles: Vec<_> = workspaces
            .iter()
            .map(|ws| {
                let repos = workspace::worktrees(&ws.path);
                let handles: Vec<_> = repos
                    .into_iter()
                    .map(|r| {
                        let path = ws.path.join(&r);
                        s.spawn(move || status::row(&r, &path))
                    })
                    .collect();
                (ws.name.clone(), handles)
            })
            .collect();
        handles
            .into_iter()
            .map(|(name, hs)| (name, hs.into_iter().map(|h| h.join().expect("status thread panicked")).collect()))
            .collect()
    });

    let remote = if registry.is_empty() {
        Statuses { warnings: registry.warnings.clone(), ..Default::default() }
    } else {
        registry.status_all(root, batches(&groups), mode)
    };
    let provider_cell = |r: &Row| -> Option<&Shown> {
        let base = base_name(r)?;
        remote.map.get(&(base, r.branch.clone()))
    };
    let links = std::io::stdout().is_terminal();

    let rows = || groups.iter().flat_map(|g| g.1.iter());
    let rw = ui::width(rows().map(|r| r.repo.as_str()));
    let bw = ui::width(rows().map(|r| r.branch.as_str()));
    let aw = ui::width(rows().map(|r| r.age.as_str()));
    let abw = rows().map(|r| ahead_behind(r).1).max().unwrap_or(0);
    let dw = rows().map(|r| dirty(r).1).max().unwrap_or(0);
    let pw = rows().filter_map(provider_cell).map(|s| badges(s, false).1).max().unwrap_or(0);
    let max_subject = rows().map(|r| r.subject.chars().count()).max().unwrap_or(0);
    let fixed = 2 + rw + 2 + bw + 2 + aw + 2 + 2 + abw + 2 + dw + if pw > 0 { 2 + pw } else { 0 };
    let sw = match terminal_size::terminal_size_of(std::io::stdout()) {
        Some((terminal_size::Width(w), _)) => max_subject.min((w as usize).saturating_sub(fixed).max(10)),
        None => max_subject,
    };

    for (i, (name, rows)) in groups.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("{}", paint(ui::BOLD, name));
        if rows.is_empty() {
            println!("  {}", paint(ui::DIM, "(empty)"));
            continue;
        }
        for r in rows {
            let branch_style = if r.is_default { ui::DIM } else { ui::CYAN };
            let subject = truncate(&r.subject, sw);
            let (ab, ab_len) = ahead_behind(r);
            let (dt, dt_len) = dirty(r);
            let (pr, _) = provider_cell(r).map(|s| badges(s, links)).unwrap_or_default();
            let line = format!(
                "  {}  {}  {}  {subject:<sw$}  {}{}  {}{}  {}",
                paint(ui::BLUE, format!("{:<rw$}", r.repo)),
                paint(branch_style, format!("{:<bw$}", r.branch)),
                paint(ui::DIM, format!("{:>aw$}", r.age)),
                ab,
                " ".repeat(abw - ab_len),
                dt,
                " ".repeat(dw - dt_len),
                pr,
            );
            println!("{}", line.trim_end());
        }
    }

    if !remote.warnings.is_empty() {
        let width = ui::width(remote.warnings.iter().map(|w| w.0.as_str()));
        for (provider, msg) in &remote.warnings {
            ui::warn(provider, width, msg);
        }
    }
    Ok(())
}

/// The base repo's dir name; providers and their cache know repos by that name.
fn base_name(r: &Row) -> Option<String> {
    Some(r.base.as_ref()?.file_name()?.to_string_lossy().into_owned())
}

/// Provider work: per base repo, the worktrees on a feature branch.
fn batches(groups: &[(String, Vec<Row>)]) -> Vec<(RepoInfo, Vec<Item>)> {
    let mut by_repo: BTreeMap<String, (RepoInfo, Vec<Item>)> = BTreeMap::new();
    for r in groups.iter().flat_map(|g| g.1.iter()) {
        if r.detached || r.is_default {
            continue;
        }
        let (Some(base), Some(name), Some(head)) = (&r.base, base_name(r), &r.head) else { continue };
        if !by_repo.contains_key(&name) {
            let Some(remote_url) = git::opt(base, &["config", "--get", "remote.origin.url"]) else { continue };
            by_repo.insert(name.clone(), (RepoInfo { name: name.clone(), remote_url }, Vec::new()));
        }
        let entry = by_repo.get_mut(&name).expect("inserted above");
        entry.1.push(Item { branch: r.branch.clone(), head: head.clone() });
    }
    by_repo.into_values().collect()
}

/// Rendered badges and their visible width. Stale results are dimmed. With `link`, the first
/// badge is a clickable OSC 8 hyperlink to the status URL.
fn badges(shown: &Shown, link: bool) -> (String, usize) {
    let mut parts = Vec::new();
    let mut len = 0;
    for (i, b) in shown.status.badges.iter().enumerate() {
        len += b.text.chars().count() + usize::from(i > 0);
        let style = if shown.stale {
            ui::DIM
        } else {
            match b.tone {
                Tone::Ok => ui::GREEN,
                Tone::Warn => ui::YELLOW,
                Tone::Fail => ui::RED,
                Tone::Info => ui::MAGENTA,
                Tone::Dim => ui::DIM,
            }
        };
        let mut text = paint(style, &b.text);
        if i == 0 && link {
            if let Some(url) = &shown.status.url {
                text = format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\");
            }
        }
        parts.push(text);
    }
    (parts.join(" "), len)
}

/// `↑2 ↓4` (colored) and its visible width.
fn ahead_behind(r: &Row) -> (String, usize) {
    let mut parts = Vec::new();
    let mut len = 0;
    if r.ahead > 0 {
        let s = format!("↑{}", r.ahead);
        len += s.chars().count();
        parts.push(paint(ui::GREEN, s));
    }
    if r.behind > 0 {
        let s = format!("↓{}", r.behind);
        len += s.chars().count();
        parts.push(paint(ui::RED, s));
    }
    if parts.len() == 2 {
        len += 1;
    }
    (parts.join(" "), len)
}

/// `+staged ~modified ?untracked` (colored) and its visible width.
fn dirty(r: &Row) -> (String, usize) {
    let mut parts = Vec::new();
    let mut len = 0;
    for (n, sym, style) in [(r.staged, '+', ui::GREEN), (r.modified, '~', ui::YELLOW), (r.untracked, '?', ui::DIM)] {
        if n > 0 {
            let s = format!("{sym}{n}");
            len += s.chars().count() + usize::from(!parts.is_empty());
            parts.push(paint(style, s));
        }
    }
    (parts.join(" "), len)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

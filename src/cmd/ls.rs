//! `wt ls`

use anstream::{eprintln, println};
use anyhow::Result;

use crate::root::Root;
use crate::status::{self, Row};
use crate::ui::{self, paint};
use crate::workspace;

pub fn run(root: &Root) -> Result<()> {
    let workspaces = workspace::list(root)?;
    if workspaces.is_empty() {
        eprintln!("No workspaces. Create one with: wt add <feature> <repo>");
        return Ok(());
    }

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

    let rows = || groups.iter().flat_map(|g| g.1.iter());
    let rw = ui::width(rows().map(|r| r.repo.as_str()));
    let bw = ui::width(rows().map(|r| r.branch.as_str()));
    let aw = ui::width(rows().map(|r| r.age.as_str()));
    let abw = rows().map(|r| ahead_behind(r).1).max().unwrap_or(0);
    let max_subject = rows().map(|r| r.subject.chars().count()).max().unwrap_or(0);
    let fixed = 2 + rw + 2 + bw + 2 + aw + 2 + 2 + abw + 2 + 12;
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
            let line = format!(
                "  {}  {}  {}  {subject:<sw$}  {}{}  {}",
                paint(ui::BLUE, format!("{:<rw$}", r.repo)),
                paint(branch_style, format!("{:<bw$}", r.branch)),
                paint(ui::DIM, format!("{:>aw$}", r.age)),
                ab,
                " ".repeat(abw - ab_len),
                dirty(r),
            );
            println!("{}", line.trim_end());
        }
    }
    Ok(())
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

fn dirty(r: &Row) -> String {
    let mut parts = Vec::new();
    if r.staged > 0 {
        parts.push(paint(ui::GREEN, format!("+{}", r.staged)));
    }
    if r.modified > 0 {
        parts.push(paint(ui::YELLOW, format!("~{}", r.modified)));
    }
    if r.untracked > 0 {
        parts.push(paint(ui::DIM, format!("?{}", r.untracked)));
    }
    parts.join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

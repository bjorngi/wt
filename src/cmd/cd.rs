//! `wt cd [feature]`

use anstream::println;
use anyhow::Result;

use crate::error::hinted;
use crate::root::Root;
use crate::ui;
use crate::workspace;

pub fn run(root: &Root, feature: Option<&str>) -> Result<()> {
    let target = match feature {
        None => root.worktrees(),
        Some(f) => resolve(root, f)?,
    };
    if !ui::request_cd(&target)? {
        // No shell integration: print the path, so `cd "$(wt cd x)"` works.
        println!("{}", target.display());
    }
    Ok(())
}

/// Exact match (either form), else the single workspace whose name contains the text.
fn resolve(root: &Root, feature: &str) -> Result<std::path::PathBuf> {
    if let Some(ws) = workspace::find(root, feature)? {
        return Ok(ws.path);
    }
    let needle = workspace::sanitize(feature)?.to_lowercase();
    let matches: Vec<workspace::Workspace> = workspace::list(root)?
        .into_iter()
        .filter(|ws| ws.name.to_lowercase().contains(&needle))
        .collect();
    match matches.as_slice() {
        [ws] => Ok(ws.path.clone()),
        [] => Err(hinted(format!("no workspace matches '{feature}'"), "see existing ones with: wt ls")),
        many => {
            let names: Vec<&str> = many.iter().map(|w| w.name.as_str()).collect();
            Err(hinted(format!("'{feature}' matches several workspaces"), format!("pick one: {}", names.join(", "))))
        }
    }
}

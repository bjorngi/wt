//! `wt shell-init zsh` and the hidden `wt __complete <kind>`.

use anstream::println;
use anyhow::Result;

use crate::config::Config;
use crate::git;
use crate::root::Root;
use crate::workspace;

const ZSH: &str = include_str!("../shell/wt.zsh");

pub fn shell_init() -> Result<()> {
    // Embed the absolute path of this binary, so an older `wt` earlier on PATH can't shadow it.
    let exe = std::env::current_exe()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let quoted = format!("'{}'", exe.to_string_lossy().replace('\'', r"'\''"));
    print!("{}", ZSH.replace("__WT_BIN__", &quoted));
    Ok(())
}

/// One completion candidate per line. Errors print nothing: completion must never be noisy.
pub fn complete(kind: &str, args: &[String]) -> Result<()> {
    let Ok(root) = Root::from_env() else { return Ok(()) };
    let items: Vec<String> = match kind {
        "specs" => {
            let mut items = root.repos()?;
            if let Ok(config) = Config::load(&root) {
                items.extend(config.presets.keys().map(|k| format!("@{k}")));
            }
            items
        }
        "branches" => {
            let Some(repo) = args.first() else { return Ok(()) };
            let path = root.repo_path(repo);
            if !path.is_dir() {
                return Ok(());
            }
            git::opt(
                &path,
                &["for-each-ref", "--sort=-committerdate", "--format=%(refname:lstrip=3)", "refs/remotes/origin"],
            )
            .unwrap_or_default()
            .lines()
            .filter(|b| *b != "HEAD")
            .map(String::from)
            .collect()
        }
        "workspaces" => workspace::list(&root)?.into_iter().map(|w| w.name).collect(),
        "workspace-repos" => {
            let Some(feature) = args.first() else { return Ok(()) };
            match workspace::find(&root, feature) {
                Ok(Some(ws)) => workspace::worktrees(&ws.path),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    for item in items {
        println!("{item}");
    }
    Ok(())
}

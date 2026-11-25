//! Output style: symbols, colors, prompts and the `WT_CD_FILE` handshake.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anstream::{eprint, eprintln};
use anstyle::{AnsiColor, Style};
use anyhow::Result;

use crate::error::hinted;

pub const GREEN: Style = AnsiColor::Green.on_default();
pub const YELLOW: Style = AnsiColor::Yellow.on_default();
pub const RED: Style = AnsiColor::Red.on_default();
pub const CYAN: Style = AnsiColor::Cyan.on_default();
pub const BLUE: Style = AnsiColor::Blue.on_default();
pub const MAGENTA: Style = AnsiColor::Magenta.on_default();
pub const DIM: Style = Style::new().dimmed();
pub const BOLD: Style = Style::new().bold();

pub fn paint(style: Style, text: impl std::fmt::Display) -> String {
    format!("{style}{text}{style:#}")
}

/// `✓ repo  message`
pub fn done(repo: &str, width: usize, msg: &str) {
    eprintln!("{} {repo:<width$}  {msg}", paint(GREEN, "✓"));
}

/// `· repo  message` (dim): skipped / already there / informational.
pub fn skip(repo: &str, width: usize, msg: &str) {
    eprintln!("{}", paint(DIM, format!("· {repo:<width$}  {msg}")));
}

/// `! repo  message` (yellow): needs attention.
pub fn warn(repo: &str, width: usize, msg: &str) {
    eprintln!("{} {repo:<width$}  {msg}", paint(YELLOW, "!"));
}

/// `✗ repo  message` (red): failed.
pub fn fail(repo: &str, width: usize, msg: &str) {
    eprintln!("{} {repo:<width$}  {msg}", paint(RED, "✗"));
}

pub fn hint(msg: &str) {
    eprintln!("{} {msg}", paint(DIM, "hint:"));
}

/// Start a slow step: `Updating a, b… ` — finish it with `step_done`.
pub fn step(msg: &str) {
    eprint!("{msg}… ");
    let _ = std::io::stderr().flush();
}

pub fn step_done(ok: bool) {
    if ok {
        eprintln!("{}", paint(GREEN, "✓"));
    } else {
        eprintln!("{}", paint(RED, "✗"));
    }
}

/// Path with `$HOME` shortened to `~`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            };
        }
    }
    path.display().to_string()
}

/// Ask `prompt [y/N]` on the terminal. `yes` skips the question.
pub fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(hinted("can't ask for confirmation: not a terminal", "rerun with --yes"));
    }
    eprint!("{prompt} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// Ask the shell function (if any) to `cd` into `path`. Returns false without shell integration.
pub fn request_cd(path: &Path) -> Result<bool> {
    match std::env::var_os("WT_CD_FILE").filter(|v| !v.is_empty()) {
        Some(file) => {
            std::fs::write(file, path.as_os_str().as_encoded_bytes())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Whether the process' cwd is `path` or inside it.
pub fn cwd_inside(path: &Path) -> bool {
    let Ok(cwd) = std::env::current_dir() else { return false };
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    cwd.starts_with(path)
}

/// Width of the longest name, for aligned columns.
pub fn width<'a>(names: impl IntoIterator<Item = &'a str>) -> usize {
    names.into_iter().map(|n| n.chars().count()).max().unwrap_or(0)
}

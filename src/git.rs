//! Thin wrapper around the `git` binary.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result};

/// A git command that exited non-zero. Carries git's stderr so it can be shown to the user.
#[derive(Debug)]
pub struct GitError {
    pub dir: PathBuf,
    pub args: String,
    pub stderr: String,
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`git {}` failed in {}", self.args, crate::ui::tilde(&self.dir))
    }
}

impl std::error::Error for GitError {}

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    // Stable, parseable output and no credential prompts from parallel fetches.
    cmd.env("LC_ALL", "C").env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn exec(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Output> {
    let mut cmd = command(dir, args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.spawn().context("failed to run git (is it installed?)")?;
    if let Some(input) = input {
        // Write in a separate thread so a full stdout pipe can't deadlock us.
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    Ok(child.wait_with_output()?)
}

fn failure(dir: &Path, args: &[&str], out: &Output) -> anyhow::Error {
    GitError {
        dir: dir.to_path_buf(),
        args: args.join(" "),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    }
    .into()
}

/// Run git and return stdout (trailing newlines trimmed). Non-zero exit is an error.
pub fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let out = exec(dir, args, None)?;
    if !out.status.success() {
        return Err(failure(dir, args, &out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
}

/// Run git and return raw stdout bytes. Non-zero exit is an error.
pub fn run_raw(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = exec(dir, args, None)?;
    if !out.status.success() {
        return Err(failure(dir, args, &out));
    }
    Ok(out.stdout)
}

/// Run git with stdin; exit codes listed in `ok_codes` count as success. Returns raw stdout.
pub fn run_stdin(dir: &Path, args: &[&str], input: &[u8], ok_codes: &[i32]) -> Result<Vec<u8>> {
    let out = exec(dir, args, Some(input))?;
    match out.status.code() {
        Some(code) if ok_codes.contains(&code) => Ok(out.stdout),
        _ => Err(failure(dir, args, &out)),
    }
}

/// Run git and report whether it succeeded. Output is discarded.
pub fn ok(dir: &Path, args: &[&str]) -> bool {
    exec(dir, args, None).map(|o| o.status.success()).unwrap_or(false)
}

/// Run git and return non-empty stdout on success, `None` otherwise.
pub fn opt(dir: &Path, args: &[&str]) -> Option<String> {
    run(dir, args).ok().filter(|s| !s.is_empty())
}

/// Split NUL-separated output (from `-z`) into strings.
pub fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

mod cmd;
mod config;
mod error;
mod git;
mod include;
mod provider;
mod remote;
mod repo;
mod root;
mod spec;
mod status;
mod ui;
mod workspace;

use std::process::ExitCode;

use anstream::eprintln;
use clap::{Parser, Subcommand, ValueEnum};

use crate::error::UserError;
use crate::git::GitError;
use crate::provider::CacheMode;
use crate::root::Root;

/// Work on one feature across several repos: one folder per feature, one git worktree per repo.
#[derive(Parser)]
#[command(name = "wt", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a workspace, or add repos to an existing one
    ///
    /// Specs: `repo` (branch = the feature name), `repo:branch` (existing or new branch),
    /// `@preset` (repos from [presets] in $WT_ROOT/.wt.toml).
    #[command(after_help = "Examples:\n  wt add feature/login @alde\n  wt add feature-login infra\n  wt add review-123 pen:fix/npe")]
    Add {
        /// Feature name; also the branch name for plain `repo` specs
        feature: String,
        /// repo, repo:branch or @preset
        #[arg(required = true, value_name = "SPEC")]
        specs: Vec<String>,
    },
    /// Remove a workspace, or one repo from it
    Rm {
        feature: String,
        /// Only remove this repo's worktree
        repo: Option<String>,
        /// Don't ask, even with uncommitted or unpushed work
        #[arg(short, long)]
        yes: bool,
    },
    /// Remove workspaces whose remote branches are all deleted (merged)
    Cleanup {
        /// Don't ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// List workspaces and the status of their repos (plus pull requests and checks, via providers)
    Ls {
        /// Don't ask providers (GitHub); show only cached results
        #[arg(long, conflicts_with = "refresh")]
        offline: bool,
        /// Ignore cached provider results
        #[arg(long)]
        refresh: bool,
    },
    /// Go to a workspace ($WT_ROOT/worktrees without a name)
    Cd { feature: Option<String> },
    /// Print shell integration (cd support + completion): eval "$(wt shell-init zsh)"
    ShellInit { shell: Shell },
    #[command(name = "__complete", hide = true)]
    Complete { kind: String, args: Vec<String> },
}

#[derive(Clone, ValueEnum)]
enum Shell {
    Zsh,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            print_error(&e);
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::ShellInit { shell: Shell::Zsh } => cmd::shell::shell_init(),
        Cmd::Complete { kind, args } => cmd::shell::complete(&kind, &args),
        cmd => {
            let root = Root::from_env()?;
            match cmd {
                Cmd::Add { feature, specs } => cmd::add::run(&root, &feature, &specs),
                Cmd::Rm { feature, repo, yes } => cmd::rm::run(&root, &feature, repo.as_deref(), yes),
                Cmd::Cleanup { yes } => cmd::cleanup::run(&root, yes),
                Cmd::Ls { offline, refresh } => {
                    let mode = match (offline, refresh) {
                        (true, _) => CacheMode::Offline,
                        (_, true) => CacheMode::Refresh,
                        _ => CacheMode::Normal,
                    };
                    cmd::ls::run(&root, mode)
                }
                Cmd::Cd { feature } => cmd::cd::run(&root, feature.as_deref()),
                Cmd::ShellInit { .. } | Cmd::Complete { .. } => unreachable!(),
            }
        }
    }
}

/// The error message, including any context, on one line.
pub fn error_text(e: &anyhow::Error) -> String {
    e.chain().map(|c| c.to_string()).collect::<Vec<_>>().join(": ")
}

/// `error: …`, git's stderr indented below it, and a `hint: …` if there is one.
pub fn print_error(e: &anyhow::Error) {
    eprintln!("{} {}", ui::paint(ui::RED, "error:"), error_text(e));
    if let Some(git) = e.downcast_ref::<GitError>() {
        for line in git.stderr.lines() {
            eprintln!("  {}", ui::paint(ui::DIM, line));
        }
    }
    if let Some(UserError { hint: Some(hint), .. }) = e.downcast_ref::<UserError>() {
        ui::hint(hint);
    }
}

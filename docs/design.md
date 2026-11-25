# Design

## Principles

- **Base is always a clean default branch.** New branches start from a local `main` that was just fast-forwarded. If
  base isn't in that state, `wt` stops instead of guessing.
- **Validate before mutating.** Bad specs, unknown presets, invalid branch names, a failed base update, or a branch
  that's already checked out all stop `wt` before anything is created.
- **Never lose work silently.** Uncommitted changes, unpushed commits, and extra files in a workspace all need
  confirmation. Branches with commits that aren't on a remote are kept.
- **Not interactive** apart from the `[y/N]` confirms in `rm` and `cleanup`, which `--yes` skips. `wt` behaves the same
  in scripts and agents.

## Non-goals

- Managing base repos beyond keeping the default branch current. Use `git clone` for that.
- Syncing or rebasing feature branches.
- Agent or editor config.
- Shells other than zsh.

## Output conventions

- **stdout is for data** (`ls`, and the `cd` path without shell integration). **stderr gets everything else**: progress,
  warnings, errors and prompts.
- **Quiet on success**: one line per action. Git's own output appears only on failure, indented under the error.
- **Symbols**: `✓` green (done), `·` dim (skipped / already there / info), `!` yellow (needs attention), `✗` red (failed).
  No emoji.
- **Errors**: `error: …` in red, plus `hint: …` where there's an obvious fix. Exit code 1.
- Columns are aligned, `$HOME` is shown as `~`, and color is used only on a TTY. `NO_COLOR` and `CLICOLOR_FORCE` are
  respected.
- No spinners. A slow step prints a line (`Updating api, web… `) and ends it with `✓` or `✗`.

## Architecture

```
src/
├── main.rs        # clap CLI, dispatch, error printing (error + git stderr + hint)
├── error.rs       # UserError: message + optional hint
├── root.rs        # $WT_ROOT, paths, repo discovery in base/
├── config.rs      # .wt.toml presets (serde + toml)
├── spec.rs        # spec parsing, preset expansion, validation, de-dup
├── git.rs         # `git` subprocess wrapper (captures stderr, LC_ALL=C, no terminal prompts)
├── repo.rs        # default branch, base update (fetch + ff), worktree create/remove, parallel fetch
├── include.rs     # .worktreeinclude copying
├── workspace.rs   # list/lookup workspaces, name sanitizing
├── remote.rs      # parse origin URLs into host / owner / name
├── provider/      # Provider trait + Registry (mod.rs), status cache (cache.rs), GitHub via `gh` (github.rs)
├── status.rs      # safety checks (rm, cleanup) and ls rows
├── ui.rs          # symbols, colors, y/N confirm, WT_CD_FILE, cwd checks
├── shell/wt.zsh   # wt() function + _wt completion (include_str!)
└── cmd/{add,rm,cleanup,ls,cd,shell}.rs
```

**Crates**: `clap` (derive), `anyhow`, `serde` + `toml`, `serde_json`, `anstream`/`anstyle`, `terminal_size`. No prompt or spinner
crates.

**Git**: `wt` runs the `git` binary rather than using `git2`/`gix`. That way credentials and SSH just work, and worktree
support is complete. Every call sets `LC_ALL=C` (for parseable output) and `GIT_TERMINAL_PROMPT=0` (so parallel fetches
can't hang on a password prompt).

**Parallelism** uses `std::thread::scope`: base updates in `add`, fetches in `cleanup`, checks in `rm`/`cleanup`, and
status in `ls` (git status, then one provider call per repo).

**Providers** implement the `Provider` trait (`src/provider/mod.rs`): `handles(repo)` decides from local data only
(the origin URL), and `status(repo, items, deadline)` answers for every branch of one repo in a single batch. They return
`Badge`s (text plus a tone: ok / warn / fail / info / dim), and `ls` does the rendering, so every provider looks the same.
The `Registry` takes care of the cache, parallelism, the 3-second deadline and turning failures into warnings. To add a
provider, implement the trait, add a config section, and register it in `Registry::from_config`. See
[extensions.md](extensions.md) for what's planned next.

## Testing

- Unit tests: spec expansion, name sanitizing, config, remote URL parsing, the provider registry and cache (with a fake
  provider), the deadline runner, and GitHub response mapping, plus one run against a fake `gh` script.
- Integration tests (`tests/workflow.rs`): each test builds a temporary `$WT_ROOT` with local bare "remotes", seeded
  branches, clones in `base/`, secrets and a `.worktreeinclude`. The tests run the real binary with `WT_CD_FILE` and
  `NO_COLOR` set, and cover add (new / tracking / extend / base update / refusals / validation), rm (clean, confirm,
  kept branches, merged, single repo, extra files), cleanup (merged vs open vs fresh vs partial), ls (including GitHub
  status from a fake `gh` on `PATH`: cache, `--offline`, `--refresh`, failure fallback), cd, completion,
  and `zsh -n` on the shell script.

```sh
cargo test
```

The integration tests need `git worktree add` and `git remote set-url`, which some sandboxed agent environments block.

## Ideas for later

- `--json` for `ls`
- `wt open` (open the workspace in an IDE)
- forced re-copy of `.worktreeinclude` files
- PR status, merged detection and `repo#123` specs through providers: see [extensions.md](extensions.md)

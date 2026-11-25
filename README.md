# wt

Work on one feature across several repos. `wt` gives each feature a **workspace folder** holding one git worktree per
repo, so you (and coding agents started in that folder) can see and change all the repos the feature touches.

```
$ wt add feature/login @app
Updating api, web… ✓
✓ api  new branch feature/login from main, copied 3 files
✓ web  tracking origin/feature/login, copied 1 file
Created ~/wt/worktrees/feature-login (api, web)
```

```
~/wt/
├── .wt.toml                  # optional: presets
├── base/<repo>/              # plain clones on the default branch (secrets and IDE config live here)
└── worktrees/<feature>/      # one workspace per feature
    └── <repo>/               # git worktree on the feature branch
```

## Install

Requires Rust (edition 2024, so a recent stable toolchain), `git` and zsh.

```sh
cargo install --path ~/ideas/wt     # installs ~/.cargo/bin/wt
```

Run the same command again after pulling changes.

## Setup

1. **Pick a root** and export it in `~/.zshrc`:
   ```sh
   export WT_ROOT=~/wt
   ```
   ```sh
   mkdir -p $WT_ROOT/base
   ```

2. **Clone your repos into `base/`.** The folder name becomes the repo name in `wt`:
   ```sh
   git clone git@github.com:acme/api.git $WT_ROOT/base/api
   git clone git@github.com:acme/web.git  $WT_ROOT/base/web
   ```
   Keep base clones on a clean default branch, because `wt` fast-forwards them before creating worktrees. Put local secrets
   (`.env`, `application-local.yml`) and IDE config (`.idea/`) in base.

3. **Shell integration** (lets `wt` change your directory, and adds completion). Add this to `~/.zshrc` *after* `compinit`:
   ```sh
   eval "$(wt shell-init zsh)"
   ```

4. **Let `git push` record the upstream**, so `wt` can tell when a branch has been merged and its remote branch deleted:
   ```sh
   git config --global push.autoSetupRemote true
   ```

5. *(Optional)* **Presets** in `$WT_ROOT/.wt.toml`:
   ```toml
   [presets]
   app = ["api", "web"]
   full = ["api", "web", "worker"]
   ```

6. *(Optional)* **Copy secrets into worktrees** by listing them in `base/<repo>/.worktreeinclude` (`.gitignore` syntax,
   the same file Claude Code uses). Only files that are also gitignored are copied:
   ```gitignore
   .env
   .idea/
   src/main/resources/application-local.yml
   ```

## Usage

```sh
wt add feature/login @app         # create workspace feature-login with api + web on branch feature/login, cd into it
wt add feature/login worker       # add another repo to it (same branch)
wt add review-123 web:fix/npe     # check out an existing (remote) branch
wt ls                             # all workspaces: branch, last commit, ahead/behind, dirty files
wt cd login                       # jump to a workspace (exact name, or a unique substring)
wt cd                             # go to $WT_ROOT/worktrees
wt rm feature/login web           # remove one repo from the workspace
wt rm feature/login               # remove the whole workspace (asks first if work would be lost)
wt cleanup                        # remove every workspace whose remote branches have been deleted (merged)
```

- **Specs**: `repo` (branch = the feature name), `repo:branch` (that branch), `@preset` (the preset's repos).
- Feature names may contain `/`. The folder name uses `-` instead (`feature/login` → `feature-login`), and both forms are
  accepted everywhere.
- `rm` and `cleanup` ask `[y/N]` before deleting anything that would lose work. Pass `-y`/`--yes` to skip the
  question, for example in scripts.
- `<TAB>` completes commands, workspaces, repos, `@presets` and remote branches (`web:<TAB>`).
- `wt help <command>` shows each command's options.

A typical loop:

```sh
wt add feature/login @app    # start: fresh branches from an up-to-date main
claude                       # or your editor/agent of choice, from the workspace folder
git push                     # per repo, open PRs, get them merged
wt cleanup                   # once the remote branches are deleted, the workspace goes away
```

## Docs

- [docs/concepts.md](docs/concepts.md): layout, naming, specs, presets, `.worktreeinclude`
- [docs/commands.md](docs/commands.md): exactly what each command does
- [docs/shell.md](docs/shell.md): shell integration and completion
- [docs/design.md](docs/design.md): output conventions, architecture, testing, non-goals and ideas for later

## Development

```sh
cargo build
cargo test                 # unit tests + integration tests (tests/workflow.rs)
cargo install --path .
```

The integration tests build a temporary `$WT_ROOT` with local bare "remotes", so they need nothing from the network.
They do need to run `git worktree add`, which some sandboxed agent environments block.

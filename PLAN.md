# wt (Rust) — plan

A small Rust CLI, inspired by `~/wt/.bin/wt` (zsh), for working on one feature across several repos.
The Rust source lives in `~/ideas/wt`, installed with `cargo install --path .` (binary on `PATH`).

## 0. Purpose
**The workspace folder (`$WT_ROOT/worktrees/<feature>/`) is the main unit.** It collects one feature's worktrees across
several applications (e.g. `alde` + `pen`), so that you, and **agents started in the workspace folder**, see all the repos
needed and can work across them.

- `wt add` creates the workspace (or adds repos to it) and `cd`s you into it. `wt cd <feature>` takes you back later.
- Repos are always direct children (`<feature>/<repo>/`) with stable names.
- Presets (`@alde`) create a feature's usual set of repos in one command.
- Base repos are plain clones that hold secrets and IDE config. The files you list in `.worktreeinclude` are copied into each worktree.
- `wt` doesn't manage agent config. It lives in the repos themselves, or in a parent dir that the harness finds by walking up.

**Non-goals**: managing base repos beyond keeping `main` current (just use `git clone`), sync, agent config, non-zsh shells.

---

## 1. Layout and concepts

```
$WT_ROOT/
├── .wt.toml                  # optional: presets
├── base/<repo>/              # plain `git clone`s on the default branch; secrets/IDE config live here
└── worktrees/<feature>/      # workspace folder: where you and agents work
    └── <repo>/               # git worktree (has a `.git` *file*)
```

- **Root** = `$WT_ROOT`, required. If it's unset or missing: `error: WT_ROOT is not set` + `hint: export WT_ROOT=~/wt`.
- **Repo** = any dir in `$WT_ROOT/base/` with a `.git` dir. Add one with `git clone <url> $WT_ROOT/base/<name>`.
  - Default branch = `git symbolic-ref --short refs/remotes/origin/HEAD` (strip `origin/`), fallback `main`.
    (Not `HEAD`, which is just whatever base has checked out.)
  - If no repos are found: `hint: clone repos into $WT_ROOT/base/, e.g. git clone <url> $WT_ROOT/base/<name>`.
- **Workspace** = a dir in `$WT_ROOT/worktrees/`. It's created on demand.
- **Worktree** = a `<feature>/<repo>/` dir whose `.git` is a **file**.
- **Flat names**: the workspace dir name is the feature name with `/` (and anything outside `[A-Za-z0-9._-]`) replaced by
  `-`. The branch keeps the original name: `wt add feature/login alde` → branch `feature/login` in `worktrees/feature-login/alde/`.
  Lookups (`add`, `rm`, `cd`) accept either form.
- **Spec** = `repo`, `repo:branch` or `@preset`. Without `:branch`, the branch is the feature name.

### Presets (`$WT_ROOT/.wt.toml`)
```toml
[presets]
alde = ["alde", "pen"]
full = ["alde", "pen", "infra"]
```
- `@alde` expands to its repo names (plain lists, no branches). Presets have their own namespace, so `@alde` and the
  repo `alde` don't clash.
- Specs are expanded, then de-duplicated by repo. An explicit `repo:branch` wins: `wt add x @alde pen:fix-123`.
- An unknown preset or a missing repo errors **before** anything is fetched or created.

### `.worktreeinclude` (per repo, `base/<repo>/.worktreeinclude`)
Uses `.gitignore` syntax. It's the same file Claude Code reads for `claude --worktree`.
```gitignore
.env
.env.*
.idea/
*.iml
src/main/resources/application-local.yml
```
- Files = `git -C base/<repo> ls-files --others --ignored --exclude-from=.worktreeinclude` (untracked files matching the
  patterns; tested). Don't use `--directory`, because it also returns untracked parent dirs that merely *contain* a match.
- Only files that are also **gitignored** (`git check-ignore`) are copied, as in Claude Code. Others are skipped with a warning.
- Copy, preserving permissions. **Never overwrite**: files that already exist in the worktree are skipped.
- Whether `.worktreeinclude` itself is committed or ignored is up to you. `wt` only reads it.

---

## 2. Commands

### `wt add <feature> <spec...>`
One command for both creating a workspace and adding repos to it.
- Error if no specs are given.
- **Workspace missing** → create it. **Workspace exists** → add to it.
- Validate/expand specs, then **update base** for the involved repos (see "Updating base", in parallel), then
  create each missing worktree.
- **Branch** for a plain `repo` spec:
  - new workspace → the feature name as given (`feature/login`);
  - existing workspace → the branch its worktrees share (keeps `feature/login` correct even if you typed
    `feature-login`). If they don't share one, use the feature name as given.
- To check out an existing branch, use `repo:branch`. Completion lists the remote branches (`pen:<TAB>`).
- For a repo that already has a worktree: just copy any missing `.worktreeinclude` files (`· alde  already there,
  copied 1 file`). This also covers the case "I added a new secret to base".
- The last line says which case happened: `Created ~/wt/worktrees/feature-login (alde, pen)` or
  `Added infra to feature-login`. That way a typo in the feature name is easy to spot.
- **cd**: into the workspace, unless your shell is already somewhere inside it (so you stay in the repo you're in).

### Updating base (before any worktree is created)
New branches are always based on an **up-to-date local `main`** in base. Per involved repo:
1. `git -C base/<r> fetch --prune origin`
2. Check that base is on the default branch and has no tracked changes (`status --porcelain --untracked-files=no`;
   ignored/untracked files like `.env` and `.idea/` don't count).
3. `git -C base/<r> merge --ff-only origin/<default>`

If step 2 or 3 fails (base on another branch, tracked changes, or a local `main` that has diverged), **stop before creating
anything**, with e.g. `error: base/pen is on branch 'x', expected 'main'` + `hint: cd $WT_ROOT/base/pen && git switch main`.
Base should always be a clean `main`, so this only fires when something is actually wrong.

### Worktree creation (core)
For repo `r`, branch `b`, path `worktrees/<feature>/<r>`:
1. If `b` is already checked out somewhere (`git worktree list --porcelain`, base included), fail and say where.
   For example, the default branch is held by base.
2. Strategy:
   - local `refs/heads/<b>` exists → `git worktree add <path> <b>`
   - else `refs/remotes/origin/<b>` exists → `git worktree add --track -b <b> <path> origin/<b>`
   - else → `git worktree add --no-track -b <b> <path> <default>`: a **new branch from the freshly updated local `main`**.
     `--no-track` keeps `main` from becoming the upstream (it gets set on the first `git push -u`, or automatically
     with `push.autoSetupRemote=true`).
3. If there's no upstream and `origin/<b>` exists → `git branch --set-upstream-to=origin/<b>`.
4. Copy `.worktreeinclude` files.

### `wt rm <feature> [repo]`
- Check per worktree (no fetch; uses the remote refs from the last `wt add`):
  - **Uncommitted changes** (`status --porcelain`) → `! alde  2 uncommitted changes`.
  - **Unpushed commits**, upstream exists (`@{upstream}..HEAD` > 0) → `! alde  3 unpushed commits`.
    If no upstream is configured but `refs/remotes/origin/<b>` exists (pushed without `-u`), that ref counts as the upstream.
  - **Never pushed** (no upstream configured) with commits not on main (`<default>..HEAD` > 0) →
    `! alde  3 commits never pushed`.
  - **Upstream gone** (`branch.<b>.merge` is set but `refs/remotes/origin/<b>` no longer exists): the branch was pushed
    and the remote branch deleted, typically after a (squash) merge. Treated as **safe**, with no prompt:
    `· alde  remote branch deleted (merged)`.
    This needs the upstream to have been recorded at push time, so **`git config --global push.autoSetupRemote true`**
    is part of setup. `wt add` prints a one-time hint if it's not set. Without it, a merged branch looks "never pushed":
    it's kept and warned about, never lost.
  - If there's a `!` warning, list them and ask `Remove anyway? [y/N]`.
- `git worktree remove --force`. Fall back to `rm -rf` + `git worktree prune`.
- **Branch cleanup** in base (`git branch -D`):
  - **Delete** if the branch was safe: fully pushed, upstream gone, or no commits beyond `main`.
  - **Keep** if you confirmed past a `!` warning about commits: `· alde  kept branch feature/login (unpushed commits)`.
    A kept branch is reused if you later `wt add` the same name.
- Removing the last repo (or the whole feature) deletes the workspace dir.
- If your shell was inside a removed dir, it `cd`s to `$WT_ROOT/worktrees`.

### `wt cleanup`
Removes finished workspaces: the ones where every remote branch has been deleted (i.e. merged, including squash merges).
1. `git fetch --prune origin` in every base repo that has worktrees (in parallel), so deleted remote branches show up
   as "upstream gone". Base `main` isn't touched.
2. For each workspace, classify every worktree with the same checks as `rm`:
   - **done**: upstream gone, or never pushed and no commits beyond `main` (nothing to lose);
   - **active**: the upstream still exists, or there are `!` warnings (uncommitted changes, unpushed/never-pushed commits).
3. A workspace is a candidate only if **all** its worktrees are done. Workspaces where only some repos are done are listed
   but left alone:
   ```
   feature-login   alde, pen   remote branches deleted
   fix-npe         pen         remote branch deleted
   · feature-x     alde done, pen active (2 uncommitted changes); skipped
   Remove 2 workspaces? [y/N]
   ```
4. One confirm for all candidates, then remove each as `rm` does (worktrees, local branches, workspace dir).
   If your shell was inside one, it `cd`s to `$WT_ROOT/worktrees`.
- Nothing to clean: `Nothing to clean up.`

### `wt ls`
```
feature-login
  alde   feature/login   2h   Fix token refresh   ↑2      ~1 ?3
  pen    feature/login   1d   Add login form      ↑1 ↓4
```
Columns: repo, branch (dim if default), age, subject (truncated to the terminal width), `↑ahead`/`↓behind` vs
`origin/<default>`, `+staged ~modified ?untracked`. Status is gathered in parallel, and nothing is fetched.
Empty: `No workspaces. Create one with: wt add <feature> <repo>`.

### `wt cd [feature]`
- `wt cd feature-login` (or `feature/login`) → the workspace folder. Use `<TAB>` to complete workspace names.
- No exact match, but exactly one workspace contains the text → jump there. Several → error listing them.
- No args → `$WT_ROOT/worktrees`.

### `wt shell-init zsh`
Prints the `wt()` function and the completion. Add `eval "$(wt shell-init zsh)"` to `.zshrc`.
- `wt()` runs `command wt "$@"` with `WT_CD_FILE=<tempfile>`. Afterwards it `cd`s into the path written there, if any.
- `cd`, `add`, `rm` and `cleanup` write that file. Stdout isn't parsed, so prompts and output behave normally.
- Without shell-init: `wt cd` prints the path (`cd "$(wt cd x)"` works), and `add` prints a `cd <path>` hint.

### `wt help` / `-h` / `--help`
Generated by clap. Unknown command → error + usage, exit 2.

---

## 3. Output style
- **stdout is for data** (`ls`, the `cd` path without shell-init). **stderr is for everything else** (progress,
  warnings, errors, prompts).
- **Quiet on success**: one line per action. Git output is shown only on failure (indented under the error).
- **Symbols**: `✓` green done, `·` dim skipped/already there, `!` yellow warning, `✗` red failed. No emoji.
- **Errors**: `error: …` (red), plus `hint: …` where useful. Exit 1.
- Aligned columns, `~` instead of `$HOME`, color only on a TTY. Respects `NO_COLOR` / `CLICOLOR_FORCE`.
- No spinners. A slow step prints one line first (`Updating alde, pen…`) and finishes it with `✓`.
- The only prompts are the `[y/N]` confirms in `rm` and `cleanup`, read from the TTY. On a non-TTY they refuse
  (`hint: run it in a terminal`).

```
$ wt add feature/login @alde
Updating alde, pen ✓
✓ alde  new branch feature/login from main, copied 3 files
✓ pen   tracking origin/feature/login, copied 1 file
Created ~/wt/worktrees/feature-login (alde, pen)
```

---

## 4. zsh completion
A hand-written `_wt` embedded in the binary and printed by `wt shell-init zsh` (with `compdef _wt wt`).
Dynamic values come from a hidden `wt __complete <kind>`: `specs` (repos + `@presets`), `branches <repo>` (remote
branches, most recent first, for `repo:<TAB>`), `workspaces`, `workspace-repos <feature>`.
- `add <feature> <specs...>` (feature completes existing workspaces too), `rm <feature> <repo-in-it>`, `cd <feature>`, plus `cleanup` in the subcommand list.

---

## 5. Architecture

```
src/
├── main.rs        # clap CLI + dispatch
├── root.rs        # $WT_ROOT, paths, repo discovery in base/
├── config.rs      # .wt.toml presets (serde + toml)
├── spec.rs        # spec parsing, preset expansion, validation
├── git.rs         # `git` subprocess wrapper (captures stderr for errors)
├── worktree.rs    # base update (fetch + ff main), create/remove, branch strategy, default-branch lookup
├── include.rs     # .worktreeinclude copying
├── workspace.rs   # list/lookup workspaces, name sanitizing
├── status.rs      # dirty/ahead/behind (ls, rm)
├── ui.rs          # output style, y/N confirm, WT_CD_FILE
├── shell/         # wt.zsh (function + _wt), include_str!
└── cmd/{add,rm,cleanup,ls,cd,shell_init,complete}.rs
```

**Crates**: `clap` (derive), `anyhow`, `serde` + `toml`, `anstream`/`anstyle`, `terminal_size`. No prompt/spinner crates.

**Git**: shell out to `git`, which gives the same behavior as the zsh version: credentials/SSH just work, and `git2`/`gix`
worktree support is weak. Parallelism via `std::thread::scope` (base update in `add`, status in `ls`).

**Not interactive** apart from the confirms in `rm`/`cleanup`, so `wt` works the same from scripts and agents.

---

## 6. Phases
1. Skeleton: CLI, `$WT_ROOT`, repo discovery, git wrapper, `ui`.
2. `ls`, `cd`, `shell-init zsh` (`WT_CD_FILE`), workspace lookup/sanitizing.
3. `add` (create + extend, specs + presets), `.worktreeinclude` copying.
4. `rm` with the dirty check and branch cleanup, then `cleanup` (reuses the same checks).
5. zsh completion (`_wt` + `__complete`).
6. Integration tests: a temp `$WT_ROOT` with a local bare "remote" + clones in `base/`.
7. Cutover: point `WT_ROOT` at a fresh folder, `git clone` the repos into `base/`, add `eval "$(wt shell-init zsh)"`,
   and `git config --global push.autoSetupRemote true`.

## 7. Later (not v1)
`--json` for `ls`, `wt open` (IDE), forced re-copy of include files, PR state via `gh` in `cleanup`.

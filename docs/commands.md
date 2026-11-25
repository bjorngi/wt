# Commands

All commands except `shell-init` need `$WT_ROOT`. Errors exit with code 1, and usage errors (unknown command, missing
argument) exit with code 2.

- [`wt add`](#wt-add)
- [`wt rm`](#wt-rm)
- [`wt cleanup`](#wt-cleanup)
- [`wt ls`](#wt-ls)
- [`wt cd`](#wt-cd)
- [`wt shell-init`](#wt-shell-init)

---

## `wt add`

```
wt add <feature> <spec>...
```

Creates the workspace if it doesn't exist, otherwise adds the repos to it. At least one spec is required (see
[specs](concepts.md#specs)).

**Branch for a plain `repo` / `@preset` spec**

- New workspace: the feature name as typed (`feature/login`).
- Existing workspace: the branch that all its worktrees share. That way `wt add feature-login worker` still uses
  `feature/login`. If the worktrees aren't all on the same branch, the feature name as typed is used.

**Steps**

1. Validate everything: specs, presets, branch names, and that `<feature>/<repo>` isn't some non-worktree folder.
2. For the repos that need a new worktree: **update base** for each of them in parallel (see below), then check that
   none of the target branches is already checked out in another worktree, base included.
3. Create each worktree, then copy its [`.worktreeinclude`](concepts.md#worktreeinclude) files.
4. For a repo that's already in the workspace: copy only the missing include files (`· api  already there, copied 1 file`).

Any failure in steps 1–2 stops `wt` **before anything is created**. If creating a worktree fails in step 3, it stops
there. A brand-new workspace folder that ends up empty is removed again.

**Updating base**, per repo:

1. `git fetch --prune origin`
2. Base must be on the default branch with no changes to tracked files. Untracked and ignored files like `.env` and
   `.idea/` don't count.
3. `git merge --ff-only origin/<default>`

If base is on another branch, has a detached HEAD, has tracked changes, or has a local default branch that diverged
from origin, `wt` stops with an error and a hint:

```
error: base/web is on branch 'x', expected 'main'
hint: cd ~/wt/base/web && git switch main
```

**Worktree creation** for branch `b`:

| Situation | Command | Output |
|---|---|---|
| local `refs/heads/b` exists | `git worktree add <path> b` | `existing branch b` |
| `origin/b` exists | `git worktree add --track -b b <path> origin/b` | `tracking origin/b` |
| neither | `git worktree add --no-track -b b <path> <default>` | `new branch b from main` |

A new branch starts from the freshly updated local default branch, with no upstream. The upstream gets set on the first
`git push` if `push.autoSetupRemote` is on. If you haven't set that, `wt` prints a hint after creating a new branch.
When the branch has no upstream but `origin/b` exists, the upstream is set to `origin/b`.

**Last line** says which case happened, so a typo in the feature name is easy to spot:

```
Created ~/wt/worktrees/feature-login (api, web)
Added worker to feature-login
feature-login is up to date (api, web)
```

**cd**: afterwards your shell moves into the workspace, unless it's already somewhere inside it. Without shell
integration, `wt` prints `hint: cd <path>` instead.

---

## `wt rm`

```
wt rm <feature> [repo] [-y|--yes]
```

Removes the whole workspace, or a single repo's worktree from it. It doesn't fetch; it uses the remote refs from the last
`add` or `cleanup`.

**Checks** per worktree:

| State | Shown as | Safe to delete the branch? |
|---|---|---|
| uncommitted changes (`git status --porcelain`, untracked files included) | `! api  2 uncommitted changes` | — (prompts) |
| upstream exists, commits not on it | `! api  3 commits not pushed` | no |
| no upstream configured, but `origin/<b>` exists (pushed without `-u`) | compared against `origin/<b>`, like the row above | if fully pushed |
| no upstream, commits beyond `origin/<default>` | `! api  3 commits never pushed` | no |
| upstream configured but `origin/<b>` is gone (deleted after merge) | `· api  remote branch deleted (merged)` | yes |
| fully pushed, or no commits beyond main | — | yes |
| detached HEAD with commits not on any remote | `! api  detached HEAD with 1 commit not on any remote` | — |

When the whole workspace is being removed and it contains anything other than worktrees (notes, scratch files), `wt`
warns about that too.

If there are any `!` warnings, `wt` asks `Remove anyway? [y/N]`. The prompt needs a terminal. Without one, `wt` refuses
with `hint: rerun with --yes`.

**Removal**

- `git worktree remove --force`. If that fails, the folder is deleted and `git worktree prune` is run.
- The local branch in base is **deleted** (`git branch -D`) if that's safe (see the table above). If commits would be lost
  it's **kept**: `✓ api  removed, kept branch feature/login (unpushed commits)`. A later `wt add` with the same branch
  name reuses a kept branch.
- Removing the last repo, or the whole feature, deletes the workspace folder.
- If your shell was inside a removed folder, it moves to the workspace (if it still exists) or to `$WT_ROOT/worktrees`.

> Detecting "merged" depends on the upstream having been recorded when you pushed (`push.autoSetupRemote`, or
> `git push -u`). Without it, a merged branch looks like "never pushed": `wt` keeps it and warns, so nothing is lost.

---

## `wt cleanup`

```
wt cleanup [-y|--yes]
```

Removes finished workspaces, meaning those whose remote branches have all been deleted (merged, squash merges included).

1. `git fetch --prune origin` in every base repo that has worktrees, in parallel. Base's default branch is left as it is.
   A repo whose fetch fails is reported, and its workspaces are skipped.
2. Each worktree is classified with the same checks as `rm`:
   - **done**: no uncommitted changes, and either the upstream is gone or it was never pushed with no commits beyond main;
   - **active**: everything else.
3. A workspace is a **candidate** if all its worktrees are done **and** at least one remote branch was deleted. This
   leaves freshly created, untouched workspaces alone. Empty workspaces (no worktrees left) are candidates too.
   Workspaces where only some repos are merged are listed with the reason and skipped.

```
Fetching api, web… ✓
  feature-login  api, web  remote branches deleted
  fix-npe        web       remote branch deleted
· partial        skipped: web 1 uncommitted change
Remove 2 workspaces? [y/N]
```

4. After one confirmation for all candidates, each one is removed the same way `rm` does it: worktrees, local branches,
   and the workspace folder. If a workspace contains other files, the list says `(also contains: …)`. If your shell was
   inside a removed workspace, it moves to `$WT_ROOT/worktrees`.

When there's nothing to remove, it prints `Nothing to clean up.`

---

## `wt ls`

```
wt ls
```

```
feature-login
  api  feature/login  2h  Fix token refresh  ↑2     ~1 ?3
  web  feature/login  1d  Add login form     ↑1 ↓4
```

Columns: repo, branch (dim when it's the default branch), age of the last commit, its subject (truncated to fit the
terminal), `↑ahead`/`↓behind` compared with `origin/<default>`, and `+staged ~modified ?untracked`. Status is gathered
in parallel, and nothing is fetched. The output goes to stdout.

With no workspaces it prints `No workspaces. Create one with: wt add <feature> <repo>`.

---

## `wt cd`

```
wt cd [feature]
```

- `wt cd feature-login` or `wt cd feature/login` goes to that workspace.
- If there's no exact match but exactly one workspace contains the text (case-insensitive), it goes there: `wt cd npe` →
  `fix-npe`. If several match, it's an error that lists them.
- With no argument it goes to `$WT_ROOT/worktrees`.
- Without shell integration it prints the path to stdout, so `cd "$(wt cd login)"` works.

---

## `wt shell-init`

```
wt shell-init zsh
```

Prints the zsh function and completion. See [shell.md](shell.md).

---

## `wt help`

`wt help`, `wt -h`, `wt --help` and `wt help <command>` come from clap. `wt --version` prints the version.

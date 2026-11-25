# Concepts

## Purpose

**The workspace folder (`$WT_ROOT/worktrees/<feature>/`) is the main unit.** It gathers one feature's worktrees from
several repos (for example `api` and `web`). You, and any agent started in the folder, see every repo the feature needs
and can work across them.

- `wt add` creates the workspace (or adds repos to it) and `cd`s you into it. `wt cd <feature>` takes you back later.
- Each repo is a direct child with a stable name: `<feature>/<repo>/`.
- Presets (`@app`) create a feature's usual set of repos in one command.
- `wt` doesn't manage agent config. Keep that in the repos themselves, or in a parent folder that your harness finds by
  walking up the tree.

## Layout

```
$WT_ROOT/
├── .wt.toml                  # optional: presets
├── base/<repo>/              # plain `git clone`s on the default branch
└── worktrees/<feature>/      # workspace folder
    └── <repo>/               # git worktree (its `.git` is a file)
```

| Term | Meaning |
|---|---|
| **Root** | `$WT_ROOT`, which is required. `~` is expanded. If it's unset or doesn't exist, `wt` stops with an error and a hint. |
| **Repo** | Any folder in `$WT_ROOT/base/` with a `.git` *directory*. To add one: `git clone <url> $WT_ROOT/base/<name>`. |
| **Default branch** | `origin/HEAD` without the `origin/` prefix. If that's missing: `master` when only `master` exists locally, otherwise `main`. It is never base's current `HEAD`. |
| **Workspace** | A folder in `$WT_ROOT/worktrees/`, created on demand. Names starting with `.` are ignored. |
| **Worktree** | A `<feature>/<repo>/` folder whose `.git` is a *file*. Anything else in a workspace (such as notes) is left alone, but `wt` warns about it before deleting the workspace. |

If `origin/HEAD` isn't set (some clones don't have it), run `git remote set-head origin --auto` in the base repo.

## Feature names

The workspace folder name is the feature name with `/` and every character outside `[A-Za-z0-9._-]` replaced by `-`.
The **branch keeps the original name**:

```
wt add feature/login api   →   branch feature/login   in   worktrees/feature-login/api/
```

`add`, `rm` and `cd` accept both forms. A name that is empty, starts with `.`, or contains only `-` once sanitized is
rejected.

## Specs

| Spec | Meaning |
|---|---|
| `repo` | That repo, on the workspace's branch (see [`wt add`](commands.md#wt-add)). |
| `repo:branch` | That repo, on `branch`. The branch may contain `/`. |
| `@preset` | Every repo in the preset, each on the workspace's branch. |

Specs are expanded and then de-duplicated by repo. An explicit `repo:branch` wins over a plain `repo` no matter the order
(`wt add x @app web:fix-123`). Two different explicit branches for the same repo is an error. So are an unknown repo, an
unknown preset, `@preset:branch` and an invalid branch name. All of these fail **before** anything is fetched or created.

## Presets (`$WT_ROOT/.wt.toml`)

```toml
[presets]
app = ["api", "web"]
full = ["api", "web", "worker"]
```

- `@app` expands to plain repo names. Members can't have branches or refer to other presets.
- Presets have their own namespace, so a preset `@web` and the repo `web` don't clash.
- The file is optional. Unknown keys make it invalid, which catches typos.

## `.worktreeinclude`

`base/<repo>/.worktreeinclude` lists local files to copy from base into each new worktree, such as secrets and IDE config.
It uses `.gitignore` syntax and is the same file Claude Code reads for `claude --worktree`.

```gitignore
.env
.env.*
.idea/
*.iml
src/main/resources/application-local.yml
```

- Candidates are the untracked files in base that match the patterns
  (`git ls-files --others --ignored --exclude-from=.worktreeinclude`).
- Only files that are **also gitignored** in the repo get copied, so a secret can't end up in a commit. A matching file
  that isn't ignored is skipped with `! repo  not copied (not gitignored): <file>`.
- Files are copied with their permissions. **Existing files are never overwritten.** Re-running `wt add <feature> <repo>`
  on a repo that's already in the workspace copies only the files that are missing, for example a secret you just added
  to base.
- You decide whether `.worktreeinclude` itself is committed or ignored. `wt` only reads it.

# Shell integration (zsh)

```sh
# ~/.zshrc, after compinit
export WT_ROOT=~/wt
eval "$(wt shell-init zsh)"
```

`wt shell-init zsh` prints [`src/shell/wt.zsh`](../src/shell/wt.zsh), which is built into the binary. It contains:

## The `wt` function

A program can't change its parent shell's directory, so the function does it:

1. Creates a temp file and runs the binary with `WT_CD_FILE=<tempfile>`.
2. `cd`, `add`, `rm` and `cleanup` write the target folder to that file when they want you to move.
3. After the binary exits, the function `cd`s to that folder (if one was written and still exists), deletes the temp file,
   and returns the binary's exit code.

`wt` never parses stdout, so prompts, colors and piping work as normal.

The function calls the binary by the **absolute path** that `shell-init` was run from, so an older `wt` earlier in
`PATH` can't take over. If you move the binary, open a new shell (or `eval` again).

Without the integration everything still works except changing directory: `wt cd x` prints the path, and `wt add` prints
a `cd <path>` hint.

## Completion

The `_wt` function is registered with `compdef _wt wt` when `compinit` has already run.

| Position | Completes |
|---|---|
| `wt <TAB>` | subcommands |
| `wt add <TAB>` | existing workspaces (you can also type a new name) |
| `wt add f <TAB>` | repos and `@presets` |
| `wt add f web:<TAB>` | `origin` branches of `web`, most recent first |
| `wt rm <TAB>` / `wt cd <TAB>` | workspaces |
| `wt rm f <TAB>` | the repos in workspace `f` |
| `wt rm -<TAB>` / `wt cleanup -<TAB>` | `--yes` |
| `wt shell-init <TAB>` | `zsh` |

The dynamic values come from a hidden command that prints one item per line, and prints nothing at all if something
goes wrong:

```
wt __complete specs                     # repos + @presets
wt __complete branches <repo>           # remote branches, newest first
wt __complete workspaces
wt __complete workspace-repos <feature>
```

Branch completion uses the remote refs from the last fetch and doesn't fetch on its own.

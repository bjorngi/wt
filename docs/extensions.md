# Extensions (proposal)

> **Status: partly implemented.** Phases 1–2 are done: the `Provider` trait, registry, cache, deadline, and GitHub
> pull request + checks status in `wt ls` (see [commands.md](commands.md#providers-github)). The merged signal (phase 3)
> and `repo#ref` resolver (phase 4) are still proposals.
>
> Differences between what was built and the sketches below:
> - `Status` has no `merged` field yet, and the trait has no `resolve`. Both get added in their own phase, so there's
>   no unused code.
> - `RepoInfo` is just `name` + `remote_url`, taken from `remote.origin.url` as configured, so `insteadOf`
>   rewrites don't hide the GitHub host.
> - A pushed branch without a PR shows `no PR` plus its checks, taken from the branch's head commit via `ref(...)`.
> - `Tone::Info` (merged) renders in magenta.
> - `CacheMode::CacheOnly` is called `Offline`. All entries share one 2-minute TTL for now, and entries older than
>   30 days are pruned when the cache is written.

## Goal

Let `wt` use information it can't get from git alone, mainly the state of the pull request for each worktree's branch.
That information should:

- show up in `wt ls`: `#123 ✓ approved`, `#124 draft`, `#120 merged`;
- tell `rm` and `cleanup` that a branch is finished even when the remote branch wasn't deleted, or when
  `push.autoSetupRemote` was off;
- let `wt add` resolve references: `wt add review-123 web#123` checks out PR 123's branch.

GitHub is the first integration, but nothing outside its module should know about GitHub. GitLab, Bitbucket or an issue
tracker should be able to plug in the same way.

## Approach

Each integration is a **provider**: a Rust type that implements a `Provider` trait and is compiled into `wt`. The rest
of `wt` talks only to the trait, through a registry.

That choice is deliberate for v1. It's typed, fast and easy to test, and it adds no protocol to keep stable. The trait's
input and output types derive `Serialize`/`Deserialize` from the start, so an `ExternalProvider` (an executable speaking
JSON) can implement the same trait later without changing any caller. Lifecycle hooks (shell commands run after
`add`/`rm`) are a separate, later addition (see [Later](#later)).

## Hook points

| Hook point | Used by | Provider method | Network |
|---|---|---|---|
| **Status** | `ls` | `status(items)` | yes, cached |
| **Merged signal** | `rm`, `cleanup` | part of `status` (`Status.merged`) | `cleanup`: fresh; `rm`: cache only |
| **Spec resolver** | `add` | `resolve(repo, reference)` | yes |

## The trait

```rust
// src/provider/mod.rs

/// What a provider needs to know about a repo. Built once per run, no network.
#[derive(Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,           // dir name in base/
    pub remote_url: String,     // `git remote get-url origin`
    pub default_branch: String,
}

/// One worktree to report on.
#[derive(Clone, Serialize, Deserialize)]
pub struct Item {
    pub repo: String,
    pub branch: String,
    pub head: String,           // local HEAD sha
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Status {
    /// Short pieces of text for `ls`, rendered in order: ["#123", "✓", "approved"].
    pub badges: Vec<Badge>,
    pub url: Option<String>,
    /// Set when the provider knows the branch was merged.
    pub merged: Option<Merged>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Badge {
    pub text: String,
    pub tone: Tone,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum Tone { Ok, Warn, Fail, Info, Dim }

#[derive(Clone, Serialize, Deserialize)]
pub struct Merged {
    /// The branch head the provider saw when it was merged (e.g. a PR's `headRefOid`).
    pub head: String,
    pub label: String,          // "PR #123 merged"
}

/// Result of resolving `repo#ref`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Resolved {
    pub branch: String,
    pub label: String,          // "PR #123: Fix token refresh"
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Whether this provider handles the repo, decided from local data only (e.g. the remote host).
    fn handles(&self, repo: &RepoInfo) -> bool;

    /// Status for several worktrees of one repo, in a single batch. Returns one entry per item,
    /// with `None` meaning "nothing to say" (e.g. no PR).
    fn status(&self, repo: &RepoInfo, items: &[Item], deadline: Instant) -> Result<Vec<Option<Status>>>;

    /// Resolve a reference such as `123` in `web#123`. `Ok(None)` = not a reference this provider understands.
    fn resolve(&self, _repo: &RepoInfo, _reference: &str) -> Result<Option<Resolved>> {
        Ok(None)
    }
}
```

Design notes:

- **Batching per repo.** Providers get every worktree of a repo at once, so GitHub can answer with one GraphQL request
  per repo instead of one per branch.
- **Providers return data, not formatted text.** `wt` renders badges with its own colors and symbols, so every
  provider's output looks the same. Tones map to the existing palette: `Ok` green, `Warn` yellow, `Fail` red, `Info`
  blue, `Dim` dim.
- **Optional capabilities have default methods.** A provider that only shows status doesn't need `resolve`.
- **`deadline`** is passed in so providers can stop their own subprocesses in time (see [Performance](#performance)).

## Registry

```rust
pub struct Registry { providers: Vec<Box<dyn Provider>> }

impl Registry {
    pub fn from_config(config: &Config) -> Registry;            // which providers are enabled
    pub fn for_repo(&self, repo: &RepoInfo) -> Option<&dyn Provider>;   // first one that handles it
    pub fn status_all(&self, repos: &[(RepoInfo, Vec<Item>)], mode: CacheMode) -> StatusMap;
    pub fn resolve(&self, repo: &RepoInfo, reference: &str) -> Result<Resolved>;
}
```

- One provider per repo, the first whose `handles` returns true. Combining several providers on one repo (say GitHub
  for PRs plus Jira for issues) can be added later by merging badge lists.
- `status_all` runs the providers in parallel per repo (`std::thread::scope`), applies the cache, and turns failures
  into one warning per provider, not per row.

## Configuration

```toml
# $WT_ROOT/.wt.toml
[github]
# enabled = true                 # default: on when `gh` is on PATH
hosts = ["github.com"]           # add GitHub Enterprise hosts here
```

- A provider with no config section uses its defaults. GitHub turns on automatically when `gh` is installed. If it's
  enabled explicitly but `gh` is missing, `wt` warns.
- `Config` keeps `deny_unknown_fields`, so each provider adds its own typed section.

## Cache

`ls` always asks the providers, but through a cache, so most runs don't touch the network.

- **Location**: `$WT_ROOT/.wt/cache/<provider>.json`. Written atomically (temp file + rename), so concurrent `wt`
  processes are safe.
- **Key**: `(repo, branch, head sha)`. A new commit or a different branch is a new key, so pushing makes `ls` refetch.
- **TTL**: 2 minutes for open or unknown state. **Merged** entries never expire, because a merge is permanent and the
  head sha in the key pins it.
- **Modes**:
  | Mode | Used by | Behavior |
  |---|---|---|
  | `Normal` | `ls` | use fresh entries, fetch stale or missing ones, and fall back to stale entries if the fetch fails |
  | `Refresh` | `cleanup`, `ls --refresh` | always fetch; fall back to the cache only on failure |
  | `CacheOnly` | `rm`, `ls --offline`, completion | never touch the network |
- Stale entries shown because a fetch failed are rendered dim.

## Performance

`ls` used to be instant and offline. It stays usable this way:

- Provider calls run in parallel per repo, alongside the existing git status gathering.
- There's a **hard deadline of 3 seconds** for the whole provider phase. Subprocesses still running after it are killed
  (poll `try_wait`, no extra crate needed), and those rows fall back to the cache or show nothing.
- `ls --offline` skips the network completely.

## Hook point: status in `ls`

A new last column, left-aligned:

```
feature-login
  api  feature/login  2h  Fix token refresh  ↑2     ~1 ?3  #123 ✓ approved
  web  feature/login  1d  Add login form     ↑1 ↓4         #124 draft
```

- The subject column's width budget subtracts this column's width.
- When stderr/stdout is a TTY, `#123` becomes a clickable link (OSC 8) to `Status.url`.
- Worktrees on the default branch, or with no PR, show nothing.
- If a provider fails, `ls` prints a single warning at the end, such as
  `! github  gh is not authenticated (gh auth login)`, and never fails as a whole.

## Hook point: merged signal (`rm`, `cleanup`)

A new variant in `status.rs`:

```rust
pub enum Commits {
    // ...existing variants
    /// The provider reports the branch merged, and local HEAD is covered by the merged head.
    Merged(String),             // label, e.g. "PR #123 merged"
}
```

**Safety rule.** A provider's "merged" is trusted only if the local `HEAD` is **equal to `Merged.head`, or an ancestor
of it** (`git merge-base --is-ancestor HEAD <head>`, when that object exists locally). Otherwise someone committed after
the merge, so `wt` ignores the signal and falls back to the existing git-only checks. Uncommitted changes still count as
before.

In `status::check`, a safe merged signal overrides the git-only result: `Pushed`, `Gone`, `Unpushed` and `NeverPushed`
all become `Merged`. Without a signal, or with an unsafe one, the git-only result stands.

- `Merged` is **safe** (the branch is deleted on `rm`) and **done** (counts for `cleanup`).
- A `cleanup` candidate needs at least one repo that is `Gone` **or `Merged`**. This is how a merged PR whose remote
  branch still exists gets cleaned up.
- `rm` uses `CacheOnly` and keeps its no-network promise. If an earlier `ls` saw the merge, `rm` shows
  `· api  PR #123 merged` and doesn't prompt.
- `cleanup` uses `Refresh` after its fetch. If a provider fails, it warns and falls back to git-only signals. It never
  treats a failure as "merged".

## Hook point: spec resolver (`add`)

New spec form: **`repo#ref`**, such as `web#123`.

```
$ wt add review-123 web#123
Resolving web#123… ✓ PR #123: Fix token refresh (fix/token)
Updating web… ✓
✓ web  tracking origin/fix/token
Created ~/wt/worktrees/review-123 (web)
```

- Parsing: a spec that contains no `:` but contains `#` is a reference. `repo:branch` is unchanged, and a branch
  containing `#` still works through `repo:branch`.
- `Spec` gets `reference: Option<String>`. `spec::expand` stays offline, and validates the repo name plus the "one of
  branch/reference" rule (`web#1 web:x` is a conflict).
- Resolution runs in `add` after expansion and **before base update**, in parallel, all-or-nothing. A reference that
  can't be resolved stops `wt` before anything is created, like every other validation error.
- The resolved branch then follows the normal path (local → tracking `origin/<b>` → new).
- References that aren't understood give `error: no provider for web can resolve '#abc'`.

## The GitHub provider

`src/provider/github.rs`. It calls the `gh` CLI, so authentication, Enterprise hosts and tokens all come from `gh`.

- **`handles`**: parses the origin URL (`git@host:owner/name.git`, `https://host/owner/name(.git)`,
  `ssh://git@host/owner/name`) and checks the host against `hosts`. URL parsing goes in a shared `src/remote.rs`, since
  GitLab and others will need it too.
- **`status`**: one `gh api graphql --hostname <host>` call per repo, with one aliased `pullRequests(headRefName: …)`
  field per branch:
  ```graphql
  query { repository(owner: "acme", name: "api") {
    b0: pullRequests(headRefName: "feature/login", first: 5, orderBy: {field: CREATED_AT, direction: DESC}) {
      nodes { number state isDraft url headRefOid reviewDecision
              headRepositoryOwner { login }
              commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } }
    }
  } }
  ```
  - It keeps only PRs whose head repo owner matches the repo owner, which ignores forks with the same branch name.
    It prefers an open PR, then the newest merged one, then the newest closed one.
  - Badges:
    | PR | Badges |
    |---|---|
    | open | `#123` (Ok) + checks `✓`/`✗`/`•` (Ok/Fail/Warn) + review `approved` / `changes requested` / `review required` (Ok/Fail/Dim) |
    | draft | `#123 draft` (Dim) + checks |
    | merged | `#123 merged` (Info), `merged = Some(Merged { head: headRefOid, … })` |
    | closed | `#123 closed` (Dim) |
- **`resolve`**: a reference made only of digits → `gh pr view <n> --repo owner/name --json headRefName,title,isCrossRepository`.
  Fork PRs (`isCrossRepository`) give an error in v1, with the hint `gh pr checkout <n>`.
- **Errors**: `gh` missing → the provider is disabled (warning only if it was explicitly enabled). Not authenticated →
  `hint: gh auth login --hostname <host>`. Rate limited or other failures → one warning, plus the stale-cache fallback.
- Calls run with `GH_PROMPT_DISABLED=1` and `NO_COLOR=1`, like the git wrapper.

## Module layout

```
src/
├── remote.rs            # parse origin URL → host/owner/name
└── provider/
    ├── mod.rs           # trait, types, Registry, CacheMode, deadline-aware subprocess helper
    ├── cache.rs         # $WT_ROOT/.wt/cache/<provider>.json
    └── github.rs
```

Changes to existing code:

| File | Change |
|---|---|
| `config.rs` | `[github]` section |
| `spec.rs` | `repo#ref` parsing, `Spec.reference` |
| `cmd/add.rs` | resolve step before base update |
| `status.rs` | `Commits::Merged`, `check` takes an optional `Merged` and applies the safety rule |
| `cmd/ls.rs` | provider column, `--offline` / `--refresh` |
| `cmd/rm.rs`, `cmd/cleanup.rs` | merged signal (`CacheOnly` / `Refresh`), candidate rule |
| `Cargo.toml` | `serde_json` |

## Testing

- **Unit**: remote URL parsing, PR selection and badge mapping (from canned GraphQL JSON), the merged safety rule,
  cache expiry and keys, and `repo#ref` parsing.
- **Integration**: a fake `gh` script placed first on `PATH` in `tests/workflow.rs`, returning canned JSON. That covers
  the real subprocess path without the network. The cases: `ls` shows badges; `cleanup` removes a workspace whose PR is
  merged while its remote branch still exists; `cleanup` keeps it when there's a commit after the merge; `rm` shows
  "PR merged" from the cache; `add web#123` checks out the PR branch; a failing `gh` leaves `ls` working with one
  warning.

## Phases

1. `remote.rs`, the provider trait and types, the registry, the cache, the deadline helper. No providers yet; `ls` is
   unchanged.
2. GitHub status in `ls` (GraphQL batch, badges, links, stale fallback, `--offline`/`--refresh`).
3. Merged signal: `Commits::Merged`, the safety rule, `rm`/`cleanup`.
4. `repo#ref` resolver.
5. Tests with the fake `gh`, and updates to `README.md`, `commands.md` and `concepts.md`.

## Open questions

- **Several providers per repo** (forge plus issue tracker): merge badges, or keep one per repo?
- **Fork PRs** in `web#123`: support them by adding a fork remote in base, or leave that to `gh pr checkout`?
- **Completion for `web#<TAB>`**: list open PRs from the cache only, or allow a network call?
- **Cache location**: `$WT_ROOT/.wt/cache` (self-contained) or the OS cache dir (`~/Library/Caches/wt`)?
- **TTL and deadline**: hardcode them (2 min / 3 s), or make them configurable?

## Later

- **`ExternalProvider`**: runs `command` from `[providers.<name>]` and sends the trait's types as JSON on stdin/stdout,
  with a `version` field. A GitLab or Jira provider could then be a script.
- **Lifecycle hooks**: `post-add` / `pre-rm` / `post-rm` shell commands in `.wt.toml` (global and per repo under
  `[repos.<name>.hooks]`), with `WT_FEATURE`, `WT_REPO`, `WT_WORKTREE` and `WT_BRANCH` in the environment.
- **Actions**: `wt open` (PR in the browser, IDE), `wt pr` (create PRs for every repo in a workspace).
- **Issue trackers**: `PROJ-42` as a spec, giving the branch name from the issue title and the issue status in `ls`.

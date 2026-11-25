//! Providers: integrations that know things git doesn't (pull requests, CI status).
//!
//! The rest of `wt` only talks to the [`Provider`] trait through a [`Registry`]. Results are cached
//! in `$WT_ROOT/.wt/cache/<provider>.json`.

pub mod cache;
pub mod github;

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::root::Root;
use cache::Cache;

/// How long a cached status counts as fresh.
pub const TTL: Duration = Duration::from_secs(120);
/// Hard limit for the whole provider phase of a command.
pub const DEADLINE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tone {
    Ok,
    Warn,
    Fail,
    Info,
    Dim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Badge {
    pub text: String,
    pub tone: Tone,
}

impl Badge {
    pub fn new(text: impl Into<String>, tone: Tone) -> Badge {
        Badge { text: text.into(), tone }
    }
}

/// What a provider has to say about one branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Rendered in order, separated by spaces: `#123` `✓` `approved`.
    pub badges: Vec<Badge>,
    /// Link for the first badge.
    pub url: Option<String>,
}

/// A base repo, as far as providers are concerned. Local data only.
#[derive(Debug, Clone)]
pub struct RepoInfo {
    /// Dir name in `base/`.
    pub name: String,
    /// `remote.origin.url`, as configured.
    pub remote_url: String,
}

/// One worktree to report on.
#[derive(Debug, Clone)]
pub struct Item {
    pub branch: String,
    /// Local HEAD sha (part of the cache key, so new commits refetch).
    pub head: String,
}

pub trait Provider: Send + Sync {
    /// Short name, used in warnings and as the cache file name.
    fn name(&self) -> &'static str;

    /// Whether this provider handles the repo. Must not touch the network.
    fn handles(&self, repo: &RepoInfo) -> bool;

    /// Status for several worktrees of one repo in one batch: one entry per item, `None` = nothing to say.
    fn status(&self, repo: &RepoInfo, items: &[Item], deadline: Instant) -> Result<Vec<Option<Status>>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Use fresh cache entries, fetch the rest; fall back to stale entries on failure.
    Normal,
    /// Always fetch; fall back to the cache on failure.
    Refresh,
    /// Never touch the network; show whatever is cached.
    Offline,
}

/// A status ready to show. `stale` = from an expired cache entry (fetch failed or offline).
#[derive(Debug, Clone)]
pub struct Shown {
    pub status: Status,
    pub stale: bool,
}

#[derive(Default)]
pub struct Statuses {
    /// (repo, branch) -> status.
    pub map: HashMap<(String, String), Shown>,
    /// (provider, message), de-duplicated.
    pub warnings: Vec<(String, String)>,
}

pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
    /// Problems found while setting up (e.g. a provider enabled in config but unavailable).
    pub warnings: Vec<(String, String)>,
}

impl Registry {
    pub fn from_config(config: &Config) -> Registry {
        let mut providers: Vec<Box<dyn Provider>> = Vec::new();
        let mut warnings = Vec::new();
        match github::GitHub::from_config(&config.github) {
            Ok(Some(gh)) => providers.push(Box::new(gh)),
            Ok(None) => {}
            Err(msg) => warnings.push(("github".to_string(), msg)),
        }
        Registry { providers, warnings }
    }

    #[cfg(test)]
    pub fn with(providers: Vec<Box<dyn Provider>>) -> Registry {
        Registry { providers, warnings: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Status for every item, per repo, through the cache. Fetches run in parallel, bounded by [`DEADLINE`].
    pub fn status_all(&self, root: &Root, batches: Vec<(RepoInfo, Vec<Item>)>, mode: CacheMode) -> Statuses {
        let mut out = Statuses { warnings: self.warnings.clone(), ..Default::default() };
        if self.providers.is_empty() {
            return out;
        }
        let cache_dir = root.cache_dir();
        let now = cache::now();
        let mut caches: BTreeMap<&'static str, Cache> = BTreeMap::new();

        // Decide per item: cached, or needs a fetch.
        struct Job<'a> {
            provider: &'a dyn Provider,
            repo: RepoInfo,
            items: Vec<Item>,
        }
        let mut jobs: Vec<Job> = Vec::new();
        for (repo, items) in batches {
            let Some(provider) = self.providers.iter().find(|p| p.handles(&repo)) else { continue };
            let cache = caches.entry(provider.name()).or_insert_with(|| Cache::load(&cache_dir, provider.name()));
            let mut to_fetch = Vec::new();
            for item in items {
                let entry = cache.get(&repo.name, &item.branch, &item.head);
                let fresh = entry.is_some_and(|e| e.is_fresh(now, TTL));
                match (mode, entry) {
                    (CacheMode::Offline, Some(e)) => insert(&mut out, &repo, &item, e.status.clone(), !fresh),
                    (CacheMode::Offline, None) => {}
                    (CacheMode::Normal, Some(e)) if fresh => insert(&mut out, &repo, &item, e.status.clone(), false),
                    _ => to_fetch.push(item),
                }
            }
            if !to_fetch.is_empty() {
                jobs.push(Job { provider: provider.as_ref(), repo, items: to_fetch });
            }
        }
        if jobs.is_empty() {
            return out;
        }

        let deadline = Instant::now() + DEADLINE;
        let results: Vec<Result<Vec<Option<Status>>>> = std::thread::scope(|s| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|job| s.spawn(move || job.provider.status(&job.repo, &job.items, deadline)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("provider panicked"))))
                .collect()
        });

        let mut dirty: Vec<&'static str> = Vec::new();
        for (job, res) in jobs.into_iter().zip(results) {
            let name = job.provider.name();
            let cache = caches.get_mut(name).expect("cache loaded above");
            match res {
                Ok(statuses) if statuses.len() == job.items.len() => {
                    for (item, status) in job.items.iter().zip(statuses) {
                        cache.put(&job.repo.name, &item.branch, &item.head, status.clone(), now);
                        insert(&mut out, &job.repo, item, status, false);
                    }
                    if !dirty.contains(&name) {
                        dirty.push(name);
                    }
                }
                other => {
                    let msg = match other {
                        Err(e) => crate::error_text(&e),
                        Ok(_) => "returned the wrong number of results".to_string(),
                    };
                    let warning = (name.to_string(), msg);
                    if !out.warnings.contains(&warning) {
                        out.warnings.push(warning);
                    }
                    // Fall back to whatever is cached, marked stale.
                    for item in &job.items {
                        if let Some(e) = cache.get(&job.repo.name, &item.branch, &item.head) {
                            insert(&mut out, &job.repo, item, e.status.clone(), true);
                        }
                    }
                }
            }
        }
        for name in dirty {
            // A cache that can't be written only costs a refetch next time.
            let _ = caches[name].save(&cache_dir, name, now);
        }
        out
    }
}

fn insert(out: &mut Statuses, repo: &RepoInfo, item: &Item, status: Option<Status>, stale: bool) {
    if let Some(status) = status {
        out.map.insert((repo.name.clone(), item.branch.clone()), Shown { status, stale });
    }
}

/// Run a command, killing it if it's still running at `deadline`.
pub fn run_until(mut cmd: Command, deadline: Instant) -> Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    // Read in threads so a full pipe can't block the child.
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = out_reader.join().unwrap_or_default();
            let stderr = err_reader.join().unwrap_or_default();
            return Ok(Output { status, stdout, stderr });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn handles(&self, repo: &RepoInfo) -> bool {
            repo.remote_url.starts_with("fake:")
        }
        fn status(&self, _: &RepoInfo, items: &[Item], _: Instant) -> Result<Vec<Option<Status>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                bail!("boom");
            }
            Ok(items
                .iter()
                .map(|i| Some(Status { badges: vec![Badge::new(format!("#{}", i.branch), Tone::Ok)], url: None }))
                .collect())
        }
    }

    fn registry(fail: bool) -> (Registry, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (Registry::with(vec![Box::new(Fake { calls: calls.clone(), fail })]), calls)
    }

    fn temp_root(tag: &str) -> Root {
        let dir = std::env::temp_dir().join(format!("wt-provider-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Root { path: dir }
    }

    fn batch(url: &str, head: &str) -> Vec<(RepoInfo, Vec<Item>)> {
        vec![(
            RepoInfo { name: "api".into(), remote_url: url.into() },
            vec![Item { branch: "b".into(), head: head.into() }],
        )]
    }

    fn key() -> (String, String) {
        ("api".into(), "b".into())
    }

    #[test]
    fn caches_by_head_and_falls_back_to_stale() {
        let root = temp_root("cache");
        let (ok, calls) = registry(false);

        let s = ok.status_all(&root, batch("fake:x", "h1"), CacheMode::Normal);
        assert_eq!(s.map[&key()].status.badges[0].text, "#b");
        assert!(!s.map[&key()].stale);
        ok.status_all(&root, batch("fake:x", "h1"), CacheMode::Normal);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "second run served from the cache");
        ok.status_all(&root, batch("fake:x", "h2"), CacheMode::Normal);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "new commit: refetch");
        ok.status_all(&root, batch("fake:x", "h2"), CacheMode::Refresh);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "refresh always fetches");

        // A failing provider shows the cached value as stale, plus one warning.
        let (failing, _) = registry(true);
        let s = failing.status_all(&root, batch("fake:x", "h2"), CacheMode::Refresh);
        assert!(s.map[&key()].stale);
        assert_eq!(s.warnings, vec![("fake".to_string(), "boom".to_string())]);

        // Unhandled repos are skipped.
        assert!(ok.status_all(&root, batch("https://elsewhere/x/y", "h1"), CacheMode::Normal).map.is_empty());
        let _ = std::fs::remove_dir_all(&root.path);
    }

    #[test]
    fn offline_never_fetches() {
        let root = temp_root("offline");
        let (reg, calls) = registry(false);
        assert!(reg.status_all(&root, batch("fake:x", "h1"), CacheMode::Offline).map.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&root.path);
    }

    #[test]
    fn run_until_kills_slow_commands() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 5"]);
        let start = Instant::now();
        let err = run_until(cmd, Instant::now() + Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.to_string(), "timed out");
        assert!(start.elapsed() < Duration::from_secs(2));

        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hi; echo oops >&2"]);
        let out = run_until(cmd, Instant::now() + Duration::from_secs(5)).unwrap();
        assert_eq!(out.stdout, b"hi\n");
        assert_eq!(out.stderr, b"oops\n");
    }
}

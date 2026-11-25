//! Per-provider status cache: `$WT_ROOT/.wt/cache/<provider>.json`.
//!
//! Keyed by (repo, branch, head sha), so a new commit refetches. Unreadable or corrupt files are
//! treated as empty; a cache is only an optimization.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::Status;

/// Entries older than this are dropped when the cache is written.
const KEEP: Duration = Duration::from_secs(30 * 86_400);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds when fetched.
    pub at: u64,
    /// `None` = the provider had nothing to say (e.g. no PR); cached too.
    pub status: Option<Status>,
}

impl Entry {
    pub fn is_fresh(&self, now: u64, ttl: Duration) -> bool {
        now.saturating_sub(self.at) < ttl.as_secs()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    entries: BTreeMap<String, Entry>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn key(repo: &str, branch: &str, head: &str) -> String {
    format!("{repo}\t{branch}\t{head}")
}

fn file(dir: &Path, provider: &str) -> PathBuf {
    dir.join(format!("{provider}.json"))
}

impl Cache {
    pub fn load(dir: &Path, provider: &str) -> Cache {
        std::fs::read(file(dir, provider))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, repo: &str, branch: &str, head: &str) -> Option<&Entry> {
        self.entries.get(&key(repo, branch, head))
    }

    pub fn put(&mut self, repo: &str, branch: &str, head: &str, status: Option<Status>, now: u64) {
        self.entries.insert(key(repo, branch, head), Entry { at: now, status });
    }

    /// Write atomically (temp file + rename), dropping old entries. Merges with what's on disk, so
    /// concurrent `wt` runs don't throw away each other's results.
    pub fn save(&self, dir: &Path, provider: &str, now: u64) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut merged = Cache::load(dir, provider);
        for (k, v) in &self.entries {
            let newer = merged.entries.get(k).is_none_or(|old| old.at <= v.at);
            if newer {
                merged.entries.insert(k.clone(), v.clone());
            }
        }
        merged.entries.retain(|_, e| now.saturating_sub(e.at) < KEEP.as_secs());
        let path = file(dir, provider);
        let tmp = dir.join(format!(".{provider}.json.{}", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(&merged)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Badge, Tone};

    #[test]
    fn round_trips_and_prunes() {
        let dir = std::env::temp_dir().join(format!("wt-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let status = Status { badges: vec![Badge::new("#1", Tone::Ok)], url: Some("u".into()) };

        let n = 10_000_000;
        let mut c = Cache::default();
        c.put("api", "b", "h", Some(status.clone()), n);
        c.put("api", "old", "h", None, 1);
        c.save(&dir, "gh", n).unwrap();

        let c = Cache::load(&dir, "gh");
        let e = c.get("api", "b", "h").unwrap();
        assert_eq!(e.status, Some(status));
        assert!(e.is_fresh(n + 100, Duration::from_secs(120)));
        assert!(!e.is_fresh(n + 120, Duration::from_secs(120)));
        assert!(c.get("api", "b", "other-head").is_none());
        assert!(c.get("api", "old", "h").is_none(), "pruned");

        std::fs::write(dir.join("gh.json"), "not json").unwrap();
        assert!(Cache::load(&dir, "gh").get("api", "b", "h").is_none(), "corrupt = empty");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

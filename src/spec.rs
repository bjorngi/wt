//! Specs: `repo`, `repo:branch` and `@preset`.

use anyhow::Result;

use crate::config::Config;
use crate::error::{hinted, user};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub repo: String,
    /// Explicit branch (`repo:branch`). `None` means "use the workspace's branch".
    pub branch: Option<String>,
}

/// Expand presets, validate repos and de-duplicate. An explicit `repo:branch` wins over a plain `repo`.
pub fn expand(args: &[String], config: &Config, repos: &[String]) -> Result<Vec<Spec>> {
    let mut raw: Vec<Spec> = Vec::new();
    for arg in args {
        if let Some(name) = arg.strip_prefix('@') {
            if name.contains(':') {
                return Err(hinted(
                    format!("presets can't take a branch: {arg}"),
                    "use repo:branch for each repo instead",
                ));
            }
            let Some(members) = config.presets.get(name) else {
                let hint = if config.presets.is_empty() {
                    "define presets under [presets] in $WT_ROOT/.wt.toml".to_string()
                } else {
                    let names: Vec<String> = config.presets.keys().map(|k| format!("@{k}")).collect();
                    format!("available presets: {}", names.join(", "))
                };
                return Err(hinted(format!("unknown preset @{name}"), hint));
            };
            for repo in members {
                if repo.contains(':') || repo.starts_with('@') {
                    return Err(user(format!(
                        "preset @{name}: '{repo}' must be a plain repo name"
                    )));
                }
                raw.push(Spec { repo: repo.clone(), branch: None });
            }
        } else if let Some((repo, branch)) = arg.split_once(':') {
            if repo.is_empty() || branch.is_empty() {
                return Err(user(format!("invalid spec '{arg}' (expected repo or repo:branch)")));
            }
            raw.push(Spec { repo: repo.to_string(), branch: Some(branch.to_string()) });
        } else {
            if arg.is_empty() {
                return Err(user("empty repo name"));
            }
            raw.push(Spec { repo: arg.clone(), branch: None });
        }
    }

    for spec in &raw {
        if !repos.contains(&spec.repo) {
            return Err(hinted(
                format!("unknown repo '{}'", spec.repo),
                format!("repos in base/: {}", repos.join(", ")),
            ));
        }
    }

    let mut out: Vec<Spec> = Vec::new();
    for spec in raw {
        match out.iter_mut().find(|s| s.repo == spec.repo) {
            None => out.push(spec),
            Some(existing) => match (&existing.branch, &spec.branch) {
                (_, None) => {}
                (None, Some(_)) => existing.branch = spec.branch,
                (Some(a), Some(b)) if a == b => {}
                (Some(a), Some(b)) => {
                    return Err(user(format!(
                        "conflicting branches for {}: {a} and {b}",
                        spec.repo
                    )));
                }
            },
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut c = Config::default();
        c.presets.insert("alde".into(), vec!["alde".into(), "pen".into()]);
        c
    }

    fn repos() -> Vec<String> {
        vec!["alde".into(), "pen".into(), "infra".into()]
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn spec(repo: &str, branch: Option<&str>) -> Spec {
        Spec { repo: repo.into(), branch: branch.map(Into::into) }
    }

    #[test]
    fn expands_presets_and_dedupes() {
        let out = expand(&args(&["@alde", "pen:fix-1", "infra", "alde"]), &config(), &repos()).unwrap();
        assert_eq!(out, vec![spec("alde", None), spec("pen", Some("fix-1")), spec("infra", None)]);
    }

    #[test]
    fn explicit_branch_wins_regardless_of_order() {
        let out = expand(&args(&["pen:x", "@alde"]), &config(), &repos()).unwrap();
        assert_eq!(out, vec![spec("pen", Some("x")), spec("alde", None)]);
    }

    #[test]
    fn errors() {
        assert!(expand(&args(&["@nope"]), &config(), &repos()).is_err());
        assert!(expand(&args(&["@alde:x"]), &config(), &repos()).is_err());
        assert!(expand(&args(&["nope"]), &config(), &repos()).is_err());
        assert!(expand(&args(&["pen:"]), &config(), &repos()).is_err());
        assert!(expand(&args(&["pen:a", "pen:b"]), &config(), &repos()).is_err());
    }

    #[test]
    fn branch_may_contain_slashes() {
        let out = expand(&args(&["pen:feature/x"]), &config(), &repos()).unwrap();
        assert_eq!(out, vec![spec("pen", Some("feature/x"))]);
    }
}

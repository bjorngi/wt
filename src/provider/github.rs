//! GitHub: pull request state, checks and review status, through the `gh` CLI.
//!
//! One `gh api graphql` call per repo, with one aliased field per branch. Auth, tokens and
//! GitHub Enterprise hosts all come from `gh`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use super::{Badge, Item, Provider, RepoInfo, Status, Tone, run_until};
use crate::config::GithubConfig;
use crate::remote;

pub struct GitHub {
    bin: PathBuf,
    hosts: Vec<String>,
}

impl GitHub {
    /// `Ok(None)` = not enabled (or `gh` not installed and not explicitly enabled).
    /// `Err(msg)` = explicitly enabled but unusable.
    pub fn from_config(config: &GithubConfig) -> Result<Option<GitHub>, String> {
        if config.enabled == Some(false) {
            return Ok(None);
        }
        let hosts = config.hosts.iter().map(|h| h.to_ascii_lowercase()).collect();
        match find_on_path("gh") {
            Some(bin) => Ok(Some(GitHub { bin, hosts })),
            None if config.enabled == Some(true) => {
                Err("`gh` not found on PATH (install it: https://cli.github.com)".to_string())
            }
            None => Ok(None),
        }
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

impl Provider for GitHub {
    fn name(&self) -> &'static str {
        "github"
    }

    fn handles(&self, repo: &RepoInfo) -> bool {
        remote::parse(&repo.remote_url).is_some_and(|r| self.hosts.contains(&r.host))
    }

    fn status(&self, repo: &RepoInfo, items: &[Item], deadline: Instant) -> Result<Vec<Option<Status>>> {
        let remote = remote::parse(&repo.remote_url).ok_or_else(|| anyhow!("can't parse {}", repo.remote_url))?;
        let mut cmd = Command::new(&self.bin);
        cmd.args(["api", "graphql", "--hostname", &remote.host]);
        cmd.arg("-f").arg(format!("query={}", query(items.len())));
        cmd.arg("-f").arg(format!("owner={}", remote.owner));
        cmd.arg("-f").arg(format!("name={}", remote.name));
        for (i, item) in items.iter().enumerate() {
            cmd.arg("-f").arg(format!("b{i}={}", item.branch));
            cmd.arg("-f").arg(format!("q{i}=refs/heads/{}", item.branch));
        }
        cmd.env("GH_PROMPT_DISABLED", "1").env("GH_NO_UPDATE_NOTIFIER", "1").env("NO_COLOR", "1");

        let out = run_until(cmd, deadline)?;
        if !out.status.success() {
            bail!(describe_failure(&String::from_utf8_lossy(&out.stderr), &remote.host));
        }
        let json: Value = serde_json::from_slice(&out.stdout).map_err(|e| anyhow!("unexpected output from gh: {e}"))?;
        parse(&json, &remote.owner, items.len())
    }
}

/// GraphQL query for `n` branches. Branch names are passed as variables, so they need no escaping.
fn query(n: usize) -> String {
    let mut vars = String::from("$owner: String!, $name: String!");
    let mut fields = String::new();
    for i in 0..n {
        vars.push_str(&format!(", $b{i}: String!, $q{i}: String!"));
        fields.push_str(&format!(
            "b{i}: pullRequests(headRefName: $b{i}, first: 5, orderBy: {{field: CREATED_AT, direction: DESC}}) \
             {{ nodes {{ ...pr }} }}\n\
             r{i}: ref(qualifiedName: $q{i}) {{ target {{ ... on Commit {{ statusCheckRollup {{ state }} }} }} }}\n"
        ));
    }
    format!(
        "query({vars}) {{ repository(owner: $owner, name: $name) {{\n{fields}}} }}\n\
         fragment pr on PullRequest {{ number state isDraft url reviewDecision headRepositoryOwner {{ login }} \
         commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }} }}"
    )
}

/// Turn `gh`'s stderr into one short line.
fn describe_failure(stderr: &str, host: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("gh auth login") || lower.contains("not logged") || lower.contains("bad credentials") {
        return format!("not logged in to {host} (run: gh auth login --hostname {host})");
    }
    if lower.contains("rate limit") {
        return "rate limited by GitHub, try again later".to_string();
    }
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("gh failed");
    line.strip_prefix("gh: ").unwrap_or(line).to_string()
}

fn parse(json: &Value, owner: &str, n: usize) -> Result<Vec<Option<Status>>> {
    let repo = &json["data"]["repository"];
    if !repo.is_object() {
        bail!("repository not found (or no access)");
    }
    Ok((0..n).map(|i| summarize(&repo[format!("b{i}")]["nodes"], &repo[format!("r{i}")], owner)).collect())
}

/// Pick the PR for a branch and turn it into badges. Without a PR, a pushed branch shows its checks.
fn summarize(prs: &Value, branch_ref: &Value, owner: &str) -> Option<Status> {
    // Only PRs from this repo, not from forks that happen to use the same branch name.
    let prs: Vec<&Value> = prs
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|p| p["headRepositoryOwner"]["login"].as_str().is_some_and(|l| l.eq_ignore_ascii_case(owner)))
                .collect()
        })
        .unwrap_or_default();
    // Newest first already; prefer an open PR, then a merged one.
    let pr = prs
        .iter()
        .find(|p| p["state"] == "OPEN")
        .or_else(|| prs.iter().find(|p| p["state"] == "MERGED"))
        .or_else(|| prs.first());

    let Some(pr) = pr else {
        if !branch_ref.is_object() {
            return None; // not pushed
        }
        let mut badges = vec![Badge::new("no PR", Tone::Dim)];
        badges.extend(checks(&branch_ref["target"]["statusCheckRollup"]["state"]));
        return Some(Status { badges, url: None });
    };

    let number = pr["number"].as_u64()?;
    let draft = pr["isDraft"].as_bool().unwrap_or(false);
    let mut badges = Vec::new();
    match (pr["state"].as_str()?, draft) {
        ("OPEN", true) => badges.push(Badge::new(format!("#{number} draft"), Tone::Dim)),
        ("OPEN", false) => badges.push(Badge::new(format!("#{number}"), Tone::Ok)),
        ("MERGED", _) => badges.push(Badge::new(format!("#{number} merged"), Tone::Info)),
        _ => badges.push(Badge::new(format!("#{number} closed"), Tone::Dim)),
    }
    if pr["state"] == "OPEN" {
        badges.extend(checks(&pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"]));
        if !draft {
            badges.extend(review(&pr["reviewDecision"]));
        }
    }
    Some(Status { badges, url: pr["url"].as_str().map(String::from) })
}

fn checks(state: &Value) -> Option<Badge> {
    Some(match state.as_str()? {
        "SUCCESS" => Badge::new("✓", Tone::Ok),
        "FAILURE" | "ERROR" => Badge::new("✗", Tone::Fail),
        "PENDING" | "EXPECTED" => Badge::new("•", Tone::Warn),
        _ => return None,
    })
}

fn review(decision: &Value) -> Option<Badge> {
    Some(match decision.as_str()? {
        "APPROVED" => Badge::new("approved", Tone::Ok),
        "CHANGES_REQUESTED" => Badge::new("changes requested", Tone::Fail),
        "REVIEW_REQUIRED" => Badge::new("review required", Tone::Dim),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pr(number: u64, state: &str, draft: bool, owner: &str, checks: Option<&str>, review: Option<&str>) -> Value {
        json!({
            "number": number, "state": state, "isDraft": draft,
            "url": format!("https://github.com/acme/api/pull/{number}"),
            "reviewDecision": review,
            "headRepositoryOwner": { "login": owner },
            "commits": { "nodes": [ { "commit": { "statusCheckRollup": checks.map(|s| json!({ "state": s })) } } ] }
        })
    }

    fn texts(s: &Option<Status>) -> Vec<&str> {
        s.as_ref().map(|s| s.badges.iter().map(|b| b.text.as_str()).collect()).unwrap_or_default()
    }

    #[test]
    fn open_pr_with_checks_and_review() {
        let prs = json!([pr(12, "OPEN", false, "acme", Some("SUCCESS"), Some("APPROVED"))]);
        let s = summarize(&prs, &Value::Null, "acme");
        assert_eq!(texts(&s), vec!["#12", "✓", "approved"]);
        assert_eq!(s.unwrap().url.unwrap(), "https://github.com/acme/api/pull/12");

        let prs = json!([pr(13, "OPEN", false, "acme", Some("FAILURE"), Some("CHANGES_REQUESTED"))]);
        assert_eq!(texts(&summarize(&prs, &Value::Null, "acme")), vec!["#13", "✗", "changes requested"]);

        let prs = json!([pr(14, "OPEN", true, "acme", Some("PENDING"), Some("REVIEW_REQUIRED"))]);
        assert_eq!(texts(&summarize(&prs, &Value::Null, "acme")), vec!["#14 draft", "•"]);
    }

    #[test]
    fn prefers_open_then_merged_and_ignores_forks() {
        let prs = json!([
            pr(30, "OPEN", false, "someone-else", None, None),
            pr(20, "CLOSED", false, "acme", None, None),
            pr(10, "MERGED", false, "acme", Some("SUCCESS"), Some("APPROVED")),
        ]);
        assert_eq!(texts(&summarize(&prs, &Value::Null, "ACME")), vec!["#10 merged"]);

        let prs = json!([pr(20, "CLOSED", false, "acme", None, None)]);
        assert_eq!(texts(&summarize(&prs, &Value::Null, "acme")), vec!["#20 closed"]);
    }

    #[test]
    fn no_pr() {
        let pushed = json!({ "target": { "statusCheckRollup": { "state": "FAILURE" } } });
        assert_eq!(texts(&summarize(&json!([]), &pushed, "acme")), vec!["no PR", "✗"]);
        let pushed_no_checks = json!({ "target": { "statusCheckRollup": null } });
        assert_eq!(texts(&summarize(&json!([]), &pushed_no_checks, "acme")), vec!["no PR"]);
        assert_eq!(summarize(&json!([]), &Value::Null, "acme"), None, "not pushed: nothing to show");
    }

    #[test]
    fn parses_response_per_alias() {
        let json = json!({ "data": { "repository": {
            "b0": { "nodes": [pr(1, "OPEN", false, "acme", Some("SUCCESS"), None)] },
            "r0": { "target": {} },
            "b1": { "nodes": [] },
            "r1": null,
        } } });
        let out = parse(&json, "acme", 2).unwrap();
        assert_eq!(texts(&out[0]), vec!["#1", "✓"]);
        assert_eq!(out[1], None);
        assert!(parse(&json!({ "data": { "repository": null } }), "acme", 1).is_err());
    }

    #[test]
    fn query_declares_every_variable() {
        let q = query(2);
        for v in ["$owner", "$name", "$b0", "$q0", "$b1", "$q1"] {
            assert!(q.contains(&format!("{v}: String!")), "{v} missing in {q}");
        }
        assert!(q.contains("b1: pullRequests(headRefName: $b1"));
        assert!(q.contains("r1: ref(qualifiedName: $q1)"));
    }

    #[test]
    fn failures_are_one_line() {
        assert_eq!(
            describe_failure("To get started with GitHub CLI, please run:  gh auth login\n", "github.com"),
            "not logged in to github.com (run: gh auth login --hostname github.com)"
        );
        assert_eq!(
            describe_failure("gh: Could not resolve to a Repository with the name 'acme/nope'.\n", "github.com"),
            "Could not resolve to a Repository with the name 'acme/nope'."
        );
    }

    /// Runs the real subprocess path against a fake `gh` script.
    #[test]
    fn calls_gh_with_variables() {
        let dir = std::env::temp_dir().join(format!("wt-fake-gh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let args_file = dir.join("args");
        let script = dir.join("gh");
        let response = json!({ "data": { "repository": {
            "b0": { "nodes": [pr(7, "OPEN", false, "acme", Some("SUCCESS"), Some("APPROVED"))] }, "r0": null
        } } });
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat <<'EOF'\n{response}\nEOF\n", args_file.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let gh = GitHub { bin: script, hosts: vec!["github.com".into()] };
        let repo = RepoInfo { name: "api".into(), remote_url: "git@github.com:acme/api.git".into() };
        assert!(gh.handles(&repo));
        assert!(!gh.handles(&RepoInfo { name: "x".into(), remote_url: "/tmp/x.git".into() }));

        let items = vec![Item { branch: "feature/login".into(), head: "abc".into() }];
        let out = gh.status(&repo, &items, Instant::now() + std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(texts(&out[0]), vec!["#7", "✓", "approved"]);

        let args = std::fs::read_to_string(&args_file).unwrap();
        for expected in ["api", "graphql", "--hostname", "github.com", "owner=acme", "name=api", "b0=feature/login", "q0=refs/heads/feature/login"] {
            assert!(args.lines().any(|l| l == expected), "{expected} missing in:\n{args}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

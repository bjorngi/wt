//! End-to-end tests: a temp `$WT_ROOT` with bare "remotes" and clones in `base/`.
//!
//! Pushing is simulated by fetching *into* the bare remote (`git -C remote fetch <worktree> b:b`),
//! so the tests never run `git push`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Env {
    dir: PathBuf,
    root: PathBuf,
    cd_file: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success()
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
    cd: String,
}

impl Run {
    fn from(out: Output, cd: String) -> Run {
        Run {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            cd,
        }
    }
}

impl Env {
    fn new() -> Env {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("wt-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("base")).unwrap();
        std::fs::write(root.join(".wt.toml"), "[presets]\nalde = [\"alde\", \"pen\"]\n").unwrap();
        Env { cd_file: dir.join("cd"), dir, root }
    }

    fn remote(&self, repo: &str) -> PathBuf {
        self.dir.join("remotes").join(format!("{repo}.git"))
    }

    fn seed(&self, repo: &str) -> PathBuf {
        self.dir.join("seeds").join(repo)
    }

    fn base(&self, repo: &str) -> PathBuf {
        self.root.join("base").join(repo)
    }

    fn ws(&self, name: &str) -> PathBuf {
        self.root.join("worktrees").join(name)
    }

    /// A remote with `main` + `feature/existing`, a clone in base/, secrets and a .worktreeinclude.
    fn add_repo(&self, repo: &str) {
        let remote = self.remote(repo);
        let seed = self.seed(repo);
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&seed).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        git(&seed, &["init", "-q", "-b", "main"]);
        std::fs::write(seed.join(".gitignore"), ".env\n.idea/\nnode_modules/\n").unwrap();
        std::fs::write(seed.join("README.md"), format!("# {repo}\n")).unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-qm", "Initial"]);
        git(&seed, &["switch", "-qc", "feature/existing"]);
        std::fs::write(seed.join("existing.txt"), "x\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-qm", "Existing work"]);
        git(&seed, &["switch", "-q", "main"]);
        let seed_str = seed.to_string_lossy().to_string();
        git(&remote, &["fetch", "-q", &seed_str, "main:main", "feature/existing:feature/existing"]);

        let base = self.base(repo);
        git(&self.root.join("base"), &["clone", "-q", &remote.to_string_lossy(), repo]);
        std::fs::write(base.join(".env"), format!("SECRET={repo}\n")).unwrap();
        std::fs::create_dir_all(base.join(".idea")).unwrap();
        std::fs::write(base.join(".idea/misc.xml"), "<xml/>\n").unwrap();
        std::fs::create_dir_all(base.join("node_modules")).unwrap();
        std::fs::write(base.join("node_modules/x"), "junk\n").unwrap();
        std::fs::write(base.join("notignored.txt"), "oops\n").unwrap();
        std::fs::write(base.join(".worktreeinclude"), ".env\n.idea/\nnotignored.txt\n").unwrap();
    }

    /// Commit on a branch in the remote (as if someone else pushed).
    fn remote_commit(&self, repo: &str, branch: &str, file: &str) {
        let seed = self.seed(repo);
        git(&seed, &["switch", "-q", branch]);
        std::fs::write(seed.join(file), file).unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-qm", &format!("Add {file}")]);
        let refspec = format!("+{branch}:{branch}");
        git(&self.remote(repo), &["fetch", "-q", &seed.to_string_lossy(), &refspec]);
        git(&seed, &["switch", "-q", "main"]);
    }

    /// Simulate `git push -u origin <branch>` from a worktree.
    fn push(&self, repo: &str, worktree: &Path, branch: &str) {
        let refspec = format!("+{branch}:{branch}");
        git(&self.remote(repo), &["fetch", "-q", &worktree.to_string_lossy(), &refspec]);
        git(worktree, &["fetch", "-q", "origin"]);
        git(worktree, &["branch", "-q", &format!("--set-upstream-to=origin/{branch}")]);
    }

    /// Simulate merging the PR and deleting the remote branch.
    fn delete_remote_branch(&self, repo: &str, branch: &str) {
        git(&self.remote(repo), &["branch", "-q", "-D", branch]);
    }

    fn commit(&self, worktree: &Path, file: &str) {
        std::fs::write(worktree.join(file), file).unwrap();
        git(worktree, &["add", "."]);
        git(worktree, &["commit", "-qm", &format!("Add {file}")]);
    }

    fn wt_in(&self, cwd: &Path, args: &[&str]) -> Run {
        let _ = std::fs::remove_file(&self.cd_file);
        std::fs::write(&self.cd_file, "").unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wt"))
            .args(args)
            .current_dir(cwd)
            .env("WT_ROOT", &self.root)
            .env("WT_CD_FILE", &self.cd_file)
            .env("NO_COLOR", "1")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let cd = std::fs::read_to_string(&self.cd_file).unwrap_or_default();
        Run::from(out, cd)
    }

    fn wt(&self, args: &[&str]) -> Run {
        self.wt_in(&self.dir, args)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn assert_ok(run: &Run) {
    assert!(run.ok, "wt failed:\nstdout: {}\nstderr: {}", run.stdout, run.stderr);
}

fn assert_fails(run: &Run, needle: &str) {
    assert!(!run.ok, "wt should have failed:\nstderr: {}", run.stderr);
    assert!(run.stderr.contains(needle), "expected {needle:?} in stderr:\n{}", run.stderr);
}

// ---------------------------------------------------------------------------------------- add

#[test]
fn add_creates_workspace_with_new_branches_and_copies_includes() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");

    let run = env.wt(&["add", "feature/login", "@alde"]);
    assert_ok(&run);
    let ws = env.ws("feature-login");
    assert_eq!(run.cd.trim(), ws.to_string_lossy());
    assert!(run.stderr.contains("Created"), "{}", run.stderr);

    for repo in ["alde", "pen"] {
        let wt = ws.join(repo);
        assert_eq!(git(&wt, &["symbolic-ref", "--short", "HEAD"]), "feature/login");
        assert_eq!(git(&wt, &["rev-parse", "HEAD"]), git(&env.base(repo), &["rev-parse", "main"]));
        // Not tracking main.
        assert!(!git_ok(&wt, &["rev-parse", "--abbrev-ref", "@{upstream}"]));
        assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), format!("SECRET={repo}\n"));
        assert!(wt.join(".idea/misc.xml").is_file());
        assert!(!wt.join("node_modules").exists(), "not listed in .worktreeinclude");
        assert!(!wt.join("notignored.txt").exists(), "listed but not gitignored");
    }
    assert!(run.stderr.contains("not copied (not gitignored): notignored.txt"), "{}", run.stderr);
    assert!(run.stderr.contains("new branch feature/login from main, copied 2 files"), "{}", run.stderr);
}

#[test]
fn add_tracks_existing_remote_branch() {
    let env = Env::new();
    env.add_repo("pen");
    let run = env.wt(&["add", "review-1", "pen:feature/existing"]);
    assert_ok(&run);
    let wt = env.ws("review-1").join("pen");
    assert_eq!(git(&wt, &["rev-parse", "--abbrev-ref", "@{upstream}"]), "origin/feature/existing");
    assert!(wt.join("existing.txt").is_file());
    assert!(run.stderr.contains("tracking origin/feature/existing"), "{}", run.stderr);
}

#[test]
fn add_to_existing_workspace_reuses_its_branch_and_stays_put() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");
    assert_ok(&env.wt(&["add", "feature/login", "alde"]));

    // From inside the workspace, using the dir-name form.
    let inside = env.ws("feature-login").join("alde");
    let run = env.wt_in(&inside, &["add", "feature-login", "pen"]);
    assert_ok(&run);
    assert_eq!(git(&env.ws("feature-login").join("pen"), &["symbolic-ref", "--short", "HEAD"]), "feature/login");
    assert!(run.stderr.contains("Added pen to feature-login"), "{}", run.stderr);
    assert_eq!(run.cd, "", "already inside the workspace: no cd");

    // Adding again only copies missing include files.
    std::fs::remove_file(inside.join(".env")).unwrap();
    let run = env.wt(&["add", "feature-login", "alde"]);
    assert_ok(&run);
    assert!(run.stderr.contains("already there, copied 1 file"), "{}", run.stderr);
    assert!(run.stderr.contains("feature-login is up to date"), "{}", run.stderr);
    assert!(inside.join(".env").is_file());
}

#[test]
fn add_updates_base_main_first() {
    let env = Env::new();
    env.add_repo("alde");
    env.remote_commit("alde", "main", "newer.txt");

    assert_ok(&env.wt(&["add", "f", "alde"]));
    let remote_main = git(&env.remote("alde"), &["rev-parse", "main"]);
    assert_eq!(git(&env.base("alde"), &["rev-parse", "main"]), remote_main, "base fast-forwarded");
    assert_eq!(git(&env.ws("f").join("alde"), &["rev-parse", "HEAD"]), remote_main, "branched from new main");
}

#[test]
fn add_refuses_when_base_is_not_a_clean_main() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");

    git(&env.base("alde"), &["switch", "-qc", "oops"]);
    let run = env.wt(&["add", "f", "@alde"]);
    assert_fails(&run, "base/alde is on branch 'oops', expected 'main'");
    assert!(!env.ws("f").exists(), "nothing created");
    git(&env.base("alde"), &["switch", "-q", "main"]);

    std::fs::write(env.base("pen").join("README.md"), "changed\n").unwrap();
    assert_fails(&env.wt(&["add", "f", "pen"]), "base/pen has uncommitted changes");
    assert!(!env.ws("f").exists());
}

#[test]
fn add_refuses_branch_checked_out_elsewhere() {
    let env = Env::new();
    env.add_repo("alde");
    assert_fails(&env.wt(&["add", "f", "alde:main"]), "branch 'main' is already checked out");
    assert_ok(&env.wt(&["add", "one", "alde:shared"]));
    assert_fails(&env.wt(&["add", "two", "alde:shared"]), "already checked out at");
    assert!(!env.ws("two").exists());
}

#[test]
fn add_validates_specs_before_doing_anything() {
    let env = Env::new();
    env.add_repo("alde");
    assert_fails(&env.wt(&["add", "f", "nope"]), "unknown repo 'nope'");
    assert_fails(&env.wt(&["add", "f", "@nope"]), "unknown preset @nope");
    assert_fails(&env.wt(&["add", "f", "@alde"]), "unknown repo 'pen'");
    assert_fails(&env.wt(&["add", "f", "alde:bad..name"]), "invalid branch name");
    assert!(!env.ws("f").exists());
}

#[test]
fn errors_without_root_or_repos() {
    let env = Env::new();
    let out = Command::new(env!("CARGO_BIN_EXE_wt")).args(["ls"]).env_remove("WT_ROOT").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("WT_ROOT is not set"));
    assert_fails(&env.wt(&["add", "f", "alde"]), "no repos in");
}

// ----------------------------------------------------------------------------------------- rm

#[test]
fn rm_removes_clean_workspace_and_branch() {
    let env = Env::new();
    env.add_repo("alde");
    assert_ok(&env.wt(&["add", "f", "alde"]));

    let run = env.wt_in(&env.ws("f").join("alde"), &["rm", "f"]);
    assert_ok(&run);
    assert!(!env.ws("f").exists());
    assert!(!git_ok(&env.base("alde"), &["rev-parse", "--verify", "--quiet", "refs/heads/f"]));
    assert_eq!(run.cd.trim(), env.root.join("worktrees").to_string_lossy(), "cwd was removed");
}

#[test]
fn rm_asks_before_losing_work_and_keeps_unpushed_branches() {
    let env = Env::new();
    env.add_repo("alde");
    assert_ok(&env.wt(&["add", "f", "alde"]));
    let wt = env.ws("f").join("alde");
    env.commit(&wt, "work.txt");
    std::fs::write(wt.join("scratch.txt"), "x").unwrap();

    let run = env.wt(&["rm", "f"]);
    assert_fails(&run, "not a terminal");
    assert!(run.stderr.contains("1 uncommitted change"), "{}", run.stderr);
    assert!(run.stderr.contains("1 commit never pushed"), "{}", run.stderr);
    assert!(wt.exists(), "nothing removed without confirmation");

    let run = env.wt(&["rm", "f", "--yes"]);
    assert_ok(&run);
    assert!(!env.ws("f").exists());
    assert!(run.stderr.contains("kept branch f"), "{}", run.stderr);
    assert!(git_ok(&env.base("alde"), &["rev-parse", "--verify", "--quiet", "refs/heads/f"]));
}

#[test]
fn rm_single_repo_and_merged_branch() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");
    assert_ok(&env.wt(&["add", "f", "@alde"]));
    let wt = env.ws("f").join("pen");
    env.commit(&wt, "work.txt");
    env.push("pen", &wt, "f");
    env.delete_remote_branch("pen", "f");
    git(&wt, &["fetch", "-q", "--prune", "origin"]);

    let run = env.wt(&["rm", "f", "pen"]);
    assert_ok(&run);
    assert!(run.stderr.contains("remote branch deleted (merged)"), "{}", run.stderr);
    assert!(run.stderr.contains("deleted branch f"), "{}", run.stderr);
    assert!(!wt.exists());
    assert!(env.ws("f").join("alde").exists(), "other repo untouched");

    assert_fails(&env.wt(&["rm", "f", "infra"]), "infra is not in f");
    assert_fails(&env.wt(&["rm", "nope"]), "no workspace 'nope'");
}

#[test]
fn rm_warns_about_other_files_in_workspace() {
    let env = Env::new();
    env.add_repo("alde");
    assert_ok(&env.wt(&["add", "f", "alde"]));
    std::fs::write(env.ws("f").join("notes.md"), "x").unwrap();
    assert_fails(&env.wt(&["rm", "f"]), "not a terminal");
    assert!(env.ws("f").join("notes.md").exists());
}

// ------------------------------------------------------------------------------------ cleanup

#[test]
fn cleanup_removes_only_fully_merged_workspaces() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");

    // merged: pushed, then the remote branch deleted. pen in the same workspace was never touched.
    assert_ok(&env.wt(&["add", "merged", "@alde"]));
    let wt = env.ws("merged").join("alde");
    env.commit(&wt, "a.txt");
    env.push("alde", &wt, "merged");
    env.delete_remote_branch("alde", "merged");

    // open: pushed, remote branch still exists.
    assert_ok(&env.wt(&["add", "open", "alde"]));
    let wt = env.ws("open").join("alde");
    env.commit(&wt, "b.txt");
    env.push("alde", &wt, "open");

    // fresh: nothing done yet.
    assert_ok(&env.wt(&["add", "fresh", "pen"]));

    // partial: one repo merged, the other has uncommitted work.
    assert_ok(&env.wt(&["add", "partial", "@alde"]));
    let wt = env.ws("partial").join("alde");
    env.commit(&wt, "c.txt");
    env.push("alde", &wt, "partial");
    env.delete_remote_branch("alde", "partial");
    std::fs::write(env.ws("partial").join("pen").join("wip.txt"), "x").unwrap();

    let run = env.wt(&["cleanup"]);
    assert_fails(&run, "not a terminal");
    assert!(env.ws("merged").exists());

    let run = env.wt(&["cleanup", "--yes"]);
    assert_ok(&run);
    assert!(!env.ws("merged").exists(), "{}", run.stderr);
    assert!(env.ws("open").exists());
    assert!(env.ws("fresh").exists());
    assert!(env.ws("partial").exists());
    assert!(run.stderr.contains("partial"), "{}", run.stderr);
    assert!(run.stderr.contains("1 uncommitted change"), "{}", run.stderr);
    assert!(!git_ok(&env.base("alde"), &["rev-parse", "--verify", "--quiet", "refs/heads/merged"]));

    let run = env.wt(&["cleanup", "--yes"]);
    assert_ok(&run);
    assert!(run.stderr.contains("Nothing to clean up."), "{}", run.stderr);
}

// ----------------------------------------------------------------------------- ls, cd, shell

#[test]
fn ls_shows_workspaces_and_status() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");
    let run = env.wt(&["ls"]);
    assert_ok(&run);
    assert!(run.stderr.contains("No workspaces"));

    assert_ok(&env.wt(&["add", "feature/login", "@alde"]));
    let wt = env.ws("feature-login").join("alde");
    env.commit(&wt, "a.txt");
    std::fs::write(wt.join("new.txt"), "x").unwrap();

    let run = env.wt(&["ls"]);
    assert_ok(&run);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], "feature-login");
    assert!(lines[1].contains("alde") && lines[1].contains("feature/login"), "{}", run.stdout);
    assert!(lines[1].contains("Add a.txt") && lines[1].contains("↑1") && lines[1].contains("?1"), "{}", run.stdout);
    assert!(lines[2].contains("pen") && lines[2].contains("Initial"), "{}", run.stdout);
}

#[test]
fn cd_resolves_names() {
    let env = Env::new();
    env.add_repo("alde");
    assert_ok(&env.wt(&["add", "feature/login", "alde"]));
    assert_ok(&env.wt(&["add", "feature/logout", "alde"]));
    assert_ok(&env.wt(&["add", "fix-npe", "alde"]));

    assert_eq!(env.wt(&["cd", "feature/login"]).cd, env.ws("feature-login").to_string_lossy());
    assert_eq!(env.wt(&["cd", "npe"]).cd, env.ws("fix-npe").to_string_lossy());
    assert_eq!(env.wt(&["cd"]).cd, env.root.join("worktrees").to_string_lossy());
    assert_fails(&env.wt(&["cd", "feature"]), "matches several workspaces");
    assert_fails(&env.wt(&["cd", "zzz"]), "no workspace matches");

    // Without shell integration the path goes to stdout.
    let out = Command::new(env!("CARGO_BIN_EXE_wt"))
        .args(["cd", "npe"])
        .env("WT_ROOT", &env.root)
        .env_remove("WT_CD_FILE")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), env.ws("fix-npe").to_string_lossy());
}

#[test]
fn completion_lists() {
    let env = Env::new();
    env.add_repo("alde");
    env.add_repo("pen");
    assert_ok(&env.wt(&["add", "feature/login", "alde"]));

    let specs = env.wt(&["__complete", "specs"]).stdout;
    assert_eq!(specs.lines().collect::<Vec<_>>(), vec!["alde", "pen", "@alde"]);
    let branches = env.wt(&["__complete", "branches", "pen"]).stdout;
    assert!(branches.lines().any(|b| b == "feature/existing"), "{branches}");
    assert!(!branches.lines().any(|b| b == "HEAD"));
    assert_eq!(env.wt(&["__complete", "workspaces"]).stdout.trim(), "feature-login");
    assert_eq!(env.wt(&["__complete", "workspace-repos", "feature/login"]).stdout.trim(), "alde");
    assert_eq!(env.wt(&["__complete", "branches", "nope"]).stdout, "");
}

#[test]
fn shell_init_is_valid_zsh() {
    let env = Env::new();
    let run = env.wt(&["shell-init", "zsh"]);
    assert_ok(&run);
    assert!(run.stdout.contains("wt() {"));
    assert!(!run.stdout.contains("__WT_BIN__"));
    if Command::new("zsh").arg("--version").output().is_ok() {
        let script = env.dir.join("init.zsh");
        std::fs::write(&script, &run.stdout).unwrap();
        let check = Command::new("zsh").arg("-n").arg(&script).output().unwrap();
        assert!(check.status.success(), "zsh -n: {}", String::from_utf8_lossy(&check.stderr));
    }
}

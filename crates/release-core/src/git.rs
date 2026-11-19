//! All interaction with the `git` binary.
//!
//! Every invocation uses an argument vector rather than a shell string. The
//! previous implementation interpolated tag names and changelogs into shell
//! commands, which made backticks and `$` in a commit message a live injection
//! risk; `Command` removes that class of bug entirely.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Error, ErrorCode, Result};

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn stdout_trimmed(&self) -> String {
        self.stdout.trim().to_string()
    }
}

/// Runs a prepared command, reading both pipes on separate threads so that
/// large output cannot deadlock, and enforcing an optional timeout.
pub fn run_command(mut command: Command, timeout: Option<Duration>) -> Result<CommandOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| Error::other(format!("Could not run command: {error}")))?;

    let stdout_handle = child.stdout.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer);
            buffer
        })
    });

    let stderr_handle = child.stderr.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer);
            buffer
        })
    });

    let status = match timeout {
        None => child
            .wait()
            .map_err(|error| Error::other(format!("Command failed: {error}")))?,
        Some(limit) => {
            let started = Instant::now();

            loop {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| Error::other(format!("Command failed: {error}")))?
                {
                    break status;
                }

                if started.elapsed() >= limit {
                    let _ = child.kill();
                    break child
                        .wait()
                        .map_err(|error| Error::other(format!("Command failed: {error}")))?;
                }

                thread::sleep(Duration::from_millis(10));
            }
        }
    };

    let stdout = stdout_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    Ok(CommandOutput {
        success: status.success(),
        code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
    })
}

/// The working copy release is operating on.
#[derive(Debug, Clone)]
pub struct Git {
    cwd: PathBuf,
    allow_prompt: Option<bool>,
}

impl Git {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            allow_prompt: None,
        }
    }

    /// Controls whether git may ask for credentials on the terminal. The TUI
    /// disables prompting for the actual push so it cannot deadlock inside raw
    /// mode, after probing the remote while the terminal was handed back.
    pub fn prompting(&self, allow: bool) -> Self {
        let mut clone = self.clone();
        clone.allow_prompt = Some(allow);
        clone
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command.current_dir(&self.cwd).args(args);

        if let Some(allow) = self.allow_prompt {
            command.env("GIT_TERMINAL_PROMPT", if allow { "1" } else { "0" });
        }

        command
    }

    /// Runs git and returns the raw result without treating a non-zero exit as
    /// an error.
    pub fn raw(&self, args: &[&str]) -> Result<CommandOutput> {
        self.raw_with_timeout(args, None)
    }

    pub fn raw_with_timeout(
        &self,
        args: &[&str],
        timeout: Option<Duration>,
    ) -> Result<CommandOutput> {
        run_command(self.command(args), timeout)
    }

    /// Runs git and requires success.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        let output = self.raw(args)?;

        if !output.success {
            return Err(self.failure(args, &output));
        }

        Ok(output.stdout_trimmed())
    }

    pub fn run_with_timeout(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let output = self.raw_with_timeout(args, Some(timeout))?;

        if !output.success {
            return Err(self.failure(args, &output));
        }

        Ok(output.stdout_trimmed())
    }

    pub fn check(&self, args: &[&str], context: &str) -> Result<CommandOutput> {
        let output = self.raw(args)?;

        if !output.success {
            let detail = output.stderr.trim();

            let mut error = Error::new(
                ErrorCode::WriteFailed,
                if detail.is_empty() {
                    context.to_string()
                } else {
                    format!("{context}: {detail}")
                },
            );

            if detail.contains("terminal prompts disabled") || detail.contains("Permission denied")
            {
                error = error.with_remediation(
                    "Authenticate git with your remote (for example `gh auth setup-git`), then try again.",
                );
            }

            return Err(error);
        }

        Ok(output)
    }

    fn failure(&self, args: &[&str], output: &CommandOutput) -> Error {
        let detail = output.stderr.trim();
        let joined = args.join(" ");

        if detail.to_lowercase().contains("not a git repository") {
            return Error::new(ErrorCode::NotARepo, "Directory is not a Git repository.");
        }

        if detail.is_empty() {
            Error::other(format!("git {joined} failed"))
        } else {
            Error::other(format!("git {joined} failed: {detail}"))
        }
    }

    pub fn is_repo(&self) -> bool {
        self.raw(&["rev-parse", "--is-inside-work-tree"])
            .map(|output| output.success && output.stdout_trimmed() == "true")
            .unwrap_or(false)
    }

    pub fn current_branch(&self) -> Result<String> {
        let branch = self.run(&["rev-parse", "--abbrev-ref", "HEAD"])?;

        if branch == "HEAD" {
            return Err(Error::other(
                "The repository is in a detached HEAD state. Check out a branch first.",
            ));
        }

        Ok(branch)
    }

    pub fn head_hash(&self) -> Result<String> {
        self.run(&["rev-parse", "HEAD"])
    }

    pub fn local_branches(&self) -> Result<Vec<String>> {
        let output = self.run(&["branch", "--format=%(refname:short)"])?;

        Ok(output
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    /// Switches branches, refusing when the working tree has local changes.
    pub fn switch_branch(&self, branch: &str) -> Result<()> {
        let reference = format!("refs/heads/{branch}");

        let existing = self.raw(&["rev-parse", "--verify", reference.as_str()])?;

        if !existing.success {
            return Err(Error::usage(format!("Branch \"{branch}\" does not exist.")));
        }

        let status = self.run(&["status", "--porcelain", "--untracked-files=no"])?;

        if !status.is_empty() {
            return Err(Error::other(format!(
                "Cannot switch to \"{branch}\" with uncommitted changes. Commit or stash them first."
            )));
        }

        self.check(
            &["switch", branch],
            &format!("Could not switch to \"{branch}\""),
        )?;

        Ok(())
    }

    /// One process instead of three: `status --porcelain=v2 --branch` carries
    /// both the dirty state and the ahead/behind counts of the upstream.
    pub fn branch_synced(&self) -> bool {
        let output = match self.raw(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ]) {
            Ok(output) => output,
            Err(_) => return false,
        };

        if !output.success {
            return false;
        }

        let mut dirty = false;
        let mut ahead: Option<i64> = None;

        for line in output.stdout.lines() {
            if let Some(rest) = line.strip_prefix("# branch.ab ") {
                if let Some(value) = rest.split_whitespace().next() {
                    ahead = value.trim_start_matches('+').parse::<i64>().ok();
                }
            } else if !line.starts_with('#') && !line.trim().is_empty() {
                dirty = true;
            }
        }

        if dirty {
            return false;
        }

        match ahead {
            Some(count) => count == 0,
            // No upstream configured: fall back to counting unpushed commits.
            None => self
                .run(&["rev-list", "--count", "HEAD", "--not", "--remotes"])
                .map(|count| count.trim().parse::<i64>().unwrap_or(0) == 0)
                .unwrap_or(false),
        }
    }

    pub fn remote_url(&self, remote: &str) -> Result<String> {
        let key = format!("remote.{remote}.url");
        self.run(&["config", "--get", key.as_str()])
    }

    pub fn fetch(&self, remote: &str, timeout: Duration) -> Result<()> {
        let output =
            self.raw_with_timeout(&["fetch", "--prune", "--prune-tags", remote], Some(timeout))?;

        if !output.success {
            let detail = if output.stderr.trim().is_empty() {
                "git fetch failed".to_string()
            } else {
                output.stderr.trim().to_string()
            };

            return Err(Error::other(detail));
        }

        Ok(())
    }

    /// Cheap reachability probe used by the TUI to decide whether a push can
    /// possibly succeed without prompting for credentials.
    pub fn remote_reachable(&self, remote: &str, timeout: Duration) -> bool {
        self.raw_with_timeout(&["ls-remote", "--exit-code", remote, "HEAD"], Some(timeout))
            .map(|output| output.success)
            .unwrap_or(false)
    }

    pub fn add_all(&self) -> Result<()> {
        self.check(&["add", "-A"], "Could not stage the version bump")?;
        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<()> {
        self.check(
            &["commit", "-a", "-m", message],
            "Could not create the release commit",
        )?;
        Ok(())
    }

    pub fn tag_lightweight(&self, name: &str) -> Result<()> {
        self.check(&["tag", name], &format!("Could not tag \"{name}\""))?;
        Ok(())
    }

    pub fn tag_annotated(&self, name: &str, messages: &[String]) -> Result<()> {
        // Git's default `strip` cleanup deletes lines starting with `#`, which
        // would silently remove the changelog headings. `whitespace` only trims
        // surrounding whitespace and keeps every content line.
        let mut args: Vec<String> = vec![
            "tag".into(),
            "--cleanup=whitespace".into(),
            "-a".into(),
            name.into(),
        ];

        for message in messages {
            args.push("-m".into());
            args.push(message.clone());
        }

        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        self.check(&borrowed, &format!("Could not tag \"{name}\""))?;
        Ok(())
    }

    pub fn push(&self) -> Result<()> {
        self.check(&["push"], "Could not push to the remote")?;
        Ok(())
    }

    pub fn push_tags(&self) -> Result<()> {
        self.check(&["push", "--tags"], "Could not push tags to the remote")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_temp_dir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "release-git-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn detects_missing_repository() {
        let dir = in_temp_dir();
        let git = Git::new(&dir);
        assert!(!git.is_repo());
    }

    #[test]
    fn initialises_and_reports_branch() {
        let dir = in_temp_dir();
        let git = Git::new(&dir);
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();
        std::fs::write(dir.join("file.txt"), "hello").unwrap();
        git.add_all().unwrap();
        git.commit("initial").unwrap();

        assert!(git.is_repo());
        let branch = git.current_branch().unwrap();
        assert!(!branch.is_empty());
        assert_eq!(git.head_hash().unwrap().len(), 40);
    }

    #[test]
    fn refuses_to_switch_with_dirty_tree() {
        let dir = in_temp_dir();
        let git = Git::new(&dir);
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();
        std::fs::write(dir.join("file.txt"), "hello").unwrap();
        git.add_all().unwrap();
        git.commit("initial").unwrap();
        std::fs::write(dir.join("file.txt"), "changed").unwrap();

        let branch = git.current_branch().unwrap();
        let error = git.switch_branch(&branch).unwrap_err();
        assert!(error.message.contains("uncommitted changes"));
    }

    /// Builds a repository with a local bare remote that is in sync.
    fn synced_repository(name: &str) -> (PathBuf, Git) {
        let base = std::env::temp_dir().join(format!(
            "release-sync-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).unwrap();

        let remote = base.join("remote.git");
        let work = base.join("work");
        std::fs::create_dir_all(&work).unwrap();

        let bare = Git::new(&base);
        bare.run(&["init", "--bare", "-q", remote.to_str().unwrap()])
            .unwrap();

        let git = Git::new(&work);
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();
        git.run(&["remote", "add", "origin", remote.to_str().unwrap()])
            .unwrap();

        std::fs::write(work.join("file.txt"), "hello").unwrap();
        git.add_all().unwrap();
        git.commit("initial").unwrap();

        let branch = git.current_branch().unwrap();
        git.push().unwrap();
        git.run(&["branch", "--set-upstream-to", &format!("origin/{branch}")])
            .unwrap();

        (work, git)
    }

    #[test]
    fn keeps_markdown_headings_in_annotated_tags() {
        let dir = in_temp_dir();
        let git = Git::new(&dir);
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();
        std::fs::write(dir.join("file.txt"), "hello").unwrap();
        git.add_all().unwrap();
        git.commit("initial").unwrap();

        git.tag_annotated(
            "v1.0.0",
            &[
                "### Patches \n\n- Fix a crash: abc123".to_string(),
                "Update-Summary: short".to_string(),
            ],
        )
        .unwrap();

        let contents = git
            .run(&["for-each-ref", "refs/tags/v1.0.0", "--format=%(contents)"])
            .unwrap();

        assert!(
            contents.contains("### Patches"),
            "headings must survive git's message cleanup, got: {contents:?}"
        );
        assert!(contents.contains("Update-Summary: short"));
    }

    #[test]
    fn reports_a_synced_branch_and_a_dirty_tree() {
        let (work, git) = synced_repository("clean");

        let remote = work.parent().unwrap().join("remote.git");
        assert_eq!(git.remote_url("origin").unwrap(), remote.to_str().unwrap());
        assert!(
            git.branch_synced(),
            "a freshly pushed branch should be synced"
        );

        std::fs::write(work.join("file.txt"), "changed").unwrap();
        assert!(!git.branch_synced(), "a dirty tree must not be synced");
    }
}

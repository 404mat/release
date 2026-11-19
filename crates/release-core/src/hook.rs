//! Custom release hook: any executable that reads JSON and writes JSON.
//!
//! The previous implementation imported a JavaScript module. Since release must
//! run without Node, the contract is now a process boundary: the changelog and
//! metadata arrive on stdin, the filtered changelog leaves on stdout.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::{ChangeType, Error, Result};

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookCommit {
    pub hash: String,
    pub title: String,
    pub description: String,
    pub author: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookGroup {
    pub change_type: String,
    pub commits: Vec<HookCommit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookChangeType {
    pub handle: String,
    pub name: String,
    pub plural_name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPayload {
    pub changelog: String,
    pub tag: String,
    pub version: Option<String>,
    pub branch: String,
    pub commits: Vec<HookCommit>,
    pub grouped: Vec<HookGroup>,
    pub change_types: Vec<HookChangeType>,
    pub authors: Vec<String>,
}

impl HookPayload {
    pub fn new(
        changelog: impl Into<String>,
        tag: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            changelog: changelog.into(),
            tag: tag.into(),
            version: None,
            branch: branch.into(),
            commits: Vec::new(),
            grouped: Vec::new(),
            change_types: Vec::new(),
            authors: Vec::new(),
        }
    }

    pub fn with_change_types(mut self, change_types: &[ChangeType]) -> Self {
        self.change_types = change_types
            .iter()
            .map(|change_type| HookChangeType {
                handle: change_type.handle.to_string(),
                name: change_type.name.to_string(),
                plural_name: change_type.plural_name.to_string(),
                description: change_type.description.to_string(),
            })
            .collect();

        self
    }
}

#[derive(Debug, Clone)]
pub struct HookOutput {
    pub changelog: String,
    pub summary: Option<String>,
}

/// Resolves which hook to run: the explicit `--hook` flag wins, then the
/// repository configuration. Both are relative to the repository root.
pub fn resolve(root: &Path, explicit: Option<&Path>, configured: Option<&Path>) -> Option<PathBuf> {
    explicit
        .map(|path| root.join(path))
        .or_else(|| configured.map(|path| root.join(path)))
}

pub fn run(path: &Path, payload: &HookPayload) -> Result<HookOutput> {
    if !path.exists() {
        return Err(Error::usage(format!(
            "The specified --hook file doesn't exist: {}",
            path.display()
        )));
    }

    let body = serde_json::to_vec_pretty(payload)?;

    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            Error::other(format!(
                "The release hook file could not be executed ({}): {error}",
                path.display()
            ))
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::other("Could not write to the release hook."))?;

    let writer = thread::spawn(move || {
        let _ = stdin.write_all(&body);
        let _ = stdin.flush();
    });

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_reader = stdout.map(|mut handle| {
        thread::spawn(move || {
            let mut buffer = Vec::new();
            use std::io::Read;
            let _ = handle.read_to_end(&mut buffer);
            buffer
        })
    });

    let stderr_reader = stderr.map(|mut handle| {
        thread::spawn(move || {
            let mut buffer = Vec::new();
            use std::io::Read;
            let _ = handle.read_to_end(&mut buffer);
            buffer
        })
    });

    let started = Instant::now();
    let mut timed_out = false;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                return Err(Error::other(format!("The release hook failed: {error}")));
            }
        }

        if started.elapsed() >= TIMEOUT {
            let _ = child.kill();
            timed_out = true;
            break child
                .wait()
                .map_err(|error| Error::other(format!("The release hook failed: {error}")))?;
        }

        thread::sleep(Duration::from_millis(10));
    };

    let _ = writer.join();

    let stdout = stdout_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if timed_out {
        return Err(Error::other(format!(
            "The release hook timed out after {} seconds.",
            TIMEOUT.as_secs()
        )));
    }

    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();

        if detail.is_empty() {
            return Err(Error::other("The release hook failed."));
        }

        return Err(Error::other(format!("The release hook failed: {detail}")));
    }

    let output = String::from_utf8_lossy(&stdout).trim().to_string();

    if output.is_empty() {
        return Err(Error::other("The release hook returned nothing on stdout."));
    }

    interpret(&output)
}

fn interpret(output: &str) -> Result<HookOutput> {
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        match value {
            Value::String(text) => {
                return Ok(HookOutput {
                    changelog: text,
                    summary: None,
                });
            }
            Value::Object(map) => {
                let changelog = map
                    .get("changelog")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let summary = map
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string);

                if let Some(changelog) = changelog {
                    return Ok(HookOutput { changelog, summary });
                }
            }
            _ => {}
        }
    }

    // Plain markdown is accepted too, so a hook can be a one-line script.
    Ok(HookOutput {
        changelog: output.to_string(),
        summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn script(directory: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("hook.sh");
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("release-hook-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn rewrites_the_changelog_from_json_input() {
        let dir = temp_dir("json");
        let path = script(
            &dir,
            "#!/bin/sh\nbody=$(cat)\ncat <<EOF\n{\"changelog\": \"REWRITTEN\", \"summary\": \"short\"}\nEOF\n",
        );

        let payload = HookPayload::new("original", "v1.0.0", "main");
        let output = run(&path, &payload).unwrap();

        assert_eq!(output.changelog, "REWRITTEN");
        assert_eq!(output.summary.as_deref(), Some("short"));
    }

    #[cfg(unix)]
    #[test]
    fn accepts_plain_markdown() {
        let dir = temp_dir("plain");
        let path = script(&dir, "#!/bin/sh\ncat > /dev/null\nprintf 'plain body'\n");

        let output = run(&path, &HookPayload::new("x", "v1", "main")).unwrap();
        assert_eq!(output.changelog, "plain body");
    }

    #[cfg(unix)]
    #[test]
    fn fails_when_the_hook_exits_non_zero() {
        let dir = temp_dir("fail");
        let path = script(&dir, "#!/bin/sh\ncat > /dev/null\necho boom >&2\nexit 3\n");

        let error = run(&path, &HookPayload::new("x", "v1", "main")).unwrap_err();
        assert!(error.message.contains("boom"));
    }

    #[cfg(unix)]
    #[test]
    fn fails_when_the_hook_is_missing() {
        let error = run(
            Path::new("/nonexistent/hook"),
            &HookPayload::new("x", "v1", "main"),
        )
        .unwrap_err();

        assert_eq!(error.code, crate::ErrorCode::Usage);
    }
}

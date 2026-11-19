//! Reads, validates and rewrites the project's version manifest and lockfile.
//!
//! Two ecosystems are supported: npm (`package.json` + `package-lock.json`) and
//! Cargo (`Cargo.toml` + `Cargo.lock`). The manifest is detected from the working
//! directory, with `package.json` winning when both exist.

use std::path::{Path, PathBuf};

use semver::Version;
use serde_json::Value;

use crate::semver_ext;
use crate::tags::Tag;
use crate::{Error, ErrorCode, Result};

const NPM_MANIFEST: &str = "package.json";
const NPM_LOCK: &str = "package-lock.json";
const CARGO_MANIFEST: &str = "Cargo.toml";
const CARGO_LOCK: &str = "Cargo.lock";

/// A prepared version bump: the new version plus the exact file contents to
/// write. Nothing touches disk until the plan is applied.
#[derive(Debug, Clone)]
pub struct Bump {
    pub old_version: Version,
    pub version: Version,
    pub tag: String,
    pub writes: Vec<(PathBuf, String)>,
}

impl Bump {
    pub fn version_string(&self) -> String {
        self.version.to_string()
    }
}

/// Computes the next version for whichever manifest the directory uses.
pub fn compute(cwd: &Path, kind: &str, suffix: Option<&str>, latest: Option<&Tag>) -> Result<Bump> {
    if cwd.join(NPM_MANIFEST).exists() {
        return compute_npm(cwd, kind, suffix, latest);
    }

    if cwd.join(CARGO_MANIFEST).exists() {
        return compute_cargo(cwd, kind, suffix, latest);
    }

    Err(Error::usage(
        "No \"package.json\" or \"Cargo.toml\" found to read the version from",
    ))
}

fn compute_npm(cwd: &Path, kind: &str, suffix: Option<&str>, latest: Option<&Tag>) -> Result<Bump> {
    let package_path = cwd.join(NPM_MANIFEST);

    let raw = std::fs::read_to_string(&package_path)
        .map_err(|_| Error::usage("Couldn't read \"package.json\""))?;

    let mut package: Value =
        serde_json::from_str(&raw).map_err(|_| Error::usage("Couldn't parse \"package.json\""))?;

    let declared = package
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(declared) = declared else {
        return Err(Error::usage("No \"version\" field inside \"package.json\""));
    };

    let (old_version, version, prefix) =
        plan_version(&declared, NPM_MANIFEST, kind, suffix, latest)?;

    if let Some(object) = package.as_object_mut() {
        object.insert("version".to_string(), Value::String(version.to_string()));
    }

    let mut writes = vec![(package_path, render_json(&package)?)];

    let lock_path = cwd.join(NPM_LOCK);

    if lock_path.exists() {
        let lock_raw = std::fs::read_to_string(&lock_path)
            .map_err(|_| Error::usage("Couldn't read \"package-lock.json\""))?;

        let mut lock: Value = serde_json::from_str(&lock_raw)
            .map_err(|_| Error::usage("Couldn't parse \"package-lock.json\""))?;

        if let Some(object) = lock.as_object_mut() {
            object.insert("version".to_string(), Value::String(version.to_string()));
        }

        writes.push((lock_path, render_json(&lock)?));
    }

    let tag = format!("{prefix}{version}");

    Ok(Bump {
        old_version,
        version,
        tag,
        writes,
    })
}

fn compute_cargo(
    cwd: &Path,
    kind: &str,
    suffix: Option<&str>,
    latest: Option<&Tag>,
) -> Result<Bump> {
    let manifest_path = cwd.join(CARGO_MANIFEST);

    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|_| Error::usage("Couldn't read \"Cargo.toml\""))?;

    let (table, declared) = cargo_version(&raw)
        .ok_or_else(|| Error::usage("No \"version\" field inside \"Cargo.toml\""))?;

    let (old_version, version, prefix) =
        plan_version(&declared, CARGO_MANIFEST, kind, suffix, latest)?;

    let updated = with_manifest_version(&raw, &table, &declared, &version.to_string());
    let mut writes = vec![(manifest_path, updated)];

    let lock_path = cwd.join(CARGO_LOCK);

    if lock_path.exists() {
        let lock_raw = std::fs::read_to_string(&lock_path)
            .map_err(|_| Error::usage("Couldn't read \"Cargo.lock\""))?;

        let lock_updated =
            with_lock_version(&lock_raw, &old_version.to_string(), &version.to_string());

        // Cargo regenerates the lock on the next build; only rewrite it when a
        // workspace member actually declares the old version.
        if lock_updated != lock_raw {
            writes.push((lock_path, lock_updated));
        }
    }

    let tag = format!("{prefix}{version}");

    Ok(Bump {
        old_version,
        version,
        tag,
        writes,
    })
}

/// Validates the declared version against the latest tag and computes the next
/// one. Shared by both manifests so the guards cannot drift apart: the version
/// must be valid SemVer and must already match the latest tag, so a release can
/// never silently skip a version.
fn plan_version(
    declared: &str,
    manifest: &str,
    kind: &str,
    suffix: Option<&str>,
    latest: Option<&Tag>,
) -> Result<(Version, Version, String)> {
    let old_version = semver_ext::parse_lenient(declared).ok_or_else(|| {
        Error::usage(format!(
            "The \"{manifest}\" version is not SemVer-compatible"
        ))
    })?;

    let mut prefix = String::new();

    if let Some(latest) = latest {
        if old_version != latest.version {
            return Err(Error::new(
                ErrorCode::VersionMismatch,
                format!(
                    "The \"{manifest}\" version ({declared}) doesn't match the latest release tag ({})",
                    latest.tag
                ),
            ));
        }

        prefix = latest.prefix();
    }

    let is_prerelease = !old_version.pre.is_empty();
    let wants_prerelease = kind == "pre";

    let mut suffix = suffix.map(str::to_string);

    if !is_prerelease && wants_prerelease && suffix.as_deref().unwrap_or("").is_empty() {
        suffix = Some("canary".to_string());
    }

    let effective_suffix = suffix.as_deref().filter(|value| !value.is_empty());
    let version = semver_ext::inc(&old_version, kind, effective_suffix)?;

    Ok((old_version, version, prefix))
}

fn render_json(value: &Value) -> Result<String> {
    let mut rendered = serde_json::to_string_pretty(value)?;

    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }

    Ok(rendered)
}

/// The version declared in `Cargo.toml` and the table it lives in. A workspace
/// version is preferred over a package version, because members inherit it.
fn cargo_version(text: &str) -> Option<(String, String)> {
    let mut table = String::new();
    let mut package: Option<String> = None;
    let mut workspace: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            table = trimmed.to_string();
            continue;
        }

        let Some(value) = version_value(trimmed) else {
            continue;
        };

        match table.as_str() {
            "[workspace.package]" if workspace.is_none() => workspace = Some(value),
            "[package]" if package.is_none() => package = Some(value),
            _ => {}
        }
    }

    workspace
        .map(|version| ("[workspace.package]".to_string(), version))
        .or_else(|| package.map(|version| ("[package]".to_string(), version)))
}

/// Reads a bare `version = "1.2.3"` line. `version.workspace = true` and inline
/// tables are ignored on purpose.
fn version_value(line: &str) -> Option<String> {
    let (key, rest) = line.split_once('=')?;

    if key.trim() != "version" {
        return None;
    }

    let start = rest.find('"')?;
    let after = &rest[start + 1..];
    let end = after.find('"')?;

    Some(after[..end].to_string())
}

/// Rewrites the version inside one table, leaving every other line — comments,
/// ordering, indentation — untouched.
fn with_manifest_version(text: &str, table: &str, old: &str, new: &str) -> String {
    let mut current = String::new();
    let mut replaced = false;
    let mut out = String::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            current = trimmed.to_string();
        } else if !replaced && current == table && version_value(trimmed).as_deref() == Some(old) {
            out.push_str(&line.replacen(&format!("\"{old}\""), &format!("\"{new}\""), 1));
            out.push('\n');
            replaced = true;
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

/// Rewrites the version of every workspace member in `Cargo.lock`. Members are
/// the entries without a `source`, so registry dependencies are never touched.
fn with_lock_version(text: &str, old: &str, new: &str) -> String {
    let mut out = String::new();
    let mut block: Vec<&str> = Vec::new();

    for line in text.lines() {
        if line.trim_start().starts_with("[[package]]") && !block.is_empty() {
            push_lock_block(&mut out, &block, old, new);
            block.clear();
        }

        block.push(line);
    }

    if !block.is_empty() {
        push_lock_block(&mut out, &block, old, new);
    }

    out
}

fn push_lock_block(out: &mut String, block: &[&str], old: &str, new: &str) {
    let is_workspace_member = block
        .iter()
        .all(|line| !line.trim_start().starts_with("source ="));

    for line in block {
        let trimmed = line.trim_start();

        if is_workspace_member && trimmed.starts_with("version =") {
            out.push_str(&line.replacen(&format!("\"{old}\""), &format!("\"{new}\""), 1));
        } else {
            out.push_str(line);
        }

        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "release-bump-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_package(dir: &Path, body: &str) {
        std::fs::write(dir.join("package.json"), body).unwrap();
    }

    fn write_cargo(dir: &Path, body: &str) {
        std::fs::write(dir.join("Cargo.toml"), body).unwrap();
    }

    #[test]
    fn bumps_and_preserves_key_order() {
        let dir = temp_dir("order");
        write_package(
            &dir,
            "{\n  \"name\": \"demo\",\n  \"version\": \"1.2.3\",\n  \"license\": \"MIT\"\n}\n",
        );

        let bump = compute(&dir, "patch", None, None).unwrap();

        assert_eq!(bump.version.to_string(), "1.2.4");
        assert_eq!(bump.tag, "1.2.4");

        let (path, contents) = &bump.writes[0];
        assert_eq!(path, &dir.join("package.json"));
        assert!(contents.contains("\"name\": \"demo\""));
        assert!(contents.contains("\"version\": \"1.2.4\""));
        assert!(contents.ends_with('\n'));
        // Insertion order is preserved: name, version, license.
        let name = contents.find("\"name\"").unwrap();
        let version = contents.find("\"version\"").unwrap();
        let license = contents.find("\"license\"").unwrap();
        assert!(name < version && version < license);
    }

    #[test]
    fn preserves_the_tag_prefix() {
        let dir = temp_dir("prefix");
        write_package(&dir, "{\"version\": \"1.2.3\"}\n");

        let latest = Tag {
            tag: "v1.2.3".to_string(),
            version: Version::parse("1.2.3").unwrap(),
            hash: "abc".to_string(),
        };

        let bump = compute(&dir, "minor", None, Some(&latest)).unwrap();
        assert_eq!(bump.tag, "v1.3.0");
    }

    #[test]
    fn updates_the_npm_lockfile_when_present() {
        let dir = temp_dir("npm-lock");
        write_package(&dir, "{\"version\": \"1.2.3\"}\n");
        std::fs::write(
            dir.join("package-lock.json"),
            "{\"name\": \"demo\", \"version\": \"1.2.3\"}\n",
        )
        .unwrap();

        let bump = compute(&dir, "patch", None, None).unwrap();

        assert_eq!(bump.writes.len(), 2);
        assert!(bump.writes[1].1.contains("\"version\": \"1.2.4\""));
    }

    #[test]
    fn rejects_a_version_mismatch() {
        let dir = temp_dir("mismatch");
        write_package(&dir, "{\"version\": \"1.2.3\"}\n");

        let latest = Tag {
            tag: "v1.2.2".to_string(),
            version: Version::parse("1.2.2").unwrap(),
            hash: "abc".to_string(),
        };

        let error = compute(&dir, "patch", None, Some(&latest)).unwrap_err();
        assert_eq!(error.code, ErrorCode::VersionMismatch);
        assert!(error.message.contains("\"package.json\" version (1.2.3)"));
    }

    #[test]
    fn rejects_a_directory_without_a_manifest() {
        let dir = temp_dir("missing");
        let error = compute(&dir, "patch", None, None).unwrap_err();

        assert!(error.message.contains("\"package.json\" or \"Cargo.toml\""));
    }

    #[test]
    fn rejects_an_invalid_version() {
        let dir = temp_dir("invalid");
        write_package(&dir, "{\"version\": \"banana\"}\n");

        let error = compute(&dir, "patch", None, None).unwrap_err();
        assert!(error.message.contains("not SemVer-compatible"));
    }

    #[test]
    fn defaults_the_prerelease_suffix_to_canary() {
        let dir = temp_dir("canary");
        write_package(&dir, "{\"version\": \"1.2.3\"}\n");

        let bump = compute(&dir, "pre", None, None).unwrap();
        assert_eq!(bump.version.to_string(), "1.2.4-canary.0");
    }

    #[test]
    fn bumps_a_cargo_manifest() {
        let dir = temp_dir("cargo");
        write_cargo(
            &dir,
            "[package]\nname = \"demo\"\nversion = \"1.2.3\"\nedition = \"2021\"\n",
        );

        let bump = compute(&dir, "patch", None, None).unwrap();

        assert_eq!(bump.version.to_string(), "1.2.4");
        assert_eq!(bump.tag, "1.2.4");

        let (path, contents) = &bump.writes[0];
        assert_eq!(path, &dir.join("Cargo.toml"));
        assert!(contents.contains("version = \"1.2.4\""));
        assert!(
            contents.contains("edition = \"2021\""),
            "other keys untouched: {contents}"
        );
        assert!(contents.starts_with("[package]"));
    }

    #[test]
    fn prefers_the_workspace_version() {
        let dir = temp_dir("workspace");
        write_cargo(
            &dir,
            "[workspace]\nmembers = [\"crates/a\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[package]\nname = \"root\"\nversion = \"9.9.9\"\n",
        );

        let bump = compute(&dir, "minor", None, None).unwrap();
        let contents = &bump.writes[0].1;

        assert!(bump.version.to_string() == "1.1.0");
        assert!(
            contents.contains("[workspace.package]\nversion = \"1.1.0\""),
            "workspace version bumped: {contents}"
        );
        assert!(
            contents.contains("version = \"9.9.9\""),
            "package table untouched: {contents}"
        );
    }

    #[test]
    fn bumps_workspace_members_in_the_cargo_lock() {
        let dir = temp_dir("cargo-lock");
        write_cargo(&dir, "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n");
        std::fs::write(
            dir.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"demo\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        )
        .unwrap();

        let bump = compute(&dir, "patch", None, None).unwrap();

        assert_eq!(bump.writes.len(), 2);

        let lock = &bump.writes[1].1;
        assert!(
            lock.contains("name = \"demo\"\nversion = \"1.0.1\""),
            "member bumped: {lock}"
        );
        assert!(
            lock.contains("name = \"serde\"\nversion = \"1.0.0\""),
            "dependency untouched: {lock}"
        );
        assert!(lock.contains("version = 4"), "lock format version kept");
    }

    #[test]
    fn leaves_the_cargo_lock_alone_without_a_version_match() {
        let dir = temp_dir("cargo-lock-stale");
        write_cargo(&dir, "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n");
        std::fs::write(
            dir.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"demo\"\nversion = \"0.9.0\"\n",
        )
        .unwrap();

        let bump = compute(&dir, "patch", None, None).unwrap();

        assert_eq!(bump.writes.len(), 1, "no stale lock rewrite");
    }

    #[test]
    fn preserves_the_cargo_tag_prefix() {
        let dir = temp_dir("cargo-prefix");
        write_cargo(&dir, "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n");

        let latest = Tag {
            tag: "v1.2.3".to_string(),
            version: Version::parse("1.2.3").unwrap(),
            hash: "abc".to_string(),
        };

        let bump = compute(&dir, "minor", None, Some(&latest)).unwrap();
        assert_eq!(bump.tag, "v1.3.0");
    }

    #[test]
    fn rejects_a_cargo_version_mismatch() {
        let dir = temp_dir("cargo-mismatch");
        write_cargo(&dir, "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n");

        let latest = Tag {
            tag: "v1.2.2".to_string(),
            version: Version::parse("1.2.2").unwrap(),
            hash: "abc".to_string(),
        };

        let error = compute(&dir, "patch", None, Some(&latest)).unwrap_err();

        assert_eq!(error.code, ErrorCode::VersionMismatch);
        assert!(error.message.contains("\"Cargo.toml\" version (1.2.3)"));
    }

    #[test]
    fn ignores_inherited_version_keys() {
        let dir = temp_dir("cargo-inherited");
        write_cargo(
            &dir,
            "[package]\nname = \"demo\"\nversion.workspace = true\n",
        );

        let error = compute(&dir, "patch", None, None).unwrap_err();
        assert!(error.message.contains("No \"version\" field"));
    }
}

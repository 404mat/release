//! Reads, validates and rewrites `package.json` (and `package-lock.json`).

use std::path::{Path, PathBuf};

use semver::Version;
use serde_json::Value;

use crate::semver_ext;
use crate::tags::Tag;
use crate::{Error, ErrorCode, Result};

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

/// Validates `package.json` against the latest release tag and computes the next
/// version. Mirrors the previous checks: the package version must be valid
/// SemVer and must already match the latest tag, so a release can never silently
/// skip a version.
pub fn compute(cwd: &Path, kind: &str, suffix: Option<&str>, latest: Option<&Tag>) -> Result<Bump> {
    let package_path = cwd.join("package.json");

    if !package_path.exists() {
        return Err(Error::usage("The \"package.json\" file doesn't exist"));
    }

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

    let old_version = semver_ext::parse_lenient(&declared)
        .ok_or_else(|| Error::usage("The \"package.json\" version is not SemVer-compatible"))?;

    let mut prefix = String::new();

    if let Some(latest) = latest {
        if old_version != latest.version {
            return Err(Error::new(
                ErrorCode::VersionMismatch,
                format!(
                    "The \"package.json\" version ({declared}) doesn't match the latest release tag ({})",
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

    if let Some(object) = package.as_object_mut() {
        object.insert("version".to_string(), Value::String(version.to_string()));
    }

    let mut writes = vec![(package_path, render(&package)?)];

    let lock_path = cwd.join("package-lock.json");

    if lock_path.exists() {
        let lock_raw = std::fs::read_to_string(&lock_path)
            .map_err(|_| Error::usage("Couldn't read \"package-lock.json\""))?;

        let mut lock: Value = serde_json::from_str(&lock_raw)
            .map_err(|_| Error::usage("Couldn't parse \"package-lock.json\""))?;

        if let Some(object) = lock.as_object_mut() {
            object.insert("version".to_string(), Value::String(version.to_string()));
        }

        writes.push((lock_path, render(&lock)?));
    }

    let tag = format!("{prefix}{version}");

    Ok(Bump {
        old_version,
        version,
        tag,
        writes,
    })
}

fn render(value: &Value) -> Result<String> {
    let mut rendered = serde_json::to_string_pretty(value)?;

    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }

    Ok(rendered)
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
    fn updates_the_lockfile_when_present() {
        let dir = temp_dir("lock");
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
    }

    #[test]
    fn rejects_a_missing_package_json() {
        let dir = temp_dir("missing");
        let error = compute(&dir, "patch", None, None).unwrap_err();
        assert!(error.message.contains("doesn't exist"));
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
}

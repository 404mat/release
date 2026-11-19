//! Daily-cached update check against this project's GitHub releases.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::github::{GitHub, RepoCoordinates};
use crate::semver_ext;

const CACHE_TTL_SECONDS: u64 = 60 * 60 * 24;
const CACHE_FILE: &str = "update.json";

#[derive(Debug, Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    latest: Option<String>,
}

pub fn cache_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        if !path.is_empty() {
            return Some(PathBuf::from(path).join("release"));
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".cache").join("release"))
}

/// Returns the newest published version when it is newer than the running
/// binary. Network failures are silent: an update notice is never worth failing
/// a release over.
pub fn check(github: &GitHub, coordinates: &RepoCoordinates) -> Option<String> {
    let cache_path = cache_dir().map(|dir| dir.join(CACHE_FILE));

    if let Some(path) = &cache_path {
        if let Some(cache) = read_cache(path) {
            if !is_stale(cache.checked_at) {
                return newer_than_current(cache.latest.as_deref());
            }
        }
    }

    let latest = github.latest_release_tag(coordinates).ok().flatten();

    if let Some(path) = &cache_path {
        write_cache(path, &latest);
    }

    newer_than_current(latest.as_deref())
}

fn newer_than_current(latest: Option<&str>) -> Option<String> {
    let latest = latest?;
    let parsed = semver_ext::parse_lenient(latest)?;
    let current = semver_ext::parse_lenient(env!("CARGO_PKG_VERSION"))?;

    if parsed > current {
        Some(parsed.to_string())
    } else {
        None
    }
}

fn is_stale(checked_at: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    now.saturating_sub(checked_at) >= CACHE_TTL_SECONDS
}

fn read_cache(path: &std::path::Path) -> Option<Cache> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(path: &std::path::Path, latest: &Option<String>) {
    let Some(parent) = path.parent() else {
        return;
    };

    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

    let cache = Cache {
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        latest: latest.clone(),
    };

    if let Ok(rendered) = serde_json::to_string(&cache) {
        let _ = std::fs::write(path, rendered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reports_versions_ahead_of_the_binary() {
        assert_eq!(newer_than_current(Some("0.0.1")), None);
        assert_eq!(
            newer_than_current(Some("v999.0.0")),
            Some("999.0.0".to_string())
        );
        assert_eq!(newer_than_current(None), None);
        assert_eq!(newer_than_current(Some("banana")), None);
    }

    #[test]
    fn treats_old_timestamps_as_stale() {
        assert!(is_stale(1));
    }
}

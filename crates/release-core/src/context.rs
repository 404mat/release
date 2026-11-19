//! Shared state for a run: repository, configuration, and the GitHub client.

use std::path::PathBuf;

use crate::config::Config;
use crate::git::Git;
use crate::github::{self, GitHub, RepoCoordinates};
use crate::{auth, Error, ErrorCode, Progress, Result, Warning};

pub struct Forge {
    pub github: GitHub,
    pub coordinates: RepoCoordinates,
}

impl Forge {
    pub fn slug(&self) -> String {
        self.coordinates.slug()
    }
}

/// Everything a run needs, resolved once.
pub struct Context {
    pub cwd: PathBuf,
    pub git: Git,
    pub config: Config,
    /// `None` when running offline or when GitHub is unreachable.
    pub forge: Option<Forge>,
    /// Whether the configured remote could be resolved. When false there is
    /// nothing to fetch and no tag to push to.
    pub remote_available: bool,
    pub warnings: Vec<Warning>,
}

impl Context {
    pub fn open(
        cwd: PathBuf,
        offline: bool,
        require_forge: bool,
        progress: &dyn Progress,
    ) -> Result<Self> {
        let config = Config::load(&cwd)?;
        let git = Git::new(cwd.clone());

        if !git.is_repo() {
            return Err(Error::new(
                ErrorCode::NotARepo,
                "Directory is not a Git repository.",
            ));
        }

        let mut warnings = Vec::new();
        let mut forge = None;
        let mut remote_available = true;

        if !offline {
            // Hard requirement: gh must be installed and authenticated.
            auth::ensure_authed()?;
            let token = auth::token()?;
            let github = GitHub::new(token);

            let remote = match git.remote_url(&config.remote) {
                Ok(remote) => Some(remote),
                Err(_) if !require_forge => {
                    remote_available = false;

                    warnings.push(Warning::new(
                        "no_remote",
                        format!(
                            "Could not read the \"{}\" remote, so GitHub was not used.",
                            config.remote
                        ),
                    ));
                    None
                }
                Err(_) => {
                    return Err(Error::other(format!(
                        "Could not read the \"{}\" remote. Configure it in release.toml or with `git remote add`.",
                        config.remote
                    )));
                }
            };

            if let Some(remote) = remote {
                let coordinates = match github::parse_remote(&remote) {
                    Ok(coordinates) => Some(coordinates),
                    Err(error) if !require_forge => {
                        progress.warn(&error.short());
                        warnings.push(Warning::new("github_unavailable", error.short()));
                        None
                    }
                    Err(error) => return Err(error),
                };

                if let Some(coordinates) = coordinates {
                    progress.start("Connecting to GitHub");

                    match github.repo(&coordinates) {
                        Ok(info) => {
                            progress.succeed(Some("Connected to GitHub"));

                            forge = Some(Forge {
                                github,
                                coordinates: RepoCoordinates {
                                    owner: info.owner.login,
                                    repo: info.name,
                                },
                            });
                        }
                        Err(error) if !require_forge => {
                            progress.warn(&format!("GitHub is unavailable: {}", error.short()));
                            warnings.push(Warning::new(
                                "github_unavailable",
                                format!("GitHub is unavailable: {}", error.short()),
                            ));
                        }
                        Err(error) => {
                            progress.succeed(None);
                            return Err(error);
                        }
                    }
                }
            }
        }

        Ok(Self {
            cwd,
            git,
            config,
            forge,
            remote_available,
            warnings,
        })
    }

    pub fn forge(&self) -> Result<&Forge> {
        self.forge.as_ref().ok_or_else(|| {
            Error::new(
                ErrorCode::Auth,
                "This action needs GitHub, but no authenticated connection is available.",
            )
            .with_remediation("Run `gh auth login` and make sure the network is reachable.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Silent;

    fn repository_without_remote(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "release-context-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let git = Git::new(&dir);
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();
        std::fs::write(dir.join("package.json"), "{\"version\": \"1.0.0\"}").unwrap();
        git.add_all().unwrap();
        git.commit("initial").unwrap();

        dir
    }

    /// These tests need an authenticated `gh`, which CI does not have. They skip
    /// instead of failing there, and still protect the branches locally.
    fn gh_available() -> bool {
        auth::ensure_authed().is_ok()
    }

    #[test]
    fn a_missing_remote_is_fatal_when_github_is_required() {
        if !gh_available() {
            return;
        }

        let dir = repository_without_remote("required");
        let error = match Context::open(dir, false, true, &Silent) {
            Ok(_) => panic!("a missing remote must fail when GitHub is required"),
            Err(error) => error,
        };

        assert!(error
            .message
            .contains("Could not read the \"origin\" remote"));
        assert!(error
            .message
            .contains("Configure it in release.toml or with `git remote add`"));
    }

    #[test]
    fn a_missing_remote_is_only_a_warning_when_github_is_optional() {
        if !gh_available() {
            return;
        }

        let dir = repository_without_remote("optional");
        let context = Context::open(dir, false, false, &Silent).unwrap();

        assert!(context.forge.is_none());
        assert!(!context.remote_available);
        assert!(context
            .warnings
            .iter()
            .any(|warning| warning.code == "no_remote"));
    }
}

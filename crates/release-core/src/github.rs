//! A small, purpose-built GitHub client.
//!
//! Release needs seven endpoints. A full SDK would drag in an async runtime and
//! a large dependency tree, so the calls are written directly against the REST
//! API; PR authorship is fetched in one batched GraphQL query instead of one
//! request per commit.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::{Error, ErrorCode, Result};

const API: &str = "https://api.github.com";
const GRAPHQL: &str = "https://api.github.com/graphql";

#[derive(Debug, Clone)]
pub struct RepoCoordinates {
    pub owner: String,
    pub repo: String,
}

impl RepoCoordinates {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub owner: Owner,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Owner {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseInfo {
    pub id: u64,
    pub tag_name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub body: Option<String>,
}

impl ReleaseInfo {
    pub fn is_published(&self) -> bool {
        !self.draft
    }
}

pub struct GitHub {
    agent: ureq::Agent,
    token: String,
}

impl GitHub {
    pub fn new(token: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent(concat!("release/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build();

        Self {
            agent,
            token: token.into(),
        }
    }

    fn request(&self, method: &str, url: &str, body: Option<Value>) -> Result<Option<Value>> {
        let request = match method {
            "GET" => self.agent.get(url),
            "POST" => self.agent.post(url),
            "PATCH" => self.agent.patch(url),
            other => return Err(Error::other(format!("Unsupported HTTP method {other}"))),
        };

        let authorization = format!("Bearer {}", self.token);

        let request = request
            .set("Authorization", &authorization)
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28");

        let outcome = match body {
            Some(value) => request.send_json(value),
            None => request.call(),
        };

        let response = match outcome {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                let text = response.into_string().unwrap_or_default();
                let detail = api_message(&text);

                if status == 404 {
                    return Ok(None);
                }

                let code = match status {
                    401 | 403 => ErrorCode::Auth,
                    422 => ErrorCode::WriteFailed,
                    _ => ErrorCode::Other,
                };

                let mut error = Error::new(
                    code,
                    if detail.is_empty() {
                        format!("GitHub request failed with status {status}")
                    } else {
                        format!("GitHub request failed with status {status}: {detail}")
                    },
                );

                if status == 401 || status == 403 {
                    error = error.with_remediation(
                        "Run `gh auth login` to refresh your GitHub credentials.",
                    );
                }

                return Err(error);
            }
            Err(ureq::Error::Transport(transport)) => {
                return Err(Error::new(
                    ErrorCode::WriteFailed,
                    format!("Could not reach GitHub: {transport}"),
                )
                .with_remediation("Check your network connection and try again."));
            }
        };

        let value: Value = response.into_json().map_err(|error| {
            Error::other(format!("Could not parse the GitHub response: {error}"))
        })?;

        Ok(Some(value))
    }

    fn get(&self, path: &str) -> Result<Option<Value>> {
        self.request("GET", &format!("{API}{path}"), None)
    }

    fn get_required(&self, path: &str) -> Result<Value> {
        self.get(path)?
            .ok_or_else(|| Error::other(format!("GitHub returned no data for {path}")))
    }

    pub fn repo(&self, coordinates: &RepoCoordinates) -> Result<RepoInfo> {
        let path = format!("/repos/{}", coordinates.slug());
        let value = self.get_required(&path)?;

        serde_json::from_value(value)
            .map_err(|error| Error::other(format!("Unexpected repository payload: {error}")))
    }

    pub fn authenticated_login(&self) -> Result<String> {
        let value = self.get_required("/user")?;
        value
            .get("login")
            .and_then(Value::as_str)
            .map(|login| login.to_string())
            .ok_or_else(|| Error::other("GitHub did not return a login for the current token."))
    }

    pub fn list_releases(&self, coordinates: &RepoCoordinates) -> Result<Vec<ReleaseInfo>> {
        let path = format!("/repos/{}/releases?per_page=100", coordinates.slug());
        let value = self.get_required(&path)?;

        serde_json::from_value(value)
            .map_err(|error| Error::other(format!("Unexpected release payload: {error}")))
    }

    pub fn create_release(
        &self,
        coordinates: &RepoCoordinates,
        payload: &Value,
    ) -> Result<ReleaseInfo> {
        let path = format!("{API}/repos/{}/releases", coordinates.slug());
        let value = self
            .request("POST", &path, Some(payload.clone()))?
            .ok_or_else(|| Error::other("GitHub did not return the created release."))?;

        serde_json::from_value(value)
            .map_err(|error| Error::other(format!("Unexpected release payload: {error}")))
    }

    pub fn update_release(
        &self,
        coordinates: &RepoCoordinates,
        id: u64,
        payload: &Value,
    ) -> Result<ReleaseInfo> {
        let path = format!("{API}/repos/{}/releases/{id}", coordinates.slug());
        let value = self
            .request("PATCH", &path, Some(payload.clone()))?
            .ok_or_else(|| Error::other("could not update the release."))?;

        serde_json::from_value(value)
            .map_err(|error| Error::other(format!("Unexpected release payload: {error}")))
    }

    pub fn latest_release_tag(&self, coordinates: &RepoCoordinates) -> Result<Option<String>> {
        let path = format!("/repos/{}/releases/latest", coordinates.slug());

        Ok(self.get(&path)?.and_then(|value| {
            value
                .get("tag_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        }))
    }

    fn pull_author(&self, coordinates: &RepoCoordinates, number: u64) -> Result<Option<String>> {
        let path = format!("/repos/{}/pulls/{number}", coordinates.slug());

        Ok(self.get(&path)?.and_then(|value| {
            value
                .get("user")
                .and_then(|user| user.get("login"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }))
    }

    /// Resolves the authors of several pull requests at once. Falls back to
    /// individual REST calls if the batched GraphQL query is unavailable.
    pub fn pull_authors(
        &self,
        coordinates: &RepoCoordinates,
        numbers: &[u64],
    ) -> BTreeMap<u64, Option<String>> {
        let mut authors: BTreeMap<u64, Option<String>> = BTreeMap::new();

        if numbers.is_empty() {
            return authors;
        }

        if let Ok(found) = self.pull_authors_graphql(coordinates, numbers) {
            authors = found;
        }

        for number in numbers {
            if authors.contains_key(number) {
                continue;
            }

            let author = self.pull_author(coordinates, *number).unwrap_or(None);
            authors.insert(*number, author);
        }

        authors
    }

    fn pull_authors_graphql(
        &self,
        coordinates: &RepoCoordinates,
        numbers: &[u64],
    ) -> Result<BTreeMap<u64, Option<String>>> {
        let mut fields = String::new();

        for (index, number) in numbers.iter().enumerate() {
            fields.push_str(&format!(
                "p{index}: pullRequest(number: {number}) {{ author {{ login }} }}\n"
            ));
        }

        let query = format!(
            "query {{ repository(owner: \"{}\", name: \"{}\") {{ {fields} }} }}",
            escape_graphql(&coordinates.owner),
            escape_graphql(&coordinates.repo)
        );

        let value = self
            .request("POST", GRAPHQL, Some(json!({ "query": query })))?
            .ok_or_else(|| Error::other("GitHub did not return a GraphQL response."))?;

        if value.get("errors").is_some() {
            return Err(Error::other(
                "GitHub rejected the batched pull request query.",
            ));
        }

        let repository = value
            .get("data")
            .and_then(|data| data.get("repository"))
            .ok_or_else(|| Error::other("GitHub returned no repository data."))?;
        let mut authors = BTreeMap::new();

        for (index, number) in numbers.iter().enumerate() {
            let alias = format!("p{index}");
            let Some(pull) = repository.get(alias.as_str()) else {
                continue;
            };

            if pull.is_null() {
                authors.insert(*number, None);
                continue;
            }

            let login = pull
                .get("author")
                .and_then(|author| author.get("login"))
                .and_then(Value::as_str)
                .map(str::to_string);

            authors.insert(*number, login);
        }

        Ok(authors)
    }

    /// Best-effort mapping of a commit email address to a GitHub login, used for
    /// the release author exclusion in the credits section.
    pub fn username_for_email(&self, email: &str) -> Option<String> {
        if email.trim().is_empty() {
            return None;
        }

        let encoded: String = url::form_urlencoded::byte_serialize(email.as_bytes()).collect();

        let commit_path = format!("/search/commits?q=author-email:{encoded}");
        if let Ok(Some(value)) = self.get(&commit_path) {
            if let Some(login) = value
                .get("items")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("author"))
                .and_then(|author| author.get("login"))
                .and_then(Value::as_str)
            {
                return Some(login.to_string());
            }
        }

        let user_path = format!("/search/users?q={encoded}+in:email");
        self.get(&user_path).ok().flatten().and_then(|value| {
            value
                .get("items")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("login"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    }
}

fn escape_graphql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn api_message(body: &str) -> String {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return body.trim().to_string(),
    };

    parsed
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Parses `remote.origin.url` into owner and repository.
pub fn parse_remote(remote: &str) -> Result<RepoCoordinates> {
    let remote = remote.trim();

    if remote.is_empty() {
        return Err(Error::other("The remote URL is empty."));
    }

    // scp-like syntax: git@github.com:owner/repo.git
    if !remote.contains("://") {
        if let Some((host, path)) = remote.split_once(':') {
            if !host.contains('@') && !host.contains("github.com") {
                return Err(Error::other("Remote origin is not hosted on GitHub."));
            }

            if !host.ends_with("github.com") {
                return Err(Error::other("Remote origin is not hosted on GitHub."));
            }

            return split_path(path);
        }

        return Err(Error::other("Could not parse the remote origin URL."));
    }

    let url = Url::parse(remote)
        .map_err(|error| Error::other(format!("Could not parse the remote origin URL: {error}")))?;

    if url.host_str() != Some("github.com") {
        return Err(Error::other("Remote origin is not hosted on GitHub."));
    }

    split_path(url.path())
}

fn split_path(path: &str) -> Result<RepoCoordinates> {
    let cleaned = path.trim_start_matches('/');
    let mut parts = cleaned.split('/');
    let owner = parts.next().unwrap_or("").to_string();
    let repo = parts
        .next()
        .unwrap_or("")
        .trim_end_matches(".git")
        .to_string();

    if owner.is_empty() || repo.is_empty() {
        return Err(Error::other(
            "Could not determine the GitHub repository from the remote URL.",
        ));
    }

    Ok(RepoCoordinates { owner, repo })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_remotes() {
        let coordinates = parse_remote("git@github.com:404mat/release.git").unwrap();
        assert_eq!(coordinates.owner, "404mat");
        assert_eq!(coordinates.repo, "release");
    }

    #[test]
    fn parses_https_remotes() {
        let coordinates = parse_remote("https://github.com/404mat/release.git").unwrap();
        assert_eq!(coordinates.owner, "404mat");
        assert_eq!(coordinates.repo, "release");

        let coordinates = parse_remote("https://github.com/404mat/release").unwrap();
        assert_eq!(coordinates.repo, "release");
    }

    #[test]
    fn parses_ssh_url_syntax() {
        let coordinates = parse_remote("ssh://git@github.com/404mat/release.git").unwrap();
        assert_eq!(coordinates.owner, "404mat");
    }

    #[test]
    fn rejects_other_hosts() {
        assert!(parse_remote("https://gitlab.com/owner/repo.git").is_err());
        assert!(parse_remote("git@gitlab.com:owner/repo.git").is_err());
    }

    #[test]
    fn extracts_api_messages() {
        assert_eq!(
            api_message(r#"{"message":"Bad credentials"}"#),
            "Bad credentials"
        );
        assert_eq!(api_message("plain text"), "plain text");
    }
}

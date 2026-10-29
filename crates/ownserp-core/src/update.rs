use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

const OWNER: &str = "devops-infinity";
const REPO: &str = "ownserp";
const LATEST_URL: &str = "https://api.github.com/repos/{owner}/{repo}/releases/latest";
const TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub command: Option<&'static str>,
}

impl UpdateStatus {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "current": self.current,
            "latest": self.latest,
            "updateAvailable": self.update_available,
            "command": self.command,
        })
    }

    pub fn summary(&self) -> String {
        match (&self.latest, self.update_available) {
            (Some(latest), true) => match self.command {
                Some(command) => format!(
                    "ownserp {} is behind {}, upgrade with: {}",
                    self.current, latest, command
                ),
                None => format!(
                    "ownserp {} is behind {}, download it from https://github.com/{OWNER}/{REPO}/releases",
                    self.current, latest
                ),
            },
            (Some(latest), false) => {
                format!("ownserp {} is the latest release ({latest})", self.current)
            }
            (None, _) => format!(
                "ownserp {}; the latest release could not be read",
                self.current
            ),
        }
    }
}

pub async fn check_for_update() -> Result<UpdateStatus> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(5))
        .user_agent(format!("ownserp/{}", crate::VERSION))
        .tls_backend_preconfigured(crate::client::tls_client_config()?)
        .build()
        .map_err(|error| Error::network("building the update client", error))?;
    let url = LATEST_URL.replace("{owner}", OWNER).replace("{repo}", REPO);
    let response = client
        .get(&url)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| Error::network("asking GitHub for the latest release", error))?;
    let status = response.status().as_u16();
    if status == 403 || status == 429 {
        return Err(Error::Refused {
            code: "serper.update_rate_limited",
            message: "GitHub declined the release lookup, usually a rate limit".to_owned(),
            remedy: "Wait a few minutes and run the command again.",
        });
    }
    if !(200..300).contains(&status) {
        return Err(Error::Upstream {
            status,
            message: "the GitHub release lookup failed".to_owned(),
        });
    }
    let body: Value = response
        .json()
        .await
        .map_err(|error| Error::network("reading the GitHub release", error))?;
    let tag = body
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(describe(tag.as_deref(), crate::VERSION, &current_exe()))
}

fn describe(tag: Option<&str>, current: &str, executable: &Path) -> UpdateStatus {
    let latest = tag.map(normalize_version).filter(|tag| !tag.is_empty());
    let update_available = latest
        .as_deref()
        .is_some_and(|latest| is_newer(latest, current));
    UpdateStatus {
        current: current.to_owned(),
        latest,
        update_available,
        command: upgrade_command(executable),
    }
}

fn normalize_version(tag: &str) -> String {
    let tag = tag.trim();
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    tag.split(['+', '-']).next().unwrap_or(tag).to_owned()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => candidate != current,
    }
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn upgrade_command(executable: &Path) -> Option<&'static str> {
    upgrade_command_for(executable, in_container())
}

fn upgrade_command_for(executable: &Path, container: bool) -> Option<&'static str> {
    if container {
        return Some(
            "docker pull ghcr.io/devops-infinity/ownserp:latest, then restart the container",
        );
    }
    let path = executable.to_string_lossy();
    if path.contains("/Cellar/") || path.contains("homebrew") || path.contains("linuxbrew") {
        return Some("brew upgrade ownserp");
    }
    if path.contains("/.cargo/bin/") {
        return Some("cargo install --locked ownserp");
    }
    if path.contains("/node_modules/") || path.contains("/_npx/") || path.contains("/.npm/") {
        return Some("npm install -g @devops-infinity/ownserp");
    }
    None
}

fn in_container() -> bool {
    std::env::var("OWN_SERPER_IN_CONTAINER").as_deref() == Ok("1")
        || Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
}

fn current_exe() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_read_without_its_prefix_or_suffix() {
        assert_eq!(normalize_version("v2.1.0"), "2.1.0");
        assert_eq!(normalize_version("2.0.1"), "2.0.1");
        assert_eq!(normalize_version("v2.0.1-rc.1"), "2.0.1");
        assert_eq!(normalize_version(" 2.0.1 "), "2.0.1");
    }

    #[test]
    fn a_higher_version_is_newer_and_an_equal_one_is_not() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("2.1.0", "2.0.9"));
        assert!(is_newer("10.0.0", "9.9.9"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("1.9.9", "2.0.0"));
    }

    #[test]
    fn an_unreadable_tag_compares_by_text() {
        assert!(is_newer("nightly", "2.0.0"));
        assert!(!is_newer("nightly", "nightly"));
    }

    #[test]
    fn the_channel_decides_the_upgrade_command() {
        assert_eq!(
            upgrade_command_for(
                Path::new("/opt/homebrew/Cellar/ownserp/2.0.0/bin/ownserp"),
                false
            ),
            Some("brew upgrade ownserp")
        );
        assert_eq!(
            upgrade_command_for(Path::new("/home/someone/.cargo/bin/ownserp"), false),
            Some("cargo install --locked ownserp")
        );
        assert_eq!(
            upgrade_command_for(Path::new("/usr/lib/node_modules/.bin/ownserp"), false),
            Some("npm install -g @devops-infinity/ownserp")
        );
        assert_eq!(
            upgrade_command_for(Path::new("/usr/local/bin/ownserp"), false),
            None
        );
        assert_eq!(
            upgrade_command_for(Path::new("/usr/local/bin/ownserp"), true),
            Some("docker pull ghcr.io/devops-infinity/ownserp:latest, then restart the container")
        );
    }

    #[test]
    fn an_up_to_date_status_says_so() {
        let status = describe(Some("v2.0.0"), "2.0.0", Path::new("/usr/local/bin/ownserp"));
        assert!(!status.update_available);
        assert!(status.summary().contains("is the latest release"));
    }

    #[test]
    fn a_newer_release_names_the_command() {
        let status = describe(Some("v2.0.1"), "2.0.0", Path::new("/usr/local/bin/ownserp"));
        assert!(status.update_available);
        assert!(status.summary().contains("releases"));
    }
}

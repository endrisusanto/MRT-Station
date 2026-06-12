use std::{collections::BTreeSet, path::PathBuf, time::Duration};

use anyhow::{Context, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Production,
    Simulator,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub endpoint: PathBuf,
    pub mode: AgentMode,
    pub allowed_uids: BTreeSet<u32>,
    pub backend_url: Option<String>,
    pub backend_timeout: Duration,
    pub backend_allow_http: bool,
}

impl AgentConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let endpoint = std::env::var_os("EM_AGENT_ENDPOINT")
            .map(PathBuf::from)
            .unwrap_or_else(default_endpoint);
        let mode = match std::env::var("EM_AGENT_MODE") {
            Ok(value) if value.eq_ignore_ascii_case("production") => AgentMode::Production,
            Ok(value) if value.eq_ignore_ascii_case("simulator") => AgentMode::Simulator,
            Ok(value) => bail!("unsupported EM_AGENT_MODE: {value}"),
            Err(_) if cfg!(debug_assertions) => AgentMode::Simulator,
            Err(_) => AgentMode::Production,
        };
        let allowed_uids = parse_allowed_uids(
            std::env::var("EM_AGENT_ALLOWED_UIDS")
                .unwrap_or_default()
                .as_str(),
        )?;
        let backend_url = std::env::var("EM_BACKEND_URL").ok();
        let backend_timeout = Duration::from_secs(
            std::env::var("EM_BACKEND_TIMEOUT_SECONDS")
                .unwrap_or_else(|_| "15".into())
                .parse()
                .context("EM_BACKEND_TIMEOUT_SECONDS must be an integer")?,
        );
        if backend_timeout.is_zero() || backend_timeout > Duration::from_secs(120) {
            bail!("EM_BACKEND_TIMEOUT_SECONDS must be between 1 and 120");
        }
        let backend_allow_http = parse_bool(
            "EM_BACKEND_ALLOW_HTTP",
            std::env::var("EM_BACKEND_ALLOW_HTTP")
                .as_deref()
                .unwrap_or("false"),
        )?;

        Ok(Self {
            endpoint,
            mode,
            allowed_uids,
            backend_url,
            backend_timeout,
            backend_allow_http,
        })
    }
}

fn parse_bool(name: &str, value: &str) -> anyhow::Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => bail!("{name} must be true, false, 1, or 0"),
    }
}

fn parse_allowed_uids(value: &str) -> anyhow::Result<BTreeSet<u32>> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.parse::<u32>()
                .with_context(|| format!("invalid UID in EM_AGENT_ALLOWED_UIDS: {item}"))
        })
        .collect()
}

#[cfg(unix)]
fn default_endpoint() -> PathBuf {
    if cfg!(debug_assertions) {
        "/tmp/em-station/agent.sock".into()
    } else {
        "/run/em-station/agent.sock".into()
    }
}

#[cfg(windows)]
fn default_endpoint() -> PathBuf {
    r"\\.\pipe\em-station-agent".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uid_allowlist() {
        assert_eq!(
            parse_allowed_uids("1000, 1001,1000").unwrap(),
            BTreeSet::from([1000, 1001])
        );
    }

    #[test]
    fn rejects_invalid_uid() {
        assert!(parse_allowed_uids("1000,operator").is_err());
    }

    #[test]
    fn parses_strict_boolean() {
        assert!(parse_bool("TEST", "true").unwrap());
        assert!(!parse_bool("TEST", "0").unwrap());
        assert!(parse_bool("TEST", "yes").is_err());
    }
}

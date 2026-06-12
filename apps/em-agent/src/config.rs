use std::{collections::BTreeSet, path::PathBuf};

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

        Ok(Self {
            endpoint,
            mode,
            allowed_uids,
        })
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
}

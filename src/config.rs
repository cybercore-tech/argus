//! Reads SigilWard's own config file — `~/.config/sigilward/config.toml` —
//! so Argus watches exactly what SigilWard baselines, with one set of
//! watched paths for the user to maintain, not two. This is a small
//! independent copy of `sigilward/src/config.rs`'s shape rather than a
//! shared library crate: SigilWard already ships and runs daily via
//! `sigilward-check.timer`, and refactoring it into a lib+bin split to
//! share ~30 lines of code isn't worth risking that.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone)]
pub struct WatchEntry {
    pub path: String,
    #[serde(default = "default_recursive")]
    pub recursive: bool,
}

fn default_recursive() -> bool {
    true
}

fn default_baseline_path() -> String {
    "~/.local/state/sigilward/baseline.json".to_string()
}

#[derive(Deserialize, Debug, Clone)]
pub struct AppConfig {
    #[serde(default = "default_baseline_path")]
    pub baseline_path: String,
    pub watch: Vec<WatchEntry>,
}

/// Same lookup order as SigilWard's own `default_config_path` — Argus reads
/// the identical file, so it must resolve it the identical way.
pub fn default_config_path() -> Option<PathBuf> {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok()?;
    let candidates = [
        config_home.join("sigilward").join("config.toml"),
        PathBuf::from("config.toml"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

pub fn load(path: &std::path::Path) -> anyhow::Result<AppConfig> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {}", path.display(), e))?;
    toml::from_str(&raw).map_err(|e| anyhow::anyhow!("failed to parse {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sigilwards_real_config_shape() {
        let raw = r#"
baseline_path = "~/.local/state/sigilward/baseline.json"

[[watch]]
path = "/etc/systemd/system"
recursive = true

[[watch]]
path = "/etc/ssh/sshd_config"
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.watch.len(), 2);
        assert!(cfg.watch[0].recursive);
        // recursive defaults true when omitted, matching SigilWard's own default
        assert!(cfg.watch[1].recursive);
    }

    #[test]
    fn expand_home_resolves_tilde() {
        let expanded = expand_home("~/.local/state/sigilward/baseline.json");
        assert!(expanded.is_absolute());
        assert!(!expanded.starts_with("~"));
    }

    #[test]
    fn expand_home_leaves_absolute_paths_alone() {
        assert_eq!(expand_home("/etc/sudoers"), PathBuf::from("/etc/sudoers"));
    }
}

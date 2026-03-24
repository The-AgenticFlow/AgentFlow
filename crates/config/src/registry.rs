// crates/config/src/registry.rs
//
// Reads .agent/registry.json — single source of truth for team membership.
// NEXUS reloads this on every poll cycle for zero-downtime team changes.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single agent entry from registry.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryEntry {
    pub id:        String,
    pub cli:       String,    // "claude" | "gemini" | "codex"
    pub active:    bool,
    pub instances: u32,       // registry.json is sole source — .agent.md has no instances field
}

/// The full registry — a thin wrapper around the team list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub team: Vec<RegistryEntry>,
}

impl Registry {
    /// Load from a path (typically `.agent/registry.json`).
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read registry at {}", path.display()))?;
        let registry: Registry = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse registry at {}", path.display()))?;
        Ok(registry)
    }

    /// Active agents only.
    pub fn active_agents(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.team.iter().filter(|e| e.active)
    }

    /// Look up a specific agent by id. Returns None if not found or inactive.
    pub fn get(&self, id: &str) -> Option<&RegistryEntry> {
        self.team.iter().find(|e| e.id == id && e.active)
    }

    /// Total active instance count across all agents.
    pub fn total_instances(&self) -> u32 {
        self.active_agents().map(|e| e.instances).sum()
    }

    /// FORGE worker slot names: ["forge-1", "forge-2", ...]
    pub fn forge_slots(&self) -> Vec<String> {
        match self.get("forge") {
            None => vec![],
            Some(entry) => (1..=entry.instances)
                .map(|i| format!("forge-{}", i))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn sample_registry_json() -> &'static str {
        r#"{
          "team": [
            { "id": "nexus",    "cli": "claude", "active": true,  "instances": 1 },
            { "id": "forge",    "cli": "claude", "active": true,  "instances": 2 },
            { "id": "sentinel", "cli": "claude", "active": true,  "instances": 1 },
            { "id": "vessel",   "cli": "claude", "active": true,  "instances": 1 },
            { "id": "lore",     "cli": "claude", "active": false, "instances": 1 }
          ]
        }"#
    }

    fn write_temp(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn test_load_registry() {
        let f = write_temp(sample_registry_json());
        let reg = Registry::load(f.path()).unwrap();
        assert_eq!(reg.team.len(), 5);
    }

    #[test]
    fn test_active_agents_excludes_inactive() {
        let f = write_temp(sample_registry_json());
        let reg = Registry::load(f.path()).unwrap();
        let active: Vec<_> = reg.active_agents().collect();
        assert_eq!(active.len(), 4); // lore is inactive
        assert!(active.iter().all(|e| e.active));
    }

    #[test]
    fn test_forge_slots() {
        let f = write_temp(sample_registry_json());
        let reg = Registry::load(f.path()).unwrap();
        assert_eq!(reg.forge_slots(), vec!["forge-1", "forge-2"]);
    }

    #[test]
    fn test_get_inactive_returns_none() {
        let f = write_temp(sample_registry_json());
        let reg = Registry::load(f.path()).unwrap();
        assert!(reg.get("lore").is_none());
    }

    #[test]
    fn test_get_active_returns_some() {
        let f = write_temp(sample_registry_json());
        let reg = Registry::load(f.path()).unwrap();
        let nexus = reg.get("nexus").unwrap();
        assert_eq!(nexus.instances, 1);
    }
}

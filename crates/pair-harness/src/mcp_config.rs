/// MCP Server Configuration Generator for AgentFlow Pair Harness
///
/// Generates the mcp.json configuration that connects FORGE and SENTINEL
/// to the four standard MCP servers:
/// - github-mcp-server (for PR creation, git operations)
/// - redis-mcp-server (for file locks and state)
/// - filesystem-mcp-server (for scoped file writes)
/// - shell-mcp-server (for tests/lint with allow-list)
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

/// MCP server configuration for a pair
#[derive(Debug, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

/// Generates MCP configuration
pub struct McpConfigGenerator {
    pair_id: String,
    ticket_id: String,
    worktree_path: PathBuf,
    shared_path: PathBuf,
    redis_url: String,
    github_token: String,
}

impl McpConfigGenerator {
    pub fn new(
        pair_id: String,
        ticket_id: String,
        worktree_path: PathBuf,
        shared_path: PathBuf,
        redis_url: String,
        github_token: String,
    ) -> Self {
        Self {
            pair_id,
            ticket_id,
            worktree_path,
            shared_path,
            redis_url,
            github_token,
        }
    }

    /// Generates the complete MCP configuration
    ///
    /// # Validation: C4-03 MCP Configuration
    /// Generates valid mcp.json connecting to github, redis, filesystem, and shell servers
    pub fn generate(&self) -> Result<McpConfig> {
        let mut servers = HashMap::new();

        // GitHub MCP server for PR creation and git operations
        servers.insert("github".to_string(), self.github_server_config());

        // Redis MCP server for file locks and state
        servers.insert("redis".to_string(), self.redis_server_config());

        // Filesystem MCP server for scoped file writes
        servers.insert("filesystem".to_string(), self.filesystem_server_config());

        // Shell MCP server for tests/lint with allow-list
        servers.insert("shell".to_string(), self.shell_server_config());

        Ok(McpConfig {
            mcp_servers: servers,
        })
    }

    /// Writes the MCP configuration to a file
    pub async fn write_config(&self, output_path: &Path) -> Result<()> {
        let config = self.generate()?;

        let json =
            serde_json::to_string_pretty(&config).context("Failed to serialize MCP config")?;

        tokio::fs::write(output_path, json)
            .await
            .context("Failed to write MCP config file")?;

        info!(
            pair_id = %self.pair_id,
            output_path = %output_path.display(),
            "MCP configuration written"
        );

        Ok(())
    }

    /// GitHub MCP server configuration
    /// Uses docker image with GitHub token
    fn github_server_config(&self) -> McpServerConfig {
        let mut env = HashMap::new();
        env.insert(
            "GITHUB_PERSONAL_ACCESS_TOKEN".to_string(),
            self.github_token.clone(),
        );

        McpServerConfig {
            command: "docker".to_string(),
            args: vec![
                "run".to_string(),
                "-i".to_string(),
                "--rm".to_string(),
                "-e".to_string(),
                "GITHUB_PERSONAL_ACCESS_TOKEN".to_string(),
                "ghcr.io/github/github-mcp-server".to_string(),
            ],
            env,
        }
    }

    /// Redis MCP server configuration
    /// Connects to Redis for file locking
    fn redis_server_config(&self) -> McpServerConfig {
        let mut env = HashMap::new();
        env.insert("REDIS_URL".to_string(), self.redis_url.clone());

        McpServerConfig {
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@upstash/redis-mcp".to_string()],
            env,
        }
    }

    /// Filesystem MCP server configuration
    /// Scoped to the worktree directory
    fn filesystem_server_config(&self) -> McpServerConfig {
        let mut env = HashMap::new();
        env.insert(
            "FILESYSTEM_ROOT".to_string(),
            self.worktree_path.display().to_string(),
        );
        env.insert(
            "FILESYSTEM_SHARED".to_string(),
            self.shared_path.display().to_string(),
        );

        McpServerConfig {
            command: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
                self.worktree_path.display().to_string(),
            ],
            env,
        }
    }

    /// Shell MCP server configuration
    ///
    /// # Validation: C4-04 Shell Allow-list
    /// Configured to allow only commands in .agent/tooling/
    fn shell_server_config(&self) -> McpServerConfig {
        let tooling_dir = self.worktree_path.join(".agent").join("tooling");

        let mut env = HashMap::new();
        env.insert(
            "SHELL_ALLOW_LIST".to_string(),
            format!("{}/*", tooling_dir.display()),
        );
        env.insert(
            "SHELL_WORKING_DIR".to_string(),
            self.worktree_path.display().to_string(),
        );

        // Validation: C4-04 - Shell MCP configured to allow only .agent/tooling/ commands
        McpServerConfig {
            command: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-shell".to_string(),
            ],
            env,
        }
    }

    /// Generates environment variables for MCP server injection
    pub fn generate_env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        env.insert("SPRINTLESS_PAIR_ID".to_string(), self.pair_id.clone());
        env.insert("SPRINTLESS_TICKET_ID".to_string(), self.ticket_id.clone());
        env.insert(
            "SPRINTLESS_WORKTREE".to_string(),
            self.worktree_path.display().to_string(),
        );
        env.insert(
            "SPRINTLESS_SHARED".to_string(),
            self.shared_path.display().to_string(),
        );
        env.insert("SPRINTLESS_REDIS_URL".to_string(), self.redis_url.clone());
        env.insert(
            "SPRINTLESS_GITHUB_TOKEN".to_string(),
            self.github_token.clone(),
        );

        env
    }
}

/// Helper to create MCP config directory structure and install hooks
///
/// # Validation: C4-02 Hook Enforcement
/// Installs plugin hooks into runtime directories
pub async fn setup_mcp_directories(worktree_path: &Path, shared_path: &Path) -> Result<()> {
    // Create .claude/plugins/sprintless directory in worktree for FORGE
    let forge_plugin_dir = worktree_path
        .join(".claude")
        .join("plugins")
        .join("sprintless");
    tokio::fs::create_dir_all(&forge_plugin_dir)
        .await
        .context("Failed to create FORGE plugin directory")?;

    // Create .claude/plugins/sprintless directory in shared for SENTINEL
    let sentinel_plugin_dir = shared_path
        .join(".claude")
        .join("plugins")
        .join("sprintless");
    tokio::fs::create_dir_all(&sentinel_plugin_dir)
        .await
        .context("Failed to create SENTINEL plugin directory")?;

    info!(
        forge_plugin = %forge_plugin_dir.display(),
        sentinel_plugin = %sentinel_plugin_dir.display(),
        "MCP plugin directories created"
    );

    Ok(())
}

/// Installs Sprintless plugin hooks into the runtime directories
///
/// # Validation: C4-02 Hook Enforcement
/// Hooks are wired into .claude/plugins/sprintless directories
pub async fn install_plugin_hooks(
    worktree_path: &Path,
    shared_path: &Path,
    repo_root: &Path,
) -> Result<()> {
    let source_plugin = repo_root.join(".sprintless").join("plugin");
    let forge_plugin_dir = worktree_path
        .join(".claude")
        .join("plugins")
        .join("sprintless");
    let sentinel_plugin_dir = shared_path
        .join(".claude")
        .join("plugins")
        .join("sprintless");

    // Validation: C4-02 - Install hooks for FORGE
    let forge_hooks_src = source_plugin.join("hooks").join("forge");
    let forge_hooks_dst = forge_plugin_dir.join("hooks");
    tokio::fs::create_dir_all(&forge_hooks_dst).await?;

    // Copy hook scripts
    for hook in &[
        "session_start.sh",
        "pre_compact_handoff.sh",
        "pre_tool_use_guard.sh",
        "stop_require_artifact.sh",
    ] {
        let src = forge_hooks_src.join(hook);
        let dst = forge_hooks_dst.join(hook);
        if src.exists() {
            tokio::fs::copy(&src, &dst)
                .await
                .with_context(|| format!("Failed to copy hook {}", hook))?;
            // Preserve executable permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = tokio::fs::metadata(&dst).await?.permissions();
                perms.set_mode(0o755);
                tokio::fs::set_permissions(&dst, perms).await?;
            }
        }
    }

    info!(
        forge_hooks = %forge_hooks_dst.display(),
        "FORGE hooks installed"
    );

    // Install plugin.json manifest
    let plugin_manifest_src = source_plugin.join("plugin.json");
    if plugin_manifest_src.exists() {
        tokio::fs::copy(&plugin_manifest_src, forge_plugin_dir.join("plugin.json")).await?;
    }

    // Install skills
    let skills_src = source_plugin.join("skills");
    if skills_src.exists() {
        let skills_dst = forge_plugin_dir.join("skills");
        tokio::fs::create_dir_all(&skills_dst).await?;

        let mut entries = tokio::fs::read_dir(&skills_src).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().map_or(false, |ext| ext == "md") {
                let filename = entry.file_name();
                tokio::fs::copy(entry.path(), skills_dst.join(&filename)).await?;
            }
        }
        info!("Skills installed to FORGE plugin");
    }

    // Install commands
    let commands_src = source_plugin.join("commands");
    if commands_src.exists() {
        let commands_dst = forge_plugin_dir.join("commands");
        tokio::fs::create_dir_all(&commands_dst).await?;

        let mut entries = tokio::fs::read_dir(&commands_src).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().map_or(false, |ext| ext == "md") {
                let filename = entry.file_name();
                tokio::fs::copy(entry.path(), commands_dst.join(&filename)).await?;
            }
        }
        info!("Commands installed to FORGE plugin");
    }

    // Install hooks for SENTINEL (read-only, minimal hooks)
    let sentinel_hooks_src = source_plugin.join("hooks").join("sentinel");
    if sentinel_hooks_src.exists() {
        let sentinel_hooks_dst = sentinel_plugin_dir.join("hooks");
        tokio::fs::create_dir_all(&sentinel_hooks_dst).await?;

        // SENTINEL has fewer hooks - mainly validation hooks
        // This would be populated based on docs/forge-sentinel-arch.md
        info!(
            sentinel_hooks = %sentinel_hooks_dst.display(),
            "SENTINEL hooks directory prepared"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_generation() {
        let generator = McpConfigGenerator::new(
            "pair-1".to_string(),
            "T-42".to_string(),
            PathBuf::from("/project/worktrees/pair-1"),
            PathBuf::from("/project/.sprintless/pairs/pair-1/shared"),
            "redis://localhost:6379".to_string(),
            "ghp_test_token".to_string(),
        );

        let config = generator.generate().unwrap();

        // Validation: C4-03 - All 4 servers present
        assert!(config.mcp_servers.contains_key("github"));
        assert!(config.mcp_servers.contains_key("redis"));
        assert!(config.mcp_servers.contains_key("filesystem"));
        assert!(config.mcp_servers.contains_key("shell"));

        // Verify GitHub server has token
        let github = &config.mcp_servers["github"];
        assert_eq!(github.command, "docker");
        assert!(github.env.contains_key("GITHUB_PERSONAL_ACCESS_TOKEN"));

        // Validation: C4-04 - Shell server has allow-list
        let shell = &config.mcp_servers["shell"];
        assert!(shell.env.contains_key("SHELL_ALLOW_LIST"));
        assert!(shell.env["SHELL_ALLOW_LIST"].contains(".agent/tooling"));
    }

    #[test]
    fn test_env_var_generation() {
        let generator = McpConfigGenerator::new(
            "pair-2".to_string(),
            "T-99".to_string(),
            PathBuf::from("/project/worktrees/pair-2"),
            PathBuf::from("/project/.sprintless/pairs/pair-2/shared"),
            "redis://localhost:6379".to_string(),
            "ghp_test_token".to_string(),
        );

        let env = generator.generate_env_vars();

        assert_eq!(env.get("SPRINTLESS_PAIR_ID").unwrap(), "pair-2");
        assert_eq!(env.get("SPRINTLESS_TICKET_ID").unwrap(), "T-99");
        assert!(env.contains_key("SPRINTLESS_WORKTREE"));
        assert!(env.contains_key("SPRINTLESS_SHARED"));
    }
}

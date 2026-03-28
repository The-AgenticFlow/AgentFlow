pub mod artifacts;
pub mod isolation;
pub mod mcp_config;
pub mod memory_locks;
pub mod pair;
pub mod process;
pub mod watcher;
/// AgentFlow Pair Harness
///
/// Core execution engine for managing isolated FORGE-SENTINEL pairs.
///
/// # Architecture
/// - Each pair runs in its own Git worktree on a dedicated branch
/// - File locking prevents concurrent writes via in-memory locks (or Redis in production)
/// - Event-driven monitoring (no polling) triggers SENTINEL evaluation
/// - FORGE is long-running, SENTINEL is ephemeral
///
/// # Key Modules
/// - [`worktree`] - Git worktree isolation and management
/// - [`isolation`] - Redis-based file locking (production)
/// - [`memory_locks`] - In-memory file locking (development/testing)
/// - [`watcher`] - Event-driven file system monitoring
/// - [`process`] - Process spawning for FORGE and SENTINEL
/// - [`mcp_config`] - MCP server configuration generation
pub mod worktree;

pub use artifacts::{Status, StatusJson, Ticket};
pub use isolation::{IsolationManager, LockError};
pub use mcp_config::{McpConfig, McpConfigGenerator};
pub use memory_locks::MemoryLockManager;
pub use pair::{ForgeSentinelPair, PairConfig, PairOutcome};
pub use process::{AgentProcess, AgentType, ProcessManager};
pub use watcher::{PairWatcher, WatchEvent};
pub use worktree::WorktreeManager;

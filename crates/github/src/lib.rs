// crates/github/src/lib.rs
//
// github crate — Tool schema library and MCP server spawn helpers.
// Agents do not call GitHub directly from Rust; the McpSession in
// agent-client handles all communication.

pub mod schemas;

pub use schemas::github_mcp_cmd;

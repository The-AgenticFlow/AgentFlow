---
name: mcp-builder
description: Comprehensive guide for developing and extending Model Context Protocol (MCP) servers.
---

# FORGE MCP Builder Skill

Use this skill when you need to build or extend MCP servers to provide new capabilities to the AgentFlow system.

## High-Level Workflow

### Phase 1: Research and Planning

- Understand the external system or data source you are integrating.
- Define the necessary tools and resources.
- Choose the appropriate language/SDK (Rust, Python, Node.js).

### Phase 2: Implementation

- Implement the core protocol (Tools, Resources, Prompts).
- Follow the official [MCP Specification](https://modelcontextprotocol.io/).
- Ensure robust error handling and type safety.

### Phase 3: Review and Test

- Verify transport layers (Stdio, HTTP/SSE).
- Test tool execution and resource retrieval.
- Check for protocol compliance.

### Phase 4: Integration

- Create evaluations and documentation.
- Add components to the `.claude-plugin/plugin.json` or `.mcp.json`.

## Documentation Reference

Consult the official SDK documentation for the language you choose. Prioritize security and minimal permission sets when exposing system capabilities via tools.

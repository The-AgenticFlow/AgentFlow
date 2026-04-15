---
name: mcp-builder
description: Skilled management and extension of the deployment automation MCP environment.
---

# VESSEL Infrastructure MCP Skill

Use this skill to maintain and extend the MCP servers used for deployment automation and monitoring.

## Focus Areas

- **Transport Reliability**: Ensure Stdio/SSE connections to deployment tools are stable.
- **Tool Mapping**: Correctly map CLI tools (Docker, K8s, Terraform) to MCP tools.
- **Environment Safety**: Ensure MCP tools have minimal necessary permissions for the target environment.
- **Monitoring Tools**: Build tools for retrieving logs and metrics into the AgentFlow context.

## Workflow

1. Identify missing infrastructure visibility.
2. Direct Forge or self-implement a new MCP tool/resource.
3. Verify tool safety and reliability in a staging environment.

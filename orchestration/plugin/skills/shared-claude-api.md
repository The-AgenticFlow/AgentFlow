---
name: claude-api
description: Technical reference for Claude-specific features and best practices.
---

# Shared Claude API Skill

Use this reference to optimize interactions with the Claude LLM, utilizing advanced features for improved performance and cost-efficiency.

## Core Features

- **Thinking & Effort**: Adjust `thinking` parameters for complex reasoning tasks.
- **Prompt Caching**: Structure prompts to take advantage of caching for repetitive context.
- **Tool Use**: Follow best practices for tool definition and selection.
- **Streaming**: Handle streaming responses efficiently in the CLI/Frontend.

## Best Practices

- **System Prompting**: Provide clear, concise role definitions.
- **Context Management**: Be mindful of the context window; use summarization or trimming when necessary.
- **Output Formatting**: Request specific formats (JSON, Markdown) explicitly for deterministic processing.

## Managed Agents (Beta)

When coordinating with other agents, follow the established orchestration protocol defined in the `AgentFlow` system instructions.

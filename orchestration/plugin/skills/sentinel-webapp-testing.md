---
name: webapp-testing
description: Toolkit for interacting with and testing local web applications using Playwright.
---

# Web Application Testing Skill

Use this skill to verify frontend functionality, debug UI behavior, and capture browser state for evaluation or deployment verification.

## Workflow

1. **Reconnaissance**:
   - Navigate to the local server URL.
   - Wait for `networkidle` state to ensure JS execution is complete.
   - Inspect the rendered DOM or take a full-page screenshot.

2. **Identification**:
   - Locate specific elements using descriptive selectors (`text=`, `role=`, CSS, or IDs).
   - Use `page.locator()` and `all()` to discover interactive elements.

3. **Execution**:
   - Write Playwright scripts to perform actions (click, type, navigate).
   - Verify assertions and capture logs for debugging.

## Helper Pattern

When working with local servers, ensure the server is running correctly before beginning reconnaissance. If managing server lifecycles, wait for port availability.

## Best Practices

- **Synchronous Logic**: Use `playwright.sync_api` for cleaner script integration where applicable.
- **Headless Mode**: Always launch browsers in headless mode for server-side execution.
- **Waits**: Use `wait_for_selector` or specific network idle states instead of arbitrary timeouts.
- **Artifacts**: Capture screenshots and console logs to provide proof of verification.

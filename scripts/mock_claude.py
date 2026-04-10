#!/usr/bin/env python3
"""
Mock claude CLI for testing AgentFlow plugins.

Simulates plugin behavior:
- SessionStart hooks
- PreToolUse hooks (bash guards)
- PostToolUse hooks (validation)
- Stop hooks (artifact requirements)

Usage:
  mock_claude.py --plugin-dir ./plugins/forge [prompt]
"""
import sys
import os
import json
import time
import argparse

def run_hook(plugin_dir, hook_type, **env_vars):
    """Simulate running a hook script."""
    hooks_dir = os.path.join(plugin_dir, "hooks")
    hooks_file = os.path.join(hooks_dir, "hooks.json")
    
    if not os.path.exists(hooks_file):
        return 0  # No hooks, pass
    
    with open(hooks_file) as f:
        hooks_config = json.load(f)
    
    hooks = hooks_config.get("hooks", {}).get(hook_type, [])
    if not hooks:
        return 0
    
    for hook_group in hooks:
        for hook in hook_group.get("hooks", []):
            if hook.get("type") == "command":
                script = hook.get("command", "").replace("${CLAUDE_PLUGIN_ROOT}", plugin_dir)
                # Simulate hook by printing what it would do
                print(f"[HOOK] {hook_type}: {os.path.basename(script)}")
    
    return 0

def simulate_forge_session(plugin_dir, prompt, danger_mode=False):
    """Simulate a FORGE agent session with hooks."""
    
    # SessionStart hook
    print("==========================================")
    print("FORGE Session Starting")
    print("==========================================")
    run_hook(plugin_dir, "SessionStart")
    print()
    
    if danger_mode:
        # Simulate dangerous command request
        print("I need to run a dangerous command: rm -rf /")
        print("Run this command? (y/N)")
        sys.stdout.flush()
        
        # Write STATUS.json with suspended outcome (for test compatibility)
        status = {
            "outcome": "suspended",
            "ticket_id": "T-DANGER-001",
            "reason": "dangerous_command",
            "branch": "forge-1/T-DANGER-001",
            "notes": "Waiting for approval to run: rm -rf /"
        }
        with open("STATUS.json", "w") as f:
            json.dump(status, f, indent=2)
        
        print("\nSTATUS.json written with suspended status.")
        time.sleep(2)
    else:
        # Simulate normal work
        print("Working on task...")
        print(f"Prompt: {prompt[:50]}..." if len(prompt) > 50 else f"Prompt: {prompt}")
        sys.stdout.flush()
        time.sleep(0.5)
        print()
        
        # Simulate segment work
        print("[HOOK] PostToolUse(Write): post-write-lint.sh")
        print("  Linter: clean")
        print()
        
        print("[HOOK] PostToolUse(Write): post-write-lint.sh")
        print("  Linter: clean")
        print()
        
        # Stop hook - write STATUS.json
        status = {
            "status": "PR_OPENED",
            "pair": "forge-1",
            "ticket_id": "T-001",
            "pr_url": "https://github.com/test/repo/pull/1",
            "pr_number": 1,
            "files_changed": ["src/main.rs"]
        }
        
        with open("STATUS.json", "w") as f:
            json.dump(status, f, indent=2)
        
        print("Done! STATUS.json written.")
        print()
        print("[HOOK] Stop: stop-require-artifact.sh")
        print("  Artifact found: STATUS.json")

def simulate_sentinel_session(plugin_dir, prompt):
    """Simulate a SENTINEL agent session with hooks."""
    
    print("==========================================")
    print("SENTINEL Session Starting")
    print("==========================================")
    run_hook(plugin_dir, "SessionStart")
    print()
    
    # Simulate review
    print("Reviewing segment...")
    print("Running tests: 42 passed, 0 failed")
    print("Running linter: clean")
    print()
    
    # Write evaluation
    eval_content = """# Segment 1 Evaluation

## Verdict
APPROVED

## Summary
Implementation follows all standards.
"""
    with open("segment-1-eval.md", "w") as f:
        f.write(eval_content)
    
    print("[HOOK] PostToolUse(Write): post-write-validate.sh")
    print("  Validation: passed")
    print()
    print("Evaluation written: segment-1-eval.md")
    print()
    print("[HOOK] Stop: stop-require-eval.sh")
    print("  Evaluation found: segment-1-eval.md")

def main():
    parser = argparse.ArgumentParser(description="Mock Claude CLI with plugin support")
    parser.add_argument("--plugin-dir", help="Path to plugin directory")
    parser.add_argument("--print", action="store_true", help="Print mode")
    parser.add_argument("--output-format", help="Output format (json, text)")
    parser.add_argument("prompt", nargs="*", default="", help="Prompt to process")
    
    args = parser.parse_args()
    # Handle prompt as list or string
    if isinstance(args.prompt, list):
        prompt = " ".join(args.prompt)
    else:
        prompt = args.prompt
    
    # Detect plugin from path
    plugin_dir = args.plugin_dir
    if not plugin_dir:
        plugin_dir = os.environ.get("CLAUDE_PLUGIN_DIR", "")
    
    # Detect agent type
    agent = "forge"
    if plugin_dir:
        plugin_name = os.path.basename(plugin_dir.rstrip("/"))
        agent = plugin_name
    
    # Check for danger mode
    danger_mode = "danger" in prompt.lower()
    
    # Simulate session based on agent
    if agent == "forge":
        simulate_forge_session(plugin_dir, prompt, danger_mode)
    elif agent == "sentinel":
        simulate_sentinel_session(plugin_dir, prompt)
    else:
        print(f"Mock session for {agent}")
        print("Done!")

if __name__ == "__main__":
    main()

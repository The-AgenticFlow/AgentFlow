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

def simulate_forge_session(plugin_dir, prompt, danger_mode=False, output_dir=None):
    """Simulate a FORGE agent session with hooks."""
    output_dir = output_dir or "."
    os.makedirs(output_dir, exist_ok=True)
    
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
        status_path = os.path.join(output_dir, "STATUS.json")
        with open(status_path, "w") as f:
            json.dump(status, f, indent=2)
        
        print(f"\nSTATUS.json written to {status_path} with suspended status.")
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
        
        status_path = os.path.join(output_dir, "STATUS.json")
        with open(status_path, "w") as f:
            json.dump(status, f, indent=2)
        
        print(f"Done! STATUS.json written to {status_path}.")
        print()
        print("[HOOK] Stop: stop-require-artifact.sh")
        print("  Artifact found: STATUS.json")

def simulate_sentinel_session(plugin_dir, prompt, output_dir=None):
    """Simulate a SENTINEL agent session with hooks."""
    output_dir = output_dir or "."
    os.makedirs(output_dir, exist_ok=True)
    
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
    eval_path = os.path.join(output_dir, "segment-1-eval.md")
    with open(eval_path, "w") as f:
        f.write(eval_content)
    
    print("[HOOK] PostToolUse(Write): post-write-validate.sh")
    print("  Validation: passed")
    print()
    print(f"Evaluation written: {eval_path}")
    print()
    print("[HOOK] Stop: stop-require-eval.sh")
    print("  Evaluation found: segment-1-eval.md")

def main():
    parser = argparse.ArgumentParser(description="Mock Claude CLI with plugin support")
    parser.add_argument("--plugin-dir", help="Path to plugin directory")
    parser.add_argument("--print", action="store_true", help="Print mode")
    parser.add_argument("--output-format", help="Output format (json, text)")
    parser.add_argument("--output-dir", help="Directory to write output artifacts (STATUS.json, etc.)")
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
    
    # Detect output dir
    output_dir = args.output_dir
    if not output_dir:
        output_dir = os.environ.get("CLAUDE_OUTPUT_DIR")
    
    # Detect agent type
    agent = "forge"
    if plugin_dir:
        plugin_name = os.path.basename(plugin_dir.rstrip("/"))
        agent = plugin_name
    
    # Check for danger mode
    danger_mode = "danger" in prompt.lower()
    
    # Simulate session based on agent
    if agent == "forge":
        simulate_forge_session(plugin_dir, prompt, danger_mode, output_dir)
    elif agent == "sentinel":
        simulate_sentinel_session(plugin_dir, prompt, output_dir)
    else:
        print(f"Mock session for {agent}")
        print("Done!")

if __name__ == "__main__":
    main()

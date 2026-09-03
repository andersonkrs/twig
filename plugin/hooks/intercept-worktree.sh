#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
tool=$(printf '%s' "$input" | jq -r '.tool_name // ""')
cmd=$(printf '%s' "$input" | jq -r '.tool_input.command // ""')

deny() {
  jq -n --arg reason "$1" '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: $reason
    }
  }'
  exit 0
}

if [[ "$tool" != "Bash" ]]; then
  exit 0
fi

if printf '%s' "$cmd" | grep -Eq '(^|[;&|[:space:]])git([[:space:]]+-[^[:space:]]+([[:space:]]+[^[:space:]]+)?)*[[:space:]]+worktree([[:space:]]|$)'; then
  deny "Blocked: $cmd
Worktrees in this environment are managed by twig, which pairs each worktree with a tmux session. To work inside a new worktree use the EnterWorktree tool (it is wired to twig). Otherwise invoke the twig skill and use the matching twig command:
  git worktree add     -> twig tree create <project> <branch>
  git worktree list    -> twig tree list <project>
  git worktree remove  -> twig tree delete <project> <branch>
  git worktree prune   -> twig tree delete <project> <branch>
Read-only inspection is fine via 'twig tree list' or 'twig list'."
fi

exit 0

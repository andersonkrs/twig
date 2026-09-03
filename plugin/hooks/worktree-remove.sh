#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/twig-lib.sh"

input=$(cat)
path=$(printf '%s' "$input" | jq -r '.worktree_path // ""')
[[ -n "$path" ]] || { echo "twig WorktreeRemove: missing worktree_path" >&2; exit 1; }
[[ -d "$path" ]] || { echo "twig WorktreeRemove: $path does not exist, nothing to do" >&2; exit 0; }

branch=$(git -C "$path" rev-parse --abbrev-ref HEAD 2>/dev/null) || { echo "twig WorktreeRemove: cannot read branch of $path" >&2; exit 1; }
root=$(twig_repo_root "$path") || { echo "twig WorktreeRemove: $path is not a git worktree" >&2; exit 1; }
project=$(twig_project_for_root "$root") || { echo "twig WorktreeRemove: no twig project has root $root" >&2; exit 1; }

twig tree delete --yes "$project" "$branch" >&2

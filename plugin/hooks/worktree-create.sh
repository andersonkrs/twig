#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/twig-lib.sh"

input=$(cat)
name=$(printf '%s' "$input" | jq -r '.name // ""')
cwd=$(printf '%s' "$input" | jq -r '.cwd // ""')

[[ -n "$name" ]] || { echo "twig WorktreeCreate: missing worktree name" >&2; exit 1; }
[[ -n "$cwd" ]] || cwd=$PWD

root=$(twig_repo_root "$cwd") || { echo "twig WorktreeCreate: $cwd is not inside a git repository" >&2; exit 1; }
project=$(twig_project_for_root "$root") || {
  echo "twig WorktreeCreate: no twig project has root $root. Create one with 'twig new' first." >&2
  exit 1
}

existing=$(twig_worktree_path_for_branch "$root" "$name")
if [[ -n "$existing" && -d "$existing" ]]; then
  echo "twig WorktreeCreate: reusing worktree for '$name' at $existing" >&2
  printf '%s\n' "$existing"
  exit 0
fi

twig tree create --no-attach "$project" "$name" >&2

path=$(twig_worktree_path_for_branch "$root" "$name")
[[ -n "$path" && -d "$path" ]] || { echo "twig WorktreeCreate: twig did not register a worktree for branch '$name'" >&2; exit 1; }
printf '%s\n' "$path"

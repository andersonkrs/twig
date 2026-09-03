#!/usr/bin/env bash

twig_config_value() {
  local key="$1" default="$2" file="${XDG_CONFIG_HOME:-$HOME/.config}/twig/config.yml" value=""
  if [[ -f "$file" ]]; then
    value=$(sed -n "s/^${key}:[[:space:]]*//p" "$file" | head -n1 | sed 's/[[:space:]]*$//; s/^"\(.*\)"$/\1/; s/^'"'"'\(.*\)'"'"'$/\1/')
  fi
  printf '%s' "${value:-$default}"
}

twig_expand_tilde() {
  local p="$1"
  case "$p" in
    "~") printf '%s' "$HOME" ;;
    "~/"*) printf '%s' "$HOME/${p#\~/}" ;;
    *) printf '%s' "$p" ;;
  esac
}

twig_projects_dir() {
  twig_expand_tilde "$(twig_config_value projects_dir "${XDG_CONFIG_HOME:-$HOME/.config}/twig/projects")"
}

twig_repo_root() {
  local dir="$1" common
  common=$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || return 1
  dirname "$common"
}

twig_project_for_root() {
  local root file name proot
  root=$(realpath -m "$1")
  for file in "$(twig_projects_dir)"/*.yml; do
    [[ -f "$file" ]] || continue
    proot=$(sed -n 's/^root:[[:space:]]*//p' "$file" | head -n1 | sed 's/[[:space:]]*$//; s/^"\(.*\)"$/\1/')
    [[ -n "$proot" ]] || continue
    if [[ "$(realpath -m "$(twig_expand_tilde "$proot")")" == "$root" ]]; then
      name=$(sed -n 's/^name:[[:space:]]*//p' "$file" | head -n1 | sed 's/[[:space:]]*$//; s/^"\(.*\)"$/\1/')
      printf '%s' "${name:-$(basename "$file" .yml)}"
      return 0
    fi
  done
  return 1
}

twig_worktree_path_for_branch() {
  local root="$1" branch="$2"
  git -C "$root" worktree list --porcelain | awk -v ref="refs/heads/$branch" '
    /^worktree / { path = substr($0, 10) }
    /^branch / && $2 == ref { print path; exit }
  '
}

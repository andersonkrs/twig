---
name: twig
description: Manage git worktrees and their tmux sessions with twig. Use whenever a worktree needs to be created, listed, merged, deleted, or set up, or when a `git worktree` command was blocked by the twig hook.
---

# twig

twig is a tmux session manager with git worktree support. Every worktree it creates gets a
dedicated tmux session, windows from the project config, and the project's post-create
commands. Never call `git worktree` directly in this environment; use twig so the session
and the worktree stay in sync.

The built-in `EnterWorktree` and `ExitWorktree` tools are wired to twig through this plugin's
`WorktreeCreate` and `WorktreeRemove` hooks. `EnterWorktree` with `name: <branch>` runs
`twig tree create --no-attach <project> <branch>` and switches the session into the resulting
worktree; `ExitWorktree` with `remove` runs `twig tree delete --yes`. Prefer those tools when
the user wants to work inside a worktree in this session. Use the twig CLI directly for
everything else (listing, merging, deleting from outside, re-running setup).

The hooks resolve the twig project by matching the repository root against the `root` of the
project files in twig's `projects_dir`. If no project matches, `EnterWorktree` fails with a
message saying so; run `twig new` to create the project first. Requires a twig build that
supports `twig tree create --no-attach` and `twig tree delete --yes`.

## Resolve the project name

twig commands take a `<project>` argument. Find it in this order:

1. `$TWIG_PROJECT` if set (twig exports it inside every session it starts).
2. `twig list` and pick the project whose root matches the current repo.

`$TWIG_WORKTREE` holds the current branch when the shell is inside a worktree session.

## Command mapping

| Intent | git | twig |
|---|---|---|
| Create worktree + session | `git worktree add <path> -b <branch>` | `twig tree create <project> <branch>` |
| List worktrees | `git worktree list` | `twig tree list <project>` |
| Remove worktree + session | `git worktree remove <path>` | `twig tree delete <project> <branch>` |
| Merge branch back to main | `git merge <branch>` (from main) | `twig tree merge <project> <branch>` |
| Re-run post-create setup | n/a | `twig setup <project> [branch]` |
| Attach or start main session | n/a | `twig start <project>` |
| Stop a session | n/a | `twig stop <session>` |

Always pass both `<project>` and `<branch>` explicitly. Omitting them opens an interactive
picker that cannot be driven from a non-interactive shell.

## Running from a non-interactive shell

`twig tree create` and `twig start` finish by attaching to or switching to the tmux session.
From a shell that is not inside tmux this attach step can fail after the worktree and session
were already created. Treat that as success and verify with:

```bash
twig tree list <project>
tmux list-sessions
```

Worktree paths are decided by twig from the project config. Read the path back from the
`Created worktree at:` line or from `twig tree list` rather than guessing it.

## Delete flow

`twig tree delete` kills the tmux session and removes the worktree. Confirm with the user
before deleting a worktree that has uncommitted changes; check with
`git -C <worktree-path> status --short` first.

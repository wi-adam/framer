---
name: framer-commit
description: Validate and commit changes in the Framer Rust workspace. Use when the user asks to commit, run framer-commit, clean up a dirty Framer worktree, or prepare a Framer commit; enforce formatting/lint, strict clippy, locked all-feature tests, and relevant docs/GPU gates before committing.
---

# Framer Commit

## Overview

Use this skill to turn a Framer worktree into a scoped, validated commit. The commit is not ready until the intended diff is understood, unrelated changes are protected, and the required Rust quality gates pass.

## Workflow

1. Inspect repository state before touching files:
   - `git status --short --branch`
   - `git log --oneline --decorate -8`
   - `git diff --stat`
   - `git diff --name-status`
   - `git ls-files --others --exclude-standard`

2. Classify scope:
   - If the user requested a specific scope, stage only that scope.
   - If the user requested "untracked only", stage only the exact paths from `git ls-files --others --exclude-standard`.
   - If the dirty tree appears to be one coherent Framer slice, it can be committed together after validation.
   - Preserve unrelated user changes. Do not revert or silently include files that are outside the requested or coherent scope.

3. Run required gates before committing:
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
   - `cargo test --workspace --all-features --locked`
   - `python3 scripts/check-markdown-links.py` when docs changed.
   - `cargo test -p framer-app --test gpu_parity --locked -- --nocapture` when render, material, shader, GPU parity, or scene-building logic changed.
   - `git diff --check`

4. If a gate fails:
   - Fix in-scope failures and rerun the failed gate plus any later gates that depend on it.
   - If the failure is unrelated to the requested commit scope, report the boundary and do not commit unless the user explicitly asks to include or override it.
   - Do not create a commit while any required gate is failing.

5. Stage deliberately:
   - Use `git add -- <paths>` for the approved scope.
   - Avoid `git add -A` unless the user explicitly asks to commit everything and the full dirty tree has been inspected.
   - Confirm the staged patch with `git diff --cached --stat` and, when useful, `git diff --cached --name-status`.
   - Run `git diff --cached --check` before committing.

6. Commit:
   - Use a concise imperative commit message that names the Framer behavior or artifact changed.
   - After committing, report the commit SHA, commit subject, validation gates that passed, and any remaining unstaged/untracked work.

## Framer Defaults

The Framer checkout is a Rust Cargo workspace. In the absence of a repo-specific lint wrapper, treat `cargo fmt --all -- --check`, strict locked workspace clippy, locked all-feature workspace tests, and `git diff --check` as the pre-commit standard.

If future repo files add a Justfile, Makefile, CI config, or documented lint command, inspect it and prefer the repo-owned lint command when it clearly supersedes the default while still keeping clippy as a hard gate.

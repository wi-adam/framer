---
name: framer-development
description: Plan, implement, and pre-review substantial Framer Rust workspace changes. Use when Codex is asked to build a Framer feature, execute a docs/plans phase, make schema/product-visible changes, touch render/solver/core/app contracts, prepare a PR, or respond to PR Review feedback before merge.
---

# Framer Development

## Overview

Use this skill for Framer feature work before the commit step. The goal is to catch the issues a strict PR review would catch: wrong abstractions, avoidable custom infrastructure, missing negative tests, CPU/GPU drift, schema/docs mismatches, and green-but-incomplete validation.

## Workflow

1. Start from repo context:
   - Read `AGENTS.md`.
   - Read the relevant durable spec in `docs/specs/` and dated plan in `docs/plans/`.
   - Skim `docs/code-map.md` for the crates and modules likely to move.
   - Pull/rebase from `origin/main` when the user asks to start from latest main.

2. Write a short risk ledger before broad edits. For each touched behavior, name:
   - The contract being changed.
   - The crate boundary it crosses.
   - The expected tests.
   - The docs/spec files that must stay consistent.

3. Avoid custom infrastructure unless there is a clear reason:
   - Do not hand-roll parsers, archive formats, checksums, serialization, geometry kernels, image codecs, or dependency-resolution logic when a maintained crate or existing project helper fits.
   - Before writing a custom implementation for a standard format or algorithm, search existing dependencies and the Rust ecosystem, then document the reason if a custom path remains necessary.
   - If custom parsing of untrusted bytes is unavoidable, add malformed/truncated/duplicate/unsafe-path/unsupported-version tests in the same change.

4. Build the implementation with repo-native seams:
   - Keep `framer-core`, `framer-solver`, and `framer-render` UI-free.
   - Preserve authored intent as the persisted source of truth; derived framing/render/export state stays regenerated.
   - Keep model data deterministic and float-free unless an existing spec says otherwise.
   - For schema changes, update `PROJECT_SCHEMA_VERSION`, checked-in `.framer` fixtures, round-trip/rejection tests, `project-files.md`, and affected specs.

## Coverage Matrix

Before opening a PR, map changed behavior to tests. Missing rows are blockers.

- New model validation invariant: one positive round-trip plus one negative test per new `ModelError`/`ProjectError`/`LibraryImportError` variant.
- New file/package/network/untrusted input path: happy path plus malformed bytes, missing required entries, unsupported version/format, unsafe paths, duplicate entries, and content/hash mismatch where relevant.
- New parser/codec/archive integration: prefer a maintained crate; still test repo-level rejection semantics around the wrapper.
- CPU/GPU mirrored render behavior: headless CPU unit tests for deterministic math and explicit GPU parity for WGSL paths. A GPU test that may skip is never the only coverage for CPU behavior.
- Fallback/degrade behavior: assert both resolved and missing-resource paths.
- Public or cross-crate helper: test the contract once at the owning crate and reuse it rather than duplicating predicates.
- Product-visible or schema-visible change: update the durable spec, any dated plan status, `docs/code-map.md`, `docs/project-files.md`, and run the markdown link checker.

## Pre-PR Review

Run a self-review pass before pushing:

1. Inspect the diff by ownership:
   - `git diff --stat`
   - `git diff --name-status`
   - `git diff --check`

2. For each changed crate, ask:
   - What invariant did this change add or weaken?
   - What happens with missing, malformed, empty, duplicate, stale, or out-of-range data?
   - Is any standard library/crate a better fit than custom code?
   - Are mirrored implementations kept in lockstep, especially CPU/GPU rendering paths?
   - Do tests fail if a new branch or validator is deleted?

3. Run the required gates from the workspace root:
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
   - `cargo test --workspace --all-features --locked`
   - `python3 scripts/check-markdown-links.py` when docs changed.
   - `cargo test -p framer-app --test gpu_parity --locked -- --nocapture` when render, material, GPU, shader, or scene-building logic changed.

## PR Follow-Through

After opening a PR, watch CI and PR Review. Treat blocking inline review comments as required work, but still classify advisory/nit comments separately. After fixing review feedback, rerun the local gates that cover the touched area, push a new commit, and watch the latest head run rather than stale runs.

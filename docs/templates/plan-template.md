<!--
  PLAN TEMPLATE — copy to docs/plans/<YYYY-MM-DD>-<feature>.md and fill in.
  A plan is TEMPORAL: it records how/when a change is built. It is a point-in-time
  artifact and is allowed to go stale after the work lands — do not chase it.
  - Name the file with a date prefix.
  - Durable intent/requirements/decisions belong in the feature's spec, not here.
  See docs/spec-driven-development.md.
-->

# <Feature> — Implementation Plan (<YYYY-MM-DD>)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/<feature>.md](../specs/<feature>.md). This file is an archival record of how the
> work was sequenced; the spec is the durable source of truth.

## Goal

The concrete outcome of this plan, and the slice of the spec it delivers.

## Architecture / stack summary

The relevant existing types/files this work builds on (cite paths). Keep brief — the spec
holds the durable architecture.

## Slices / phases

Break the work into shippable slices. Each task names the files it touches, how to verify it,
and a commit message. Every task should leave the workspace green.

### Slice 1 — <name>

- **Task 1.1** — <what to do>
  - Files: `path/to/file.rs`, …
  - Verify: `cargo test -p <crate>` / specific assertion / manual check
  - Commit: `verb(scope): description`
- **Task 1.2** — …

### Slice 2 — <name>

- **Task 2.1** — …

## Final verification

The full gate to run before the feature is considered done:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Plus any feature-specific checks (golden render, GPU parity, example round-trip). When done,
update the spec's **Status** and **Last reviewed**, and update any affected docs.

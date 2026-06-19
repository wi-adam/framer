<!--
  SPEC TEMPLATE — copy to docs/specs/<feature>.md and fill in.
  A spec is DURABLE: it states what a feature is, what it must do, and why.
  - Name the file by feature (no date). Keep it current as the feature evolves.
  - Do NOT put task breakdowns, build order, or phase plans here — those are temporal
    and belong in docs/plans/<date>-<feature>.md.
  See docs/spec-driven-development.md.
-->

# <Feature name>

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Proposed | Partial | Implemented (evolving) · **Linked goal:** G-0XX (or —) ·
> **Plan:** [optional link to the latest implementation plan](../plans/) ·
> **Last reviewed:** <YYYY-MM-DD>

## Intent / Purpose

What this feature is for, in product terms. One or two paragraphs. Link the relevant part of
[vision.md](../vision.md) and the goal it serves.

## Requirements & behavior

What the feature must do — the observable contract, not the implementation. Bullet the rules,
states, and edge cases. Prefer testable statements ("a driving dimension that contradicts
another on the same wall is rejected" rather than "dimensions should be consistent").

## Decisions (locked)

The deliberate choices this feature commits to, each with a one-line rationale. These are the
things a future contributor must not silently undo. State the alternative rejected when it
clarifies the choice.

## Architecture (grounded in the codebase)

How the requirements map onto real types and files — cite paths
(e.g. `framer-core/src/model.rs`, `framer-solver/src/lib.rs`). Keep this current with the
code; it is the bridge to [code-map.md](../code-map.md). Describe the data shape and the key
functions, not a line-by-line plan.

## Constraints & invariants

The non-negotiables this feature must preserve: determinism, UI-free core/solver, authored
intent as the only source of truth, schema/serialization rules, etc.
See [architecture.md](../architecture.md) and [project-files.md](../project-files.md).

## Out of scope (YAGNI)

What this feature deliberately does *not* do, and what is left architecturally open for later.

## Open questions

Anything unresolved. Remove the section when empty.

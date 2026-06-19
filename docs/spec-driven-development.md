# Spec-Driven Development in Framer

Framer is moving toward spec-driven development: **intent is written down, lives in version
control, and is the thing we keep current — code and tests follow from it.** We are pragmatic
about it (we don't need 100% ceremony), but we hold one rule firmly:

> **Separate durable intent from temporal planning artifacts.**

- A **spec** answers *what a feature is, what it must do, and why.* It is **durable**: named by
  feature (no date), kept current as the feature evolves. Specs live in
  [`docs/specs/`](specs/).
- A **plan** answers *how and when a change is built* (task breakdown, build order, slices). It
  is **temporal**: dated, archival, and allowed to go stale once the work lands. Plans live in
  [`docs/plans/`](plans/).

Mixing the two is what makes docs rot: a "design doc" full of `Branch: feat/...` and
`Status: pre-implementation` is half durable truth and half stale to-do list. Splitting them
means specs are worth maintaining and plans are honestly disposable.

## The document layers

```
vision.md            product source of truth — north star, principles, milestones, goal backlog
   │                 (durable; the "why" and "what at the product level")
   ▼
docs/specs/<feature>.md   per-feature intent / requirements / locked decisions
   │                      (durable; the "what & why" of one feature)
   ▼
docs/plans/<date>-<feature>.md   implementation plan — slices, tasks, verify, commits
   │                             (temporal; the "how & when")
   ▼
code + tests         the implementation; tests encode the spec's behavior
   │
   ▼
docs kept current    architecture.md / code-map.md / project-files.md / the spec's Status
```

Supporting durable docs: [architecture.md](architecture.md) (conceptual system shape),
[code-map.md](code-map.md) (concrete file/type navigation), and
[project-files.md](project-files.md) (the `.framer` format — effectively the spec for the file
format + the agent editing contract).

## The loop

1. **Frame the work against the product.** Tie it to a goal in
   [vision.md](vision.md#goal-backlog) (G-0XX) or note that it's a new direction (and update
   the vision if it changes product intent).
2. **Write or update the spec** ([template](templates/spec-template.md)) → `docs/specs/<feature>.md`.
   Capture intent, requirements/behavior, locked decisions, and the grounded architecture.
   Lock the decisions before writing code.
3. **Write the plan** ([template](templates/plan-template.md)) →
   `docs/plans/<date>-<feature>.md`. Slice the work; each task names its files, a `Verify:`
   step, and a commit message; each task leaves the workspace green.
4. **Implement**, slice by slice. Tests encode the spec's observable behavior.
5. **Close the loop.** Update the spec's **Status**/**Last reviewed**, refresh any affected
   durable docs (`code-map.md`, `project-files.md`, `architecture.md`), and meet the
   [Definition of Done](vision.md#definition-of-done).

## When do I need a spec?

- **Spec required:** a new feature or subsystem; a change to the `.framer` schema or a
  product-visible behavior; anything where future contributors need to know *why* a decision
  was made. The spec can be short — a few paragraphs is fine.
- **Spec not required:** bug fixes, refactors with no behavior change, small mechanical edits.
  If a fix reveals that a decision was wrong, update the relevant spec.
- **Plan optional for small specs:** trivial features can skip a separate plan file. Reach for
  one when the work spans several slices or needs sequencing.

## Conventions (carried over from existing design docs)

- **Decisions (locked):** list the deliberate choices with a one-line rationale; don't silently
  reverse them later.
- **Grounded in the codebase:** justify architecture with real file paths and types, so a spec
  stays a navigation aid, not an abstraction.
- **Out of scope (YAGNI):** state what the feature deliberately doesn't do and what's left open.
- **Determinism first:** every change preserves the [invariants](architecture.md) — UI-free
  core/solver, integer ticks, ID-sorted canonical `.framer`, seeded renderer.

## Traceability

- A spec names its **goal** (G-0XX) where one applies.
- A plan links its **spec**; a spec may link its latest **plan**.
- A schema-affecting spec is reflected in [project-files.md](project-files.md) and the
  `examples/projects/*.framer` round-trip fixtures.

## Future work (not built now)

This is deliberately lightweight. Possible later additions, none required today:

- `/spec` and `/goal` slash-commands/skills that scaffold a spec/plan from the templates.
- A CI lint that flags specs whose **Last reviewed** is stale. *(Internal markdown link
  checking is already enforced — see [build-and-ci.md](specs/build-and-ci.md).)*
- Migrating the remaining historical `docs/plans/` research notes under per-feature spec
  folders if the spec set grows large.

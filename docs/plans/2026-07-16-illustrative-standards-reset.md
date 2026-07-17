# Illustrative Standards Reset — Implementation Plan (2026-07-16)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/standards-engine.md](../specs/standards-engine.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth. The
> calculation findings live in the companion
> [header span calculation research](2026-07-16-header-span-calculation-research.md).

## Goal

Remove the built-in starter pack's specific model-code identity and claims from code,
fixtures, UI-facing provenance, and documentation. Replace it with an explicitly
illustrative, not-for-construction Framer pack that exercises the standards engine without
presenting approximate data as an authoritative compliance source. Preserve the generic
standards-pack architecture for user-authored, jurisdiction-provided, and explicitly
licensed content.

This slice also records a bounded research spike for independently calculating header span
capacities. It does not ship structural engineering calculations or claim equivalence to a
published code table.

## Risk ledger

| Contract | Boundary | Expected proof | Docs kept consistent | Likely review failure if missed |
| --- | --- | --- | --- | --- |
| Built-in pack/API identity changes from the legacy code-branded starter to `illustrative_starter` | `framer-core` API consumed by every logic/app crate | Core canonical JSON, project round trips, starter-library tests, repository-wide legacy-name search | standards spec, architecture, code map, project files, libraries spec | A stale id/rule silently breaks stack resolution, vendoring, overlays, or report provenance |
| Built-in data is visibly illustrative and not for construction | core data → solver provenance → standards report → app | Assertions on pack name, edition, tags, citations, and exported report/BOM text | README, vision, AGENTS contract, standards spec | UI still looks like an authoritative compliance verdict despite renamed Rust symbols |
| Default demo headers fit their available vertical bands | core demo → solver generation | Exact selected profile/plies and absence of `opening.header.depth-clipped` for the default shell | standards spec and calculation research | The original startup violation survives the branding cleanup |
| Same v14 schema, new seeded ids | project/library serialization | Current examples remain canonical; old arbitrary pack ids remain structurally loadable; no alias is added | project-files and libraries spec | Unnecessary schema bump or hidden compatibility source of truth |
| Calculation spike stays research-only | docs → future solver design | Worked independent example, explicit assumptions, go/no-go boundary, proposed tests | standards spec open questions and research reference | Approximate math lands as production compliance behavior without material/load provenance |

## Architecture / stack summary

- `crates/framer-core/src/standards.rs` owns the built-in pack, framing defaults, table
  shapes, rule ids, citations, and pure resolution.
- `crates/framer-solver/src/lib.rs` consumes resolved header/fastening tables and emits
  provenance and diagnostics.
- `crates/framer-standards/src/lib.rs` evaluates checks and exports reports.
- `crates/framer-library` and `libraries/framer-starter.framerlib` distribute the starter
  pack through the existing vendor-on-use path.
- `.framer` remains schema v14 and `.framerlib` remains schema v3; only authored content
  and built-in identities change.

## Locked names and behavior

- Rust constructors: `StandardsPack::illustrative_starter()` and
  `FramingDefaults::illustrative_starter()`.
- Pack id/name/edition: `std-framer-illustrative`, `Framer Illustrative Starter`, and
  `illustrative-v1`.
- Built-in rule ids use the `framer.starter.*` namespace.
- Pack tags include `illustrative` and `not-for-construction`; every built-in citation says
  that it is illustrative and not for construction.
- The demo-only header rows cover 4-foot openings with a two-ply 2x6 and 8-foot openings
  with a two-ply 2x8. Their load-band fields are sentinels for the unset demo site, not
  published design data. Any configured positive snow load falls outside these rows and
  fails closed to the existing unsupported/fallback path.
- Existing files containing arbitrary legacy pack ids remain loadable because ids and rule
  strings are authored data. New code does not add an alias or silently rewrite them.

## Slices

### Slice 1 — Durable intent and research boundary

- Update the standards spec so built-in content is illustrative and authoritative packs are
  external/user-authored/explicitly licensed.
- Record the independent calculation research, required inputs, equations, deterministic
  implementation approach, and unresolved engineering boundaries.
- Add both dated artifacts to `docs/plans/README.md`.

Verify: `python3 scripts/check-markdown-links.py`.

### Slice 2 — Core identity and demo assumptions

- Rename constructors, pack id, rule ids, citations, test helpers, and expected canonical
  JSON in `framer-core`.
- Mark the pack as illustrative/not-for-construction.
- Replace the starter header rows with the two bounded demo rows and make the fallback
  header default physically self-consistent.
- Add a solver regression that the default shell emits no clipped-header diagnostic.

Verify: `cargo test -p framer-core --all-features --locked` and
`cargo test -p framer-solver --all-features --locked`.

### Slice 3 — Cross-crate, fixtures, and product language

- Update every crate call site and assertion.
- Update all checked project fixtures and the starter library.
- Rewrite current and archival documentation so the repository contains no specific legacy
  model-code branding or section/table identifiers.
- Preserve the user's unrelated untracked example exports.

Verify: canonical project/library tests plus a repository-wide case-insensitive search.

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```

No GPU gate is required because this slice does not touch rendering, geometry, shaders, or
scene lowering.

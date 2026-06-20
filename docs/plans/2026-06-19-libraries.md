# Libraries — Implementation Plan (2026-06-19)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/libraries.md](../specs/libraries.md). This file is an archival record of how the
> work is sequenced; the spec is the durable source of truth.

## Goal

Deliver a unified, distributable **Libraries** system (G-013) incrementally, vendor-on-use
with typed provenance, without ever breaking determinism or the self-contained `.framer`.
Early slices ship real reuse for wall systems + materials and retire the triplicated
`starter_library()`; later slices add binary assets, remote/managed libraries, and
furnishings/MEP through the **same** spine. Each slice is independently shippable and leaves the
workspace green.

## Architecture / stack summary

Builds on (cite paths; durable detail in the [spec](../specs/libraries.md)):

- `framer-core/src/model.rs` — `Material` / `MaterialSource` / `Appearance` (~1320–1443),
  `ConstructionSystem`, `ElementId` + charset (`is_id_continue`), `validate()`'s single
  `BTreeSet`, `starter_library()` (~537), `system_for` / `material` resolvers.
- `framer-core/src/project.rs` — `ProjectDocument`, `SchemaHeader` peek, `to_canonical_json`,
  the v-only discipline + round-trip/rejection tests (the template the `.framerlib` loader
  mirrors).
- `framer-app/src/app/panels.rs` — `library_tree()` browser (insert action + provenance badge).
- `framer-app/src/app/model_edit.rs` — id-generation helpers to reuse for remap; `edit()` for
  one-undo imports.
- `framer-render/src/build.rs` + `material.rs` — where `Appearance` lowers into render materials
  (asset sampling, Slice 3).

## Slices / phases

| Slice | Scope | Schema bump |
| --- | --- | --- |
| **0** | `LibraryDocument`/`Library` types + canonical (de)serialization + `Library::validate()`; checked-in `framer-starter.framerlib`; `new`/demos source the starter from it. | none (new format only) |
| **1** | Typed **symmetric** provenance + `framer-library` crate + insert-from-library with atomic remap + provenance badge + pinned `blake3` + golden-hash test. | **v7 → v8** |
| **2** | Update lifecycle: re-sync / detach / divergence detection as diagnostics. | none |
| **3** | Binary assets (textures + depth maps) + `.framerpkg` + render wiring. | **v8 → v9** |
| **4** | Remote / URL libraries (mandatory hash pin, cache-first, fail-closed); provider interface shaped for a future managed/RPC backend. | none |
| **5** | Furnishings / MEP element families + catalog placement through the same spine. | bump when families land |

Slices 0–1 are the near-term focus. The detailed task breakdown below covers them; later
slices are sketched and will get their own task detail when scheduled.

### Slice 0 — Library file format + dogfood the starter library

- **Task 0.1** — Add `LibraryDocument { format, schema_version, … }` and `Library { uid,
  version_id, version, coordinate, materials, systems }` (identity = a stable `uid` UUID + a
  UUIDv7 `version_id`; coordinate is a non-identity hint — see the spec's Identity model) with
  `deny_unknown_fields`, a `SchemaHeader`-style peek-and-reject loader, and `to_canonical_json`
  mirroring `project.rs`. For P0 the starter library's `uid`/`version_id` are checked-in
  literals — no runtime UUID generation until a publish tool exists.
  - Files: `framer-core/src/library.rs` (new), `framer-core/src/lib.rs` (re-export).
  - Verify: `cargo test -p framer-core` — round-trip + canonical + rejection tests for
    `.framerlib`, modeled on the `ProjectDocument` tests.
  - Commit: `feat(core): .framerlib library document format + canonical serialization`
- **Task 0.2** — Add `Library::validate()` reusing `ConstructionSystem` validation against the
  library's own material set (self-consistent before publish).
  - Files: `framer-core/src/library.rs`.
  - Verify: `cargo test -p framer-core` — a library with a dangling layer→material ref is
    rejected.
  - Commit: `feat(core): validate .framerlib internal consistency`
- **Task 0.3** — Add a checked-in `libraries/framer-starter.framerlib` (the canonical starter
  content) and have `BuildingModel::starter_library()` / `new` / `demo_*` source from it (a
  build-time `include_str!` or a small loader), producing byte-identical embedded copies.
  - Files: `libraries/framer-starter.framerlib` (new), `framer-core/src/model.rs`.
  - Verify: `cargo test --workspace` — the three `*_example_is_canonical` tests stay green
    (example files unchanged on disk); a test asserts the starter `.framerlib` parses and
    matches the previous starter content.
  - Commit: `refactor(core): source starter library from framer-starter.framerlib`

### Slice 1 — Vendor-on-use with symmetric provenance (v7 → v8)

- **Task 1.1** — Add the identity types — `Provenance { library_uid, version_id, source_id,
  content_hash }` and a project-level `LibraryStamp { uid, version_id, content_hash, coordinate,
  version }` on `BuildingModel.libraries` (normalized, skip-empty, sorted). Evolve
  `MaterialSource::External { reference }` → `Library(Provenance)`; add
  `ConstructionSystem.source: Option<Provenance>` (`skip_serializing_if = Option::is_none`).
  - Files: `framer-core/src/model.rs`, `framer-core/src/lib.rs`.
  - Verify: `cargo test -p framer-core` — provenance + library table round-trip; a no-library
    model body is byte-identical after deserialize/serialize (only the version stamp differs).
  - Commit: `feat(core): typed symmetric library provenance + library identity table`
- **Task 1.2** — The v8 schema bump ritual: bump `PROJECT_SCHEMA_VERSION = 8`; regenerate the
  three `examples/projects/*.framer`; update the rejection test for the now-old v7; update
  `docs/project-files.md`.
  - Files: `framer-core/src/project.rs`, `examples/projects/*.framer`, `docs/project-files.md`.
  - Verify: `cargo test --workspace` — round-trip + v7-rejection green; markdown link check.
  - Commit: `feat(core)!: bump .framer schema to v8 for library provenance`
- **Task 1.3** — Pin `blake3`; add canonical-bytes hashing over a `.framerlib`
  (`"blake3:<full-lowercase-hex>"`) + a golden-hash regression test.
  - Files: `Cargo.toml` (workspace dep, exact-pinned), `framer-core/src/library.rs` (or
    `framer-library`), a golden-hash test.
  - Verify: `cargo test` — the golden hash matches a frozen expected value.
  - Commit: `feat(library): pinned blake3 content hashing for libraries`
- **Task 1.4** — New `framer-library` crate (depends only on `framer-core`): `Locator`,
  `LibraryResolver` trait, a local search-path resolver, and the **import pipeline** (resolve →
  verify hash → atomic namespace-remap of the subgraph → vendor into project collections →
  stamp provenance).
  - Files: `crates/framer-library/*` (new), workspace `Cargo.toml`.
  - Verify: `cargo test -p framer-library` — namespacing-closure test (system import rewrites
    `material` + `cavity_material`); collision mints lowest-free suffix; **absent-library still
    opens + validates** test.
  - Commit: `feat(library): framer-library crate + vendor-on-use import pipeline`
- **Task 1.5** — Wire "Insert from library" + a provenance badge into `library_tree()`; route
  import through `edit()` for one-undo.
  - Files: `framer-app/src/app/panels.rs`, `framer-app/src/app/model_edit.rs`.
  - Verify: headless UI smoke test (`ui_harness_tests.rs`) + manual via the
    [`install-app`](../../.claude/skills/install-app) skill.
  - Commit: `feat(app): insert-from-library with provenance badge`

### Slice 2 — Update lifecycle: diagnostics, re-sync, detach

- **Task 2.1** — Add pure lifecycle inspection in `framer-library`: compute vendored item
  hashes in source/library-local id space, detect **diverged**, **out-of-date**, and
  **source-missing** states, and keep missing libraries non-fatal.
  - Files: `crates/framer-library/src/lib.rs`.
  - Verify: `cargo test -p framer-library --locked` — freshly imported systems/materials have
    no false divergence despite project-local id remap; local edits emit divergence; changed
    source library content emits out-of-date.
  - Commit: `feat(library): detect vendored content lifecycle state`
- **Task 2.2** — Add transactional detach/re-sync APIs in `framer-library`: detach clears
  selected item provenance and prunes unused stamps; re-sync overwrites the selected vendored
  material/system from an available library while preserving project-local ids and validating
  before commit.
  - Files: `crates/framer-library/src/lib.rs`.
  - Verify: `cargo test -p framer-library --locked` — material re-sync updates provenance to the
    current library version; system re-sync refreshes the referenced material closure and keeps
    local ids stable.
  - Commit: `feat(library): re-sync and detach vendored definitions`
- **Task 2.3** — Surface lifecycle issues on the existing diagnostics channel without making
  project open/load/solve depend on library I/O. The app appends derived `library.item.*`
  diagnostics after solver plan generation for libraries it can currently resolve.
  - Files: `crates/framer-app/src/app/mod.rs`.
  - Verify: `cargo test -p framer-app --locked imported_material -- --nocapture` — local edits to
    an imported material emit `library.item.diverged`.
  - Commit: `feat(app): surface library lifecycle diagnostics`
- **Task 2.4** — Add selected-item lifecycle controls in the inspector: source/status display,
  **Re-sync** when the source library is available, and **Detach** for library-backed materials
  and systems. Route both through `edit()` so undo stays one labelled step.
  - Files: `crates/framer-app/src/app/panels.rs`.
  - Verify: `cargo test -p framer-app --locked imported_material -- --nocapture` — detach clears
    provenance and diagnostics; re-sync restores source content.
  - Commit: `feat(app): add library re-sync and detach controls`
- **Task 2.5** — Refresh durable docs for the implemented lifecycle behavior.
  - Files: `docs/specs/libraries.md`, `docs/plans/2026-06-19-libraries.md`,
    `docs/code-map.md`, `docs/project-files.md`.
  - Verify: `python3 scripts/check-markdown-links.py`.
  - Commit: `docs(libraries): document update lifecycle`

### Slices 3–5 — sketch (task detail when scheduled)

- **Slice 3 (v8 → v9):** `AssetRef` + `Appearance::Textured`/`DepthMapped`; disposable
  content-addressed asset store; deterministic `.framerpkg`; texture/depth sampling wired into
  `framer-render` (CPU reference first), keeping `tests/gpu_parity.rs` green.
- **Slice 4:** `Remote { url, hash }` resolver — mandatory pin, cache-first, fail-closed; shape
  the provider interface so a managed/RPC backend slots in (publish/edit catalog remotely;
  consume always pins a snapshot).
- **Slice 5:** `Furnishing` / `MepObject` families + library vectors + drag-and-drop placement,
  through the identical spine.

## Final verification

The full gate before any slice is considered done:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Feature-specific:

- **Slice 0/1:** `.framerlib` round-trip; **absent-library-still-opens** + **no-library body
  byte-identical after the v8 bump** tests; golden-hash regression; namespacing-closure tests;
  regenerated example fixtures; `python3 scripts/check-markdown-links.py` for doc edits.
- **Slice 3:** `cargo test -p framer-app --test gpu_parity` stays green after asset sampling;
  deterministic-zip test for `.framerpkg`.

When a slice lands, update the spec's **Status** / **Last reviewed**, and refresh affected
durable docs — [project-files.md](../project-files.md), [code-map.md](../code-map.md) (a "where
to add a library item kind" entry), and [architecture.md](../architecture.md) where relevant.

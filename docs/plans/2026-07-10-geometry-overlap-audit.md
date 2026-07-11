# Physical Geometry Overlap Audit — Implementation Plan (2026-07-10)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/geometry-overlap-audit.md](../specs/geometry-overlap-audit.md). This file is an
> archival record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Deliver the proposed physical-geometry overlap audit in three independently reviewable PRs:

1. establish an identity-preserving, UI-free physical-solid layer and make interactive member
   rendering consume it;
2. add deterministic broad/narrow collision auditing, headless output, and regression gates; and
3. integrate violations, focus/highlighting, and final documentation into the desktop product.

The completed plan must catch historical wall/member and rafter/ridge penetrations while accepting
valid face contact, preserving schema v13, and leaving authored intent and semantic solver placement
unchanged.

## Architecture / stack summary

`framer-core` already owns integer-tick assembly derivations such as wall lap spans and roof/ceiling
frames. `framer-solver::ProjectFramePlan` owns semantic generated members, including explicit
`SlopedPlacement`, but deliberately preserves construction reference locations such as ridge
centerlines. The app currently turns those semantics into `WallCuboid`, generic `BoardPrism`, and
cut-profile `RafterPrism` geometry inside `viewport/scene_build`; the rafter's birdsmouth and
ridge-face setback therefore exist only at the presentation boundary.

This plan adds `framer-geometry` below the app and above core/solver. The crate owns reusable physical
solids and audits while the app and renderer retain presentation-specific vertices, materials, and
GPU data. Maintained collision/spatial-index crates provide the query kernels; Framer owns semantic
body identity, domain policy, deterministic ordering, and diagnostics.

## Risk ledger and coverage matrix

| Contract / risk | Boundary | Required tests | Likely review failure if missed |
| --- | --- | --- | --- |
| Audit geometry matches visible cut members | solver semantics → geometry → app | Common rafter positive/fallback tests; pre-#113 ridge penetration fixture; mesh/pick equivalence | The gate audits a generic prism while the viewport draws a different body |
| Every generated member becomes one semantic body | all plan families → geometry | Wall/floor/ceiling/roof member inventory tests; empty and degenerate cases | A new or fallback member silently disappears from the audit |
| Assembly envelopes use current derived boundaries | core → geometry/app/render | Wall laps/openings, roof overhangs, sloped ceilings, concave outlines | Audit results disagree with the occupied assembly shown elsewhere |
| Concave profiles remain exact | 2-D triangulation → convex pieces | Birdsmouth notch is empty space; containment and piece-union overlap tests | Convex-hull lowering fills the notch and invents collisions |
| Contact is not penetration | floating query boundary | Face/edge/point touch, separated, shallow/deep penetration at multiple scales | Every valid framing joint becomes a violation or real slivers are hidden |
| Broad phase is complete | AABB index → narrow phase | Indexed candidates equal a brute-force oracle on small deterministic scenes | Performance improves by skipping a real colliding pair |
| Collision-domain filtering is semantic | physical scene → audit | Framing/framing and assembly/assembly positive tests; framing-in-wall negative test | Contained framing floods diagnostics or cross-owner clashes are suppressed |
| Query failures fail closed | maintained query crate → audit result | Unsupported shape-pair and unbuildable-body tests | Missing library coverage reports a false clean project |
| Results are deterministic | audit → CLI/app/tests | Reversed bodies, shuffled model vectors, repeated runs, canonical sorting | CI or diagnostic ordering changes by platform/traversal order |
| Product diagnostics retain both bodies | geometry → app state/UI | Diagnostic lowering, focus/highlight, deletion/regeneration recovery | UI can name only one side or holds stale body ids after an edit |
| New product behavior is visually legible | app scene/panels | UI smoke and off-screen screenshots in both themes | Violations exist but cannot be located or obscure ordinary selection |
| No schema/persistence drift | derived state boundary | Existing byte-exact round trips; no geometry fields in `.framer` | Transient audit state leaks into schema v13 |

## PR 1 — Shared physical-solid layer

### Outcome

Add the `framer-geometry` crate and make it the physical-solid authority for generated framing
members. Build assembly-envelope bodies from the existing shared core derivations. The app lowers
geometry-owned member solids into its existing GPU and pick representations with no intended visual
change.

### Tasks

- **Task 1.1 — Add the crate and stable physical-scene types.**
  - Define `PhysicalScene`, `PhysicalBody`, canonical `BodyRef`, `CollisionDomain`, `BodyKind`,
    `ConvexPiece`, body AABBs, and geometry build diagnostics.
  - Keep collision-library types and floating-point values out of `framer-core` and
    `framer-solver` public data.
  - Files: `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, `crates/framer-geometry/Cargo.toml`,
    `crates/framer-geometry/src/lib.rs`, `crates/framer-geometry/src/solid.rs`,
    `crates/framer-geometry/README.md`
  - Verify: crate dependency-direction tests/build; stable `BodyRef` ordering; empty-scene behavior;
    no UI dependency in the new crate
  - Commit: `feat(geometry): add physical scene primitives`

- **Task 1.2 — Lower every generated member into identity-preserving bodies.**
  - Move wall-local cuboid, arbitrary board-prism, rake-plate, matched-bearing,
    common-rafter-profile, birdsmouth, and ridge-face-setback derivation out of the app.
  - Represent a concave rafter profile as the union of its triangulated convex extrusions rather
    than its convex hull.
  - Report a body-build violation when a generated non-degenerate member cannot be lowered; keep
    explicit empty/invalid input behavior tested.
  - Files: `crates/framer-geometry/src/build/mod.rs`,
    `crates/framer-geometry/src/build/members.rs`,
    `crates/framer-app/src/app/viewport/scene_build/members.rs`,
    `crates/framer-app/src/app/viewport/scene_build/picking.rs`,
    `crates/framer-app/src/app/viewport/scene_build/tests.rs`
  - Verify: one body per wall/floor/ceiling/roof member; cuboid and spatial-member bounds; forward and
    reverse common rafters; matched/unmatched bearing; shed/truss fallback; hip/valley/jack/rake
    regression coverage; rendered triangles and pick triangles come from the same geometry body
  - Commit: `refactor(geometry): share generated member solids`

- **Task 1.3 — Build finished assembly-envelope bodies from shared derivations.**
  - Lower walls with lapped envelope spans and opening cavities, floor decks, ceilings, and roof
    planes into assembly-domain bodies. Reuse `RoofPlaneFrame`, ceiling frames, overhang outlines,
    level elevations, and resolved construction-system thickness rather than duplicating them.
  - Keep assembly envelopes semantically separate from per-layer presentation meshes.
  - Files: `crates/framer-geometry/src/build/assemblies.rs`,
    `crates/framer-geometry/src/build/mod.rs`, `crates/framer-core/src/model.rs` only if a shared
    read-only helper must be exposed, `crates/framer-app/src/app/viewport/scene_build/walls.rs`,
    `crates/framer-app/src/app/viewport/scene_build/surfaces.rs`,
    `crates/framer-render/src/build.rs`
  - Verify: lapped wall bounds and cavities; unequal-thickness corners; horizontal/sloped/concave
    surfaces; overhang outlines; app/render/audit boundary parity; no intentional golden change
  - Commit: `feat(geometry): derive physical assembly bodies`

- **Task 1.4 — Document the new workspace seam and complete adversarial review.**
  - Update the six-crate repo map to seven crates and document where physical-solid derivation lives.
  - Confirm deleting any new fallback/build-violation branch makes a focused test fail.
  - Files: `AGENTS.md`, `docs/architecture.md`, `docs/code-map.md`,
    `docs/specs/geometry-overlap-audit.md`, `crates/framer-geometry/README.md`
  - Verify: `python3 scripts/check-markdown-links.py`; diff review by core/solver/geometry/app/render
    ownership; UI-shot deck inspected for unintended geometry changes
  - Commit: `docs: map the physical geometry layer`

### PR 1 gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
scripts/ui-shots.sh
```

Set the spec status to **Partial** when PR 1 lands.

## PR 2 — Deterministic overlap audit and regression gate

### Outcome

Add maintained broad- and narrow-phase collision queries, structured overlap diagnostics, a headless
audit command, historical regression fixtures, and a clean-project gate covering every checked-in
example. No app UI behavior changes in this PR.

### Tasks

- **Task 2.1 — Integrate maintained broad/narrow query dependencies behind local adapters.**
  - Use an `rstar` AABB spatial index for candidate generation and `parry3d-f64` contact queries for
    convex pieces; do not introduce a physics world or custom GJK/SAT implementation.
  - Canonicalize candidate pair keys before evaluation. Treat unsupported query pairs as explicit
    failures.
  - Files: `Cargo.toml`, `Cargo.lock`, `crates/framer-geometry/Cargo.toml`,
    `crates/framer-geometry/src/spatial.rs`, `crates/framer-geometry/src/query.rs`
  - Verify: candidate completeness against an O(n²) test oracle; separated/touching/penetrating
    cuboids; containment; rotated/sloped convex pieces; unsupported adapter path
  - Commit: `feat(geometry): add solid contact queries`

- **Task 2.2 — Implement collision policy and deterministic diagnostics.**
  - Audit framing/framing and assembly/assembly pairs, skip cross-detail domains and self-pairs,
    evaluate every convex-piece candidate, and retain the deepest valid penetration and witness.
  - Define a scale-aware numerical epsilon substantially below one tick. Canonically sort body refs
    and final issues; round only human-readable output, never the pass/fail comparison.
  - Return structured build/query/overlap violations with stable codes such as
    `geometry.body.unbuildable`, `geometry.query.unsupported`, and `geometry.overlap`.
  - Files: `crates/framer-geometry/src/audit.rs`, `crates/framer-geometry/src/diagnostic.rs`,
    `crates/framer-geometry/src/lib.rs`
  - Verify: face/edge/point contact accepted; sub-tick real penetration rejected above epsilon;
    shuffled inputs and reversed query order yield identical body pairs/order; framing inside its
    hosting assembly is not compared; cross-owner framing clashes are compared
  - Commit: `feat(geometry): audit physical body overlaps`

- **Task 2.3 — Add historical and branch-complete regression fixtures.**
  - Reproduce the pre-wall-lap assembly overlap and duplicate corner-post collision with synthetic
    historical bodies; assert the current demo shell is clean.
  - Reproduce the pre-ridge-setback 0.75-inch rafter/ridge penetration; assert current common rafters
    and ridge boards only contact. Include the bearing/birdsmouth relationship without filling the
    notch's empty volume.
  - Cover square and portrait gables, both roof fields, shed/truss fallback, hip, valley, jack,
    mirrored-L, multiple levels/elevations, openings, and concave assembly outlines.
  - Files: `crates/framer-geometry/src/audit/tests.rs`,
    `crates/framer-geometry/src/build/tests.rs`, `crates/framer-geometry/tests/examples.rs`
  - Verify: each positive fixture fails if narrow-phase auditing is removed; each valid-contact
    fixture fails if contact is treated as penetration; broad-phase and domain-policy branches are
    mutation-sensitive
  - Commit: `test(geometry): cover historical overlap regressions`

- **Task 2.4 — Add the headless command and checked-in example gate.**
  - Add `geometry-audit <project.framer>` with deterministic text output, nonzero exit on any geometry
    violation, and clear load/solve/build/query error reporting. Keep the library result as the
    machine-readable API.
  - Audit every `examples/projects/*.framer` file in a normal workspace test so existing CI runs the
    gate without a separate best-effort job.
  - Files: `crates/framer-geometry/src/bin/geometry-audit.rs`,
    `crates/framer-geometry/Cargo.toml`, `crates/framer-geometry/tests/examples.rs`,
    `CONTRIBUTING.md`, `crates/framer-geometry/README.md`
  - Verify: clean/overlapping/malformed/missing-file CLI cases; stable pair ids and nonzero status;
    all checked-in examples clean
  - Commit: `feat(geometry): add headless overlap audit`

- **Task 2.5 — Baseline the repository and fix, do not suppress, discovered violations.**
  - Run the audit across checked-in projects and focused scene fixtures. Classify each finding as a
    real geometry defect, invalid fixture, unsupported body, or audit bug.
  - Fix real geometry defects in their owning derivation with a regression test. Do not add id-based
    exclusions or increase the global epsilon to make the baseline green.
  - Files: determined by findings; keep unrelated fixes split into focused commits within this PR or
    explicitly defer the PR until prerequisite fixes land
  - Verify: zero build/query/overlap violations across the supported example matrix
  - Commit: `fix(geometry): resolve overlap audit baseline` (only when findings require it)

### PR 2 gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo run -p framer-geometry --bin geometry-audit -- examples/projects/demo-shell.framer
```

Keep the spec status **Partial** when PR 2 lands.

## PR 3 — App diagnostics, focus, and completion

### Outcome

Run the audit with each regenerated project plan, present geometry violations through the desktop
diagnostics surfaces, and let a user focus/highlight both conflicting bodies and their witness point.
Close the durable docs and visual/test coverage.

### Tasks

- **Task 3.1 — Cache geometry audits with regenerated plan state.**
  - Rebuild the physical scene/audit whenever the cached `ProjectFramePlan` changes. Clear stale
    active issues when body refs disappear after edits or undo/redo.
  - Keep audit/highlight state session-only and continue showing the scene when violations exist.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/history.rs` only if required
    by existing regeneration hooks
  - Verify: load/edit/undo/redo/regenerate tests; clean→violation→clean transition; deleted owner and
    stale member id recovery; no serialization change
  - Commit: `feat(app): cache physical geometry audits`

- **Task 3.2 — Present structured violations in diagnostics surfaces.**
  - Count geometry violations with existing error/violation styling and show both body labels,
    penetration depth, witness coordinates, and stable diagnostic code.
  - Preserve the structured geometry issue behind the row action rather than flattening the second
    body into a `PlanDiagnostic` message.
  - Files: `crates/framer-app/src/app/panels.rs`, `crates/framer-app/src/app/labels.rs`,
    `crates/framer-app/src/app/mod.rs`
  - Verify: diagnostic count/status/menu/panel tests; mixed plan/compliance/geometry diagnostics;
    deterministic ordering; empty audit; unbuildable/unsupported query display
  - Commit: `feat(app): show geometry overlap violations`

- **Task 3.3 — Focus and danger-highlight both bodies.**
  - Activating a row switches to the relevant Plan 3-D context, frames the pair/witness, danger-tints
    both bodies without destroying ordinary selection, and draws a compact witness marker.
  - Support authored assembly bodies and generated member bodies owned by walls, floors, ceilings,
    and roof planes. If one body disappears during an edit, safely clear the focus.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/camera_3d.rs`,
    `crates/framer-app/src/app/viewport/scene_build/mod.rs`,
    `crates/framer-app/src/app/viewport/scene_build/members.rs`,
    `crates/framer-app/src/app/viewport/scene_build/walls.rs`,
    `crates/framer-app/src/app/viewport/scene_build/surfaces.rs`,
    `crates/framer-app/src/app/ui_shots_tests.rs`
  - Verify: focus for member/member and assembly/assembly issues; two-body highlight; witness marker;
    ordinary selection restoration; both themes and relevant Plan 3-D UI shots
  - Commit: `feat(viewport): focus geometry overlaps`

- **Task 3.4 — Close durable documentation and perform adversarial pre-review.**
  - Update the spec status/last-reviewed date, spec index, architecture, code map, contributor audit
    command, and Definition-of-Done references where appropriate.
  - Review every build fallback, domain filter, query error, diagnostic adapter, and focus branch;
    require a test that fails if each is removed.
  - Files: `docs/specs/geometry-overlap-audit.md`, `docs/specs/README.md`,
    `docs/architecture.md`, `docs/code-map.md`, `CONTRIBUTING.md`, `AGENTS.md`
  - Verify: markdown links; full coverage-matrix review; no stale `Proposed`/`Partial` status; no
    schema-version documentation changes
  - Commit: `docs: complete the geometry overlap audit`

### PR 3 gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
scripts/ui-shots.sh
```

Inspect diagnostics and focused-overlap screenshots in both themes. Set the spec status to
**Implemented** only after app focus/highlighting, the headless/example gate, and all supported
body-family coverage are complete.

## Final verification

After PR 3 is rebased on the merged first two PRs, run the complete gate from the workspace root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
cargo run -p framer-geometry --bin geometry-audit -- examples/projects/demo-wall.framer
cargo run -p framer-geometry --bin geometry-audit -- examples/projects/demo-shell.framer
cargo run -p framer-geometry --bin geometry-audit -- examples/projects/demo-two-bedroom.framer
scripts/ui-shots.sh
```

The three PRs are sequential: PR 2 starts from merged PR 1, and PR 3 starts from merged PR 2. Each PR
must pass automated review and merge before the next begins so query, diagnostic, and UI feedback is
incorporated at the owning layer rather than carried across stacked diffs.

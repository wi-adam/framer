# Intent Model and Resolution — Implementation Plan (2026-07-15)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/intent-model-and-resolution.md](../specs/intent-model-and-resolution.md). This file
> is an archival record of how the work was sequenced; the spec is the durable source of truth.
>
> **Status:** In progress. Slice 1 merged in PR #129 and Slice 2 merged in PR
> #130. Slice 3 implementation is in progress from the resulting `origin/main`
> head; verification, review, and merge remain pending. Slice 4 remains proposed.
> The initiative started from `origin/main` at `cf8f2e0` on 2026-07-15.

## Goal

Deliver Framer's intent model incrementally without replacing `BuildingModel`, changing persisted
project data before the representation is proven, or pretending one solver can resolve every
domain. The plan first builds a read-only graph over current truth, then normalizes assertion and
outcome protocols, introduces one schema-backed cross-object intent slice, and finally adds
human-approved candidate authored changes.

The completed initiative should let a user or agent inspect why an object was generated, what
depends on an authored choice, which intents are unresolved, and which explicit alternatives are
available. Structural synthesis involving posts, beams, load paths, or engineered members remains
gated on those domain primitives being modeled honestly.

## Architecture / stack summary

Current resolution starts in `FramerApp::rebuild()` in
`crates/framer-app/src/app/mod.rs`: apply wall-local driving dimensions, then call
`framer_analysis::analyze_project`. That UI-free entry point generates the
`ProjectFramePlan`, builds and audits `framer_geometry::PhysicalScene`, evaluates
the resolved standards stack through `framer-standards`, evaluates starter-library
lifecycle state through `framer-library`, lowers both standards and lifecycle
diagnostics, and compiles the current project graph as one coherent generation.

The implementation builds on:

- `framer-core::BuildingModel`, globally unique `ElementId`, typed direct references, room
  topology, `ConstraintSystem`, and the standards `Fact`/`Predicate` vocabulary;
- `framer-solver::ProjectFramePlan`, `FrameMember.source`, `RuleProvenance`, and diagnostics;
- `framer-standards::ComplianceReport` and three-valued fact evaluation;
- `framer-geometry::BodyRef`, `GeometryAudit`, and structured violations;
- `framer-library::library_lifecycle_issues` and typed lifecycle issues; and
- app-only `ComponentKey`/`Selection`, which demonstrate identity needs but must not become the
  cross-crate contract.

The eighth workspace crate, `framer-analysis`, depends on core, library, solver,
standards, and geometry to combine these outputs without creating a lower-crate
dependency cycle. Core owns only persisted or lower-level semantic types required
below the solver. Quantitative measurement remains in `framer-standards`:
standards checks and schema-v14 project assertions consume the same
`FactSnapshot` observations instead of implementing parallel clearance, span, or
performance calculations. Analysis owns outcome/evidence/diagnostic adaptation;
the app remains the sole interactive mutation and history owner.

## Risk and coverage ledger

| Behavior | Boundary | Required proof | Likely failure if omitted |
| --- | --- | --- | --- |
| Stable project graph | core/library/solver/standards/geometry → analysis | Same input builds byte/logically identical ordered nodes and edges; missing optional evidence does not panic | Graph becomes presentation-only or nondeterministic |
| Common semantic references | core/domain crates → analysis/app | Every supported authored/generated kind resolves; authored/derived assertion namespaces cannot collide; revision-scoped generated refs reject stale use | Another app-only identity enum becomes accidental truth |
| Shared fact measurement | core/standards → analysis | A standards check and authored assertion over the same fact receive the identical observation and three-valued result | Standards and intent engines disagree about the same distance/span |
| Common outcomes/evidence | standards/solver/geometry → analysis | Exact standards mapping includes Advisory → violated preference and retains domain payload/citation | Diagnostics lose severity or information during normalization |
| Mode-specific result channels | analysis → ranking/evidence | Requirement/preference yield `IntentOutcome`; objective yields an exact scalar observation; assumption yields typed premise evidence and can make dependents unknown | Objectives become fake booleans or assumptions look inapplicable |
| Waiver normalization | core standards/project intent → analysis | `RuleOverlay::Waive` and `IntentOverride::Waive` lower to one targeted graph shape; empty/dangling/duplicate overrides reject | Waivers become assertions or silently detach from their targets |
| Persisted cross-object assertions | `.framer` → core | Positive canonical round-trip plus unknown/wrong-kind/duplicate/invalid-value rejection | Schema accepts dangling or stringly intent |
| Directional fixture clearance | core/standards → analysis | Deg0/90/180/270 front/side facts, centerline-vs-face datum, containment, open-room unknown, and standards-vs-project parity | Flagship slice cannot express real fixture requirements |
| Soft preference resolution | core/analysis | Priority ordering and stable ties are deterministic; required constraints never weaken | Vector/hash order silently chooses a design |
| Candidate authored changes | analysis → app/history | Candidates never mutate until accepted; stale graph revision rejects; accepted patch validates, is undoable, and reruns resolution | Solver silently changes topology or applies a stale option |
| Standards/geometry/library integration | analysis/app | Unsupported and unknown remain visible; source/rule/body/lifecycle evidence survives | A missing evaluator looks like success |
| Existing diagnostics integration | analysis → solver/app | Required/preference/unknown lowering follows one severity matrix; pass/N/A/waived remain report-only | App grows a second competing problems surface |
| Product-visible explanation UI | analysis → app | UI harness/screenshots cover why/impact/options and no-result states; lazy query work stays revision-cached | Graph exists but cannot answer user questions or bloats every rebuild |

### Slice 1 execution ledger

This slice begins with the following narrowed proof obligations. They refine the initiative-wide
ledger above without widening Slice 1 into the assertion/outcome work reserved for Slice 2.

| Contract at risk | Slice 1 boundary | Required focused proof before PR | Review failure prevented |
| --- | --- | --- | --- |
| Typed semantic identity | core/library/solver/standards/geometry → `framer-analysis` | Every supported authored family, generated member host/source, physical body, room schedule/boundary consequence, standards rule/report entry, and diagnostic resolves through a closed typed reference; missing or wrong-family generated host/source evidence fails closed; authored and derived assertion ids with identical text remain unequal | App-only ids, wrong-family references, or vector positions become graph truth |
| Deterministic graph revision | canonical post-propagation model + starter-library source → analysis cache | Canonical non-semantic reordering preserves the revision and graph; standards-stack reordering, graph contract changes, and starter-library availability/content-hash changes alter the revision; external input is length-delimited | Cache entries survive a semantic/external-input change or churn on harmless ordering |
| Deterministic compilation | existing regenerated outputs → project graph | Two compiles are `Eq`; nodes and edges are canonical; missing member/rule/room evidence becomes explicit unknown nodes; a missing edge endpoint returns typed `GraphBuildError` through `AnalysisError` instead of panicking; library diagnostics enter the plan before graph compilation | The graph is nondeterministic, optional evidence looks complete, or a compiler invariant crashes rebuild |
| Current evidence chain | opening/host/rule/member/report/audit/room/library/site → graph | A checked example proves opening → generated header → rule/source traceability, room schedule/boundary dependencies, matching library lifecycle status/diagnostics, site impact on solver/compliance consequences, plus one compliance or geometry issue with all participants retained | “Why generated” is only a label without machine-readable provenance |
| Lazy impact/explanation queries | graph → query cache | First and repeated dependency/dependent closures are equal, the second query is a cache hit, cycles terminate, evidence walks only toward support, `Project` is an endpoint rather than a bridge, and a changed `GraphRevision` clears cached closures | Transitive work runs every frame, crosses into downstream evidence, links unrelated project nodes, or returns stale results |
| App orchestration | analysis → `FramerApp::rebuild` | Successful rebuild installs matching domain outputs, lifecycle status, and graph; authored rebuild changes its revision; solver failure clears graph/cache but refreshes lifecycle status; selection adapters cover authored and generated selections | UI reads a graph or lifecycle status from a different document generation |
| Read-only explanation UI | app selection → inspector | UI harness and screenshot deck cover an authored object, generated member, compliance evidence, and honest no-selection/no-evidence states | The graph is inaccessible, misleading, or becomes an editing surface |
| No schema or behavior change | v13 project/core/library/solver/standards/geometry | `PROJECT_SCHEMA_VERSION` and canonical examples remain unchanged; representative plan/report/audit/lifecycle values match compilation inputs; full workspace gates remain green | Slice 1 accidentally persists derived state or changes generation |
| Rebuild/query cost | app + analysis | Record representative rebuild, first-query, and cached-query timings in the PR; candidate generation remains absent | Derived graph work silently enters the per-frame or rebuild hot path |

## Slices / phases

### Slice 1 — Read-only unified project graph, no schema change

Prove the graph shape against information Framer already computes. This slice must not add an
`intents` field to `BuildingModel`, change `.framer`, or alter framing/compliance behavior.

- **Task 1.1 — Lock semantic node and edge vocabulary for current entities**
  - Define typed graph references for current authored entities, generated members, physical
    bodies, room schedule/boundary consequences, standards rules, diagnostics, and derived reports.
  - Keep authored references independent of app selection and keep generated types out of
    `framer-core` unless a lower crate genuinely needs them.
  - Define `AssertionRef::Authored` and `AssertionRef::Derived` as type-disjoint namespaces.
    Derived ids include provider + semantic source + role and are authoritative only for the
    current graph revision; generated member/body ids are never persisted.
  - Files: new UI-free analysis module/crate after its boundary is named; `Cargo.toml` workspace
    wiring; focused adapters in `crates/framer-core`, `crates/framer-library`,
    `crates/framer-solver`, `crates/framer-standards`, and `crates/framer-geometry` only where
    necessary.
  - Verify: positive resolution for every current entity family; explicit missing-source handling;
    authored/derived ids with identical text cannot collide; compile-time exhaustive matches for
    closed kinds.
  - Commit: `feat(analysis): add typed project graph identity`

- **Task 1.2 — Compile current truth into a deterministic graph**
  - Compile ownership/reference edges from `BuildingModel`, dimension/anchor relationships,
    typed room schedule/topology consequences, member/source/rule evidence, compliance entries,
    library-lifecycle diagnostics, and geometry violations. Missing room boundary, boundary-wall,
    or schedule input becomes explicit unknown evidence.
  - Sort id-keyed graph records canonically while preserving semantic order such as standards
    stack chains.
  - Key the graph/cache by a deterministic `GraphRevision` fingerprint over the fact/evaluator
    contract version, a length-delimited deterministic starter-library availability/content-hash
    input, and the canonical post-propagation model. Reuse the project's pinned
    hashing/canonicalization helpers rather than inventing an identity algorithm. Keep the app's
    process-local `document_revision` separate.
  - Eagerly compile only the current relationships/outcomes required by diagnostics; memoize
    explanation/impact projections on first query and invalidate them on `FramerApp::rebuild()`.
  - Expose localized `incoming_intent`, `derived_from`, dependency, dependent, and `evidence_for`
    queries. Evidence traversal moves only toward whitelisted support, and `Project` is an endpoint
    rather than a bridge between unrelated project-owned entities.
  - Files: analysis graph/compiler files; tests with checked example projects.
  - Verify: two compiles are `Eq`; permuting non-semantic source vectors before canonical model
    sorting produces the same graph; semantic order remains visible; changing the starter-library
    source fingerprint changes the revision; incomplete optional evidence produces an explicit
    unknown edge/outcome; a missing graph endpoint returns typed `GraphBuildError` through
    `AnalysisError` rather than panicking; repeated queries reuse the revision cache and one
    authored edit invalidates it.
  - Commit: `feat(analysis): compile deterministic project graph`

- **Task 1.3 — Add a read-only explanation surface**
  - Add a focused inspector/panel view for "why generated" and "affected by" over selected
    authored objects and generated members. Do not add a whole-graph editing canvas.
  - Reuse or adapt app selection into graph references without making presentation state
    canonical.
  - Record a representative `rebuild()` baseline and first-query/cached-query timing in the PR;
    candidate generation remains absent from the rebuild path.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`, app tests,
    `docs/architecture.md`, `docs/code-map.md`.
  - Verify: UI harness tests for authored object, generated member, compliance evidence, and no
    available evidence; `scripts/ui-shots.sh` visual review.
  - Commit: `feat(app): explain project graph relationships`

#### Slice 1 implementation checklist

- [x] Name and wire the UI-free `framer-analysis` crate without reversing lower-crate
  dependencies; it consumes core, library, solver, standards, and geometry.
- [x] Add closed authored and revision-scoped derived graph identities, including type-disjoint
  authored/derived assertion ids and explicit unknown evidence.
- [x] Generate the plan, resolved standards, compliance report, physical scene/audit,
  `LibraryLifecycleStatus`, and graph as one coherent `ProjectAnalysis`; lifecycle diagnostics are
  lowered before graph compilation, and graph compilation remains independently fallible.
- [x] Compile canonically ordered current ownership/reference and derivation/evidence relationships,
  including the opening → generated header → solver rule/source chain, typed room schedule/boundary
  consequences, matching library lifecycle diagnostics/status, site-context impact on solver and
  compliance consequences, and compliance/geometry evidence. Missing or wrong-family generated
  hosts/sources lower to explicit unknown evidence.
- [x] Fingerprint `GRAPH_CONTRACT_VERSION`, a length-delimited deterministic starter-library
  availability/content-hash input, and canonical post-propagation project bytes; keep that revision
  separate from app document revision and bind lazy cycle-safe query closures to it.
- [x] Validate dependent and dependency endpoints at graph finalization and route typed
  `GraphBuildError` through independently fallible `AnalysisError` rather than panicking rebuild.
- [x] Restrict evidence closures to directional supporting dependencies and stop localized
  traversal at the project ownership node instead of using it as a cross-project bridge.
- [x] Adapt authored/generated app selection into `ProjectNodeRef` and add the existing inspector's
  read-only "Depends on"/"Why generated" and "Affected by" surface with honest empty/error states.
- [x] Keep schema v13, `BuildingModel`, canonical project examples, solver behavior, and persisted
  authored semantics unchanged; do not add candidate generation.
- [x] Add focused graph revision/compilation/query tests for external library-source fingerprint
  changes, typed endpoint failures, missing/wrong-family generated hosts and sources, lifecycle
  parity, typed room consequences and unknowns, site impact, directional evidence, and
  project-endpoint traversal, plus app harness and screenshot-deck states for authored, generated,
  compliance, graph-error, no-evidence, and no-selection cases.
- [x] Execute and record focused/core/app tests and full workspace format, clippy, and test gates.
- [x] Run and review `scripts/ui-shots.sh` for the inspector states.
- [x] Record representative rebuild, first-query, and cached-query timing in the PR.

Verification evidence recorded on 2026-07-15:

- `cargo fmt --all -- --check`, strict all-target/all-feature workspace clippy, and the locked
  all-feature workspace test suite passed; 1,019 tests passed and 3 manual probes were ignored.
  The workspace run included all 8 GPU parity tests and the checked-example geometry audit tests.
- `python3 scripts/check-markdown-links.py` checked 389 links with no failures. Schema remains v13,
  and `git diff -- examples` is empty; byte-exact canonical example tests passed in the workspace
  suite.
- `scripts/ui-shots.sh` rendered all 54 frames. Frames 43-47 were visually reviewed for authored
  impact, generated provenance, compliance evidence, no recorded evidence, and no selection.
- The release-mode 40-sample probe measured rebuild median 3.266 ms / p95 5.054 ms, first-query
  median 21.458 us / p95 34.375 us, and cached-query median 1.334 us / p95 2.000 us.

Slice 1's implementation exit criteria were satisfied and merged in PR #129 before Slice 2
started. At that merge point, Slices 3-4 were still proposed/not started.

**Slice 1 exit criteria:** A then-current v13 project opens byte-identically; its plan/report/audit and
existing library-lifecycle diagnostic behavior are unchanged; and graph queries can explain at
least a wall opening → header/member → rule/source chain, room schedule/boundary consequences, and
one geometry or compliance issue.

### Slice 2 — Common assertion, outcome, and evidence protocol

Normalize the cross-domain protocol while leaving specialized evaluators intact. Still no project
schema change.

- **Task 2.1 — Define the non-persisted common assertion envelope**
  - Introduce `AssertionRef`, domain vocabulary, requirement/preference/objective/assumption modes,
    deterministic scope projections, source/rationale, and qualified participants for compiled
    current intent. Prohibition normalizes to required negation; decision and waiver are not modes.
  - Dispatch results by mode: requirement/preference → `IntentOutcome`; objective → exact
    `ObjectiveObservation` with minimize/maximize direction and named vector component; assumption
    → typed `AssumptionEvidence`. Objective/assumption records never receive a synthetic boolean
    outcome.
  - Lower current `DimensionConstraint`, construction selections, site assumptions, standards
    checks, and `RuleOverlay::Waive` into the envelope/override graph without duplicating storage.
  - Give each derived assertion a deterministic provider/source/role identity and keep it disjoint
    from future persisted `AuthoredIntentId`s.
  - Files: core-owned lower-level types only if required by lower crates; otherwise analysis types
    and adapters; `crates/framer-core/src/constraints.rs` tests.
  - Verify: each current assertion has stable identity and participants; assertion namespaces do
    not collide; prohibition and equivalent required negation produce one evaluator form;
    requirement/preference dispatch to boolean outcome, objective dispatches to scalar observation,
    assumption dispatches to premise evidence, and neither objective nor assumption becomes
    `NotApplicable` merely because it is non-boolean; wall-local dimension solving remains
    byte/behavior identical.
  - Commit: `feat(intent): normalize existing authored assertions`

- **Task 2.2 — Establish one shared fact and predicate path**
  - Reuse the core-owned `Fact`, `FactOperand`, `CompareOp`, and `Predicate` vocabulary. Refactor
    `framer-standards::fact_value` behind a shared UI-free fact provider/snapshot that remains the
    sole quantitative measurement implementation.
  - Keep standards `CheckScope` as selector-only pack syntax and introduce a common sorted
    fact-subject projection that can also accept exact project participants later. Analysis calls
    the shared provider; it does not calculate a second copy of any fact.
  - Files: `crates/framer-core/src/standards.rs`, `crates/framer-standards/src/lib.rs`, analysis fact
    adapter/tests; standards spec/code map if the public contract moves.
  - Verify: existing compliance output is byte/behavior identical; a compiled assertion and a
    standards check over the same fact share the exact `FactValue`/unknown result; scope ordering is
    deterministic; missing and wrong-scope facts fail closed.
  - Commit: `refactor(standards): share facts with intent analysis`

- **Task 2.3 — Normalize domain results without erasing domain payloads**
  - Add the common `Satisfied | Violated | Unknown | NotApplicable | Waived` outcome and structured
    evidence references for requirement/preference assertions. Carry objective observations and
    assumption evidence through their separate `IntentEvaluation` variants.
  - Adapt standards outcomes, plan diagnostics, library lifecycle issues where applicable, and
    geometry violations. Keep their existing detailed types available.
  - Lock standards lowering as `Pass → Satisfied`, required `Violation → Violated`, failed
    `Advisory → Violated` on a preference-tier assertion, `NeedsReview → Unknown`, with
    `NotApplicable` and `Waived` direct. Preserve the original severity, citation, and report entry
    as evidence.
  - Lower actionable assertion outcomes into the existing `PlanDiagnostic` channel:
    required violation → `Violation`, preference/advisory violation → `Warning`, missing fact →
    `NeedsReview`, unsupported fact/evaluator → `Unsupported`; satisfied/not-applicable/waived stay
    report/inspector-only.
  - Files: analysis adapters; focused shared types; `crates/framer-standards/src/lib.rs` and
    `crates/framer-geometry` tests where public contracts change.
  - Verify: table-driven coverage for every standards outcome/severity mapping and every diagnostic
    severity mapping; waived/not-applicable compliance entries remain report-only unless the intent
    inspector requests them; geometry witness data remains recoverable.
  - Commit: `feat(analysis): unify intent outcomes and evidence`

- **Task 2.4 — Expose status and impact in the app**
  - Group selected-entity assertions by domain and outcome and show the dependency impact of an
    authored edit as a projection, not a promise that every value changes.
  - Files: app panels/actions, UI harness tests, `docs/code-map.md`.
  - Verify: status groups include unknown and waived; empty/no-evaluator states are honest;
    `scripts/ui-shots.sh`.
  - Commit: `feat(app): show intent status and impact`

#### Slice 2 implementation checklist

- [x] Add a closed, non-persisted `IntentReport` protocol with type-disjoint
  boolean, objective, and assumption records; typed domains, participants,
  sources, scopes, unknown reasons, waivers, and revision-bound evidence.
- [x] Normalize current driving dimensions, explicit construction choices, site
  premises, standards checks, non-standards diagnostics, and geometry findings
  without adding a persisted `BuildingModel` field.
- [x] Make `framer-standards::FactSnapshot` the sole fact measurement and
  predicate-evaluation path; preserve the frozen `ComplianceReport`, CSV, and
  compatibility diagnostic behavior while adding structured detailed evidence.
- [x] Retain winning waived check definitions and exact scoped waiver provenance;
  reject opening tag selectors that cannot match authored openings instead of
  silently evaluating an empty scope.
- [x] Lower actionable results into `ProjectFramePlan::diagnostics` exactly once
  through provider-specific semantics. Preserve and recover exact compliance,
  plan-diagnostic, and geometry witness payloads with stale-revision rejection.
- [x] Compile typed assertion/evidence relationships after all evidence families;
  unresolved assertion, compliance-entry, and diagnostic evidence becomes exact
  `UnknownEvidence` nodes rather than disappearing.
- [x] Add cached filtered impact projection and typed site-premise evidence while
  preserving directional generated-member explanation queries.
- [x] Treat an explicit standards `RuleRef` as authoritative over diagnostic-code
  namespace fallbacks, and materialize absent standards-referenced site flags as
  one typed unavailable premise so their evidence never becomes a dangling graph
  reference.
- [x] Replace the inspector's relationship-only block with one read-only **Intent**
  section: domain/outcome-grouped current status, authored potential impact and
  dependencies, generated **Why generated**, and honest empty, stale, solver,
  intent, graph, no-selection, and multi-selection states.
- [x] Keep geometry diagnostics owned by the structured geometry audit in the app
  while exposing the same actionable rows through the common protocol, avoiding
  duplicate counts and list rows.
- [x] Keep schema v13 and all checked example project bytes unchanged; run focused
  parity/evidence tests, strict full-workspace gates, and visual review.

Verification evidence recorded on 2026-07-15 and refreshed on 2026-07-16:

- `cargo fmt --all -- --check`, strict all-target/all-feature workspace clippy,
  and `cargo test --workspace --all-features --locked` passed. The locked suite
  passed 1,049 tests with 3 manual/visual probes ignored, including all 8 GPU
  parity tests and checked-example geometry audits.
- Focused tests pin the frozen standards CSV and empty-member jack-stud behavior;
  distinguish detailed unsupported facts from report-only needs-review; cover
  every common standards/diagnostic mapping, including explicit rule provenance
  before code-prefix fallbacks; recover duplicate geometry and plan diagnostic
  payloads exactly; reject stale references; and prove typed site, absent
  standards-referenced site-flag, and missing-evidence graph paths.
- App harness coverage passed for mixed outcomes, assumptions, generated evidence,
  unavailable/stale/empty states, graph-only failure, missing dynamic site
  premises, and geometry diagnostic de-duplication. `scripts/ui-shots.sh`
  rendered 57 frames; Intent frames 43-50
  were visually reviewed for impact, generated provenance, compliance/geometry
  evidence, all outcome groups, empty/no-selection states, and graph-only failure.
- `python3 scripts/check-markdown-links.py` checked 389 local links with no
  failures. `PROJECT_SCHEMA_VERSION` remains 13, `git diff 40ab043 -- examples`
  is empty, and canonical example round-trip tests passed.

**Slice 2 exit criteria:** Dimensions, construction selections, standards checks, and geometry
issues are queryable through one assertion/outcome/evidence protocol; standards checks and
compiled intent share one fact measurement path; actionable results use the existing diagnostics
surface; all existing domain tests and generated outputs remain unchanged.

### Slice 3 — First schema-backed cross-object intent

Add only the minimum persisted assertion set required for a real cross-domain vertical slice:
placed-object **containment and directional clearance**. This slice is locked because it exercises
design intent, semantic object relationships, shared standards facts, and construction influence.
It does not widen `ConstraintSystem`; clearances are evaluated facts and movement is a later
candidate edit.

- **Task 3.1 — Lock and serialize typed authored assertions**
  - Add core-owned `IntentAssertion`, typed authored references, domain, mode/priority, exact
    project scope, shared fact-predicate expression, source, and rationale. Do not add a parallel
    `SpatialIntent` threshold language.
  - Keep the complete `IntentDomain` vocabulary but initially validate only
    `SpatialProgram`, `Mep`, `Compliance`, and `OperationalMaintenance`. Persist
    `IntentSource::User`; use a nonzero `u16` preference priority, with larger values stronger.
    The exact scope is one furnishing/MEP instance and exactly one same-level room.
  - Keep the first persisted vertical slice to requirement/preference assertions. Objective and
    assumption result shapes exist in the compiled protocol, but their authored schema waits for a
    feature that can exercise ranking or premise consumption end to end.
  - Add `IntentOverride::Waive` targeting an `AuthoredIntentId`; empty reasons and unknown targets
    are invalid. `RuleOverlay::Waive` remains the standards rule storage form, and both compile to
    one graph waiver record/outcome. A standards overlay waiver is referenced by overlay pack +
    rule and applies to every scoped rule-instance assertion from that resolved rule.
  - Add skip-empty `BuildingModel.intents` and `intent_overrides` as id-sorted collections. Their
    ids share the global authored id pool. Do not duplicate existing direct field relationships.
  - Lock the placed-object local frame: model `+X` is right, `+Y` is up; width is left/right, depth
    is back/front, origin is the center, and Deg0 front is local `+Y`. `QuarterTurn` is
    counterclockwise, so Deg90 front is `-X`, Deg180 is `-Y`, and Deg270 is `+X`; screen coordinates
    do not change the mapping. Clearance fact requests carry `Left/Right/Front/Back/Around` and
    centerline-versus-footprint-face datum.
  - Project assertions may use exact typed refs; selector-scoped reusable policy stays in
    `StandardsPack`. `.framerlib` stays v3 and does not distribute project assertions in this slice.
  - Bump the then-current project schema from v13 to v14, update all example
    projects, explicit old-schema rejection, canonical round-trips, `project-files.md`,
    architecture, code map, and this spec.
  - Files: `crates/framer-core/src/model.rs`, a focused intent module, `src/project.rs`, public
    exports, examples, docs.
  - Verify: positive round-trip; duplicate assertion/override id; authored/derived namespace
    collision attempt; unknown/wrong-kind assertion target; unknown waiver target; empty waiver
    reason; self/empty invalid relation; invalid priority/value; unsupported domain/expression pair
    including objective/assumption in this first persisted slice; canonical assertion/override
    order; old schema rejected explicitly.
  - Commit: `feat(core): persist typed cross-object intent`

- **Task 3.2 — Evaluate containment and clearance**
  - Extend the shared core `Fact` request/subject vocabulary and the single
    `framer-standards` fact provider for placed furnishing/MEP footprints, containing room, and
    directional nearest-obstacle clearance. Both standards checks and project assertions evaluate
    those exact observations through the common `Predicate` evaluator.
  - Measure side centerline clearance independently from front/back footprint-face clearance after
    applying counterclockwise `QuarterTurn`; sweep the perpendicular footprint span against
    finished room-wall faces and every other same-level furnishing/MEP footprint. Do not reduce the
    requirement to one radial bounding-box gap. `Around` is the minimum cardinal observation.
  - Standards `CheckScope::PlacedObjects { tags }` accepts required tags from the instance or
    family and retains one subject with exact, unresolved, or ambiguous room binding.
  - An open/ambiguous room, unknown family geometry, missing wall-system input, or unsupported
    clearance participant yields `Unknown`, not pass. A closed geometric containment miss is known
    false with zero clearance and may yield `Violated` without making the project unserializable.
  - Lower actionable results into `ProjectFramePlan.diagnostics` through the mapping locked in
    Slice 2. The intent inspector/report retains satisfied, not-applicable, and waived entries plus
    all cross-object participants.
  - Files: core fact/scope vocabulary and topology helpers, `framer-standards` fact provider,
    analysis adapter/compiler, diagnostics lowering, focused tests,
    `docs/specs/standards-engine.md`, and `docs/code-map.md`.
  - Verify: containment plus front/left/right clearance at Deg0/90/180/270 using asymmetric
    obstacles; explicitly assert Deg90 front=`-X` and that left/right do not mirror; cover
    centerline-versus-face datum, nearest wall and object obstacle, satisfied/violated/open-room
    unknown/missing-capability unknown/not-applicable/waived cases, and deterministic multi-object
    ordering; the same fixture rule expressed once as a required standards check and once as
    required project intent observes the identical fact and outcome status.
  - Commit: `feat(analysis): evaluate object containment and clearance`

- **Task 3.3 — Author and inspect cross-object intent**
  - Add inspector/catalog actions for declaring required versus preferred containment/clearance,
    selecting participants, displaying rationale/source, editing/deleting assertions, waiving with
    reason where permitted, and focusing all implicated entities.
  - Waiving a project assertion writes `IntentOverride::Waive`; waiving a standards rule continues
    to write `RuleOverlay::Waive`. Both display the same compiled waived outcome with provenance.
  - All mutation uses `FramerApp::edit()` and ordinary undo/redo. Mid-authoring incomplete state is
    transient app state, not a malformed persisted assertion. The existing diagnostics panel is
    the sole problems surface; the intent inspector adds context rather than a competing list.
  - Files: app model-edit helpers, actions/panels/viewport focus, history/UI tests, screenshots.
  - Verify: create/edit/delete/waive undo/redo; project-vs-standards waiver provenance; reopen
    round-trip; invalid participant selection cannot author a bad model; one diagnostic can focus
    its primary subject and the inspector can focus every participant; `scripts/ui-shots.sh`.
  - Commit: `feat(app): author cross-object intent`

#### Slice 3 implementation checklist

- [x] Add the core-owned v14 assertion/override types, skip-empty model collections, complete
  domain vocabulary with the four-domain allowlist, user requirement/preference modes, nonzero
  priority validation, exact instance-plus-room scope validation, and global stable-id ownership.
- [x] Bump `PROJECT_SCHEMA_VERSION` to 14 and restamp the three checked examples; keep
  `.framerlib` at schema v3.
- [x] Extend the one core `Fact` vocabulary with placed-object containment and parameterized
  directional clearance, plus standards `PlacedObjects` selector scope.
- [x] Implement placed-object room binding, rotated footprints, finished-wall/other-object
  obstacles, containment, and directional clearance in `framer-standards::FactSnapshot` rather
  than analysis or app.
- [x] Adapt persisted assertions and project waivers into the common intent report, graph, and
  existing plan-diagnostic channel while leaving those outputs derived.
- [x] Add app author/edit/delete/waive controls, transient invalid-state guards,
  all-participant focus, dependent-intent cleanup, and ordinary validated edit/history routing;
  add the corresponding history, UI harness, and screenshot-deck cases.
- [x] Run focused core/standards/analysis/app tests, the screenshot deck, and the complete format,
  strict clippy, locked all-feature workspace test, and markdown-link gates on the final combined
  diff.
- [ ] Record final verification evidence, complete review, and merge the Slice 3 PR.

Pre-PR verification on 2026-07-16 passed `cargo fmt --all -- --check`, strict workspace Clippy,
the locked all-feature workspace test suite (including all eight GPU parity tests), the 61-frame
off-screen UI deck with direct review of the new authoring/waiver/focus states, the 392-link local
Markdown check, and `git diff --check`. Focused core, standards, analysis, app-history, UI-harness,
and Plan multi-selection tests also passed. PR review, latest-head CI, and merge remain pending.

**Slice 3 exit criteria:** A user can persist, reopen, evaluate, inspect, focus, edit/delete, and
waive one real cross-object containment/clearance requirement or preference; an unresolved
requirement is saveable and clearly visible.

### Slice 4 — Candidate authored changes and explicit resolution

Introduce option generation only for edits Framer can already represent and validate. Do not claim
general structural optimization.

- **Task 4.1 — Define typed authored patches and resolution options**
  - Define typed patch operations for existing safe mutations such as moving/rotating a placed
    object, changing an applicable construction system, or editing an existing parameter.
  - Each option reports satisfied, violated, waived, and unknown intents plus a deterministic
    lexicographic objective vector and evidence.
  - Arbitrary JSON patching is rejected. Graph-derived options remain disposable and carry the
    exact graph/document revision against which their tradeoffs were evaluated.
  - Files: analysis resolution/patch modules; app edit adapter; unit tests.
  - Verify: stable option ordering and tie-breaking; malformed target rejection; any intervening
    authored rebuild makes an option stale; evaluating an option does not mutate the source model;
    applying a selected current option validates and round-trips.
  - Commit: `feat(resolution): add typed authored change options`

- **Task 4.2 — Generate and preview placement-resolution options**
  - For a violated placed-object clearance, generate bounded candidates such as moving or rotating
    the existing object within its containing room. Do not add walls, routes, or framing objects in
    this first provider.
  - Preview the authored diff and its intent tradeoffs. Acceptance is one undoable edit followed by
    a full rebuild; dismissal changes nothing.
  - Files: analysis candidate provider, app option panel/viewport preview, history/UI tests.
  - Verify: unique candidate; several ranked candidates; no feasible candidate; required intent is
    never traded for a lower-tier preference; no silent application; UI shots.
  - Commit: `feat(app): preview intent resolution options`

- **Task 4.3 — Gate structural alternatives on honest prerequisites**
  - Before implementing dimensional-lumber versus engineered-member alternatives, add or link
    durable specs for authored posts/beams/supports, bearing and load paths, member capacities and
    deflection, engineered member families, and the responsible standards/analysis boundary.
  - Once those primitives exist, add candidate providers for deeper/closer dimensional members,
    changed bearing direction, added support, or engineered substitution. Every option must expose
    affected spatial, resource, construction, and compliance intent.
  - Files: new durable specs/plans first; core/solver/standards/geometry/app only in later scoped
    implementation plans.
  - Verify: structural evaluator proof appropriate to its spec, geometry audit for every accepted
    physical option, and explicit unsupported behavior outside the modeled domain.
  - Commit: deferred to the later structural feature plans.

**Slice 4 exit criteria:** Framer can propose, preview, accept, undo, and explain deterministic
authored changes for at least one supported conflict. Structural alternatives remain unavailable
and explicitly labeled until their domain prerequisites are implemented.

## Final verification

Each implementation slice runs focused owning-crate tests plus the full workspace gate before its
final commit:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```

Additional gates:

- Run `scripts/ui-shots.sh` for every intent inspector, status, impact, or option UI change.
- Run `cargo test -p framer-app --test gpu_parity --locked -- --nocapture` if option previews or
  accepted changes alter shared render/scene-building behavior.
- Run the headless `geometry-audit` command on checked examples for accepted options that alter
  physical assemblies or generated members.
- Confirm canonical `.framer` output and explicit old-schema rejection in the schema-backed slice.
- Adversarially delete each new validator, unknown branch, adapter mapping, and candidate-approval
  guard; a focused test must fail.

When a slice lands, update the durable spec's **Status** and **Last reviewed**, refresh
`docs/architecture.md`, `docs/code-map.md`, and `docs/project-files.md` where relevant, and record
which later slices remain proposed.

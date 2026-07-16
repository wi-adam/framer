# Intent Model and Resolution

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** In progress — Slices 1-2 merged; Slice 3 implementation in progress; Slice 4
> remains proposed · **Linked goal:** G-016
> (Intent Model and Resolution) ·
> **Plan:**
> [2026-07-15 — Intent Model and Resolution](../plans/2026-07-15-intent-model-and-resolution.md) ·
> **Last reviewed:** 2026-07-16

## Intent / Purpose

Framer already persists semantic building objects rather than meshes, but its authored intent is
distributed across object fields, references, dimensions, standards packs, site assumptions, and
construction-system choices. Related consequences are distributed across room topology, the
framing solver, standards evaluation, physical geometry, and diagnostics. These systems form
several implicit graphs without one shared vocabulary for identity, assertions, evidence,
outcomes, conflicts, or alternatives.

This feature introduces a unified **intent model and resolution protocol**. Framer will continue
to persist domain-specific authored objects as the canonical project model, then compile those
objects plus explicit cross-object assertions into a typed, disposable project graph. Specialized
constraint, generation, compliance, structural, and geometry evaluators contribute evidence to
that graph. The resulting resolution surface lets Framer answer:

- What was explicitly requested or selected?
- What is derived from that intent?
- Why was a particular member, assembly, or diagnostic produced?
- Which intents are satisfied, violated, unknown, inapplicable, or waived?
- What other authored objects and requirements would an edit influence?
- Which explicit authored changes could resolve a conflict, and what would each option trade off?

The near-term product remains an **intent-aware evaluator with constrained parametric
propagation**. Broad design synthesis from a room program is a later capability. Framer may
propose semantic or topological changes, but it does not silently make them.

## Conceptual vocabulary

The unified model distinguishes four classes of information:

1. **Entities** — semantic things such as a room, wall, opening, fixture, construction system,
   standards rule, generated framing member, or physical body.
2. **Assertions** — authored or authoritative statements about entities, such as containment,
   adjacency, clearance, alignment, material restrictions, performance requirements, preferences,
   objectives, and assumptions.
3. **Consequences** — regenerated facts and artifacts such as a room boundary, framing member,
   compliance result, physical solid, overlap, or diagnostic.
4. **Resolutions** — candidate or accepted changes. An accepted semantic choice becomes an
   ordinary typed authored-model edit; an accepted exception becomes an explicit override that
   targets an assertion. Resolution history is not a second persisted design model.

An entity is not itself an assertion. For example, a `Wall` is a semantic building object;
`Wall.system` records a construction selection; a requirement that every exterior wall use a
particular system is an assertion. Keeping these concepts distinct prevents "everything is an
intent" from becoming an untyped property bag.

## Requirements & behavior

### Canonical authored model and compiled graph

- `BuildingModel` remains the only persisted source of project truth. Walls, rooms, surfaces,
  systems, materials, standards packs, and placed objects keep their typed domain structs and
  fields.
- Framer compiles a deterministic, UI-free **project graph** from the authored model and the
  current regenerated outputs. The graph is disposable and is never required to recover the
  project.
- Current derived room schedules and topology boundaries are typed, revision-scoped consequence
  nodes. They link back to the authored room and deterministic boundary walls; open, unmatched, or
  absent inputs produce explicit unknown evidence instead of disappearing.
- Existing direct fields remain authoritative. The compiler exposes `Wall.system`,
  `Ceiling.region`, instance-family references, nested opening ownership, and similar fields as
  graph relationships without serializing duplicate edges.
- Persisted explicit assertions are reserved for intent that has no honest single-object owner,
  especially cross-object constraints, policies, preferences, objectives, and assumptions.
- Authored assertion ids and compiled assertion ids occupy disjoint typed namespaces:
  `AssertionRef::Authored(AuthoredIntentId)` versus
  `AssertionRef::Derived(DerivedAssertionId)`. A derived id is deterministically constructed from
  its provider, semantic source, and role; it can never collide with a persisted authored id.
- Graph nodes and relationships use typed semantic references. Vector indices, display labels,
  and app-only selection state are not project identity. Authored references are durable;
  generated-member/body references are deterministic only for the current authored, analysis-
  contract, and starter-library source inputs and must not be persisted or assumed to survive an
  authored or external analysis-input change.
- Persisted assertions may target authored entities, stable rules, or typed deterministic scopes.
  They do not target disposable render meshes or opaque cache identities.

### Graph families

The project graph exposes related subgraphs with different invariants rather than flattening all
relationships into one untyped adjacency list:

- **Ownership/reference:** hosts, contains, belongs-to, uses-system, uses-material, family-of,
  and standards-stack relationships.
- **Constraint/assertion:** assertions and the entities, parameters, conditions, and scopes they
  govern. Cycles are valid in this subgraph.
- **Derivation/evidence:** generated-from, justified-by, evaluated-from, and lowered-to
  relationships. This subgraph is regenerated and acyclic for one resolution run.
- **Conflict/alternative:** incompatible assertions, candidate authored changes, and the intents
  each candidate satisfies, violates, or leaves unknown.

The graph may be projected as ordinary binary edges for queries or visualization, but an assertion
is logically a typed hyperedge: it may relate multiple entities and also carry a threshold,
condition, strength, source, rationale, applicability, and waiver state.

### Intent domains and modes

- **Domain** describes what an assertion concerns. The initial vocabulary is spatial/program,
  construction, structural/performance, envelope/building-science, MEP, compliance, resource,
  fabrication/installation, operational/maintenance, and aesthetic intent.
- **Mode** describes how the assertion governs a design: requirement, preference, objective, or
  assumption. An accepted object/system/material selection is ordinary authored state, not a
  `Decision` mode. A waiver is an override targeting another assertion, not a mode.
- Mode also selects the result channel. Requirement and preference are boolean assertions and
  produce `IntentOutcome`. An objective produces an exact scalar `ObjectiveObservation` that feeds
  a named, direction-aware component of `ObjectiveVector`; it is not labeled satisfied or
  violated. An assumption is a typed premise with provenance that enters downstream evaluation as
  `AssumptionEvidence`; it receives no outcome of its own, and an unusable/missing premise makes
  the dependent assertion `Unknown` rather than making the assumption `NotApplicable`.
- A prohibition is authoring/UI sugar for a required negated predicate or relationship. It is
  normalized to `Requirement(Not(...))` before evaluation rather than creating a second evaluator
  path or persisted mode.
- Domain and mode are independent. "Use dimensional lumber" may be a required negated
  construction predicate or a preference that may be traded off. A room clearance may be a user
  preference or a standards-derived requirement.
- Required intent is never weakened merely because another source has a larger optimization
  weight. Waiving an assertion requires an explicit, attributable override with a non-empty
  reason.
- Preferences and objectives have deterministic priority tiers. Ordering is lexicographic by tier
  with stable tie-breaking; iteration or hash-map order never decides a design.
- Source/authority and rationale are separate from the assertion body. User intent, imported
  library policy, standards rules, and generated evidence remain distinguishable.

Schema v14 persists only the exercised boolean subset: user-authored requirements and preferences
in `SpatialProgram`, `Mep`, `Compliance`, or `OperationalMaintenance`. The domain enum remains
complete, but other authored domains fail validation until an evaluator consumes them. Persisted
preference priority is a nonzero `u16`; larger values are stronger. Objective and assumption
records remain part of the non-persisted common protocol, not authored schema variants.

### Typed scopes, predicates, and values

- An assertion scope is either the project, an exact set of typed authored references, or a closed
  deterministic selector such as "all exterior walls" or "rooms tagged accessible."
- Scope has two deliberate tiers. Project-authored assertions may contain exact `ElementId`-backed
  references. Reusable standards/library policy uses only closed selectors and never embeds
  project ids. Both resolve to the same sorted typed fact-subject set before evaluation.
- Solver-visible relationships and metrics use closed, typed enums and exact values. Strings and
  open property maps may carry descriptive extension data but are not the hidden source of
  constraint semantics.
- Quantitative intent and standards checks share the existing core-owned `Fact`, `FactOperand`,
  `CompareOp`, and `Predicate` vocabulary, plus one UI-free fact measurement implementation in
  `framer-standards`. An intent payload does not independently
  calculate a metric already expressible as a fact. New parameterized/cross-object measurements
  extend the shared fact request and subject vocabulary so the same observation feeds both a
  `ComplianceCheck` and a project-authored assertion.
- `CheckScope` remains the standards-pack selector syntax; project `IntentScope` adds exact typed
  participants. They are different persisted scope forms by design, but both lower to the same
  fact subjects and invoke the same predicate evaluator.
- Lengths remain integer ticks. Other persisted numeric facts use exact integer/fixed-point forms
  appropriate to their domain.
- Applicability and evaluation use three-valued logic where data may be missing. Unknown input
  yields an honest unknown/needs-review result rather than a silent pass or guessed value.
- Adding a new rule over an existing fact may be data-only. Adding a fact or relationship that the
  application must compute requires a typed implementation and tests.

The first persisted exact scope is deliberately narrower than the general vocabulary: its subject
is one furnishing or MEP instance, its sole participant is one same-level authored room, and both
references are kind-checked. Empty, repeated, self, wrong-kind, unknown, multi-room, or cross-level
scopes are invalid model data rather than inert assertions.

For placed objects, the shared fact vocabulary must express directional clearances. Family-local
`width` is left-to-right, `depth` is back-to-front, the family origin is its footprint center, and
the initial `Deg0` convention defines front as local `+Y`; `QuarterTurn` rotates that frame in plan.
Model plan coordinates are right-handed with `+X` right and `+Y` up. Positive quarter turns are
counterclockwise: `Deg90` maps front `+Y` to `-X`, `Deg180` maps it to `-Y`, and `Deg270` maps it to
`+X`. Screen-space Y direction does not alter this model convention. Every future 2-D/3-D object
mesh, picker, fact provider, and transform helper must use this same mapping.
Clearance facts distinguish `Left`, `Right`, `Front`, `Back`, and `Around` and state whether the
datum is an object centerline or a footprint face. This can express, for example, side clearance
from a toilet centerline separately from clear space in front of its footprint. The same rotated
facts are used regardless of whether the threshold came from a standards rule or a user assertion.
Containment tests the complete rotated rectangular family footprint. Directional clearance sweeps
the target footprint's perpendicular span to the nearest finished room-wall face or other
same-level furnishing/MEP footprint; `Around` is the minimum cardinal observation and overlapping
footprints yield zero. Wall faces come from centerline geometry plus half the authored assembly
thickness. The exact tick result conservatively floors a positive half tick.

An exact project assertion names its room. A selector-scoped standards check infers a binding only
when the instance center lies in exactly one closed same-level authored room; zero closed matches
remain unresolved and multiple matches remain ambiguous. An open room, unresolved/ambiguous
binding, missing family footprint, missing wall-system input, or unsupported cross-level input
yields `Unknown`, never a pass. A closed geometric containment miss is known `false`, with zero
clearance.

### Overrides, waivers, and accepted choices

- `RuleOverlay::Waive` remains the persisted standards-pack representation for waiving a standards
  rule. A project-authored assertion uses a separate typed `IntentOverride::Waive { target,
  reason, ... }` record because it targets an `AuthoredIntentId`, not a standards rule string.
- Schema v14 project overrides are user-authored, globally id-unique records. The target must be a
  known authored assertion, the reason must contain non-whitespace text, and at most one project
  override may target any assertion.
- Both persisted forms lower into one compiled `WaiverRecord` shape with target, authority/source,
  rationale, and provenance. The target assertion then evaluates to `Waived { waiver, reason }`;
  no independent waiver assertion is evaluated. A standards rule overlay applies that same waiver
  reference to every scoped rule-instance assertion produced by the waived resolved rule.
- An ordinary accepted choice, such as `Wall.system` or an instance position, remains in its
  existing typed field and compiles into the graph as committed authored state. A separate
  assertion is persisted only when the requirement, preference, objective, or assumption has no
  honest single-object owner.
- Applying a resolution option either edits those ordinary typed fields or adds an explicit
  override. It does not persist a generic "decision" node or duplicate change history.

### Common outcomes, observations, and conflicts

- Every evaluated requirement or preference assertion produces exactly one common outcome:
  `Satisfied`, `Violated`, `Unknown`, `NotApplicable`, or `Waived { waiver, reason }`.
- Objectives and assumptions are intentionally excluded from `IntentOutcome`. An objective emits
  `ObjectiveObservation::Known(exact_value)`, `Unknown(reason)`, or `NotApplicable`; candidate
  ranking consumes known observations through deterministic `ObjectiveVector` components and
  exposes unknown components rather than inventing a score. An assumption emits typed
  `AssumptionEvidence` only.
- `Unknown` carries a structured reason such as missing input, unsupported domain, unresolved
  reference, or unavailable evaluator. Unsupported behavior is not collapsed into success.
- A conflict identifies the smallest deterministically known set of incompatible required
  assertions when the responsible evaluator can provide it. When a minimal conflict set is not
  available, Framer reports the complete implicated set rather than guessing.
- Diagnostics are a presentation/lowering of structured outcomes and evidence. They are not the
  only machine-readable representation of a conflict.
- Standards outcomes, solver diagnostics, and geometry violations retain their domain-specific
  payloads while adapting to the common outcome/evidence protocol.
- Standards checks lower without inventing a sixth common outcome:
  `Pass → Satisfied`, required `Violation → Violated`, failed `Advisory → Violated` on a
  preference-tier assertion, `NeedsReview → Unknown`, and `NotApplicable`/`Waived` map directly.
  The original standards entry remains attached as evidence, including its severity and citation.
- Assertion diagnostics join the existing `PlanDiagnostic` channel. Violated requirements lower
  to `Violation`; violated preferences/advisories lower to `Warning`; unknown missing-input cases
  lower to `NeedsReview`; unknown unsupported-capability cases lower to `Unsupported`. Satisfied,
  not-applicable, and waived outcomes stay report/intent-inspector-only. A cross-object diagnostic
  uses its primary subject as the current single `source`; the graph retains all participants.

### Resolution behavior

Framer distinguishes four resolution behaviors instead of treating the graph as one universal
solver:

1. **Propagation** updates declared variables within existing authored degrees of freedom, such
   as satisfying compatible driving dimensions. A propagation result may apply automatically only
   when it is deterministic and does not introduce a new semantic object or construction choice.
2. **Generation** deterministically derives framing, takeoffs, topology, physical solids, and
   presentation artifacts from resolved authored choices and assertions.
3. **Evaluation** measures compliance, performance, clearance, and physical validity without
   mutating authored intent.
4. **Synthesis** creates ranked candidate authored changes. Moving a fixture, changing a system,
   adding a post, changing bearing direction, or substituting engineered lumber is never silently
   applied.

- Each evaluator declares the fact/assertion families it consumes and the facts, evidence, or
  candidate changes it produces. The orchestrator schedules evaluators through explicit phase
  dependencies rather than an uncontrolled global fixed-point loop.
- A candidate resolution is a typed authored-model patch plus the intents it would satisfy,
  violate, waive, or leave unknown and an ordered objective-cost vector.
- Candidate ordering is deterministic. Equal objective vectors use a documented stable semantic
  tie-breaker.
- Candidate patches are disposable. Accepting one routes through ordinary validated app edits and
  undo/redo, then reruns the complete resolution pipeline.
- Every candidate carries the exact graph/document revision from which it was evaluated. Applying
  it after any authored rebuild fails as stale and requires regeneration; generated node ids are
  never used as cross-revision authority.
- A candidate that changes topology, functional layout, material family, construction method, or
  an authoritative waiver always requires explicit acceptance.
- The staged work does not add inequalities, soft strengths, or project-global anchors to
  `ConstraintSystem`. Existing driving dimensions continue to use linear equality propagation;
  clearances are facts evaluated by `framer-standards`, and nontrivial movement is synthesized as
  an explicit candidate authored edit.

### Validity versus unresolved design

- Referential corruption, duplicate IDs, invalid values, and structurally unrepresentable model
  state remain `ModelError`/`ProjectError` failures and may block save.
- A legitimate but incomplete design may be saved with violated or unknown assertions. Real
  projects remain editable while clearances, spans, routes, or compliance inputs are unresolved.
- A "required" assertion means a candidate cannot be presented as fully resolved while it is
  violated; it does not automatically make the `.framer` document unserializable.
- Framer never emits one aggregate "compliant" or "correct" verdict. Resolution is reported per
  assertion and per candidate, with unsupported domains visible.

### Explanation and impact queries

- Given an authored or generated entity, callers can query incoming intent, outgoing consequences,
  supporting evidence, current outcomes, conflicts, and alternatives.
- Supporting-evidence traversal is directional: it walks from a consequence only toward
  whitelisted dependencies that justify it. It does not cross into downstream bodies or
  diagnostics. The project ownership node is a valid endpoint but never a transitive bridge between
  otherwise unrelated project-owned entities.
- A generated member can trace to its semantic source, construction selection, relevant standards
  rule/table, and the facts that selected it.
- An impact query can answer which assertions and derived entities may change if an authored
  entity or parameter changes. It reports dependency, not a guarantee that every dependent value
  will differ.
- The app presents localized "why," "affected by," and "resolution options" views. A whole-graph
  canvas may exist for diagnostics, but it is not the primary editing experience.
- The same structured graph and reports are accessible to future CLI and agent workflows without
  depending on `framer-app`.

### Rebuild and query cost

- Analysis is revision-cached and never recomputed per frame. A successful authored `rebuild()`
  invalidates the previous cache and eagerly computes only fact observations/outcomes needed for
  the existing diagnostics surface.
- Whole-graph projections, transitive impact/explanation closures, and candidate generation are
  lazy and memoized for the current deterministic `GraphRevision`; candidate providers run only on
  explicit request. The app's process-local `document_revision` remains a separate mutation guard.
  Slice 1 records a representative rebuild/query budget before the graph grows.

## Decisions (locked)

1. **Compile a graph; do not replace the domain model with one.** Typed building objects remain
   easier to validate, serialize, diff, and evolve than a generic node/property store.
2. **Do not persist duplicate relationships.** Existing object fields and nesting compile into
   graph edges; only genuinely cross-cutting assertions gain new persisted records.
3. **Assertions are first-class, qualified relationships.** A binary edge cannot honestly carry
   all participants, strength, condition, source, rationale, and waiver data.
4. **Domain and mode are orthogonal.** Design, construction, and compliance are subjects;
   requirement, preference, objective, and assumption are governing semantics. Prohibition is
   normalized negation; decisions and waivers are not modes. Requirement/preference use boolean
   outcomes, objectives use scalar observations, and assumptions use premise evidence.
5. **One quantitative fact engine.** Standards checks and explicit quantitative intent reuse the
   core `Fact`/`Predicate` language and the `framer-standards` measurement path. No second
   clearance/span/performance calculator lands in the analysis layer.
6. **Waivers are targeted overrides.** Standards overlays and project intent overrides keep their
   appropriate persisted forms but lower into one graph waiver shape and one `Waived` outcome.
7. **Authored and derived identity are type-disjoint.** Authored ids are durable. Derived ids are
   deterministic for a resolution revision but disposable across edits.
8. **One protocol, multiple evaluators.** Linear geometry, topology, standards, structural
   analysis, MEP routing, and physical geometry keep specialized deterministic implementations.
9. **Auto-propagate parameters; propose semantic changes.** Unique numeric propagation may be
   automatic. Topology and construction decisions require acceptance.
10. **Unresolved design is saveable.** Model validity and intent satisfaction are distinct states.
11. **Unknown is first-class.** Missing facts and unsupported capability fail closed to review,
   never to an inferred pass.
12. **Derived evidence is disposable.** Outcomes, graph projections, candidate patches, and
   alternative rankings regenerate from authored intent.
13. **One diagnostics surface.** Actionable intent outcomes lower into `PlanDiagnostic`; the full
    structured report/graph remains available without creating a competing problems channel.
14. **No graph database or RDF dependency is required.** The contract is a deterministic typed
    Rust model with canonical project serialization; external graph formats may be exports later.
15. **Evaluator/synthesizer before broad generative design.** The initial product explains and
    resolves an authored design. Automatic room-program layout is a separate future feature.
16. **Persist the exercised boolean slice, not dormant variants.** Schema v14 accepts user-authored
    requirement/preference fact predicates only in the four supported domains. Preferences carry
    a nonzero numeric priority (larger is stronger); objective, assumption, relationship,
    selection, and candidate-patch persistence wait for an end-to-end consumer.

## Architecture (grounded in the codebase)

### Existing seams

- `crates/framer-core/src/model.rs`: `BuildingModel` and globally unique `ElementId`s are the
  authored entity foundation. Schema v14 adds skip-empty, id-sorted `intents` and
  `intent_overrides`; validation keeps their ids in the global authored pool and rejects invalid
  exact scopes, predicates, priorities, and waiver targets/reasons. Direct references and nesting
  continue to define most ownership edges.
- `crates/framer-core/src/intent.rs`: owns the persisted assertion, waiver, exact reference,
  domain, mode, scope, source, and stable id vocabulary. It depends on no solver, standards
  evaluator, analysis, or app type.
- `crates/framer-core/src/constraints.rs`: `ConstraintSystem` provides a generic linear equality
  layer for wall-local driving dimensions, but currently has no inequalities, soft strengths, or
  project-global anchors. This feature deliberately leaves it that way; clearance is fact
  evaluation plus candidate synthesis, not a constraint-system extension.
- `crates/framer-core/src/standards.rs`: standards packs, typed facts/predicates, deterministic
  stack resolution, overlays, and rule provenance are the shared vocabulary and policy source for
  quantitative intent—not merely an analogous implementation.
- `crates/framer-core/src/topology.rs`: derives rooms and wall-side facts from a wall graph without
  persisting duplicate boundaries.
- `crates/framer-solver/src/lib.rs`: `ProjectFramePlan`, `FrameMember.source`,
  `RuleProvenance`, and `PlanDiagnostic` provide generated entities and partial evidence.
- `crates/framer-standards/src/lib.rs`: `FactSnapshot` is the single measurement
  and predicate-evaluation implementation for the core `Fact` vocabulary. Both
  compatibility `fact_value` calls, detailed standards evaluation, and exact
  project assertion evaluation use it. Its placed-object subjects retain exact,
  unresolved, or ambiguous room binding; its containment/clearance provider owns
  rotated family footprints, finished wall faces, and other-object obstacles.
  `StandardsEvaluation` pairs the unchanged deterministic `ComplianceReport`
  with structured subject, severity, predicate, synthetic-entry, and waiver
  provenance consumed by common intent analysis.
- `crates/framer-geometry`: `BodyRef`, `PhysicalScene`, and structured geometry violations provide
  stable physical identity and post-generation evidence.
- `crates/framer-analysis`: `analyze_project()` now generates the plan, resolved
  standards, detailed standards evaluation, physical scene/audit,
  starter-library lifecycle status, fallible common `IntentReport`, and fallible
  graph as one coherent UI-free generation. Persisted assertions are evaluated
  through `FactSnapshot` before report compilation; `lower.rs` adapts the one
  observation to common outcomes, targeted project waivers, evidence, and the
  existing diagnostics channel without remeasuring it. `intent.rs` owns the closed common
  assertion, mode-specific result, waiver, and evidence vocabulary; `lower.rs`
  normalizes current dimensions, construction selections, site premises,
  standards results, diagnostics, and geometry findings while preserving native
  payloads. `framer-standards::StandardsEvaluation::diagnostics()` owns detailed
  standards diagnostic lowering; `analyze_project()` appends those rows once
  before intent and graph compilation.
  `identity.rs`, `revision.rs`, `graph.rs`, `compile.rs`, and `query.rs` own the
  closed cross-domain references, typed room consequences, canonical graph,
  explicit unknown evidence, deterministic revision fingerprint, and lazy
  revision-bound directional/impact closures. An intent-report failure also makes
  the graph unavailable; graph endpoint failure can occur after a valid report,
  without discarding valid plan, standards, geometry, or lifecycle outputs.
- `crates/framer-app/src/app/mod.rs`: `rebuild()` remains the authored
  orchestration seam, applying driving dimensions before calling
  `framer_analysis::analyze_project()`. It installs or clears the common intent
  report and graph independently alongside the matching regenerated outputs and
  adapts authored selection without requiring graph availability. Transient
  authoring drafts never enter history or `.framer`; create/edit/delete/waive
  commits a sorted, validated candidate through ordinary `edit()` history, and
  participant deletion removes dependent assertions and waivers.
- `crates/framer-app/src/app/panels.rs`: the inspector consumes the common report
  for domain/outcome-grouped current status, the filtered impact projection for
  authored selections, and directional evidence traces for generated
  selections. It reports unavailable, stale, empty, and multi-selection states
  explicitly and does not author graph state.
  Slice 3 adds project assertion author/edit/delete/waive and all-participant
  focus controls; candidate resolution remains Slice 4.
- `crates/framer-app/src/app/component_visibility.rs`: `ComponentKey` demonstrates the need for
  stable authored/generated identity, but remains app-only presentation state and is not the
  cross-crate semantic reference type.
- `crates/framer-core/src/library.rs` and `crates/framer-library`: reusable content is selector-
  scoped and vendor-on-use. `framer-analysis` consumes current starter-library lifecycle issues so
  they participate in the same plan/graph generation. General intent-policy distribution is not
  part of the first project assertion schema; existing `StandardsPack` is the reusable
  quantitative-policy path.

### Persisted Slice 3 data shape

Schema v14 adds skip-empty `BuildingModel.intents` and
`BuildingModel.intent_overrides`. The persisted core-owned contract is:

```rust
pub struct IntentAssertion {
    pub id: AuthoredIntentId,
    pub domain: IntentDomain,
    pub mode: AuthoredIntentMode,
    pub scope: ProjectIntentScope,
    pub expression: IntentExpression,
    pub source: IntentSource,
    pub rationale: Option<String>,
}

pub enum AuthoredIntentMode {
    Requirement,
    Preference { priority: PreferencePriority }, // nonzero u16; larger is stronger
}

pub enum ProjectIntentScope {
    Exact(ExactIntentScope),
}

pub struct ExactIntentScope {
    pub subject: AuthoredEntityRef,              // furnishing or MEP instance
    pub participants: Vec<AuthoredEntityRef>,    // exactly one same-level room in v14
}

pub enum IntentExpression {
    FactPredicate(Predicate),
}

pub enum IntentSource { User }

pub enum IntentOverride {
    Waive {
        id: IntentOverrideId,
        target: AuthoredIntentId,
        reason: String,
        source: IntentSource,
    },
}
```

`AuthoredIntentId` and `IntentOverrideId` are transparent stable `ElementId`
wrappers and share the project's global authored id pool. `IntentDomain` carries
the complete product vocabulary; schema v14 validation currently accepts only
`SpatialProgram`, `Mep`, `Compliance`, and `OperationalMaintenance`. Only
`PlacedObjectContainedInRoom` and parameterized `PlacedObjectClearance` facts may
appear in persisted predicates, and both operate on an exact furnishing/MEP
instance plus room. Unsupported domain, mode, expression, subject, participant,
operator, operand, priority, or threshold combinations fail validation instead
of becoming inert data.

The common outcomes and graph identities remain non-persisted analysis types:

```rust
pub enum AssertionRef {
    Authored(AuthoredIntentId),
    Derived(DerivedAssertionId),
}

pub enum WaiverRef {
    Project { override_id: IntentOverrideId },
    Standards { overlay_pack: ElementId, rule: String },
}

pub enum IntentOutcome {
    Satisfied,
    Violated,
    Unknown(IntentUnknown),
    NotApplicable,
    Waived { waiver: WaiverRef, reason: String },
}

pub enum IntentRecord {
    Boolean(BooleanIntentRecord),
    Objective(ObjectiveIntentRecord),
    Assumption(AssumptionIntentRecord),
}
```

Objective and assumption result shapes therefore still exist without authored
schema variants. `ResolutionOption` and typed authored patches also remain
disposable Slice 4 analysis types; neither is part of v14.

Slice 1 implements the compiled graph as a project-wide `ProjectNodeRef` capable of addressing
authored entities, revision-scoped generated members/bodies and room schedule/boundary consequences,
rules, compliance entries, assertions, diagnostics, solver provenance, and explicit unknown evidence
without making lower crates depend on app types. Core-owned authored reference and assertion-id types
remain independent of solver or geometry types; `framer-analysis` combines all node families.
`GraphRevision` is a deterministic, disposable BLAKE3 fingerprint over domain separation,
`GRAPH_CONTRACT_VERSION`, a length-delimited deterministic starter-library source input
(`available` plus content hash, or `unavailable`), and the canonical post-propagation project bytes.
It is not the app's process-local `document_revision` and is never serialized; future candidate
application will also check current app revision before editing. `GraphBuilder::finish` separately
validates that every edge's dependent and dependency nodes exist; a missed endpoint returns typed
`GraphBuildError::MissingDependent` or `MissingDependency` through `AnalysisError::Graph` rather
than panicking.

### Crate direction

- `framer-core` owns persisted assertion types, exact values, authored references, validation, and
  deterministic canonical ordering for schema-v14 assertions and overrides. It continues to own the
  shared fact/predicate types already used by standards packs.
- `framer-standards` owns the single quantitative fact-measurement and predicate-evaluation path
  for both standards and project intent.
- Other domain crates keep their specialized solving/evaluation responsibilities and emit
  structured evidence through adapters or shared lower-level types.
- The UI-free top-level `framer-analysis` crate depends on core, library, solver, standards, and
  geometry to compile the common current outcomes and whole graph and produce
  cross-domain queries. It evaluates persisted assertions through the standards-owned snapshot;
  later slices add resolution options there. No lower crate depends on it.
- `framer-app` consumes the common report and compiled graph for the derived
  current-status/evidence inspector and remains the sole owner of interactive mutation/history.
  Slice 3 assertion authoring/focus and later Slice 4 option UI preserve that direction.

## Constraints & invariants

- Authored intent remains the only persisted project truth; all graph projections, outcomes, and
  options are regenerated.
- Same `.framer`, starter-library availability/content hash, and evaluator/graph-contract versions
  produce identical graph ordering, outcomes, evidence, options, and ranking.
- Persisted data is `Eq`, float-free, ID-sorted where order is not semantic, and agent-readable.
- Semantic lists such as standards stacks, construction layers, exact participants, and predicate
  children retain documented stable order and are never accidentally canonicalized by sorting.
- `framer-core`, `framer-library`, `framer-solver`, `framer-standards`, `framer-geometry`, and
  `framer-analysis` remain UI-free.
- Schema changes follow the complete project-file ritual: version bump, checked examples,
  round-trip and explicit old-schema rejection tests, `project-files.md`, architecture/code map,
  and this spec.
- The current project format is v14-only. `intents` and `intent_overrides` are omitted when empty,
  while non-empty collections canonicalize by id. `.framerlib` remains v3 and does not distribute
  project assertions or project ids.
- Every new reference kind has positive round-trip coverage and negative unknown/wrong-kind tests.
- Every new evaluator has satisfied, violated, unknown, and not-applicable/waived coverage where
  those outcomes are meaningful.
- Objective evaluators cover known, unknown, and not-applicable scalar observations; assumption
  lowering covers valid typed evidence and dependent-assertion unknown behavior. Neither is
  tested or displayed as a boolean outcome.
- Candidate application uses typed validated edits and ordinary undo/redo; arbitrary JSON patches
  do not bypass model invariants.
- Physical changes still pass the deterministic geometry audit, and render-visible changes keep
  CPU/GPU parity where relevant.

## Out of scope (YAGNI)

- Replacing `.framer` JSON with RDF, a graph database, or an opaque binary graph store.
- One universal solver for every intent domain.
- Arbitrary user scripting or plugins inside the canonical constraint evaluator.
- Automatic whole-building generation from a room schedule or natural-language prompt.
- Silent topology changes, material substitutions, standards waivers, or functional-layout edits.
- Claiming complete structural engineering, MEP design, energy compliance, or permit compliance
  before the necessary domain models and evaluators exist.
- Persisting regenerated alternatives, outcomes, dependency closures, or graph visualization state.
- Distributing general project `IntentAssertion`s in `.framerlib` during the initial slices.
  Reusable quantitative policy continues to ship as selector-scoped `StandardsPack` content;
  `.framerlib` remains schema v3 for Slice 3.

## Open questions

- After the project assertion/evaluator contract is proven, should reusable non-compliance policy
  remain an expanded `StandardsPack` capability or become a separate selector-only intent-policy
  library item? Either way, `.framerlib` policy must not contain project `ElementId` references.

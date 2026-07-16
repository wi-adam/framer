# Standards Engine (Layered Building Standards & Compliance)

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented ·
> **Linked goal:** G-015 (Standards Engine; subsumes G-008 Code
> Profile Data, advances G-006 Rule Explanations) ·
> **Plan:** [2026-07-04 — Standards Engine](../plans/2026-07-04-standards-engine.md) ·
> **Last reviewed:** 2026-07-16

## Intent / Purpose

Framer generates framing from construction intent, and the [vision](../vision.md) commits to
making the *rules* behind that generation explicit: every member already carries
`RuleProvenance`, and every plan carries diagnostics. But today the rules live in one
hard-coded `CodeProfile::irc_2021_prescriptive()` — a handful of defaults, one jurisdiction,
no way to verify a model against anything.

Real buildings answer to **layered standards**: semi-universal model codes (IRC, IBC),
state amendments, municipal ordinances, and project-specific requirements — covering framing,
fastening/nailing, wall bracing, seismic and wind provisions, and (later) electrical,
plumbing, and mechanical systems. The **Standards Engine** makes standards first-class,
data-driven, and layered:

- Framer **ships pre-canned packs** (IRC 2021 prescriptive first).
- Users **augment** them (add rules), **override** them (shadow a rule with their own),
  or **define their own** standards entirely — without writing Rust.
- Standards both **prescribe** (feed deterministic generation: sizing tables, spacing,
  fastening schedules) and **verify** (evaluate the finished model + derived plan and report
  rule-by-rule outcomes with citations).

This delivers G-008 ("expand the IRC 2021 starter profile into explicit rule tables and
unsupported-condition warnings") and extends G-006's rule provenance from generated members
to the full compliance surface. It preserves invariant #6: **code compliance is explicit,
never implied** — Framer reports per-rule outcomes with citations; it never claims a
building "is compliant."

## Requirements & behavior

The observable contract (prefer testable statements):

### Packs, the stack, and resolution

- A **standards pack** is a typed, versioned collection of **prescriptive tables** and
  **declarative compliance checks**, each bearing a stable **rule id** and a human
  **citation** (e.g. `IRC 2021 Table R602.3(5)`).
- The project holds an ordered **standards stack** (`base → most specific`): built-in or
  library packs first, jurisdiction overlay packs after, and optionally a project-local
  pack last. **Stack order is semantic and never sorted** (like construction-layer order);
  the pack *definitions* live in an id-sorted collection.
- **Resolution is a deterministic pure fold** over the stack: a rule id introduced by a
  later pack **shadows** the same id from an earlier pack. Shadowing is the one override
  mechanism — "augment" = add a new rule id, "override" = re-declare an existing rule id,
  "define your own" = author a base pack. Two explicit overlay actions exist besides
  shadowing: **waive** (disable a rule; a non-empty reason is required) and **re-severity**
  (downgrade/upgrade a check). The resolved rule set is **derived, never persisted**.
- Every resolved rule carries a **provenance chain** — the ordered list of (pack,
  action) that produced it — so the app can answer *"why 12″ o.c. here?"* with
  *"introduced by IRC 2021 R602.3(5); shadowed by Seattle Amendments 2024"*.
- `ResolvedStandards.checks` remains the active non-waived check set used by
  compatibility evaluators. A separate derived `check_definitions` catalog
  retains each winning post-shadow/post-reseverity check definition, including
  waived checks, so detailed evaluation can recover scope and predicate without
  changing prescription behavior.
- A waive/re-severity overlay whose target rule id matches nothing in the stack below emits
  a `standards.overlay.unmatched` **Warning** diagnostic and is skipped; it never blocks
  open or save (a jurisdiction pack may legitimately outlive the base edition it was
  written against).

### Site context and applicability

- The model gains an authored **`SiteContext`**: jurisdiction label, seismic design
  category (A–E incl. D0/D1/D2), ultimate design wind speed (integer mph), ground snow
  load (integer psf), frost depth (ticks), plus an open `properties` map for
  jurisdiction-specific values. Every field except the label is optional — **unknown is a
  first-class state**.
- Each rule has an **applicability predicate** over the site context (e.g. *active when
  SDC ≥ D0*). Applicability evaluates in three-valued logic: **applicable / not
  applicable / unknown**. A rule whose applicability is *unknown* (because a needed
  context field is unset) reports **needs-review** — it never silently passes and never
  hard-fails. Fail-closed, per invariant #6.

### Prescriptive tables (the solver consumes these)

- v1 table kinds, all typed and closed-schema:
  - **Framing defaults** — the absorbed `CodeProfile` fields (default wall height, stud
    spacing, plate/top-plate policy, stud/plate/header profiles).
  - **Stud tables** — allowed (profile × spacing) rows with height limits
    (IRC Table R602.3(5) shape).
  - **Header span tables** — (profile, plies, ground-snow band, building-width band) →
    max span + required jack studs (IRC Table R602.7(1) shape). The solver sizes each
    opening's header from the table instead of a single `default_header_depth`.
  - **Fastening schedules** — (connection kind → fastener, count or edge/field spacing)
    (IRC Table R602.3(1) shape). The solver emits fastener line items into the BOM and
    the schedule feeds connection checks.
  - **Bracing tables** — required braced length per braced-wall-line by (seismic design
    category or wind band, line spacing/length, bracing method) (IRC R602.10.3 shape).
- Generation is **table-driven where a table exists, defaults otherwise**, and each
  generated member's existing `RuleProvenance.rule_id` cites the resolved rule that sized
  or placed it (realizing G-006 with real citations).
- A condition outside a table's domain (e.g. an opening wider than any header row) emits
  an **Unsupported** diagnostic naming the table and citation — never a silent guess.

### Declarative checks (the engine verifies these)

- A **check** = rule id + citation + severity (**required** or **advisory**) +
  applicability + **scope** (an entity selector: walls, openings, rooms, levels, systems,
  braced wall lines, placed furnishing/MEP instances, generated members — with typed filters
  such as exposure, kind, and tags) + **requirement** (a predicate over a closed vocabulary of engine-computed
  **facts**).
- **Facts are typed values the engine computes** per scoped entity — wall length/height,
  actual stud spacing, header depth/plies, opening rough width, room area, ceiling height,
  clear-wall R-value (milli), braced-line required vs. provided length, member counts,
  placed-object containment, and directional placed-object clearance.
  New *rules over existing facts* are pure data (user-authorable); new *fact kinds*
  require Rust (like adding a `LayerFunction` variant).
- Predicates are a small, total, serializable tree: `all` / `any` / `not` /
  comparisons (`<`, `≤`, `=`, `≥`, `>`, `≠`) between a fact and a literal or another
  fact of the same type. No loops, no recursion beyond the data-bounded tree, no
  arbitrary code — evaluation is deterministic and bounded by construction.
- Evaluation is three-valued: **pass / fail / unknown**. A fact the engine cannot compute
  for an entity (missing data, unsupported condition) makes that rule instance
  **needs-review**, never a silent pass.
- Per rule instance, the outcome is exactly one of **pass · violation (or advisory) ·
  needs-review · waived** (waived carries its reason).

### Placed-object containment and clearance facts

- `CheckScope::PlacedObjects { tags }` selects furnishing and MEP instances in one
  canonical id order. Every requested tag may be supplied by either the instance or its
  family. Each selected object remains one fact subject even when room binding is unresolved.
- `PlacedObjectContainedInRoom` is a flag over a placed object plus room binding.
  `PlacedObjectClearance { direction, datum }` is an exact tick length with direction
  `Left`, `Right`, `Front`, `Back`, or `Around` and datum `Centerline` or
  `FootprintFace`. Standards checks and project-authored intent use these same core-owned
  `Fact` variants and the same `Predicate` evaluator.
- A family footprint is the full width-by-depth rectangle centered on the instance position.
  Model `+X` is right and `+Y` is up; `Deg0` front is local `+Y`, and `QuarterTurn` rotates
  counterclockwise (`Deg90` front is `-X`). Screen-space orientation is irrelevant.
- Directional clearance sweeps the target footprint's perpendicular span toward the nearest
  finished room-wall face or other same-level furnishing/MEP footprint. Wall centerlines are
  offset by half the authored assembly thickness; object overlap is zero clearance; `Around`
  is the minimum of the four cardinal observations. `Centerline` measures from the target
  object's centerline and `FootprintFace` from its directional footprint face.
- Selector scope binds an object to an exact room only when its center lies in exactly one
  closed same-level authored room. Zero closed matches remain unresolved; multiple matches remain
  ambiguous. Either binding, an open exact room, unknown family geometry, missing wall-system
  input, or unsupported cross-level input yields needs-review/unknown rather than pass. A closed
  geometric containment miss is a known false observation and directional clearance is zero.

### Bracing & seismic (v1 domains alongside framing/openings/fastening)

- **Braced wall lines are authored intent**: a plan-view line per level. **Braced panels
  are authored intent on walls**: a span along the wall (offset + length) plus a bracing
  **method** (closed enum: LIB, DWB, WSP, SFB, GB, PCP, HPS, CS-WSP). The user declares
  *what braces*; the engine verifies *whether it is enough*.
- The engine associates each panel to the nearest parallel braced wall line within the
  table's offset tolerance, computes provided vs. required braced length per line (from
  the bracing table, gated on seismic/wind context), and reports per-line outcomes.
  Unassociated panels and lines with zero panels are called out individually.
- Seismic requirements need no special mechanism: they are ordinary rules whose
  applicability reads the site context's seismic design category.

### Reporting

- **Violations, advisories, and needs-review flow through the existing diagnostics
  channel** (`PlanDiagnostic`), each carrying the rule id, citation, and severity, anchored
  to the offending element (`source`).
- A derived **Compliance Report** lists *every* resolved rule instance — including passes
  and waivers with reasons — with citation, outcome, affected elements, and the pack
  provenance chain. It is regenerated, disposable, deterministic (ordered by rule id, then
  element id), and exportable as CSV alongside the existing BOM/SVG exports. This is the
  artifact a user hands a plan reviewer.
- The blanket `code-profile.starter-only` warning is replaced by honest per-rule coverage:
  what was checked, what passed, what needs review, what Framer cannot evaluate.

### Distribution & self-containment

- **Standards packs are library content**: `.framerlib` gains a `standards` collection and
  packs ride the exact [Libraries](libraries.md) spine — vendor-on-use, id-remap,
  `LibraryStamp` + per-item `Provenance`, blake3 content hashes, update/divergence/detach
  detection, and (future) publisher signing. A municipality signing its amendment pack is
  the signing story's clearest use case.
- **A project always opens and verifies offline.** The stack references only vendored
  packs embedded in the `.framer`; resolution and evaluation perform zero I/O. Same
  `.framer` → same resolved rules → same plan → same diagnostics → same report, on any
  machine.
- **Packs are self-contained**: rules reference model content only via closed enums, kinds,
  and tags — never `ElementId`s — so vendoring a pack is a single-item copy with no
  subgraph closure, and a pack can ship independently of any material/system library.

## Decisions (locked)

- **Standards are data, not code.** Typed prescriptive tables + a declarative check form;
  no embedded scripting in v1. *Rejected:* tables-only (users couldn't author novel
  checks, reducing "define your own standards" to "tune our numbers") and sandboxed
  WASM/scripting now (a determinism/security/versioning burden that competes with growing
  the fact vocabulary; left architecturally open — see Out of scope).
- **Two roles, one pack: prescribe and verify.** Tables feed generation; checks verify the
  finished model + plan independently. Rationale: users hand-edit, override, and import —
  verification must not assume generation followed the same rules.
- **Shadowing is the override mechanism.** Later pack + same rule id wins, recorded in the
  provenance chain; explicit overlays are only *waive* (reason required) and *re-severity*.
  *Rejected:* field-level amend/patch operations (JSON-merge-patch semantics are subtle,
  hard to validate, and hard to explain in a provenance chain; copy-and-shadow expresses
  the same intent inspectably).
- **The resolved rule set is derived, never persisted.** Only the stack order, the vendored
  packs, and the site context are authored. Mirrors framing: intent in, derivation out.
- **Three-valued evaluation; unknown fails closed to needs-review.** A rule never silently
  passes because context is missing or a fact is uncomputable. This is invariant #6 as an
  evaluation semantics.
- **Rule ids are strings with the `ElementId` charset plus `.`** (e.g.
  `irc2021.r602.3-5.stud-spacing`), unique per pack, shadowable across packs. They are
  *not* `ElementId`s — they name rules, not model elements, and cross the pack boundary by
  design. The existing `RuleProvenance.rule_id` and `PlanDiagnostic.code` string channels
  carry them unchanged.
- **Site context is authored intent with open extension.** Closed enums/integers for what
  shipped rules reason about (SDC, wind mph, snow psf); a `properties:
  BTreeMap<String, PropertyValue>` map for jurisdiction-specific values user checks can
  reference. Same open/closed split as `Material.properties`.
- **Braced panels are authored, not generated, in v1.** The engine verifies declared
  bracing against required amounts; it does not (yet) auto-place panels. Rationale:
  bracing method is a construction choice (authored intent), and verification is the
  compliance-critical half. Auto-suggestion is future work.
- **Packs distribute via `.framerlib`, not a bespoke format.** One spine for resolution,
  vendoring, provenance, hashing, signing. *Rejected:* a dedicated `.framerstd` format
  (duplicates the machinery, splits the trust story).
- **`CodeProfile` is absorbed, not kept alongside.** The flat profile becomes the framing
  defaults table of the built-in starter pack; `BuildingModel.code` is replaced by
  `site` + the standards stack in one schema bump (v13). *Rejected:* dual sources of
  truth for the same defaults.
- **Fact vocabulary is a closed, versioned enum in Rust.** Facts must be deterministic,
  typed, and computable from `BuildingModel` + `ProjectFramePlan`; an open fact namespace
  would make pack portability and evaluation semantics undefined. The enum is the widening
  seam (add variants; never repurpose). Parameterized placed-object clearance lives in this
  shared enum rather than in a project-intent-only threshold language.
- **Check evaluation lives in a new `framer-standards` crate** (`core → solver →
  standards → app`). Core holds the data types and the pure stack resolution (mirroring
  `validate()`); the solver consumes resolved tables exactly as it consumed `CodeProfile`;
  the evaluator needs the derived plan, so it sits above the solver. *Rejected:* growing
  the evaluator inside `framer-solver` (compliance will outgrow framing — MEP, egress,
  energy) and putting resolution outside core (the solver needs resolved tables without a
  new dependency).

## Architecture (grounded in the codebase)

### Authored model (`framer-core/src/model.rs`)

Replaces `CodeProfile` (currently `model.rs:1156`) and `BuildingModel.code`:

```rust
pub struct SiteContext {
    pub jurisdiction: String,                       // display label, e.g. "Seattle, WA"
    pub seismic: Option<SeismicDesignCategory>,     // A, B, C, D0, D1, D2, E (ordered)
    pub wind_speed_mph: Option<u32>,                // ultimate design wind speed
    pub ground_snow_load_psf: Option<u32>,
    pub frost_depth: Option<Length>,                // ticks
    pub properties: BTreeMap<String, PropertyValue>, // open jurisdiction-specific values
}

pub struct StandardsPack {
    pub id: ElementId,                 // project-local id (remapped on vendor, like materials)
    pub name: String,                  // "IRC 2021 Prescriptive"
    pub edition: String,               // "2021"
    pub source: Option<Provenance>,    // vendored-from-library stamp (same as systems)
    pub tables: StandardsTables,       // typed prescriptive tables (each with rule id + citation)
    pub checks: Vec<ComplianceCheck>,  // sorted by rule id (id-keyed canonicalization)
    pub overlays: Vec<RuleOverlay>,    // waive / re-severity against earlier packs, sorted by target
    pub tags: Vec<String>,
    pub properties: BTreeMap<String, PropertyValue>,
}

pub struct StandardsTables {
    pub defaults: FramingDefaults,             // absorbed CodeProfile fields
    pub studs: Vec<StudTable>,                 // R602.3(5) shape
    pub headers: Vec<HeaderSpanTable>,         // R602.7(1) shape
    pub fastening: Vec<FasteningSchedule>,     // R602.3(1) shape
    pub bracing: Vec<BracingTable>,            // R602.10.3 shape
}

pub struct ComplianceCheck {
    pub rule: String,              // e.g. "irc2021.r602.3-5.stud-height"
    pub citation: String,          // "IRC 2021 Table R602.3(5)"
    pub title: String,
    pub severity: CheckSeverity,   // Required | Advisory
    pub applies: Applicability,    // 3-valued predicate over SiteContext
    pub scope: CheckScope,         // typed entity selector + filters (exposure, kind, tags)
    pub requirement: Predicate,    // all/any/not/compare over Facts
}

pub enum Predicate {
    All(Vec<Predicate>), Any(Vec<Predicate>), Not(Box<Predicate>),
    Compare { fact: Fact, op: CompareOp, value: FactOperand }, // literal or second fact
}

pub enum Fact {
    // existing wall/opening/room/bracing facts ...
    PlacedObjectContainedInRoom,
    PlacedObjectClearance {
        direction: ClearanceDirection,
        datum: ClearanceDatum,
    },
}

pub enum ClearanceDirection { Left, Right, Front, Back, Around }
pub enum ClearanceDatum { Centerline, FootprintFace }

pub enum RuleOverlay {
    Waive { target: String, reason: String },        // reason must be non-empty
    Severity { target: String, severity: CheckSeverity },
}

// Original standards-engine BuildingModel changes (introduced in schema v13):
//   - code: CodeProfile                     REMOVED
//   + site: SiteContext
//   + standards: Vec<ElementId>             // stack order, semantic, never sorted
//   + standards_packs: Vec<StandardsPack>   // definitions, id-sorted
//   + braced_wall_lines: Vec<BracedWallLine>
// Wall gains:
//   + bracing: Vec<BracedPanel>             // skip-empty

pub struct BracedWallLine { pub id: ElementId, pub name: String, pub level: ElementId,
                            pub start: Point2, pub end: Point2 }
pub struct BracedPanel   { pub id: ElementId, pub offset: Length, pub length: Length,
                            pub method: BracingMethod }
```

`StandardsPack::irc_2021_starter()` replaces `CodeProfile::irc_2021_prescriptive()` as the
built-in seeded by `BuildingModel::new()` / `starter_library()`, carrying real (starter-
scoped) IRC 2021 rows with per-table citations. `validate()` gains: stack entries resolve
to packs, no duplicate stack entries, pack ids pool into the global id set, rule ids unique
within a pack, waive reasons non-empty, predicates type-check (fact type vs. operand type),
table rows strictly ordered by natural key, panel spans inside their wall. Non-empty
`CheckScope::Openings.tags` is rejected because authored openings do not currently carry tags;
accepting that selector would silently produce an empty scope.

**Resolution** (pure, in core): `resolve_standards(&BuildingModel) -> ResolvedStandards` —
fold the stack; shadow by rule id; apply waive/re-severity; record per-rule provenance
chains; emit `standards.overlay.unmatched` warnings for dangling targets. `Eq`, no I/O,
no clock.

### Solver (`framer-solver/src/lib.rs`)

- Takes `&ResolvedStandards` where it took `&CodeProfile` (`generate` at `lib.rs:604`).
- Header sizing walks the resolved header table (smallest adequate row for the opening's
  rough span within snow/width bands) instead of `default_header_depth`; the chosen row's
  rule id lands in the member's existing `RuleProvenance` (`lib.rs:412`).
- Fastening schedules emit fastener BOM line items keyed by connection kind counts
  (studs×plates, plate laps, header bearings, sheathing area where known).
- Out-of-table-domain conditions emit `Unsupported` diagnostics citing the table;
  `starter_profile_diagnostics` (`lib.rs:2857`) retires in favor of per-rule coverage.

### Evaluator (new crate `crates/framer-standards`)

```
FactSnapshot::new(&BuildingModel, &ResolvedStandards, &ProjectFramePlan)
evaluate_detailed(...) -> StandardsEvaluation
evaluate(...) -> ComplianceReport
```

- **Fact engine:** `FactSnapshot` is the sole calculator for the closed `Fact`
  vocabulary from model + plan (wall/opening/room/level/system facts from
  `BuildingModel`; spacing/member/header facts from `ProjectFramePlan`;
  braced-line required/provided from the bracing tables + panel association;
  placed furnishing/MEP containment and finished-face/object clearance from
  authored room topology, family footprints, and wall systems).
  It provides canonical sorted subject projection, exact observations, and
  structured `MissingInput | UnresolvedSubject | WrongSubjectKind |
  UnsupportedCondition` reasons. Compatibility `fact_value` delegates to the
  same snapshot.
- **Check evaluator:** 3-valued predicate evaluation (Kleene semantics for
  all/any/not) per scoped entity; outcome per rule instance:
  `Pass | Violation | Advisory | NeedsReview | Waived { reason }`.
- **Detailed evaluation:** `StandardsEvaluation` pairs the unchanged legacy
  report with canonically indexed detail records carrying effective severity,
  resolved check definition, subject/scope, applicability and fact observations,
  synthetic-entry kind, and exact waiver overlay provenance. A scoped waiver
  keeps one legacy subjectless report entry while exposing one detail per current
  subject.
- **`ComplianceReport`:** every resolved rule instance with citation, outcome, elements,
  and pack provenance chain; ordered by rule id then element id; `to_csv()` export.
- **Diagnostics lowering:** compatibility `diagnostics(&ComplianceReport)` is
  preserved and cannot distinguish every missing fact from an unsupported fact.
  `StandardsEvaluation::diagnostics()` is the canonical detailed lowering path,
  so structured unsupported conditions lower to `Unsupported`; top-level analysis
  appends those rows once without duplicating them. `PlanDiagnostic.rule` retains
  the pack, rule, and citation payload.
- Crate depends on `framer-core` + `framer-solver` only; UI-free, I/O-free.

### Distribution (`framer-library`)

`.framerlib` schema bump: a `standards` collection of full `StandardsPack` items
(library-local ids). Vendoring a pack is the existing single-item pipeline — no closure,
because packs hold no `ElementId` references into other collections. Update/divergence/
detach and the provenance-excluded item hash work unchanged. The starter library gains the
IRC 2021 starter pack.

### App (`framer-app`)

Implemented authoring surface: site-context editor, stack manager (add/reorder/remove packs,
import starter-library packs, author project-local packs), and waive-with-reason controls that
write `RuleOverlay::Waive` into a project-local pack. Implemented verification surface:
a Plan-workspace compliance panel over the derived report with per-rule outcomes, element
focus, and CSV export. Detailed UX is out of this spec (see
[command-surfaces.md](command-surfaces.md) conventions).

## Constraints & invariants

Every [architecture invariant](../architecture.md) holds; specifically:

- **Determinism.** Same `.framer` → same resolved rules → same plan → same report. All
  standards data is `Eq` and float-free: lengths in ticks, R-values in milli, loads/speeds
  as integers, no clock or RNG anywhere in resolution/evaluation. Iteration orders are
  ID-sorted (packs, checks) or semantic-and-stable (stack order, table rows by natural
  key).
- **UI-free, I/O-free logic.** Data types + resolution in `framer-core`; evaluation in
  `framer-standards`; both free of UI, I/O, network. Pack *distribution* I/O stays in
  `framer-library`.
- **Authored intent is the only persisted truth.** Stack, packs, site context, braced
  lines/panels are authored; resolved standards, facts, outcomes, report are derived and
  disposable.
- **Schema discipline.** `.framer` v12 → v13 follows the full ritual
  ([AGENTS.md](../../AGENTS.md#architecture-invariants-do-not-break) #4): bump
  `PROJECT_SCHEMA_VERSION`, regenerate `examples/projects/*.framer`, round-trip +
  rejection tests, update [project-files.md](../project-files.md) and the version
  references. The `.framerlib` schema bumps independently.
  The later intent slice moves projects to v14 while reusing these fact types;
  `.framerlib` remains schema v3 because project assertions and overrides are not
  library content.
- **Compliance is explicit, never implied** (invariant #6). No aggregate "compliant"
  verdict exists anywhere in the API or UI — only per-rule outcomes, with needs-review and
  unsupported conditions labeled.
- **Library invariants** ([libraries.md](libraries.md)): vendored packs are stamped with
  the same `Provenance`; content hashes pinned blake3, full hex; provenance never
  load-bearing.

## Out of scope (YAGNI)

- **Sandboxed scripting (WASM/Rhai) for checks** the declarative form can't express.
  Architected-for: `ComplianceCheck` is a tagged enum-in-waiting (`requirement` can grow
  sibling variants), and needs-review is the honest fallback meanwhile. Revisit when real
  packs hit the vocabulary wall.
- **Field-level amend/patch overlays** — copy-and-shadow covers it; revisit only with
  evidence that pack authors need surgical patches.
- **Automatic bracing panel placement/suggestion** — v1 verifies authored bracing.
- **Engineering calculations** (load paths, shear walls beyond prescriptive bracing,
  engineered lumber sizing) — prescriptive tables only; out-of-domain conditions are
  labeled unsupported.
- **MEP rule content** (NEC/IPC/IMC) — the engine, scopes, and tags are ready, but
  electrical/plumbing/mechanical facts wait on MEP connectivity modeling (circuits,
  runs); today's MEP instances support only presence/clearance-style checks via tags.
- **Energy-code compliance** (IECC envelope/UA calculations) — the R-value fact exists;
  whole-building energy math is a separate feature.
- **Live jurisdiction data feeds / a standards registry** — distribution rides libraries;
  a registry is the libraries spec's open question, not this one's.
- **Localization of citations/units display** — model is unit-exact (ticks); display
  formatting is the app's concern.

## Open questions

- **Starter pack depth.** Which IRC 2021 rows ship in v1's built-in pack (full Table
  R602.3(5)/R602.7(1)/R602.3(1)/R602.10.3 transcriptions vs. the common-case subset)?
  Transcription is mechanical but licensing/fidelity review is prudent before shipping
  verbatim table content.
- **Fact vocabulary growth.** The closed list now covers wall length/height/exposure/system-R/
  stud facts; opening rough size/header/jack facts; room area/ceiling height; braced-line
  length/required/provided facts; and placed-object containment/directional clearance. Add the
  next fact family only when a real portable pack or authored-intent slice needs it, keeping one
  `FactSnapshot` measurement path.
- **Report format beyond CSV** — a printable (PDF/SVG) permit-support layout later?

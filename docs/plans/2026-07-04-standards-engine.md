# Standards Engine — Implementation Plan (2026-07-04)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/standards-engine.md](../specs/standards-engine.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Deliver the Standards Engine v1 across **nine PRs**, each independently green and mergeable:
layered standards packs (prescriptive tables + declarative checks), site-context
applicability, table-driven generation (headers, fastening BOM), a compliance evaluator with
a rule-by-rule report, authored bracing verified against seismic/wind-gated tables, pack
distribution via `.framerlib`, and the app surface. Schema `.framer` v12→v13 (PR 2 only).

This plan is written to be executed **one PR per agent session, in order**, by an
implementer that has NOT read the design conversation. Each PR section is self-contained:
scope, exact types, algorithms, tests, gates, and a PR title. Where the plan pins a shape
(field names, serde attributes, orderings, tie-breaks), implement it as written — the
decisions are already made in the spec; do not redesign.

## How to use this plan (instructions to the implementing agent)

1. Read [AGENTS.md](../../AGENTS.md), the [spec](../specs/standards-engine.md), and
   [project-files.md](../project-files.md) **before writing code**.
2. Implement **only your assigned PR**. Do not start the next PR's work, do not refactor
   adjacent code, do not "improve" existing behavior outside scope.
3. Every PR must leave the workspace green. Before opening the PR, run:
   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
   cargo test --workspace --all-features --locked
   python3 scripts/check-markdown-links.py   # whenever markdown changed
   ```
4. If a pinned shape in this plan does not compile against current code (names drifted),
   keep the plan's *semantics* and match the codebase's *naming*; note the deviation in the
   PR description. If a *decision* seems wrong, stop and flag it — do not silently redesign.
5. Update [code-map.md](../code-map.md) in the same PR whenever you add a module, crate, or
   public data-flow (each PR section says what to add).

## Global guardrails (apply to every PR)

- **No floats in the model or in any authored/serialized type.** Lengths are `Length`
  (integer ticks, 16 = 1 inch); R-values are milli (`i64`); wind is integer mph; snow is
  integer psf; areas are integer square inches. All model types derive
  `Debug, Clone, PartialEq, Eq, Serialize, Deserialize` and use
  `#[serde(deny_unknown_fields)]`. Maps are `BTreeMap`, never `HashMap`.
- **Serde conventions:** `Option` fields get `#[serde(default, skip_serializing_if =
  "Option::is_none")]`; `Vec`/map fields that default empty get
  `#[serde(default, skip_serializing_if = "…::is_empty")]` — mirror how `Wall`/`Material`
  fields already do it.
- **Ordering:** id-keyed collections are sorted by id at canonicalization
  (`standards_packs`, `checks` by rule id, `overlays` by target). **Semantic orders are
  never sorted**: the standards *stack* (`BuildingModel.standards`) and table *rows*
  (validated strictly ordered by natural key, author-supplied).
- **Rule ids** match `^[a-z0-9][a-z0-9.-]*$` (the `ElementId` charset plus `.`), e.g.
  `irc2021.r602.3-5.stud-height`. They are plain `String`s, **not** `ElementId`s.
- **No clock, RNG, `Date`, or I/O** anywhere in `framer-core` / `framer-solver` /
  `framer-standards` code paths.
- **Fail closed:** unknown site context or an uncomputable fact must surface as
  *needs-review* / `Unknown` — never a silent pass, never a panic. No `unwrap`/`expect` on
  data-dependent paths.
- **Every behavior bullet in a PR gets at least one test**, including the fail-closed and
  out-of-domain cases.

## Architecture / stack summary

Builds on: `CodeProfile` / `BuildingModel.code` (`framer-core/src/model.rs`, absorbed in
PR 2), `PlanDiagnostic` + `RuleProvenance` + `DiagnosticSeverity`
(`framer-solver/src/lib.rs`, extended in PR 5), the libraries spine
(`Provenance`, vendor pipeline in `framer-library`, extended in PR 7), and the
`properties: BTreeMap<String, PropertyValue>` open-map pattern. New crate in PR 5:
`crates/framer-standards`. The durable shape lives in the spec's Architecture section.

---

## PR 1 — Core standards types + pure resolution (no schema change)

**Scope:** a new `framer-core` module with all standards data types, pack validation, the
IRC 2021 starter pack, and the pure stack-resolution fold. `BuildingModel` is **not**
touched; nothing serializes into `.framer` yet. Difficulty: medium (large but fully pinned).

- **Task 1.1 — Types.** New module `crates/framer-core/src/standards.rs`, re-exported from
  `lib.rs` alongside the existing model types. Implement exactly:

  ```rust
  pub enum SeismicDesignCategory { A, B, C, D0, D1, D2, E }   // derive Ord: A < … < E
  pub struct SiteContext {
      pub jurisdiction: String,                       // display label; empty = unset
      pub seismic: Option<SeismicDesignCategory>,
      pub wind_speed_mph: Option<u32>,
      pub ground_snow_load_psf: Option<u32>,
      pub frost_depth: Option<Length>,
      pub properties: BTreeMap<String, PropertyValue>,
  }                                                    // + Default
  pub enum BracingMethod { Lib, Dwb, Wsp, Sfb, Gb, Pcp, Hps, CsWsp } // + label() -> &'static str
  pub struct BracedWallLine { pub id: ElementId, pub name: String, pub level: ElementId,
                              pub start: Point2, pub end: Point2 }
  pub struct BracedPanel { pub id: ElementId, pub offset: Length, pub length: Length,
                           pub method: BracingMethod }

  pub struct FramingDefaults {   // absorbs CodeProfile minus code/display_name
      pub default_wall_height: Length, pub default_stud_spacing: Length,
      pub double_top_plate: bool, pub default_header_depth: Length,
      pub stud_profile: BoardProfile, pub plate_profile: BoardProfile,
      pub header_profile: BoardProfile,
  }
  pub struct StudTable { pub rule: String, pub citation: String, pub rows: Vec<StudRow> }
  pub struct StudRow { pub profile: BoardProfile, pub spacing: Length,
                       pub max_height_bearing: Length, pub max_height_nonbearing: Length }
  pub struct HeaderSpanTable { pub rule: String, pub citation: String, pub rows: Vec<HeaderRow> }
  pub struct HeaderRow { pub profile: BoardProfile, pub plies: u8,
                         pub max_ground_snow_psf: u32, pub max_building_width: Length,
                         pub max_span: Length, pub jack_studs: u8 }
  pub enum ConnectionKind { StudToPlateEnd, StudToPlateToe, TopPlateLap, DoubleTopPlate,
                            SolePlateToJoist, HeaderToKingStud, SheathingEdge, SheathingField }
  pub enum FastenerSchedule { Count(u32), Spacing { on_center: Length },
                              EdgeField { edge: Length, field: Length } }
  pub struct FasteningSchedule { pub rule: String, pub citation: String,
                                 pub rows: Vec<FasteningRow> }
  pub struct FasteningRow { pub connection: ConnectionKind, pub fastener: String,
                            pub schedule: FastenerSchedule }
  pub struct BracingTable { pub rule: String, pub citation: String, pub rows: Vec<BracingRow> }
  pub struct BracingRow { pub method: BracingMethod,
                          pub max_seismic: Option<SeismicDesignCategory>,
                          pub max_wind_speed_mph: Option<u32>,
                          pub line_length: Length,      // row applies to lines ≤ this length
                          pub required_length: Length } // total braced length required
  pub struct StandardsTables { pub defaults: FramingDefaults, pub studs: Vec<StudTable>,
      pub headers: Vec<HeaderSpanTable>, pub fastening: Vec<FasteningSchedule>,
      pub bracing: Vec<BracingTable> }

  pub enum CheckSeverity { Required, Advisory }
  pub enum Applicability { Always, All(Vec<Applicability>), Any(Vec<Applicability>),
      Not(Box<Applicability>), SeismicAtLeast(SeismicDesignCategory),
      SeismicAtMost(SeismicDesignCategory), WindSpeedAtLeast(u32), SnowLoadAtLeast(u32),
      SiteFlag { key: String } }   // reads SiteContext.properties Flag entries
  pub enum Fact {                  // frozen v1 vocabulary — see the fact table in PR 5
      WallLength, WallHeight, WallIsExterior, WallStudSpacing, WallSystemRValueMilli,
      WallStudMaxHeight,
      OpeningRoughWidth, OpeningRoughHeight, OpeningHeaderDepth, OpeningJackStuds,
      OpeningHeaderMaxSpan,
      RoomAreaSquareInches, RoomCeilingHeight,
      BracedLineLength, BracedLineRequiredLength, BracedLineProvidedLength }
  pub enum FactType { Length, Int, Flag }              // Fact::value_type() -> FactType
  pub enum CompareOp { Lt, Le, Eq, Ge, Gt, Ne }
  pub enum FactOperand { LengthLiteral(Length), IntLiteral(i64), FlagLiteral(bool), Fact(Fact) }
  pub enum Predicate { All(Vec<Predicate>), Any(Vec<Predicate>), Not(Box<Predicate>),
      Compare { fact: Fact, op: CompareOp, value: FactOperand } }
  pub enum CheckScope {
      Walls { exterior_only: Option<bool>, tags: Vec<String> },
      Openings { tags: Vec<String> },
      Rooms { tags: Vec<String> },
      BracedWallLines }
  pub struct ComplianceCheck { pub rule: String, pub citation: String, pub title: String,
      pub severity: CheckSeverity, pub applies: Applicability, pub scope: CheckScope,
      pub requirement: Predicate }
  pub enum RuleOverlay { Waive { target: String, reason: String },
                         Severity { target: String, severity: CheckSeverity } }
  pub struct StandardsPack { pub id: ElementId, pub name: String, pub edition: String,
      pub source: Option<Provenance>, pub tables: StandardsTables,
      pub checks: Vec<ComplianceCheck>, pub overlays: Vec<RuleOverlay>,
      pub tags: Vec<String>, pub properties: BTreeMap<String, PropertyValue> }
  ```
  - Files: `crates/framer-core/src/standards.rs` (new), `crates/framer-core/src/lib.rs`
  - Verify: `cargo test -p framer-core` (serde round-trip of a fully-populated pack via
    `serde_json`)
  - Commit: `feat(core): standards pack, site context, and bracing data types`

- **Task 1.2 — Pack validation.** `StandardsPack::validate(&self) -> Result<(), ModelError>`
  (new `ModelError` variants, message style matching existing ones): rule-id charset; rule
  ids unique within the pack across tables **and** checks; waive reasons non-empty;
  `Predicate` type-check (`Compare` requires `fact.value_type() == value type`; a
  `FactOperand::Fact` must have the same `FactType`; `Flag` facts only allow `Eq`/`Ne`);
  scope/fact agreement (wall facts only under `Walls` scope, etc. — implement a
  `Fact::scope()` helper); table rows strictly ordered by natural key (studs:
  `(profile, spacing)`; headers: `(profile, plies, max_ground_snow_psf,
  max_building_width)`; fastening: `connection`; bracing: `(method, max_seismic,
  max_wind_speed_mph, line_length)`), duplicates rejected.
  - Files: `crates/framer-core/src/standards.rs`, error enum file
  - Verify: one failing-case test per rule above
  - Commit: `feat(core): standards pack validation`

- **Task 1.3 — Starter pack.** `StandardsPack::irc_2021_starter()` — id `std-irc-2021`,
  name `IRC 2021 Prescriptive (starter)`, edition `2021`. Defaults = the exact current
  `CodeProfile::irc_2021_prescriptive()` values. Starter table rows (a common-case subset,
  NOT full transcriptions — see the spec's open question on licensing): one `StudTable`
  (`irc2021.r602.3-5.studs`, citation `IRC 2021 Table R602.3(5)`) with 2x4/2x6 rows at
  16″/24″ (bearing 10 ft, non-bearing 14 ft for 2x4@16 as anchor row; keep rows plausible
  and clearly starter-scoped); one `HeaderSpanTable` (`irc2021.r602.7-1.headers`) with
  2x10/2x12 single+double-ply rows at 30 psf / 36 ft width bands; one `FasteningSchedule`
  (`irc2021.r602.3-1.fastening`) with `StudToPlateEnd` (2×16d, `Count(2)`),
  `StudToPlateToe` (4×8d, `Count(4)`), `DoubleTopPlate` (16d @ 16″ o.c.,
  `Spacing { on_center: 16" }`), `TopPlateLap` (8×16d, `Count(8)`). No checks yet (PR 5)
  and no bracing rows yet (PR 6). Keep `checks: vec![]`, `overlays: vec![]`.
  - Verify: `irc_2021_starter().validate()` passes; a golden test asserts its canonical
    JSON is stable (guards accidental churn)
  - Commit: `feat(core): IRC 2021 starter standards pack`

- **Task 1.4 — Resolution.** Pure fold, no model dependency yet:
  ```rust
  pub struct ResolvedRule { pub pack: ElementId, pub rule: String, pub citation: String,
      pub severity: Option<CheckSeverity>,          // None for tables
      pub waived: Option<String>,                   // waive reason
      pub chain: Vec<(ElementId, ResolutionAction)> } // Introduced | Shadowed | Waived | Reseverity
  pub struct ResolvedStandards {
      pub defaults: FramingDefaults,                 // last pack in stack that set them wins
      pub studs: Vec<(ElementId, StudTable)>, /* headers, fastening, bracing likewise */
      pub checks: Vec<(ElementId, ComplianceCheck)>, // post-shadow, severity applied, waived excluded from evaluation but retained for the report
      pub rules: Vec<ResolvedRule>,                  // every rule id, sorted by rule id
      pub warnings: Vec<(String, String)> }          // (code, message): "standards.overlay.unmatched"
  pub fn resolve_standards(stack: &[&StandardsPack]) -> ResolvedStandards
  ```
  Semantics (all locked by the spec): iterate packs in stack order; a rule id introduced
  later **shadows** the earlier one entirely (tables and checks alike); `Waive`/`Severity`
  overlays apply to the rule id as resolved *so far* — an unmatched target appends a
  `standards.overlay.unmatched` warning and is skipped; chains record every action in
  stack order. Output vectors sorted by rule id; determinism test runs the fold twice on a
  3-pack stack and asserts equality.
  - Verify: tests — shadow order, waive (reason surfaces), re-severity, dangling target
    warning, chain contents, defaults last-writer-wins
  - Commit: `feat(core): deterministic standards stack resolution`
- **Docs:** add `standards.rs` to [code-map.md](../code-map.md)'s framer-core section.
- **PR title:** `feat(core): standards engine data model and resolution (G-015, 1/9)`

## PR 2 — Schema v13: the model carries standards (mechanical, wide)

**Scope:** replace `CodeProfile` with the standards stack in `BuildingModel`, bump the
schema, update every call site. **Behavior must not change**: framing output for the
example projects is identical before/after (defaults are value-equal). Difficulty:
mechanical but wide — touches many tests.

- **Task 2.1 — Model wiring.** In `BuildingModel`: remove `code: CodeProfile`; add
  `site: SiteContext` (default), `standards: Vec<ElementId>` (stack order, semantic),
  `standards_packs: Vec<StandardsPack>` (id-sorted at canonicalization),
  `braced_wall_lines: Vec<BracedWallLine>` (skip-empty). `Wall` gains
  `bracing: Vec<BracedPanel>` (default + skip-empty). Delete `CodeProfile` and
  `PrescriptiveCode`. `BuildingModel::new()` seeds `standards_packs =
  vec![StandardsPack::irc_2021_starter()]`, `standards = vec![that id]`, default site.
  Add `BuildingModel::resolved_standards(&self) -> ResolvedStandards` (resolves the stack
  refs; a dangling stack entry is a `ModelError` from `validate()`). Extend `validate()`:
  stack entries resolve and are duplicate-free; pack ids pool into the global id set;
  every pack passes `StandardsPack::validate()`; panel spans fit inside their wall
  (`offset + length ≤ wall length`, positive length); `BracedWallLine.level` resolves.
- **Task 2.2 — Call sites.** `Wall::new(…, &CodeProfile)` → takes `&FramingDefaults`.
  Solver `generate*` signatures take `&ResolvedStandards` instead of `&CodeProfile` and
  read **only** `resolved.defaults` in this PR (a mechanical swap). The
  `code-profile.starter-only` diagnostic text now uses the base pack's `name` (the first
  stack entry); it is retired later (PR 5). Update `framer-app` call sites and every test
  (`CodeProfile::irc_2021_prescriptive()` → seed a model / use
  `StandardsPack::irc_2021_starter().tables.defaults`).
- **Task 2.3 — Schema bump ritual** (checklist, all in this PR):
  1. `PROJECT_SCHEMA_VERSION` 12 → 13 in `crates/framer-core/src/project.rs`.
  2. Regenerate every `examples/projects/*.framer` (load-with-migration is NOT a thing —
     hand-update the JSON: replace the `"code"` object with `"site"`, `"standards"`,
     `"standards_packs"`; run the round-trip test to confirm canonical form).
  3. Round-trip fixtures updated; add a v12-rejection test (old header → load error).
  4. Update [project-files.md](../project-files.md) (new authored keys + v13),
     `crates/framer-core/README.md`, [architecture.md](../architecture.md),
     [code-map.md](../code-map.md) version references.
- Verify: full gates; plus a before/after check that the example projects' generated plan
  JSON is unchanged from `main` (run the solver on each example on both branches — this is
  the PR's core acceptance criterion; put the assertion in the PR description).
- **PR title:** `feat(core)!: .framer v13 — standards stack replaces code profile (G-015, 2/9)`

## PR 3 — Table-driven header sizing

**Scope:** the solver sizes opening headers from the resolved header tables. Difficulty:
medium.

- **Selection algorithm (pinned):** for an opening with rough span `s` on an exterior or
  bearing wall: eligible rows are those in every resolved `HeaderSpanTable` where
  `max_span ≥ s`, and — when the site context knows them — `max_ground_snow_psf ≥
  site.ground_snow_load_psf`. Unknown snow load ⇒ only rows in the **highest** snow band
  are eligible (conservative). Building width is not modeled ⇒ use the **widest**
  `max_building_width` band (conservative; leave a `// building width: conservative band`
  comment). Among eligible rows pick: smallest `max_span`, then fewest `plies`, then
  shallowest `profile`. The chosen row sets header profile, plies (generate that many
  header members), and `jack_studs` (generate that many jacks per side).
- **No eligible row** ⇒ emit `DiagnosticSeverity::Unsupported` diagnostic
  `standards.header.out-of-domain` citing the table's `citation`, and fall back to the
  resolved `defaults.default_header_depth`/`header_profile` (current behavior).
- **Provenance:** the header members' existing `RuleProvenance.rule_id` = the table's
  `rule`; `summary` includes the citation and the row chosen (e.g.
  `"2-ply 2x10 ≤ 6'0\" span — IRC 2021 Table R602.7(1)"`).
- Files: `crates/framer-solver/src/lib.rs`
- Verify: unit tests — row selection incl. tie-breaks, unknown-snow conservatism,
  out-of-domain fallback + diagnostic, provenance rule id on header members.
- **PR title:** `feat(solver): table-driven header sizing with citations (G-015, 3/9)`

## PR 4 — Fastening schedules → BOM

**Scope:** fastener take-off line items derived from resolved fastening schedules.
Difficulty: low.

- Connection counting from the generated plan, v1 (pinned):
  - `StudToPlateEnd`: per wall, `stud_count × 2` (top + bottom plate) connections.
  - `TopPlateLap`: one connection per top-plate butt joint (count plate pieces − 1 per
    plate line, when stock-length breaks exist).
  - `DoubleTopPlate`: when the system/defaults use a double top plate — spacing-based:
    `ceil(wall_length / on_center)` fasteners.
  - `HeaderToKingStud`: 2 connections per opening (one per side) when a header exists.
  - `SheathingEdge`/`SheathingField`: **skip** (no panel layout is generated yet); emit a
    single `Info` diagnostic `standards.fastening.sheathing-not-counted` per project when
    the schedule has sheathing rows.
- For `Count(n)` rows: `quantity = connections × n`. For `Spacing`: as above. Aggregate
  per (fastener string, connection kind) into a new plan output:
  `pub struct FastenerTakeoff { pub fastener: String, pub connection: ConnectionKind,
  pub quantity: u32, pub rule: String, pub citation: String }` on the project plan
  (`fasteners: Vec<FastenerTakeoff>`, sorted by `(fastener, connection)`), included in the
  CSV BOM export as a distinct section.
- Files: `crates/framer-solver/src/lib.rs` (+ the CSV export module, near the existing
  `layer_bom()` take-off code)
- Verify: BOM assertions on an example project (exact expected quantities for a known
  wall), determinism (two runs equal), CSV golden updated.
- **PR title:** `feat(solver): fastening schedule BOM take-off (G-015, 4/9)`

## PR 5 — `framer-standards` crate: facts, evaluator, report, diagnostics

**Scope:** the verification half. New crate `crates/framer-standards`
(deps: `framer-core`, `framer-solver`, `serde`; UI-free, I/O-free). Difficulty: **high —
assign the strongest available implementer.** The three-valued semantics below are exact.

- **Task 5.1 — Tri-valued logic.** `enum Tri { False, Unknown, True }` with Kleene
  semantics, `Ord` as listed: `not(x)` flips True/False, Unknown fixed;
  `all(xs) = min(xs)` (empty ⇒ True); `any(xs) = max(xs)` (empty ⇒ False).
  Applicability evaluation → `Tri` (`SeismicAtLeast` with `site.seismic == None` ⇒
  Unknown; `SiteFlag` missing key ⇒ Unknown, non-Flag value ⇒ Unknown).
- **Task 5.2 — Fact engine.** `fn fact_value(fact, entity, model, resolved, plan) ->
  Option<FactValue>` (None = unknown). The frozen v1 fact table:

  | Fact | Type | Source (exact) |
  | --- | --- | --- |
  | WallLength / WallHeight | Length | authored `Wall` |
  | WallIsExterior | Flag | system `exposure()` == Exterior |
  | WallStudSpacing | Length | wall system's `FramingSpec.spacing` |
  | WallSystemRValueMilli | Int | `system.r_value_milli(materials)` |
  | WallStudMaxHeight | Length | resolved stud tables: row matching (profile, spacing); bearing walls (exterior ⇒ bearing in v1) use `max_height_bearing`, else non-bearing; no row ⇒ None |
  | OpeningRoughWidth / RoughHeight | Length | authored `Opening` |
  | OpeningHeaderDepth | Length | generated header member depth from the plan; no header ⇒ None |
  | OpeningJackStuds | Int | generated jack count per side from the plan |
  | OpeningHeaderMaxSpan | Length | resolved header tables: the PR-3 selection for this opening's chosen profile/plies; none ⇒ None |
  | RoomAreaSquareInches | Int | room schedule area; ticks² → in² via floor(/256) |
  | RoomCeilingHeight | Length | the room's level `height`; zero ⇒ None |
  | BracedLine* | — | **stubbed None in this PR**; implemented in PR 6 |

- **Task 5.3 — Evaluation.** For each non-waived resolved check: applicability `Tri`
  (False ⇒ instances skipped, rule reported `NotApplicable` at rule level — represent as
  outcome on a single element-less entry); scope selection (typed filters; tag filters
  require **all** listed tags); requirement per entity → `Tri`. Outcome per instance:
  True ⇒ `Pass`; False ⇒ `Violation` (severity Required) or `Advisory`;
  Unknown ⇒ `NeedsReview`. Waived rules ⇒ one `Waived { reason }` entry, no evaluation.
- **Task 5.4 — Report + CSV.**
  ```rust
  pub struct ComplianceEntry { pub rule: String, pub citation: String, pub pack: ElementId,
      pub outcome: Outcome, pub element: Option<ElementId>, pub message: String,
      pub chain: Vec<(ElementId, ResolutionAction)> }
  pub struct ComplianceReport { pub entries: Vec<ComplianceEntry> } // sorted (rule, element)
  pub fn evaluate(model, resolved, plan) -> ComplianceReport;      // pure
  impl ComplianceReport { pub fn to_csv(&self) -> String }         // header row + one line per entry
  ```
- **Task 5.5 — Diagnostics lowering.** In `framer-solver`: add
  `DiagnosticSeverity::{Violation, NeedsReview}`; add `pub rule: Option<RuleRef>` to
  `PlanDiagnostic` (`RuleRef { pack: ElementId, rule: String, citation: String }`,
  skip-if-none). In `framer-standards`: `fn diagnostics(&ComplianceReport) ->
  Vec<PlanDiagnostic>` — Violation/Advisory(→Warning)/NeedsReview entries only; Pass and
  Waived stay report-only. Retire the blanket `code-profile.starter-only` warning from the
  solver (it is superseded by per-rule coverage).
- **Task 5.6 — Starter checks** (added to `irc_2021_starter()`, evaluable green-or-honest
  on the example projects):
  1. `irc2021.r602.3-5.stud-height` / Required / Walls / `WallHeight ≤
     Fact(WallStudMaxHeight)` — cites Table R602.3(5).
  2. `irc2021.r602.7.header-span` / Required / Openings / `OpeningRoughWidth ≤
     Fact(OpeningHeaderMaxSpan)` — cites R602.7(1).
  3. `irc2021.r305.1.ceiling-height` / Advisory / Rooms tagged `habitable` /
     `RoomCeilingHeight ≥ 7'0"` — cites R305.1 (needs-review when level height unset).
- **Docs:** new crate section in [code-map.md](../code-map.md); repo-map row in
  [AGENTS.md](../../AGENTS.md).
- Verify: Kleene truth-table tests; per-fact known/unknown tests; outcome mapping tests;
  golden CSV on an example project; determinism (two evaluations byte-equal CSV);
  workspace gates.
- **PR title:** `feat(standards): compliance evaluator, report, and diagnostics (G-015, 5/9)`

## PR 6 — Bracing + seismic verification

**Scope:** braced-line facts, panel association, bracing table rows + checks. Difficulty:
high (geometry + table semantics; pinned below).

- **Association (pinned, deterministic):** a `BracedPanel` belongs to the braced wall line
  on the same level that is (a) **parallel** to the panel's wall (exact integer cross
  product of direction vectors == 0), (b) within **4 ft** perpendicular distance from the
  panel's midpoint (integer arithmetic; distance² compare in i128 to avoid overflow), and
  (c) nearest by that distance; ties → lowest line id. Panels with no qualifying line are
  **unassociated**.
- **Facts (replace PR 5 stubs):** `BracedLineLength` = line segment length.
  `BracedLineProvidedLength` = sum of associated panel lengths.
  `BracedLineRequiredLength` = from resolved bracing tables: eligible rows match the
  line's panels' methods (per method: rows where `max_seismic ≥ site SDC` when row has
  one, `max_wind_speed_mph ≥ site wind` when row has one), pick the row with smallest
  `line_length ≥` actual line length; required = **max across the methods used on the
  line** (conservative). SDC unknown while any eligible row is seismic-gated ⇒ None
  (needs-review). Line longer than every row ⇒ None + an `Unsupported`
  `standards.bracing.out-of-domain` diagnostic.
- **Starter rows + checks** in `irc_2021_starter()`: one `BracingTable`
  (`irc2021.r602.10-3.bracing`, citation `IRC 2021 R602.10.3`) with starter WSP + GB rows
  across SDC bands (≤C, D0–D2) at 10/20/30/40 ft line lengths; checks:
  4. `irc2021.r602.10.braced-length` / Required / BracedWallLines /
     `BracedLineProvidedLength ≥ Fact(BracedLineRequiredLength)`.
  5. `irc2021.r602.10.line-has-panels` / Advisory / BracedWallLines /
     `BracedLineProvidedLength > 0"`.
  Unassociated panels ⇒ one Advisory diagnostic each (`standards.bracing.unassociated-panel`).
- Files: `crates/framer-standards/src/lib.rs`, `crates/framer-core/src/standards.rs`
  (starter rows/checks)
- Verify: association tests (parallel/tolerance/tie-break), required-length lookup across
  SDC bands, unknown-SDC ⇒ NeedsReview, out-of-domain, multi-method max, determinism.
- **PR title:** `feat(standards): braced wall line verification (G-015, 6/9)`

## PR 7 — Distribute packs via `.framerlib`

**Scope:** packs as library content on the existing spine. Difficulty: medium (follow the
established patterns in `framer-library` exactly).

- `.framerlib` schema bump (follow the existing bump pattern + header-reject test): add
  `standards: Vec<StandardsPack>` (library-local ids). `Library::validate()` runs
  `StandardsPack::validate()` per pack.
- Vendor pipeline: a pack is a **single-item copy** — remap only `pack.id` (existing
  namespace-remap: library-id prefix + lowest-free numeric suffix), stamp
  `source: Some(Provenance { … })`. There is no subgraph closure: packs hold no
  `ElementId` references to materials/systems (`CheckScope` filters are tags/enums only) —
  add a test asserting vendoring a pack adds exactly one element.
- Update/divergence/detach: reuse the provenance-excluded item-hash machinery unchanged
  (`source: None` during hashing, library-local id space). Starter library gains the IRC
  2021 pack.
- Docs: [libraries.md](../specs/libraries.md) collection list + `.framerlib` snippet;
  [code-map.md](../code-map.md).
- Verify: `cargo test -p framer-library` — vendor round-trip, divergence detection on an
  edited vendored pack, re-sync, detach, schema-reject.
- **PR title:** `feat(library): distribute standards packs (G-015, 7/9)`

## PR 8 — App: site context + stack management

**Scope:** authoring UI. Difficulty: medium (egui; follow existing inspector patterns and
[command-surfaces.md](../specs/command-surfaces.md); all mutations via the existing
undoable `edit()` path).

- Site-context editor in the inspector: jurisdiction text field; SDC dropdown (incl.
  "Unknown"); integer fields for wind/snow; frost depth as a length field. Every change
  one undo step.
- Standards stack panel: ordered list (base at top); reorder/remove; "Add pack" from the
  starter library (vendor flow from PR 7); "New project pack" (creates an empty local pack
  appended to the stack); waive UI on a rule (writes `RuleOverlay::Waive` into the
  project-local pack, creating it if absent; reason text required — disable confirm when
  empty).
- Headless `egui_kittest` coverage where the harness allows (see the CI notes on
  `ui_root`/font warm-up in [build-and-ci.md](../specs/build-and-ci.md)); otherwise unit-test the
  edit-ops the UI invokes.
- **PR title:** `feat(app): site context and standards stack authoring (G-015, 8/9)`

## PR 9 — App: compliance panel + report export

**Scope:** verification surface. Difficulty: medium.

- Compliance panel: entries grouped by outcome (Violations, Needs review, Advisories,
  Waived, Passed — collapsed by default for Passed), each row shows rule, citation,
  message; clicking focuses/selects the source element (existing selection focus path).
  Re-evaluates whenever the plan regenerates (derived, like framing).
- "Export compliance report (CSV)" action beside the existing BOM export, writing
  `ComplianceReport::to_csv()`.
- Update the spec's **Status** to Implemented, **Last reviewed**, and close out
  [code-map.md](../code-map.md) / [project-files.md](../project-files.md) drift.
- **PR title:** `feat(app): compliance panel and report export (G-015, 9/9)`

---

## Final verification (after PR 9)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```

Plus: v13 example round-trips, report CSV golden, GPU parity suite still green
(`cargo test -p framer-app --test gpu_parity -- --nocapture`), and the spec/docs close-out
in PR 9.

# framer-analysis

`framer-analysis` is Framer's UI-free, top-level derived-analysis layer. It depends
on `framer-core`, `framer-library`, `framer-solver`, `framer-standards`, and
`framer-geometry` to compile the canonical authored `BuildingModel` and current
regenerated outputs into one deterministic, disposable project analysis.

`analyze_project(&BuildingModel)` returns a `ProjectAnalysis` containing the plan,
resolved standards, detailed standards evaluation, physical scene and audit,
`LibraryLifecycleStatus`, a fallible `IntentReport`, and a fallible project graph.
An intent-report failure also makes the graph unavailable; graph endpoint
finalization can fail after a valid report, leaving current status usable.
Starter-library lifecycle warnings are lowered into the plan before intent and
graph compilation, so the returned lifecycle status, plan diagnostics, intent
outcomes, and graph describe the same generation.
`library_lifecycle_status(&BuildingModel)` remains available when framing cannot
solve.

The crate owns the non-persisted common intent protocol, normalization of current
domain results, cross-domain graph identity, revision fingerprints, graph
compilation, and lazy explanation/impact query caching. Boolean requirements and
preferences use common outcomes; objectives retain exact scalar observations;
assumptions retain typed premise evidence rather than receiving invented boolean
states. It never mutates or serializes authored intent. Lower crates do not depend
on it; `framer-app` is currently its only workspace consumer.

Placement resolution is a separate explicit path, never part of
`analyze_project()`. `generate_resolution_options` performs a bounded deterministic
search for an existing furnishing/MEP pose, measures containment and clearance
only through `framer-standards::FactSnapshot`, rejects required-intent regressions,
and returns revision-bound typed patches with categorized boolean outcomes, exact
named objective observations, typed assumption evidence, and lexicographic costs.
Search metadata discloses its fact-measurement/full-analysis caps and whether
pose measurement or candidate ranking was truncated; an empty bounded result does not claim
mathematical infeasibility. Preview/staging clones and validates the authored
model; application remains the app's explicit undoable edit. `GraphRevision`
identifies the immutable evaluation/cache result, while an explicit same-graph
request may reauthorize it for a newer process-local document revision; an
already displayed set remains stale. Structural alternatives are reported
unavailable until their authored and evaluated prerequisites exist.

`GraphRevision` is BLAKE3 over domain separation, `GRAPH_CONTRACT_VERSION`, a
length-delimited deterministic starter-library source input (`available` plus
its content hash, or `unavailable`), and canonical project bytes. Changing the
authored model, graph contract, or available bundled library source therefore
invalidates revision-scoped identities and cached closures.

Graph finalization validates that both endpoints of every edge exist. Missing
domain evidence should normally compile to an explicit `UnknownEvidence` node;
if the compiler instead emits an edge with a missing endpoint,
`GraphBuilder::finish` returns typed `GraphBuildError::MissingDependent` or
`MissingDependency`. That error flows through `AnalysisError::Graph` in the
fallible `ProjectAnalysis.graph` result rather than panicking or discarding the
otherwise valid plan, intent report, standards evaluation, geometry, and
lifecycle status.

Generated-member hosting is an ownership relationship, while `derived_from` follows only
derivation/evidence edges. `evidence_for` is directional: it walks from a consequence toward its
whitelisted supporting evidence and never crosses into downstream bodies or diagnostics. The
project ownership node may appear as a traversal endpoint but is never a transitive bridge between
unrelated project-owned entities.
Generated member hosts and sources are both kind-checked; missing or wrong-family values become
typed unknown evidence instead of false provenance. Solver provenance references site context, and
compliance entries are evaluated from it, so site impact reaches their downstream consequences.
Regenerated room schedules and topology boundaries are revision-scoped consequence nodes linked
to their authored room and deterministic bounding walls; open or inconsistent inputs remain
explicit unknown evidence.

Current authored driving dimensions, explicit construction selections, site
premises, standards checks, non-standards diagnostics, and geometry findings are
lowered into one canonical `IntentReport`. Standards facts are measured only by
`framer_standards::FactSnapshot`; analysis consumes its detailed observations
instead of implementing a second fact calculator.
`framer_standards::StandardsEvaluation::diagnostics()` owns detailed standards
diagnostic lowering; analysis installs those rows once and links them as evidence,
so normalized intent does not duplicate plan diagnostics.

`GraphQueryCache::impact_of` returns only assertion traces and user-relevant
generated consequences for an authored entity. It is dependency evidence, not a
prediction that every returned value will change. `evidence_for` remains the
directional query for explaining a generated selection.

## Entry points

- `analyze_project(model) -> Result<ProjectAnalysis, SolverError>` regenerates
  one coherent derived generation and compiles its common intent report and
  project graph.
- `ProjectAnalysis.intent_report` exposes normalized current direct intent and
  evaluator results without requiring graph finalization.
- `GraphQueryCache::impact_of(graph, start)` returns cached, filtered
  assertion and derived-result impact traces for one authored entity.
- `generate_resolution_options(model, revision, request, cache)` explicitly
  synthesizes and memoizes bounded placement-clearance options for the exact graph
  and app document revision.
- `stage_resolution_option(model, option, current_revision)` rejects stale
  authority/expected values and returns a sorted, validated candidate model
  without mutating its source.
- `library_lifecycle_status(model)` evaluates library lifecycle state even when
  project regeneration fails.

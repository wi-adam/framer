# framer-analysis

`framer-analysis` is Framer's UI-free, top-level derived-analysis layer. It depends
on `framer-core`, `framer-library`, `framer-solver`, `framer-standards`, and
`framer-geometry` to compile the canonical authored `BuildingModel` and current
regenerated outputs into one deterministic, disposable project analysis.

`analyze_project(&BuildingModel)` returns a `ProjectAnalysis` containing the plan,
resolved standards, compliance report, physical scene and audit,
`LibraryLifecycleStatus`, and a fallible project graph. Starter-library lifecycle
warnings are lowered into the plan before graph compilation, so the returned
lifecycle status, plan diagnostics, and graph describe the same generation.
`library_lifecycle_status(&BuildingModel)` remains available when framing cannot
solve.

The crate owns cross-domain graph identity, revision fingerprints, graph
compilation, and lazy explanation/impact query caching. It never mutates or
serializes authored intent. Lower crates do not depend on it; `framer-app` is
currently its only workspace consumer.

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
independently fallible `ProjectAnalysis.graph` result rather than panicking or
discarding the otherwise valid plan, report, geometry, and lifecycle status.

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

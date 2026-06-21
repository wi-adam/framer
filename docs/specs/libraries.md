# Libraries (Reusable, Distributable Content)

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Phase 4 implemented · **Linked goal:** G-013 (Libraries) ·
> **Plan:** [2026-06-19 — Libraries](../plans/2026-06-19-libraries.md) ·
> **Last reviewed:** 2026-06-21

## Intent / Purpose

Users will want to define, reuse, and **distribute** reusable content: wall systems and
materials today; materials with texture/depth maps; and later furnishings/objects (appliances,
cabinets, toilets) and electrical/mechanical objects (lights, panels). Today there is **no
sharing** — `BuildingModel::starter_library()` (`framer-core/src/model.rs`) is the only catalog,
and the three `examples/projects/*.framer` files each embed a full duplicate copy of it.

A **Library** is a versioned, distributable collection of typed, reusable definitions. This
spec defines **one unified mechanism** for all content kinds, designed so that adding a kind
(materials → systems → furnishings → MEP) reuses the same distribution machinery rather than
inventing a bespoke pipeline per type.

This realizes the [vision](../vision.md)'s commitments that the project format "allow future
bundled assets, but keep the authoritative model inspectable without proprietary tooling," that
binary caches stay "disposable" and the canonical state remain "recoverable from open,
documented data" ([vision.md](../vision.md#agent-accessible-project-files)), and that the model
carry enough "provenance for an agent to explain or modify the design safely." It builds on
seams the codebase already left: `MaterialSource::External` (representable, read nowhere today),
the `Appearance` growth-path comment, and the "resolver widens later" notes on `system_for` /
`material`. It extends the [Construction Systems & Material Library](construction-systems.md)
spec from a per-project library to a shared, distributable one.

## Requirements & behavior

The observable contract (prefer testable statements):

- **A project always opens without any library present.** `load_project` deserializes a
  self-contained model and `validate()` resolves every reference against the embedded
  `materials` / `systems`, with zero library I/O. A missing library disables "check for
  updates" / "insert new", never "open".
- **"Using" a library item vendors it** — copies the full definition into the project's own
  collections, id-remapped to stay unique, and stamps it with provenance (origin library
  coordinate, immutable version, content hash). After import the project is self-contained.
- **Importing a definition is an atomic subgraph copy.** Importing a `ConstructionSystem` also
  vendors and remaps every definition it references (each layer's `material` and any
  `FramingSpec.cavity_material`), so no reference dangles and `validate()` passes fail-closed.
- **Imported ids never collide.** A unique project-local `ElementId` is minted on import; the
  original library-local id is preserved in provenance for update alignment.
- **Provenance is descriptive, never load-bearing.** Nothing the solver, renderer, or
  `load_project` reads consults it; it drives only author-time update/divergence/detach UI.
- **Library identity is stable and matchable.** Each library carries a publisher-minted `uid`
  (a UUID) that is its identity independent of name/coordinate/host. A project can tell whether
  a file's content came from "the same library" by matching `uid`; how its version relates *in
  time* via the version's UUIDv7 `version_id` (time-ordered); and whether a specific item is
  byte-identical via its `content_hash`. Same coordinate but a different `uid` is a *different*
  library wearing the same human name, and is flagged.
- **Update & divergence are detectable, and surfaced as diagnostics** (the existing channel,
  like `room.boundary.open`): "out of date" = the library's current content hash for that item
  differs from the vendored copy's stamped hash; "detached/divergent" = the in-project copy's
  recomputed hash differs from its own stamped hash (the user edited a vendored copy).
- **Detach** drops provenance, turning a vendored copy into a plain project-owned element.
- **A library file (`.framerlib`) is itself canonical and self-validating** — deterministic
  serialization (its own header, id-sorted, pretty, trailing newline) and a `validate()` that
  checks internal consistency before publish, mirroring `.framer`.
- **Binary assets never enter the model.** A material's texture/depth map is referenced by
  content hash; the bytes live in a disposable content-addressed store. A missing asset
  degrades to the material's flat `color`; it never blocks open or render.
- **Remote libraries, when added, impose no load-time or network dependency.** They are
  fetched once, hash-verified, content-addressed, then behave like local; a project never
  stores a URL, only the vendored copy plus provenance. A hash mismatch on fetch fails closed.

## Decisions (locked)

- **Vendor-on-use is the default and resting state.** Using a library item copies its
  definition into the project. *Rejected:* link-by-default (Blender style) — it makes a
  `.framer` not self-contained and makes determinism *conditional on resolution* ("same
  resolved model → same render"), eroding a core invariant. Linking survives only as an
  internal cache/bundle mechanism, never the persisted resting state.
- **Embedding is the default for structured definitions; the only toggle is Detach.** A true
  embed-vs-reference axis exists *only* for binary assets.
- **Per-kind typed collections + one shared spine.** Materials, systems, and future
  furnishings/MEP stay distinct typed collections (keeping type-specific validation and
  editor ergonomics); only the distribution machinery — one `Provenance`, one resolver, one
  import/remap pass — is shared. *Rejected:* a single generic `LibraryItem` type (loses
  type-specific validation; weakens the strongly-typed model).
- **Symmetric provenance.** Material **and** system provenance land in the same schema bump;
  never ship a state where a vendored system's origin is invisible in the open file (that would
  violate "recoverable from open data").
- **Definitions are vendored inline; binary assets are content-addressed out-of-band.** The two
  sub-problems have different identity, storage, and versioning (see Architecture).
- **Identity is layered and location-independent.** A library's identity is a stable,
  publisher-minted `uid` (UUID), *not* its coordinate/URL (a location can move; identity must
  not). A published version's identity is a **UUIDv7** `version_id` — globally unique *and*
  time-ordered, so age/ordering comes for free without a central counter. Content equality/trust
  is a separate `blake3` hash. The three answer distinct questions — *same library?* (`uid`),
  *newer/older / when?* (`version_id`), *same bytes?* (`content_hash`) — and are complementary,
  not redundant. *Rejected:* using the coordinate as identity (breaks on rename/re-host) and a
  monotonic publish counter (needs a central authority; doesn't survive forks/mirrors).
- **Two orthogonal version axes:** file schema versions (`.framer` and `.framerlib`,
  independent) vs immutable content versions identified by a content hash (with a human semver
  alongside). Publishing edited content yields a new hash; old hashes stay valid forever.
- **Content hashing is determinism-critical and pinned.** A fixed, exactly-pinned algorithm
  (`blake3`), encoded as full lowercase hex with the algorithm self-described in the
  string (`"blake3:<hex>"`), **no truncation**, stored as `Text`, never recomputed at load, and
  guarded by a golden-hash regression test. Two granularities: a **library-version hash** over
  the whole `.framerlib` canonical bytes (on `LibraryStamp`, the content/trust anchor), and an
  **item hash** over each element's **provenance-excluded** canonical form (on `Provenance`, for
  item-level divergence/upgrade). Excluding provenance from the item hash removes the
  self-referential trap (an element's hash depending on its own provenance field); it requires a
  frozen per-element canonical form — a bounded, specified cost.
- **No `ElementId` charset change.** Mint remapped ids using only `[a-z0-9-]` (a library-id
  prefix + lowest-free numeric suffix on clash). **No digest embedded in ids** (protects
  human-inspectability). *Rejected:* widening the charset to add `_`.
- **Remote is architected, deferred — and must support a fully managed (RPC) backend later.**
  A project only ever vendors immutable, content-hashed snapshots, so however dynamic the
  remote backend is (even live RPC mutations to its catalog), what lands in a `.framer` is
  pinned bytes; RPC affects the *library's* published versions, while *consuming* always pins a
  snapshot at vendor-time.
- **Distribution/IO lives in a new `framer-library` crate.** `framer-core`/`framer-solver`/
  `framer-render` stay UI-free *and* IO-free; core holds only the small `Eq` data types and the
  pure vendoring/validation logic, with **no** registry/lockfile cross-consistency checks.

## Architecture (grounded in the codebase)

### The unifying spine

A **library** is a headless `BuildingModel`-shaped bag of typed definitions. The import
pipeline is type-agnostic:

```
resolve (Locator → bytes) → verify content hash → namespace-remap (atomic subgraph closure)
  → copy into the project's collections → stamp Provenance → one undoable edit()
```

Per-kind types stay distinct; only this pipeline + `Provenance` + the resolver + the asset
store are shared. Adding a content kind = add a `Vec<_>` to both `BuildingModel` and `Library`
and reuse the pipeline. Fixture/MEP substance rides the existing float-free
`properties: BTreeMap<String, PropertyValue>` (e.g. wattage `Int`, clearance `Length`,
model-no `Text`) — the same pattern materials already use for `r_per_inch_milli`.

### Types (`framer-core/src/model.rs`)

Identity separates three concerns — *which library* (stable, survives renames/re-hosting),
*which version & when* (ordering/age), and *which exact bytes* (equality/trust):

```rust
// A project records one stamp per (library, version) it has drawn from. Identity is the
// publisher-minted `uid`, NOT the coordinate (a location can move; identity must not).
#[serde(deny_unknown_fields)]
pub struct LibraryStamp {
    pub uid: String,          // stable library identity — UUID, minted once at creation, immutable
    pub version_id: String,   // published-version identity — UUIDv7 (embeds publish time → age/ordering)
    pub content_hash: String, // "blake3:<hex>" of the whole .framerlib at that version — content/trust anchor
    pub coordinate: String,   // resolvable hint/alias, e.g. "framer-lib://acme/envelopes" (NOT identity)
    pub version: String,      // human semver label (advisory)
}
// BuildingModel gains:  pub libraries: Vec<LibraryStamp>  (skip-empty; sorted by uid then version_id).

// Each vendored element points into that table (normalized — no per-element repetition of
// library metadata) and pins its exact content for item-level divergence detection.
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub library_uid: String,  // which library (→ a LibraryStamp)
    pub version_id: String,   // which version it was vendored from (→ a LibraryStamp)
    pub source_id: ElementId, // the element's id INSIDE the library (pre-remap)
    pub content_hash: String, // "blake3:<hex>" of THIS element's provenance-EXCLUDED canonical form
}

// Evolve the existing, unread seam (was External { reference: String }):
pub enum MaterialSource { Project, Library(Provenance) }
// Symmetric — same schema bump as the material change:
pub struct ConstructionSystem { /* … */ pub source: Option<Provenance> }

// Asset seam (phase 3; charset/Eq-safe — scale is Length, never f32):
pub struct AssetRef { pub hash: String, pub media_type: String, pub role: TextureRole }
pub enum Appearance {
    SolidColor([u8; 3]),
    Textured    { color: [u8; 3], texture: AssetRef, scale: Length },
    DepthMapped { color: [u8; 3], height:  AssetRef, scale: Length },
}
```

`Material.source` already serializes via `skip_serializing_if = "MaterialSource::is_project"`,
so a project that uses no library content keeps the same body shape.

### Library file format (`.framerlib`)

A new, independently-versioned format that mirrors `BuildingModel`'s collection shapes so the
**same serde types round-trip** at both ends:

```jsonc
{ "format": "framer.library", "schema_version": 1,
  "uid": "0a8f5c2e-…",           // stable library identity (UUID), immutable across the library's life
  "version_id": "018f3b9a-…",    // this published version's identity (UUIDv7 → embeds publish time)
  "version": "1.4.0",            // human semver label
  "coordinate": "framer-lib://acme/envelopes",  // resolvable hint/alias, not identity
  "materials": [ /* full Material objects, library-local ids */ ],
  "systems":   [ /* full ConstructionSystem objects */ ] }
```

It gets its own `to_canonical_json` (re-stamp version, sort by id, pretty-print, trailing
newline) and a header-peek-and-reject loader, mirroring `ProjectDocument` /`SchemaHeader` in
`framer-core/src/project.rs`. `Library::validate()` reuses `ConstructionSystem` validation
against the library's own material set so a library is self-consistent before publish. The
built-in starter library replaces the triplicated `starter_library()` copies.

### ElementId remap (the collision crux)

`validate()` (`framer-core/src/model.rs`) pools **every** id into one `BTreeSet` and rejects
duplicates; references are flat `ElementId`s. Import therefore runs a deterministic
namespace-remap in author-time app code: prefix the library id (`mat-cedar` from `acme-walls` →
`acme-walls-mat-cedar`; lowest-free `-2`, `-3` on residual clash), rewriting the imported
subgraph's internal references in lockstep. `ConstructionLayer` has no id — only `system.id` is
owned — so the only rewrites are `system.id` plus each `ConstructionLayer.material` and
`FramingSpec.cavity_material`. **The solver and renderer see nothing new**: a flat,
locally-unique, fully-resolved `BuildingModel`, exactly as today.

### Update lifecycle

Lifecycle state is derived, never persisted. `framer-library` recomputes an imported material or
system's provenance-excluded item hash in the original library-local id space, using
`source_id` and matching vendored material provenance to reverse the project-local remap. A
fresh import therefore reads "current" even though its project ids differ from the library ids.
If the recomputed project hash differs from the stamped item hash, the item is **divergent**. If
an available current library with the same `uid` carries a different source-item hash, the item
is **out of date**. If the available library no longer carries `source_id`, the item is **source
missing**. These are surfaced as derived Plan diagnostics; missing libraries simply mean
out-of-date/source-missing checks are unavailable.

Re-sync is an explicit author-time operation. It replaces the selected vendored material/system
from the available source library, keeps the project-local id stable, stamps the current
`version_id` + item hash, and prunes unused `LibraryStamp`s. Re-syncing a system also refreshes
the material closure it references, preserving existing local material ids where possible and
minting ids only for newly introduced source materials. Detach clears provenance from the
selected item, making it project-owned content; it does not delete or rewrite unrelated vendored
closure items.

### Resolver, assets, bundle (`framer-library` crate)

- A `Locator` (opaque to core) abstracts origin: `Builtin`, `Local { path }` /
  `Installed { id }` (search path, like fonts), `Remote { url, hash }` (**hash mandatory**),
  and a future RPC-backed managed provider. A `LibraryResolver` trait returns library bytes.
  Remote resolution is cache-first: it validates the pinned hash, tries a content-addressed
  `.framerlib` cache entry keyed by the full `blake3:<hex>` hash, verifies cached bytes before
  returning them, and only then calls an injected `RemoteLibraryProvider`. Fetched bytes are
  parsed, canonicalized, hash-verified, and written back to the cache before behaving like a
  local library. Hash mismatch, invalid URL, non-UTF-8 bytes, or fetch failure all fail closed.
- **Assets** (textures/depth maps now; meshes later) live in a disposable content-addressed
  store keyed by `blake3` of the bytes; the model holds only the `AssetRef` hash string.
- **Portable bundle** (`.framerpkg`): a deterministic zip (stored entries, sorted paths,
  zeroed mtimes) of the canonical `project.framer` + `assets/blake3-<hash>` blobs + a
  manifest. The bare `.framer` stays the primary format.
- **Render v1:** `framer-render` lowers asset-backed appearances only when callers provide a
  resolved in-memory texture map. Missing assets fall back to the authored color. Textures use a
  deterministic world-space projection; depth maps currently modulate diffuse albedo as a relief
  cue rather than displacing geometry. The app GPU shader mirrors the CPU path and keeps
  GPU↔CPU parity green.

## Constraints & invariants

This feature must preserve every [architecture invariant](../architecture.md) and the
[file-format contract](../project-files.md):

- **Determinism:** same model + profile → byte-identical `.framer` and identical
  framing/render. The content hash feeds nothing the solver/renderer reads; it is stored as
  `Text`, never recomputed at load. The hash crate is exactly pinned and golden-tested (it
  becomes determinism-critical like the seeded PCG renderer — see
  [render-view.md](render-view.md)). No truncation. Hash the library file's canonical bytes.
- **Float-free + deterministic provenance:** `AssetRef.scale` is `Length` (ticks), not `f32`.
  Identity stamps (`uid`, `version_id`/UUIDv7, hashes) are **publisher-minted once and then
  frozen**; the consumer only ever *copies* them, never generating a value from the clock or RNG
  during vendoring/load/save. A UUIDv7's embedded timestamp is therefore an opaque, pre-existing
  string at every point a deterministic operation touches it, so two machines vendoring the same
  library version produce identical JSON. Clock/RNG-using id *generation* lives only in
  `framer-library`'s publish path.
- **UI-free *and* IO-free core:** resolver/HTTP/RPC/CAS live in `framer-library`; core gains
  only small `Eq` data types and pure logic, no registry/lockfile cross-checks.
- **Authored intent is the only source of truth:** vendored copies are authored intent;
  binary assets are disposable caches recoverable from the open hash.
- **Schema discipline:** each schema bump follows the full ritual
  ([AGENTS.md](../../AGENTS.md#architecture-invariants-do-not-break) #4) — bump
  `PROJECT_SCHEMA_VERSION`, regenerate the three `examples/projects/*.framer`, add round-trip +
  rejection tests, and update [project-files.md](../project-files.md). The version stamp itself
  changes on a bump, so "byte-identical" holds for the model *body*, not the whole file —
  proven by a test that loads a no-library body unchanged.

## Out of scope (YAGNI)

- **Live shared editing / automatic propagation.** Vendoring trades this for determinism and
  self-containment; updates are an explicit author-time re-sync. Left architecturally open via
  the provenance/update-detection machinery.
- **Managed RPC backend / registry editing.** URL fetch + cache consumption is implemented;
  publishing, live remote catalog mutation, registry search, and managed/RPC editing are still
  deferred. The provider seam is intentionally narrow so those backends can supply pinned
  library bytes without changing project persistence.
- **Furnishing/MEP geometry, placement, and drawing.** Only the *spine* (shared `Provenance` +
  per-kind collection pattern) is established early; the element families and drag-and-drop
  catalog placement are a later, larger feature.
- **Geometric displacement / normal mapping.** Phase 3 samples textures and depth maps in the
  path tracer, but true surface displacement/normal mapping is deferred behind a separate render
  spec update and GPU↔CPU parity work ([render-view.md](render-view.md)).
- **A library coordinate registry / namespace authority** — see Open questions.

## Authenticity & signing (future direction — design-for, not building now)

Publishers (e.g. a siding manufacturer publishing their product line) will want their libraries
**cryptographically verifiable**, and a consumer opening a shared file will want to know content
is **authentic**. This layers cleanly onto the hash + identity substrate above. It is recorded
here so the substrate stays compatible; the trust-model choices below are open, not locked.

- **Sign the hash, not the bytes.** A publisher signs the canonical `content_hash` (blake3 is
  collision-resistant, so signing the digest signs the content). Proposed primitive: **Ed25519**
  (small, deterministic, no nonce/curve footguns), over a domain-separated message that binds the
  identity to the version: `"framer.library.sig.v1" ‖ uid ‖ version_id ‖ content_hash`. The
  `uid`/`version_id` binding prevents replaying a signature onto a different library/version.
- **Signable form must be remap-invariant.** Vendor-on-use rewrites bytes (id-remap + provenance
  stamp), so the signed/hashed canonical form is computed in the item's **library-local id
  space** (the provenance-excluded form already defined for divergence detection, which
  `source_id` lets us reconstruct). A vendored item therefore stays verifiable against the
  publisher's signature *after* remap. Signing (authenticates origin) **composes** with the
  divergence hash (detects local edits): an untouched item reads "✔ verified publisher", an
  edited one drops to "based on <publisher> (locally modified)".
- **Self-contained, offline-verifiable.** Signature + public key + the publisher's identity
  attestation travel **inline** in the `LibraryStamp`, so authenticity is checkable from the open
  file alone — only revocation/freshness needs the network. Mirrors how provenance already makes
  a file openable offline. Sketch: `signatures: Vec<Signature { scheme, public_key, sig,
  identity }>` (a `Vec` so a library can be self-signed-by-domain *and* registry-notarized).
- **Pluggable trust layer (like the resolver).** The signature primitive is fixed; binding a key
  to "James Hardie" is policy. Tiers, lightest → heaviest: **TOFU/self-key** (pin a fingerprint);
  **domain-bound** (recommended default — publisher = controller of `jameshardie.com`, key served
  at a `.well-known` HTTPS path / DNS, reusing the same trust substrate as TLS; gives a
  human-meaningful "verified: jameshardie.com"); **registry "Verified Publisher"** (the Framer
  store counter-signs vetted publisher keys — fits the managed/RPC backend); **X.509 / public
  transparency log** (CA / Sigstore-Rekor-style append-only `(uid, version_id, hash, signer)`
  log) for high assurance — our `version_id` + `content_hash` *are* the log entry, so it bolts on
  cheaply later.
- **Trust as derived diagnostics, never load-bearing.** Verification yields a non-persisted trust
  verdict (recomputed like framing) surfaced as a badge/diagnostic — **Unsigned · Signed
  (untrusted key) · ✔ Verified publisher · ⚠ Invalid/tampered/revoked**. It must **never gate
  opening** (degrade to a warning; always open + inspect) and **never feed the solver/renderer**.
  Crypto lives in `framer-library` (signing only at publish time); `framer-core` stores opaque
  base64 `Text`. The crypto crate is pinned like `blake3`.
- **Deferred hard parts:** revocation/freshness (best-effort online feed; offline shows
  "revocation: unknown"), key **rotation** (identity maps to a *set* of keys over time), version
  **yanking** (withdrawn copies still open, just badged), and the transparency log. **Signing
  authenticates source, not correctness** — a signed library can still hold wrong data;
  endorsement/quality is a separate (registry "official partner") concern.

## Open questions

- **Trust anchor & registry.** Which identity tier is the v1 default (recommended: domain-bound),
  and does Framer operate a registry/notary ("Verified Publisher") — which is also the
  managed/RPC backend's home? This decides revocation, vetting, and the signing UX.
- **UID assignment / registry & lineage.** Library identity is the publisher-minted `uid`, so
  two independent `acme` coordinates with different `uid`s are distinct, and a renamed/re-hosted
  library keeps its `uid`. Open: who *guarantees* `uid` uniqueness — a pure random UUID
  (collision-negligible, needs no authority) vs a registry-issued UID once the managed/RPC
  backend lands. Also open: whether to record version **lineage** (a parent `version_id`) so we
  can tell *descent* (B derived from A), not just wall-clock *age* — deferred unless needed.

# Header Span Calculation — Research Reference (2026-07-16)

> Research spike for a future UI-free mechanics kernel that can calculate header capacity
> from explicit loads and material design inputs. Related durable spec:
> [standards engine](../specs/standards-engine.md). Implementation sequencing:
> [illustrative standards reset](2026-07-16-illustrative-standards-reset.md).

## Decision

**Go** for a bounded mechanics kernel that accepts fully specified design inputs and reports
the controlling limit state with evidence. **No-go** for generating authoritative or
code-equivalent span tables from the current model alone.

The equations are tractable and independently implementable. The unavailable inputs are the
real boundary: trustworthy grade/species design values, adjustment factors, load
combinations, tributary load paths, deflection criteria, multi-ply connection behavior,
bearing, and lateral restraint. Those must be explicit authored inputs or come from a pack
whose provenance and redistribution rights are known.

## Source and licensing boundary

- The USDA Forest Products Laboratory's public Wood Handbook provides fundamental
  [structural analysis equations](https://www.fpl.fs.usda.gov/documnts/fplgtr/fplgtr282/chapter_09_fpl_gtr282.pdf)
  and explains why lumber grade design properties are lower than clear-wood averages in its
  [stress grades and design properties chapter](https://www.fpl.fs.usda.gov/documnts/fplgtr/fplgtr282/chapter_07_fpl_gtr282.pdf).
- The American Wood Council's public
  [span calculator methodology](https://awc.org/resources/calculator-help/) confirms the
  controlling input families: adjusted bending strength, shear strength, modulus of
  elasticity, compression perpendicular to grain, total/variable loads, deflection limits,
  and service/size/duration factors. It is a research cross-check, not a source to copy data
  or implementation from.
- Framer must not copy proprietary tables, notes, presentation, design-value datasets, or
  adjustment-factor datasets into the repository. A future authoritative pack must be
  user-provided, jurisdiction-provided with clear reuse rights, or explicitly licensed.

## Bounded mechanics model

The first useful kernel is a simply supported, prismatic rectangular solid-sawn header under
uniform gravity line load. It excludes point loads, notches/holes, cantilevers, continuity,
unbraced instability, engineered lumber, and unverified composite action.

For actual breadth `b`, actual depth `d`, span `L`, total uniform line load `w_t`, variable
line load `w_v`, adjusted bending value `F_b`, adjusted shear value `F_v`, and modulus `E`:

```text
section modulus                 S = b d^2 / 6
second moment of area           I = b d^3 / 12
maximum moment                  M = w_t L^2 / 8
maximum rectangular-beam shear tau = 3 w_t L / (4 b d)
midspan variable-load deflection delta = 5 w_v L^4 / (384 E I)
```

The candidate passes only when:

```text
M / S <= F_b
tau <= F_v
delta <= L / deflection_denominator
reaction / (bearing_width * b) <= adjusted F_c_perp
```

Total-load deflection, creep, and any pack-specific serviceability limit are separate checks,
not hidden constants. A multi-ply candidate may divide load between plies only when an
explicit connection/load-sharing assumption says the plies act together; otherwise it is
unsupported.

## Worked feasibility check — deliberately non-authoritative

This arithmetic only demonstrates the kernel shape. It is not a material grade, building
code, or construction recommendation.

Inputs: `F_b = 1,000 psi`, `F_v = 135 psi`, `E = 1,200,000 psi`, total line load
`600 lb/ft`, variable line load `400 lb/ft`, variable-load limit `L/360`, dressed member
dimensions 1.5x5.5 inches and 1.5x7.25 inches, and perfect equal load sharing for two plies.
Bearing is excluded from this arithmetic because neither an effective compression-perpendicular
value nor a bearing length is assumed.

| Candidate | Bending limit | Shear limit | Deflection limit | Controlling span |
| --- | ---: | ---: | ---: | ---: |
| 1-ply 2x6 | 2.90 ft | 2.48 ft | 4.52 ft | 2.48 ft shear |
| 2-ply 2x6 | 4.10 ft | 4.95 ft | 5.70 ft | 4.10 ft bending |
| 1-ply 2x8 | 3.82 ft | 3.26 ft | 5.96 ft | 3.26 ft shear |
| 2-ply 2x8 | 5.40 ft | 6.52 ft | 7.51 ft | 5.40 ft bending |

The result demonstrates why Framer must calculate and expose the controlling limit state
instead of storing only a maximum-span cell.

## Proposed production seam

Keep the kernel in a private `framer-solver` structural module until a second member family
proves a separate crate is justified.

```rust
struct EffectiveWoodDesignValues {
    bending_psi: u32,
    shear_psi: u32,
    elasticity_psi: u32,
    compression_perpendicular_psi: u32,
    provenance: String,
}

struct HeaderLoadCase {
    total_line_load_millipounds_per_foot: u64,
    variable_line_load_millipounds_per_foot: u64,
    variable_deflection_denominator: u32,
    total_deflection_denominator: Option<u32>,
}

enum HeaderLimitState { Bending, Shear, VariableDeflection, TotalDeflection, Bearing }

struct HeaderCapacity {
    max_span: Length,
    controlling: HeaderLimitState,
    evidence: Vec<StructuralCheckEvidence>,
}
```

The persisted model remains float-free. Production calculations should use scaled integers
and `i128` intermediates, compare squared/cubed inequalities without lossy roots where
possible, and find the greatest passing `Length` tick with a monotonic integer binary search.
This avoids a new rational-number dependency unless the prototype proves one necessary.

## Inputs the current model cannot yet derive honestly

1. Actual dressed member dimensions separate from nominal board labels.
2. Species, grade, grading agency/design-value source, moisture/service condition, and all
   effective strength/stiffness adjustments.
3. Supported load category and explicit dead/live/roof-live/snow line loads.
4. Tributary width and load path through roof/floor framing into a particular wall header.
5. Variable- and total-load deflection criteria plus creep treatment.
6. Multi-ply fastening/load-sharing assumptions and through-wall fit.
7. Bearing length, jack-stud capacity, compression perpendicular to grain, and support
   stability.
8. Concentrated loads, girder reactions, point-bearing offsets, lateral restraint,
   notches/holes, and engineered-product manufacturer data.

Unknown inputs must produce `NeedsReview` or `Unsupported`; they must never select a member
by silent default.

## Recommended delivery sequence

1. Implement and unit-test the pure uniform-load kernel using explicitly supplied effective
   values and line loads. Test hand-worked examples, exact boundary pass/fail, monotonicity,
   overflow limits, and each controlling limit state.
2. Add typed authored/library inputs for actual dimensions, effective material values, load
   cases, deflection criteria, and provenance. Do not infer these from a nominal profile.
3. Add load-path derivation only after roof/floor framing direction and tributary support are
   explicit in authored intent.
4. Generate disposable comparison tables from the kernel for a chosen input grid. Store the
   inputs and evidence, not a claim that the grid reproduces an external publication.
5. Treat authoritative compliance as a separate verification layer supplied by licensed or
   otherwise reusable packs.

## Prototype acceptance tests

- Analytic hand examples for bending, shear, deflection, and bearing.
- Exact one-tick boundary tests on both sides of each limit.
- Increasing breadth/depth/plies/design values never reduces calculated capacity.
- Increasing load or a stricter deflection denominator never increases capacity.
- Missing or zero effective values fail closed without panic.
- `i128` overflow bounds are proven for every accepted serialized maximum.
- Two runs produce byte-identical evidence and maximum spans.

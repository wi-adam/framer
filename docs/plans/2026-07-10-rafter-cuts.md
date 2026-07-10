# Stick-Rafter Plumb Cuts & Birdsmouth — Implementation Plan (2026-07-10)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/ceilings-and-roofs.md](../specs/ceilings-and-roofs.md). The spec is the durable
> source of truth.

## Goal

Replace the rectangular-prism presentation of generated common stick rafters in Plan-mode 3-D
with recognizable plumb tail/ridge cuts and a wall-bearing birdsmouth. Keep truss-tagged roof
systems and all non-common-rafter member kinds unchanged. Preserve schema v13, solver placement,
and BOM cut-length semantics.

## Risk ledger

| Contract / path | Boundary | Required proof | Review failure if missed |
| --- | --- | --- | --- |
| Common stick rafters use a cut profile | `framer-app` Plan-mode scene extraction | Unit geometry assertions for vertical end faces and the horizontal seat | A green build still renders the same square-ended prism |
| Birdsmouth requires a real wall bearing | Authored roof/wall intent → app-derived mesh | Matched-wall positive and unmatched-eave negative tests | Floating/manual roofs receive invented notches |
| Trusses and other members remain unchanged | `MemberFamily` + `MemberKind` dispatch | Truss-family, ridge/blocking, and non-common-rafter negative assertions | The detail leaks onto manufactured assemblies or compound-cut members |
| Cut mesh stays selectable | Scene mesh → pick path | Pick geometry uses the same cut-profile triangles | Clicking includes stale cuboid-only geometry or misses the rafter |
| Product-visible behavior stays documented | Spec / code map / screenshot deck | Markdown links and an inspected close Plan-mode shot | Docs claim generic prisms and the visual regression is too small to judge |

## Implementation

1. Add a rafter-profile prism beside the existing generic `BoardPrism`. Build its longitudinal
   profile from the solver's exact spatial endpoints, the matched eave bearing, the wall framing
   depth, and the rafter framing-band depth. Reuse core polygon triangulation for its end faces.
2. Route only `MemberKind::Rafter` + `MemberFamily::Rafter` through the cut-profile path. A matched
   wall with enough tail stock adds the birdsmouth; an unmatched wall uses plumb ends only. Keep
   jack/hip/valley members on the generic prism until their compound cuts have an explicit contract.
3. Feed the same profile triangles to rendering and picking. Add orientation, unmatched-bearing,
   truss-family, and unchanged-member regression coverage.
4. Add a close roof-framing checkpoint to the off-screen screenshot deck, inspect it, then run the
   app tests, workspace gates, markdown links, GPU parity, and the full screenshot deck.

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
scripts/ui-shots.sh
```

# xsvg-boolfuzz

A seeded, panic-tolerant fuzz of the curve-preserving boolean algebra
(`xsvg_core::boolean_svg_paths_curved`, linesweeper) that convert-to-path uses
for `<x:boolean>` (docs/Incremental.md §11).

```
cargo run --release -p xsvg-boolfuzz -- run --seed 1 --count 100000 --budget-secs 3600 --out target/boolfuzz
cargo run --release -p xsvg-boolfuzz -- repro --seed 4389 [--subset 0,2] [--op union] [--operand 0 "M…"] [--raw]
```

Case `seed` is the same case on every run and machine (splitmix64, no
platform randomness). A case is an op and two to four operands from a shape
zoo — rects, rounded rects, ellipses as cubics, circles as arcs, regular and
star polygons, random and self-crossing polygons, cubic and quadratic blobs,
rings, bowties, degenerate point sets — related as copies, copies nudged by
10⁻⁹…10⁻², rotated copies, shared edges, tangencies, nestings, at ordinary,
tiny (×0.01), huge (×1000) and far-off (+10⁶) coordinates, some under
`evenodd`.

Each case runs through the curve engine and the flattened one, and the
sampled truth from the operands themselves. The oracles, each a `kind` in
`DIR/issues.jsonl`:

| kind | meaning |
|---|---|
| `panic` | the engine or the passes panicked (caught; the campaign continues) |
| `timeout` | no result within 20 s (the child is killed; the campaign continues) |
| `crash` | the child died without reporting (an abort, a stack overflow) |
| `engine-error` | the engine declined the input |
| `pixel` | the curved result disagrees with the sampled truth where the flattened one does not |
| `area` | the curved and flattened areas differ beyond the flattening tolerance |
| `preserve` | a union of disjoint operands lost an input handle |
| `sliver` | a zero-area contour in the output |
| `dup-points` | a zero-length segment in the output |
| `nan` | a non-finite coordinate in the output |
| `slow` | over 500 ms |

Every issue line carries the operands' path data, so a case can be replayed
without the generator, and `repro` prints a 64×64 map of where the curved
(`c`), flattened (`f`) or both (`x`) results disagree with the truth.

The campaign is a parent that runs batches in child processes of itself, so a
panic or a hang in one case costs that case alone. `DIR/progress.log` has the
running counts.

## Campaign of 2026-10-23

About 3.06 million cases over seven hours on one machine (three runs, two of
them on the binary as shipped): seeds 1000000–2586835, 5000000–5428979 and
9000000–10042107. Every flagged seed was replayed on the final binary.

What the fuzz found and what changed because of it, in order: holes oriented
by the engine's own outer/hole flag (wrong on a bowtie's two lobes, which
touch at the crossing) → oriented by containment depth; a pairwise fold that
fed the engine its own output (pieces within epsilon of the next operand's
curves, where XOR and difference went wrong, and the only panics seen then) →
one n-ary sweep over the raw operands; the depth read beside a shared edge →
read beside an unshared one; epsilon slivers and ~0.003 handle drift at sweep
extrema → the snap-to-input pass; a real contour pruned because kurbo's
quadratic arc length went non-finite → a robust perimeter; panics and
twenty-second stalls on tiny (×0.01) and far-off (+10⁶) drawings → the sweep
runs in a normalised frame; a contour mis-snapped at huge scale → a snapped
contour must keep its region.

What remains on the final binary is about 0.02% of cases, engine-side (the
raw sweep already disagrees): operands that are near-coincident copies of one
another (nudged by 10⁻⁹…10⁻³, under XOR or difference) make up most of it,
then drawings at ×1000 and +10⁶, then self-crossing "wild-handle" cubic blobs
against a rotated copy or a nested shape in ordinary coordinates (about 1 in
30 000 cases). One ordinary-input panic was seen in 1.04 million cases
(`linesweeper` `entry_at` unwrap, seed 9123382 subset `0,2,3`: a rounded
rect plus a rect, a circle, a hexagon); a native host catches it, a wasm host
aborts. Stalls over twenty seconds remain for near-coincident copies of a
cubic blob (about 1 in 50 000). Convert-to-path checks the curve result
against the flattened one before writing it, so every disagreement above is
written as the flattened path instead.

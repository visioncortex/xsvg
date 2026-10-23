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

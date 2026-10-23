# The fuzz harness of xsvg-path-ops

A seeded, panic-tolerant fuzz of the curve-preserving boolean algebra
(`xsvg_path_ops::combine`, linesweeper) that convert-to-path uses
for `<x:boolean>` (docs/Incremental.md §11).

```
cargo run --release -p xsvg-path-ops --example fuzz -- run --seed 1 --count 100000 --budget-secs 3600 --mem-mib 1024 --out target/path-ops-fuzz
cargo run --release -p xsvg-path-ops --example fuzz -- repro --seed 4389 [--subset 0,2] [--op union] [--operand 0 "M…"] [--raw]
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
| `oom` | the child hit the memory cap (`--mem-mib`, default 1024) and aborted — an allocation storm in the engine; a probe run without the cap once reached 90 GB |
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

3,057,921 cases over seven hours on one machine, three runs (seeds
1000000–2586835, 5000000–5428979, 9000000–10042107) on successive builds;
every flagged seed replayed on the final binary (`a19d874`).

What the fuzz found and what changed because of it, in order: holes oriented
by the engine's own outer/hole flag (wrong on a bowtie's two lobes, which
touch at the crossing) → oriented by containment depth; a pairwise fold that
fed the engine its own output (pieces within epsilon of the next operand's
curves, where XOR and difference went wrong, and where the first panics were)
→ one n-ary sweep over the raw operands; the depth read beside a shared edge
→ read beside an unshared one; epsilon slivers and ~0.003 handle drift at
sweep extrema → the snap-to-input pass; a real contour pruned because kurbo's
quadratic arc length went non-finite → a robust perimeter; panics and stalls
on tiny (×0.01) drawings and stalls on ordinary ones → the sweep runs with the
drawing moved to the origin and scaled up to about a thousand units (never
down: scaled down, the engine draws the seam between a shape and its exact
copy); a contour mis-snapped at huge scale → a snapped contour must keep its
region.

On the final binary, 644 of the 3.06 million cases remain (0.021%), all
engine-side (the raw sweep already disagrees with the sampled truth):

| regime | remaining | of cases |
|---|---|---|
| near-coincident copy of an operand (nudged by 10⁻⁹…10⁻⁵) | 204 | — |
| parallel copy (nudged by 10⁻⁵…10⁻²) | 111 | — |
| at ×1000 | 117 | — |
| at +10⁶ | 57 | — |
| at ×0.01 | 43 | — |
| ordinary coordinates | 112 | 1 in 27 000 |

The ordinary ones are self-crossing "wild-handle" cubic blobs against a
rotated copy or a nested shape, and exact copies under mixed fill rules. One
panic in 3.06 million (`linesweeper` `entry_at` unwrap, seed 9123382 subset
`0,2,3`: a rounded rect plus a rect, a circle, a hexagon; a native host
catches it, a wasm host aborts) and 18 stalls over twenty seconds, all but one
a cubic blob against a copy of itself nudged by ≤10⁻⁷ (about 1 in 170 000).
Convert-to-path checks the curve result against the flattened one before
writing it, so every disagreement above is written as the flattened path.

**The stalls are allocation storms.** Under the memory jail every one of the
18 reaches a 1 GiB cap in 5–20 s (so 90 GB in minutes), all on an operand
that is a copy of another nudged by 10⁻¹¹…2·10⁻⁹ of the extent. The curve
algebra now declines such a pair before the engine sees it (`near_coincident`,
10⁻⁷ of the extent, exact copies and shared edges excepted), and the harness
reports any storm that gets through as `oom` rather than letting it run.

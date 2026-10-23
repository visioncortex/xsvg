# xsvg-path-ops

Boolean operations on SVG paths **with the curves kept**: union, intersection,
difference and exclusion over any number of operands, where every Bézier the
operation did not cut comes back exactly as it went in, and every one it did
cut comes back as the exact sub-curve between the cuts. It is the algebra
behind an editor's *convert to path* of an xsvg `<x:boolean>`
([docs/Incremental.md §11](../../docs/Incremental.md)); it is not the
compiler's render path, which stays on the flattened, integer-exact algebra in
`xsvg-core`.

```rust
use xsvg_path_ops::{combine, combine_checked, Op, Operand};

let rect   = "M10 10 L110 10 L110 60 L10 60 Z";
let circle = "M140 35 C140 51.569 126.569 65 110 65 C93.431 65 80 51.569 80 35 \
              C80 18.431 93.431 5 110 5 C126.569 5 140 18.431 140 35 Z";
let ops = [
    Operand { paths: vec![rect], even_odd: false },
    Operand { paths: vec![circle], even_odd: false },
];
let d = combine(&ops, Op::Union)?;   // Ok(Some("M93.411 10 C98.162 6.841 103.866 5 110 5 C126.569 5 140 18.431 140 35 …"))
```

The circle's two uncut quarter cubics are in that answer to the bit; the two
the rectangle cut are exact sub-curves; the rectangle's edges are lines. Pure
Rust (`kurbo`, `linesweeper`), builds for `wasm32-unknown-unknown`.

## The API

- **`combine(operands, op) -> Result<Option<String>, String>`** — the
  curve-keeping result. `Ok(None)` is no usable geometry, `Ok(Some(""))` a
  legitimately empty region, `Err` the engine declining (see below).
- **`combine_checked(operands, op, flat) -> Option<String>`** — what a
  conversion should write. `flat` is the compiler's flattened path data for
  the same operands and op; the curve result is sampled against it on a grid
  and written only if the two agree, otherwise `flat` is. An error, a declined
  input or (on a host that unwinds) a panic in the engine all answer `flat`.
- **`Op`**: `Union`, `Intersect`, `Subtract` (the first operand less every
  later one), `Exclude`. **`Operand`**: the path data of one shape — several
  `d` strings when one element lowered to several — and its fill rule.

Operands are absolute or relative SVG path data with **arcs already expanded
to cubics** (`xsvg_core::arcs_to_cubics`); an arc left in is approximated by
kurbo's parser instead. The result is absolute `M`/`L`/`Q`/`C`/`Z` at three
decimals, outer contours wound positive and holes negative, so it renders the
same under either fill rule.

In xsvg, `CompileSession::boolean_operands(offset)` hands out exactly this
input for an `<x:boolean>` — operands lowered as convert-to-path lowers them
(text outlined), the op, and the flattened `d` — and `to_path(offset)` gives
the markup whose `d` the answer replaces. `xsvg-cli --to-path` does that
in-process; a browser host should do it in its own module (below).

## What is done around the engine, and why

The engine is [linesweeper](https://crates.io/crates/linesweeper) 0.5, a
robust Bentley–Ottmann sweep over Bézier paths (the one Graphite adopted in
2026). A seeded fuzz of three million synthetic cases
(`xsvg-path-ops-fuzz`) shaped everything else here:

1. **One n-ary sweep over the raw operands**, each read under its own fill
   rule in the winding predicate. Folding operands pairwise feeds the engine
   its own output, whose pieces lie within epsilon of the next operand's
   curves, and there its coincident-boundary decisions go wrong (XOR and
   difference first) — and that is where its panics were.
2. **A normalised frame**: the drawing moved to the origin, and scaled up to
   about a thousand units when smaller — never down. The engine's accuracy is
   absolute (an epsilon floored at 10⁻⁶), so a tiny drawing was swept
   coarsely (panics and stalls), a far-off one at the mercy of its magnitude;
   scaled *down*, it mistakes a shape's exact copy for a neighbour and draws
   the seam.
3. **The near-coincident pre-check**: an operand whose boundary runs along
   another's within 10⁻⁷ of the extent (over half its sampled points; exact
   coincidence excepted) is declined before the engine sees it. Such a pair
   — a copy nudged by 10⁻¹¹…10⁻⁹ of the extent, as another tool's arithmetic
   leaves one — is where the engine's subdivision never settles and its
   memory grows without bound: a gigabyte in five to twenty seconds, ninety
   gigabytes before one unjailed probe took a machine down.
4. **Snap to input**: the engine splits curves at sweep extrema, leaves
   epsilon-sized slivers around them and moves handles by its tolerance; each
   run of result pieces lying on one input segment is replaced by that
   segment's exact sub-curve — the segment itself when whole — and a snapped
   contour that would not keep its region (a self-crossing cubic names one
   point at two parameters) stands down to the engine's pieces.
5. **Orientation by containment depth**, not the engine's own outer/hole
   flag, which is wrong where two lobes of a bowtie touch; the depth is read
   beside an edge no other contour shares. Zero-length segments and contours
   thinner than the output precision are dropped.
6. **The gate** (`combine_checked`), which turns every residual engine error
   into the flattened path.

## What the engine still gets wrong

Measured on the final build over 3.06 million seeded cases, every flagged seed
replayed; `xsvg-path-ops-fuzz/README.md` has the account.

- **Disagreement with the sampled truth in 0.02% of cases**, all engine-side
  (its raw output already wrong): mostly operands that are near-copies of one
  another (now declined by the pre-check), then self-crossing "wild-handle"
  cubic blobs against a rotated copy or a nested shape (about 1 in 27 000
  ordinary cases), and exact copies under mixed fill rules. The gate writes
  the flattened path for all of them.
- **One panic in 3 million** on ordinary geometry: a rounded rect with an
  overlapping rect as one operand (what a stroked shape's ink makes), a
  circle nested inside with small clearance, a hexagon crossing both; it
  survives any translation, rotation or scaling and has a basin about a unit
  wide (`linesweeper` `entry_at`, "comparison range ends at or before y").
  The reproducer is in `xsvg-path-ops-fuzz/linesweeper-findings.md`. A native
  host catches it in `combine_checked`; **a wasm host aborts**.
- **Memory storms** on hair-nudged copies: declined by the pre-check; none
  reached the engine in 192 500 jailed cases after it.

## Using it from a browser

Because of the panic and the storms, do not link this into the module that
holds your document. Build it as its own `wasm32-unknown-unknown` module and
run it in a Worker with a `WebAssembly.Memory` maximum and a timeout: an abort
then costs one conversion, which falls back to the flattened path the compiler
already gave you, and a storm that somehow passed the pre-check hits the
memory maximum instead of the machine. `xsvg-core` stays unaffected either
way; it never links the engine.

## Running the fuzz

```
cargo run --release -p xsvg-path-ops-fuzz -- run --seed 1 --count 100000 --budget-secs 3600 --mem-mib 1024 --out target/path-ops-fuzz
cargo run --release -p xsvg-path-ops-fuzz -- repro --seed 9123382 --subset 0,2,3
```

The harness runs cases in child processes under a counting allocator that
aborts at the cap, so a storm or a panic is recorded and the campaign
continues; `repro` replays one seed with maps of where the curved and the
flattened results disagree with the truth.

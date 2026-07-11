# Bench — fidelity & performance evidence for lowering trade-offs

Source: `cargo run --release -p gradient --example bench_fit` (Apple Silicon, release build;
re-run after solver or alpha-pipeline changes and update the tables).

## 1. Grid-fit solver: banded CG vs dense Gauss–Jordan

The grid fit's normal matrix couples each vertex only to its ≤9 cell-sharing neighbours, so it is
a 9-diagonal band. The shipped solver is **conjugate gradient over that band** (SPD thanks to the
ridge); the dense Gauss–Jordan it replaced is O(nv³) with an O(nv²) matrix — 14 Gflop and 46 MB
at the 48×48 profile cap.

Hard radial field, 256×256 region, square grids:

| grid | CG (shipped) | dense GJ | RMSE delta |
|---|---|---|---|
| 8×8 | 13 ms | 8 ms | 0.000000 |
| 16×16 | 16 ms | 14 ms | 0.000000 |
| 24×24 | 10 ms | 50 ms | 0.000000 |
| 32×32 | 9 ms | 238 ms | 0.000000 |
| 48×48 | 23 ms | ~1 s (extrapolated cubic) | — |

**Conclusion:** identical fidelity (same normal equations, CG converged to machine-level
agreement), and the time is flat instead of cubic — the `highest`-profile cliff is gone. The
per-fit cost is now dominated by the linear per-pixel accumulation, not the solve.

## 2. Alpha pipeline: straight-space fit vs premultiplied-space fit

Question: browsers premultiply texels **before** interpolating, while the fit optimizes
straight-alpha values — should the fit happen in premultiplied space instead (emitting straight
texels as premul ÷ alpha)?

Feathered field (color varying horizontally, alpha fading nonlinearly vertically — the layered
gloss case), error measured in **browser-compositing space** (premultiply the texels the PNG
would carry, interpolate that, compare against premultiplied truth):

| grid | straight-fit (shipped) | premultiplied-fit |
|---|---|---|
| 3×3 | 2.07 RMSE / 41.8 dB | 7.01 RMSE / 31.2 dB |
| 6×6 | 0.58 RMSE / 52.8 dB | 0.72 RMSE / 51.0 dB |
| 12×12 | 0.29 RMSE / 58.8 dB | 0.30 RMSE / 58.6 dB |

**Conclusion:** the intuitive "fit premultiplied to match the compositor" loses at every grid
size — emitting straight texels as `premul ÷ alpha` amplifies fit noise wherever alpha is small,
and the browser's premultiply-then-interpolate of straight-fitted texels is already near-optimal.
The shipped straight-space fit stays, and §8.2's "straight-alpha fringe at extreme zoom" caveat
is the *cheaper* side of the trade, by measurement.

# Canvas 2D M0 baseline results

Raw captured numbers for the pre-migration Canvas 2D cost model. The CDP
wall-clock rows are captured by `runner.py` and land here as
`<UTC ISO timestamp>_baseline.json`; none have been produced yet because the
capture environment lacks libclang to rebuild `aws-lc-sys` (see
`README.md` in this directory).

## Native cost model (this machine)

Branch `canvas_2D_moli`, commit `4d6e0373`, **debug** build,
`moli-canvas/tests/baseline_cost.rs`, run via:

```sh
cargo test -p moli-canvas --test baseline_cost -- --nocapture
```

### Arithmetic byte-cost evidence (asserted; instant)

The current design performs **two full-plane byte copies per ordinary draw**
(`with_canvas_like_pixels_mut`: copy backing view to a Vec, mutate, write back),
so the cost is linear in canvas area regardless of paint size:

| canvas | ops | bytes/plane | full copies/draw | bytes copied (total) |
|---|---|---|---|---|
| 256² | 100 | 262,144 | 2 | 52,428,800 |
| 1024² | 1000 | 4,194,304 | 2 | 8,388,608,000 |
| 2048² | 1000 | 16,777,216 | 2 | 33,554,432,000 |

Moving 33.5 GB to draw 1,000 small shapes on a 2048² canvas is the structural
problem this project removes.

### Reduced timing matrix (debug; kept fast so the check suite stays quick)

| canvas | ops | full-copy secs | premultiply-convert secs | encode (×10) secs |
|---|---|---|---|---|
| 256² | 100 | 0.0016 | 0.175 | 0.015 |
| 1024² | 100 | 0.0288 | 2.582 | 2.090 |

The full-timing matrix at 1000 ops is intentionally not run in the crate test to
keep `cargo nextest` fast; it can be measured at M6 on a machine with a built
browser via `moli-benchmark/fixtures/canvas/runner.py`.

Machine / toolchain: x86_64-unknown-linux-gnu, rustc 1.96.1 (debug).

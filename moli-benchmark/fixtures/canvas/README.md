# Canvas 2D M0 baseline fixtures

Reproducible benchmark inputs + runner for the Canvas 2D re-architecture
(see `docs/canvas-architecture-m0.md`). These measure the **pre-migration**
cost model so M6 can compare old vs new.

Contents:

- `workloads.py` — JS workload definitions (deterministic, emits self-contained
  HTML). Workloads cover the proposal §11.1 matrix: many small path fills and
  strokes with a single readback; many small rect, image, and text draws; a
  readback-after-every-draw workload (where batching is impossible); a mixed
  draw/clear/pixel-write sequence; repeated clean `getImageData`; and
  canvas-to-canvas/self-drawing.
- `runner.py` — CDP runner that launches `moli serve`, navigates each workload
  as a `data:` URL, and writes results to `results/<timestamp>_baseline.json`.
- `results/` — captured raw baselines (see below for what is captured so far).

## Workload shape

Every workload HTML defines `SIZE` and `N`, builds a canvas and a 2D context,
runs a synchronous draw loop, then sets:

```js
globalThis.__canvasResult = {
  workload, size, ops,
  recordMs,   // cumulative time of the draw-call loop (input/recording cost)
  flushMs,    // time of the single forced pixel observation
  totalMs,    // total wall time
  probe, acc, // sanity pixel/accumulator values
};
```

## Prerequisites

Building Moli (not this runner): the full workspace build pulls `aws-lc-sys`,
which compiles with bindgen, so a `cargo` build requires libclang.

```sh
sudo apt-get install libclang-dev clang
cargo build --release -p moli
```

Python deps for `moli_benchmark` (which this runner imports): see
`moli-benchmark/pyproject.toml` (`websockets`, `pillow`).

## Running

```sh
python moli-benchmark/fixtures/canvas/runner.py \
    --binary ./target/release/moli \
    --sizes 256 1024 2048 \
    --ops 100 1000
```

Results are written to `results/<UTC ISO timestamp>_baseline.json`, a list of
rows keyed by `workload`, `size`, `ops`, plus `recordMs`/`flushMs`/`totalMs`
(and `probe`/`acc`).

## Captured baseline (this machine)

Recorded during M0 on branch `canvas_2D_moli`, commit `4d6e0373`, debug build.

### Native cost model (`moli-canvas/tests/baseline_cost.rs`, no V8)

Models the current design's per-draw full-plane copy + format-conversion work.
Run with:

```sh
cargo test -p moli-canvas --test baseline_cost -- --nocapture
```

Raw (debug) output:

| canvas | ops | bytes/plane | full copies/draw | bytes copied (total) | copy secs | convert secs |
|---|---|---|---|---|---|---|
| 256² | 100 | 262,144 | 2 | 52,428,800 | 0.0015 | 0.162 |
| 1024² | 1000 | 4,194,304 | 2 | 8,388,608,000 | 0.229 | 26.04 |
| 2048² | 1000 | 16,777,216 | 2 | 33,554,432,000 | 1.204 | 100.99 |

These numbers expose the structural problem the proposal targets: cost scales
with canvas area even for small shapes, because every draw copies the whole
plane and (for path fills) allocates a fresh full-canvas raster to composite.

### Environment note on the JS/CDP baseline

The full `moli-renderer-v8` test suite and the CDP benchmark **could not be
executed in the M0 capture environment** because rebuilding `aws-lc-sys`
requires libclang/bindgen, which is not installed here (the once-built debug
binary predates this session). `workloads.py` + `runner.py` are validated for JS
correctness (see the Node harness notes in the commit) but the wall-clock JS
orders of magnitude were not measured against a live Moli instance in this
session. They must be captured as part of M6 on a machine with libclang and a
built `moli serve`.

### JS correctness baseline (existing regression suite)

`moli-canvas` native tests pass (22/22). The full
`canvas_paths`/`canvas_arguments` JS regressions require the `moli-renderer-v8`
test build (blocked by libclang here):
`cargo nextest run -p moli-renderer-v8 --lib canvas_paths canvas_arguments`.

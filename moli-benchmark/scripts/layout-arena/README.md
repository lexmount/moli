# Layout arena comparison on public pages

Compare a Vec baseline with two alternative stable-key layouts (SlotMap and
DenseSlotMap). Both alternatives use the same generational handles, logical box
order, Taffy bridge, sparse IFC owner indices, and dense geometry publication.
Only the storage type differs.

This Linux harness records public pages with a real navigation and captures their
initialized DOM, CSSOM styles, canvas pixels, scroll offsets, focus, and response
bodies. Snapshot replay makes original application scripts inert and serves the
same captured assets, then performs 96 alternating insert/remove operations in a
zero-height block at the start of the actual page. Every operation synchronously
reads the body's bounds. This measures retained layout after structural changes,
**not unmodified application initialization or network performance**.

Use the repository's Python environment (`uv sync --project moli-benchmark`), then
run these from the repository root, adjusting binary paths:

```sh
SCRIPT=moli-benchmark/scripts/layout-arena
RESULT=moli-benchmark/results/layout-arena-example
mkdir -p "$RESULT"
for SITE in finance baidu rust; do
  uv run --project moli-benchmark python "$SCRIPT/web_bench.py" \
    --binary /absolute/path/baseline-moli --site "$SITE" --mode record \
    --archive "$RESULT/snapshot-archive-$SITE" \
    --output "$RESULT/record-$SITE" --seconds 30
done
uv run --project moli-benchmark python "$SCRIPT/compare_webpages.py" \
  --baseline /absolute/path/baseline-moli \
  --slotmap /absolute/path/slotmap-moli \
  --dense /absolute/path/dense-moli \
  --archives "$RESULT" --output "$RESULT/comparison" \
  --repetitions 5 --seconds 2 --cpus 3,4
```

Build all three binaries from the same base, toolchain, Cargo.lock dependency
versions, release settings and features. Save separate executable copies and
record their source revisions; never reuse a target executable that another
build can overwrite. Choose CPUs appropriate for the machine, or omit `--cpus`.
Finish compilation and tests before collecting timings. All runs use a fresh
process, 1280x720 viewport, DPR 1, disabled HTTP cache, two Tokio workers, Realtime
layout, and native layout tracing. Variant order rotates between repetitions.

A replay is rejected if a resource is missing, a CDP handler fails, navigation
fails, the page is incomplete, or its title, element count or first 1,000 visible
element rectangles differ from the original capture. CSSOM serialization changes
hidden stylesheet text, so raw `textContent` length is not compared with the live
page. The three engines must also agree on snapshot text length and every sampled
rectangle; pulse bounds must match, and the final DOM and rectangles must equal
the pre-pulse observation. Shadow trees and nonempty child documents are not
supported and are rejected. Empty child documents are allowed. Error pages are
never timed as successful samples.

`result.json` contains process CPU ticks (Linux `/proc`, normally 10 ms
resolution), JS wall times, load-event timing, RSS/PSS/peak RSS, and observations
before and after the structural pulse. Engine logs provide per-pass and per-phase
native durations in the pulse's timestamp window. `compare_webpages.py` writes all
runs, binary/input SHA-256 hashes, medians and min/max ranges. Load timings in
snapshot mode describe static replay only. Use `--mode live` separately for an
instrumented live-page navigation check; network and application work can vary
between those runs.

Keep response archives and snapshots private: they may include cookies, session
identifiers, copyrighted content, or dynamic data. The benchmark results directory
is ignored by Git. Only aggregate measurements, input hashes, and scripts belong
in a PR. Captured script bodies can exceed the Inspector body budget; capture
warnings are preserved and such archives cannot be used for full script replay.
Snapshot runs do not execute or request those scripts, and still require every
requested resource to be present. Failed and excluded runs remain on disk.

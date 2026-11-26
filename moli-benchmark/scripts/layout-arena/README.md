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

## Expanded memory and common-page matrix

[Expanded measurements](results-extended-d7e0a70f.md) include matched snapshots and
separately observed live pages. The benchmark still uses pinned Vec/SlotMap/
DenseSlotMap executables built from the same baseline. Rust source is unchanged.

The live catalogue contains 28 search, finance, news, community, shopping, video,
recruitment and documentation pages. Scripts execute normally: `--observe-only`
does not inject the timer freezer and does not enable Fetch interception. The
initial window is 8 seconds (20 for Finance and hh.ru), followed by 2 seconds of
ongoing page activity; memory is observed again 2 seconds after the target-close
reply. Request content and loading progress can vary. Always inspect load counts,
DOM counts and ranges before interpreting live memory differences.

The controlled snapshot cohort admits only inputs whose original root element
count and first 1,000 visible rectangles survive capture/replay. Three independent
runs per variant rotate order; each performs three batches of 48 structural edits
(12 per batch for roots above 5,000 elements). The initial full matrix is retained.
Observed CPU regressions receive two further balanced rounds, not replacement
samples. No forced GC, allocator trim, empty child-document substitution, or
geometry tolerance is used to make a page pass.

Process memory sampling runs every 100 ms and records phase names, RSS, PSS,
anonymous PSS, private clean/dirty memory, CPU ticks and kernel RSS high-water.
Kernel-reported RSS high-water captures peaks between samples; PSS and
window-specific peaks are sampled. All trials have the same 4 GiB RSS guard. A guard kill or failed
navigation is a failed/censored record, never a fast successful page. Blank,
initialization, workload, idle and page-closure points make code/cache/allocator
residency visible, while three update batches provide only a bounded growth check.

Keep input archives, cookies and event logs in the ignored results directory. The
public reports contain numeric measurements and hashes only. Some sites require
the environment's proxy; `network_mode` records the route without exposing its
configuration. Shadow DOM, live child documents, missing captured bodies and
CSS/DOM fidelity failures are explicit exclusions from matched snapshots; their
unmodified live-page observations can still be recorded.

From the repository root:

```sh
ARENA_SCRIPT=moli-benchmark/scripts/layout-arena
ARENA_DATA=moli-benchmark/results/layout-memory-new
ARENA_BINS=/absolute/path/to/saved/binaries
uv run --project moli-benchmark python "$ARENA_SCRIPT/capture_memory_pages.py" \
  --binary "$ARENA_BINS/baseline-moli" \
  --catalogue "$ARENA_SCRIPT/common-pages.json" --output "$ARENA_DATA"
uv run --project moli-benchmark python "$ARENA_SCRIPT/extended_compare.py" \
  --mode snapshot --plan "$ARENA_DATA/snapshot-plan-v2.json" \
  --binaries "$ARENA_BINS" --output "$ARENA_DATA/snapshot-rounds" --repetitions 3
uv run --project moli-benchmark python "$ARENA_SCRIPT/extended_compare.py" \
  --mode live --plan "$ARENA_DATA/live-plan-v2.json" \
  --binaries "$ARENA_BINS" --output "$ARENA_DATA/live-round1" --repetitions 1
uv run --project moli-benchmark python "$ARENA_SCRIPT/choose_followups.py" "$ARENA_DATA"
uv run --project moli-benchmark python "$ARENA_SCRIPT/extended_compare.py" \
  --mode live --plan "$ARENA_DATA/live-followup-plan.json" \
  --binaries "$ARENA_BINS" --output "$ARENA_DATA/live-followups" \
  --repetitions 2 --repeat-offset 1
uv run --project moli-benchmark python "$ARENA_SCRIPT/select_cpu_followups.py" "$ARENA_DATA"
uv run --project moli-benchmark python "$ARENA_SCRIPT/extended_compare.py" \
  --mode snapshot --plan "$ARENA_DATA/cpu-confirmation-plan.json" \
  --binaries "$ARENA_BINS" --output "$ARENA_DATA/snapshot-confirmation" \
  --repetitions 2 --repeat-offset 3
uv run --project moli-benchmark python "$ARENA_SCRIPT/summarize_memory.py" \
  "$ARENA_DATA" --output "$ARENA_DATA/numeric-report"
```

`--binaries` expects `baseline-moli`, `slotmap-moli` and `dense-moli`. Keep every
initial and confirmation trial. `choose_followups.py` follows the recorded rule:
mandatory Finance/Wikipedia/hh.ru, the five highest baseline initial-window peak
RSS cases, and up to three largest absolute idle-PSS deviations above 5%. Other
live cases remain n=1 exploratory observations. Failed trials remain explicit
rows; the exporter additionally rejects root HTTP errors using Network events.
Input capture may produce a different eligible snapshot subset as sites change.

`render_memory_plots.py` accepts an exported report directory; run it separately
after measurements with Matplotlib and NumPy available. Do not compile or run
other benchmark processes during timed trials.

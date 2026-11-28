# Layout arena benchmarks

The Linux harness captures DOM, CSS and response assets from public pages.
Replay keeps application scripts inert and checks sampled geometry across
layout variants and after restoring each scenario's initial page state.
The comparison records all-thread process CPU, wall time, RSS and PSS.

## Run a two-build comparison

Save matching release builds as `baseline-moli` and `dense-moli` in the
binary directory. Use the same toolchain, features and dependency versions.
Run from the repository root.

```sh
uv sync --locked --project moli-benchmark
SCRIPT=moli-benchmark/scripts/layout-arena
RESULT=moli-benchmark/results/layout-arena-example
BINS=/absolute/path/to/saved/binaries

uv run --locked --project moli-benchmark python "$SCRIPT/capture_memory_pages.py" \
  --binary "$BINS/baseline-moli" --catalogue "$SCRIPT/common-pages.json" \
  --output "$RESULT/capture" --cpus ""

uv run --locked --project moli-benchmark python "$SCRIPT/compare_layout_followup.py" \
  --plan "$RESULT/capture/snapshot-plan-v2.json" --binaries "$BINS" \
  --variants baseline,dense --output "$RESULT/comparison" \
  --repetitions 3 --steps 8 --large-steps 8 --cpus ""
```

Each build runs in a fresh process for each page and round. The harness rotates
build order and saves the run measurements and input and binary hashes in the
output directory. Pass a CPU list to `--cpus` to pin both builds to the same CPUs.

## Other workflows

- Use `web_bench.py` to record or replay one page.
- Use `layout_compare.py` with `slotmap-moli` to run the three-build comparison.
  Export it with `summarize_layout_changes.py` and plot it with
  `render_layout_changes.py`.
- Add `slotmap-moli`, `slotmap_before-moli` and `dense_before-moli` and use
  `compare_layout_followup.py` with its default variant list for the five-build
  comparison. Export it with `summarize_followup.py` and plot it with
  `render_followup.py`. The plotting scripts use Matplotlib and NumPy.

Keep snapshot archives, measurements and plots under the ignored
`moli-benchmark/results/` directory. The checked-in catalogue supplies the
input pages. [Benchmark summary in PR #3795](https://github.com/lexmount/moli/pull/3795)

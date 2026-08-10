# moli-benchmark

Python benchmark harness for the Moli benchmark standard in `docs/moli-benchmark-standard.md`.

This project writes development benchmark artifacts under `moli-benchmark/results/<timestamp>/` by default. Use `--report-date YYYY-MM-DD` for formal artifacts under `benchmarks/results/YYYY-MM-DD/`. Most suites run their own Python harnesses; Suite C WPT archival intentionally shells out to `cargo nextest run -p moli-core --test wpt_compat --release --no-fail-fast` so WPT reports use the same release profile as normal repository verification:

- `environment.json`
- `versions.json`
- `summary.json`
- `summary.md`
- `publish-readiness.json`
- `report-data.json`
- `index.html`
- optional `report-diff.{json,csv}` when `--baseline-report` is provided
- `startup/runs.{csv,json}`, `startup/gate-rows.json`, and `startup/summary.json`
- `synthetic/runs.{csv,json}` and `synthetic/summary.json`
- `synthetic-matrix/matrix.{csv,json}`, `synthetic-matrix/gate-rows.json`, `synthetic-matrix/run-summaries.json`, and `synthetic-matrix/summary.json`
- `wpt/moli-wpt-compat-report.json`, `wpt/by-tag.csv`, optional `wpt/diff.{csv,json}`, and `wpt/summary.md`
- `cdp-smoke/group-listing.json`, `cdp-smoke/preflight.json`, `cdp-smoke/client-rows.json`, `cdp-smoke/moli-cdp-smoke.json`, and `cdp-smoke/summary.md`
- `synthetic-compare/runs.{csv,json}`, `synthetic-compare/summary.json`, and top-level `index.html`
- `cdp-session/runs.{csv,json}` for long-lived CDP sessions.
- `agent-episode/report-data.json`, offline `agent-episode/index.html`, raw
  episode/step rows, resource timelines, phase markers, and bounded failure
  artifacts for deterministic RL-shaped CDP workflows.
- `crawler/raw-runs.csv` for local multi-page crawler runs.
- `amiibo-crawler/raw-runs.csv` for the Python raw-CDP Amiibo crawler.
- `wild-web/raw-runs.csv`, `wild-web/failures/`, and optional `wild-web/replay/` captures for real-site seed classification, extraction assertions, failure snapshots, and explicit replay fixtures.
- `top-sites/raw-runs.csv`, `top-sites/runs.json`, `top-sites/summary.json`, and `top-sites/failures/` for public-web fetch/DCL benchmarks. The default source remains `docs/chinese-community-top100-websites.md` (`quick` = top 20, `full` = top 100). `--target moli-cdp`, `--target lightpanda-cdp`, and `--target obscura-cdp` run the same URL set through each engine's CDP server and dump DOM after the CDP DOMContentLoaded signal. `--target moli-full` and `--target moli-full-cdp` reuse the Moli binary with `--layout --resource`; ordinary Moli targets keep the product-default Mock layout and disabled optional resources. `--source webfetch-mix --profile webfetch` combines 100 mixed Chinese/global top sites with observed longtail WebFetch URL paths from `docs/webfetch-longtail-seed-list.md`. `--source render-quality` uses concrete article/document URLs from `docs/render-quality-seed-list.md` for rendered-DOM quality regression checks. `--source legacy-encoding` uses non-UTF-8 pages from `docs/legacy-encoding-websites-seed-list.md`.
- `render-compare/raw-runs.csv`, `render-compare/runs.json`, `render-compare/fetch-runs.json`, `render-compare/baseline-runs.json`, `render-compare/baseline-sites.{csv,json}`, `render-compare/summary.json`, and `render-compare/failures/` for baseline-relative public-web render quality checks. The suite defaults to Chrome as the baseline, first runs the full baseline URL set, filters to `baseline-usable` pages, then runs target browsers only for that evaluated set. It compares target visible text with character n-gram containment, key phrase hit rate, visible text ratio, and a combined render quality score. Rows can distinguish `render-match`, `render-partial`, `render-mismatch`, `state-only-content`, and fetch-level failures. Baseline-unusable or baseline-thin pages are recorded in `baseline-sites` and skipped before target scoring.

`versions.json` records detected engine binaries for `moli`, `lightpanda`, `chrome`, and `obscura`. Suite result rows use engine/driver target variants: CLI fetch targets use `moli`, `moli-full`, `lightpanda`, and `obscura`; public-web DCL targets also support `moli-cdp`, `moli-full-cdp`, `lightpanda-cdp`, `obscura-cdp`, and `chrome` with `driver=cdp-dcl`; synthetic CDP suites use `moli-cdp`, `moli-full-cdp`, `lightpanda-cdp`, `chrome-cdp`, and `obscura-cdp`. Every row and target summary carries `engine`, `driver`, `label`, and `binary_key`, so `report-data.json` is independent of the HTML renderer. Defaults come from `MOLI_BIN` / `LIGHTPANDA_BIN` / `CHROME_BIN` / `OBSCURA_BIN`, repository `target/release/moli`, and then `PATH`. Obscura local fixture runs use an external-looking global IPv6 fixture URL when one is available, so Obscura does not see a `127.0.0.1` URL.

The first landing slice covers:

- Suite E startup/deploy subset: binary size, stripped binary size, tar.gz package size, SHA256, daemonless minimal rootfs image tar/tar.gz size, `moli serve` readiness, optional CDP first page, optional warm-process CDP page creation, optional idle footprint, `moli fetch about:blank`, local JS fetch startup, `/usr/bin/time -v` raw artifacts when GNU time is available, cgroup/procfs raw artifacts, and structured formal gate rows.
- Suite B synthetic subset: `static-html`, `js-xhr-fetch`, `dynamic-script`, `dom-heavy`, `storage-cookie`, `forms-events` through a local Python fixture server.
- Suite B synthetic matrix: repeats the synthetic suite across a concurrency matrix and records median drift for stability checks. The `formal` profile expands default runs/repeats to the P0 floor from `docs/moli-benchmark-standard.md` and emits structured `gate-rows.json`.
- Suite D CDP smoke archival: runs or archives `moli-cdp-smoke` JSON output. `cdp-smoke --profile formal` requires raw CDP, Playwright, and Puppeteer client coverage; Puppeteer coverage requires local `node` and `puppeteer-core`.
- Suite C WPT archival: runs local WPT compat per-fixture reports through release nextest, or collects existing reports with overall/per-tag pass-rate summaries and optional baseline diffing.
- Synthetic horizontal compare: runs the same fixture set across fetch-style variants `moli`, `moli-full`, `lightpanda`, `chrome`, and `obscura`, then renders a top-level professional static `index.html` report with KPI cards, P0 scorecard, headline charts, drilldown tables, and artifact links.
- CDP session compare: starts each engine as a CDP endpoint through `moli-cdp`, `moli-full-cdp`, `lightpanda-cdp`, `chrome-cdp`, and `obscura-cdp`, navigates multiple cases through a reused page session, and records compact console / JavaScript exception / network failure traces under `cdp-session/traces/` when failures or error events occur.
- Crawler and wild-web suites: separate local many-page crawling from real external seed classification.
- Amiibo crawler suite: uses a Python raw-CDP crawler against the Lightpanda demo Amiibo site so the real 933-page workload can be run from the same benchmark harness.
- Synthetic fixture cases are split by domain under `moli_benchmark/synthetic_case_groups/`; current groups are `basic`, `modules`, `dom`, and `io`.
- `synthetic-compare` exits on the selected gate target only. The default gate target is `moli`; competitor failures stay visible in JSON/CSV/HTML without blocking report generation. Obscura is included as a local adapter; the benchmark harness gives Obscura an external-looking global IPv6 fixture URL instead of a `127.0.0.1` URL.

Resource sampling runs at 100 ms and records process-tree PSS when `/proc/<pid>/smaps_rollup` is available, RSS fallback, and aggregate `ps` CPU percentage. Startup cases also request `/usr/bin/time -v` artifacts under `startup/time/`; if GNU time is unavailable, the runner archives an explicit unavailability marker instead of silently omitting the evidence. Startup cases also archive `/proc/<pid>/cgroup` plus common cgroup v2/v1 files under `startup/cgroup/` when the environment exposes them. Image size artifacts are written under `startup/image-size/` by packaging the benchmark binary plus `ldd`-discovered dynamic dependencies into a minimal rootfs tar and tar.gz without requiring a container daemon. Startup rows explicitly record `process_cache_mode` and `kernel_cache_mode`; `--drop-os-cache` attempts `/proc/sys/vm/drop_caches` as root and otherwise records an unavailable marker under `startup/cache/`.

The HTML report is a Chart.js static dashboard generated from `report-data.json`. The JSON file is the renderer-independent contract; `index.html` embeds the same payload so the report can still be opened directly from disk without a local web server. When `synthetic-compare` and `cdp-session` are present together, `report-data.json` also includes a derived `horizontal_comparisons[0]` entry named `web-scraping-variants` that joins shared cases across fetch-style and CDP variants for one side-by-side chart/table view.

The standalone `browser-spider-local/bench.mjs` runner samples at 500 ms by
default. Its sampler runs in a Node worker thread, derives interval CPU from
`/proc` tick deltas, and records complete browser process-tree RSS/PSS alongside
case and site markers. Every run automatically writes
`resource-samples.{json,csv}`, `report-data.json`, and a resource/correctness
dashboard at `index.html`. The dashboard combines resource timelines, case
quality/resource comparisons, process topology, sampler health, site latency,
outcome distribution, and per-site diagnostics. Use
`--no-resource-sampling` for an explicit
sampling-free run, or `--sample-interval-ms N` to select a custom interval
(minimum 100 ms).
The detailed contract is in
`docs/browser-spider-resource-sampling-report-design-2026-07-26.md`.

Pull requests from branches in this repository also run an exact base/HEAD
Spider Bench comparison in GitHub Actions. The deterministic local fixture is
reported as a correctness and stale-page-leakage diagnostic, while the public
48-site run provides noisier performance and compatibility evidence. Neither
result fails the PR: the check fails only when the observer cannot produce its
artifact. A trusted follow-up workflow creates a new PR comment for each
completed current-head run and links the full `spider-bench-results` artifact.
The execution and permission model is documented in
`docs/browser-spider-ci-pr-benchmark-2026-08-03.md`.

Expected rows are derived from the selected workload. Public sites default to
five rows each (240 for the full 48-site set); deterministic fixture routes
declare whether they intentionally produce five rows or no rows (40 total).
Consequently fixture failures, `--site-limit`, and full48 no longer share a
misleading hard-coded denominator. The PR comment opens the public result first
and includes site coverage, bounded outcome counts, and per-category rows.

For a compact leadership-facing local comparison report, use:

```bash
uv run moli-benchmark run --profile horizontal --timeout 10
```

This preset runs `synthetic-compare` and `cdp-session` against the default target matrix with 10 runs per case unless `--runs` is explicitly provided. It is designed to produce the ten-way `moli` / `moli-cdp` / `moli-full` / `moli-full-cdp` / `lightpanda` / `lightpanda-cdp` / `chrome` / `chrome-cdp` / `obscura` / `obscura-cdp` comparison view quickly. It remains an investigation report until the independent P0 formal gates in `publish-readiness.json` also pass.

## Running

Build moli first:

```bash
cargo build --release
```

Run the default initial suite set:

```bash
cd moli-benchmark
python3 -m moli_benchmark run
```

Write a formal report directory. Generated date directories under `benchmarks/results/` are ignored by git and should be uploaded by CI or release tooling:

```bash
python3 -m moli_benchmark run --report-date 2026-05-07
```

Run focused suites:

```bash
python3 -m moli_benchmark startup --runs 5 --timeout 30
uv run moli-benchmark startup --runs 5 --include-cdp-first-page --include-cdp-warm-pages --cdp-warm-pages 10 --idle-seconds 1 --idle-seconds 5 --idle-seconds 30 --timeout 30
uv run moli-benchmark startup --profile formal --timeout 30
sudo uv run moli-benchmark startup --runs 5 --drop-os-cache --timeout 30
python3 -m moli_benchmark synthetic --case static-html --case js-xhr-fetch --runs 5 --concurrency 5 --timeout 30
python3 -m moli_benchmark synthetic-matrix --case static-html --runs 5 --matrix-concurrency 1 --matrix-concurrency 5 --matrix-repeats 2 --timeout 30
python3 -m moli_benchmark synthetic-matrix --profile formal --timeout 30
python3 -m moli_benchmark synthetic-compare --runs 5 --concurrency 3 --timeout 30
python3 -m moli_benchmark synthetic-compare --gate-target moli --runs 5 --concurrency 3 --timeout 30
uv run moli-benchmark cdp-session --case static-html --runs 5 --timeout 30
uv run moli-benchmark agent-episode --target moli-cdp --target chrome-cdp --workers 1 --parallelism 1 --runs 1 --step-dwell-ms 14000 --sample-interval-ms 500 --timeout 30 --output-dir ../target/agent-episode-bench
uv run moli-benchmark run --suite synthetic-compare --suite cdp-session --case static-html --runs 5 --timeout 30
uv run moli-benchmark run --profile horizontal --timeout 10
python3 -m moli_benchmark crawler --pages 50 --timeout 30
uv run moli-benchmark amiibo-crawler --target moli --pool 1 --limit 5 --timeout 60
uv run moli-benchmark amiibo-crawler --target moli --pool 2 --limit 5 --amiibo-mode process --timeout 60
uv run moli-benchmark amiibo-crawler --target moli --amiibo-profile formal --timeout 60
python3 -m moli_benchmark wild-web --seed baidu-home --timeout 30
python3 -m moli_benchmark wild-web --seed zhihu-home --capture-replay --timeout 30
python3 -m moli_benchmark top-sites --profile quick --timeout 15
python3 -m moli_benchmark top-sites --profile full --timeout 15 --parallelism 6
uv run moli-benchmark top-sites --target moli --target moli-cdp --target moli-full --target moli-full-cdp --profile full --timeout 30 --parallelism 2
python3 -m moli_benchmark top-sites --source webfetch-mix --profile webfetch --target moli --target moli-cdp --target moli-full --target moli-full-cdp --target obscura --target obscura-cdp --timeout 30 --parallelism 6
uv run moli-benchmark top-sites --source legacy-encoding --limit 6 --target moli --target lightpanda --target chrome --timeout 30 --parallelism 3 --chrome-parallelism 2
./scripts/run-webfetch-mix-benchmark.sh
uv run moli-benchmark render-compare --source webfetch-mix --profile webfetch --limit 50 --target moli --target lightpanda --baseline-target chrome --timeout 30 --parallelism 6
uv run moli-benchmark render-compare --source webfetch-mix --profile webfetch --limit 50 --target moli --target lightpanda --baseline-target chrome --match-threshold 0.65 --key-hit-threshold 0.70 --timeout 30 --parallelism 6
uv run moli-benchmark render-compare --source render-quality --profile quick --limit 12 --target moli --target lightpanda --baseline-target chrome --timeout 30 --parallelism 3
python3 -m moli_benchmark cdp-smoke --group protocol --timeout 30
uv run moli-benchmark cdp-smoke --profile formal --timeout 120
python3 -m moli_benchmark wpt --case abortcontroller-basic --timeout 60
python3 -m moli_benchmark wpt --no-run --baseline ../previous/wpt/moli-wpt-compat-report.json
```

### Cross-engine layout WPT

The standalone cross-engine runner has separate layout profiles, so its
existing semantic baseline is unchanged. Layout runs require an upstream WPT
checkout with `MANIFEST.json` and use CDP with a fixed `800x600` viewport at
DPR 1:

```bash
uv run python -m moli_benchmark.wpt_cross \
  --wpt-root ../../wpt \
  --engine moli --engine chrome \
  --output-dir /tmp/moli-layout-wpt \
  --profile layout-testharness

uv run python -m moli_benchmark.wpt_cross \
  --wpt-root ../../wpt \
  --engine moli --engine chrome \
  --output-dir /tmp/moli-layout-reftest \
  --profile layout-reftest

uv run python -m moli_benchmark.wpt_cross \
  --wpt-root ../../wpt \
  --engine moli \
  --output-dir /tmp/moli-wpt-all \
  --profile all
```

`--profile layout` combines both layout sets; `--profile all` merges the
default semantic baseline and both layout sets into one deduplicated matrix.
The stable layout profile covers
`css/css-flexbox`, `css/css-grid`, `css/css-sizing`, and `css/cssom-view`;
repeat `--dir-prefix` to override that list. Reftests are loaded from the
manifest and support `==`, `!=`, and fuzzy bounds. The initial static subset
filters wptserve Python handlers, HTTP/2, testdriver, animation, media, and
canvas dependencies. Failed reftests retain `test.png`, `reference-N.png`, and
`diff-N.png` under `OUTPUT_DIR/artifacts/ENGINE/`, with links in `index.html`.
An unfiltered full `default` or `all` run refreshes the unified status lists
directly under `wpt-cross-current/`.

`agent-episode` is one fixed local benchmark, not a family of smoke/stress/live
profiles. Both engines consume the same checked-in manifest, fixture,
`Runtime.evaluate(awaitPromise=true)` expressions, and assertions. The
canonical `14,000 ms` dwell is workload idle between steps; readiness still
comes from CDP responses and lifecycle events. Only correct episodes contribute
to latency summaries. `report-data.json` is the suite-level authority and the
HTML report is a self-contained renderer of that payload.

Compare a new top-level report with a previous report directory or `summary.json`:

```bash
python3 -m moli_benchmark run --report-date 2026-05-07 --baseline-report ../benchmarks/results/2026-05-01
```

Use an exact binary:

```bash
MOLI_BIN=../target/release/moli python3 -m moli_benchmark run --runs 5
LIGHTPANDA_BIN=/usr/local/bin/lightpanda CHROME_BIN=/usr/bin/chromium OBSCURA_BIN=$HOME/.cargo/bin/obscura python3 -m moli_benchmark collect-env
```

For publishable P0 synthetic data, use at least:

```bash
python3 -m moli_benchmark synthetic-matrix --profile formal --timeout 30
```

`--profile formal` uses all synthetic cases, the `1/5/10/25/100` concurrency matrix, `runs=100`, and `matrix-repeats=5` unless those values are explicitly overridden. The report marks formal profile requirement failures separately from workload failures.

## Remaining P0 Gaps

The current harness is a runnable benchmark skeleton with smoke coverage. It is not yet a publishable P0 benchmark report. Each run writes `publish-readiness.json`; a report remains `investigation` until the formal synthetic matrix, formal Amiibo crawler, WPT P0 smoke, CDP, startup/size, wild-web, top-level artifacts, and four-way target matrix checks all pass.
The default execution order is documented in `docs/moli-benchmark-standard.md`: formalize synthetic first, fill startup/size second, then add the real Amiibo crawler.

- `Targets Available` means the binary was detected and recorded in `versions.json`; it does not mean every suite measured that target. Each suite summary must be read separately. For example, `startup --profile formal` is currently Moli-only, while `synthetic-compare` measures fetch-style variants and `cdp-session` measures `*-cdp` variants.
- Synthetic has a formal profile for the required `1/5/10/25/100` concurrency matrix, repeated stability checks, and `RUNS=100` floor; P0 still needs an actual completed formal run artifact and follow-up profiling for slow cases.
- Startup now has a `formal` profile with `runs=10`, CDP first page, 10 warm-process page creations, idle footprint at `1s/5s/30s`, and structured `gate-rows.json`. The 2026-05-07 local formal verification passed with 0 failures; P0 still needs release/report artifact retention rather than committing large raw outputs to git. Container image measurement is intentionally out of scope.
- CDP smoke now has a formal profile that requires raw CDP, Playwright, and Puppeteer coverage from `moli-cdp-smoke`, and CDP session records compact Runtime/Log/Network traces for failed or error-bearing runs. CDP still needs wider Playwright/Puppeteer workflow depth beyond the current smoke gate.
- Crawler has a Python raw-CDP Amiibo crawler for the Lightpanda demo workload. Amiibo rows are bounded by `--timeout`, CDP page-session setup failures are archived as failure artifacts instead of blocking the queue, and fetched pages assert URL/title/body text/link-count plus Amiibo name/series/image fields. The suite supports `session` mode for one browser process with multiple page sessions and `process` mode for one browser process per worker. The default `smoke` profile runs `pool=1`, `limit=5`, and `session`; `formal` expands to the full pool matrix, both modes, and all 933 pages. P0 still needs an actual completed formal artifact.
- Wild-web has first-pass title/body keyword extraction assertions, failure taxonomy, failure snapshots, and explicit opt-in replay capture via `--capture-replay`; it still needs deeper per-site business-field extraction and curated replay usage.
- Obscura is now a runnable target adapter for binary discovery, CLI fetch, and CDP serve startup. Local benchmark fixtures use an external-looking global IPv6 URL for Obscura when the host has one, so the target binary stays unmodified.

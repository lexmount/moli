# Moli Benchmark

`moli-benchmark` is the reproducible benchmark runner used to evaluate Moli.
It answers four practical questions:

- How quickly does Moli start and complete common browser workloads?
- How much memory and CPU does it use?
- Does it return correct, useful content across synthetic and public websites?
- How does it compare with Chrome, Lightpanda, and Obscura on the same work?

The runner records the environment and browser versions, keeps raw measurements,
and produces a self-contained HTML report. It supports quick local checks,
cross-engine investigations, and formal release-readiness runs.

## Quick start

You need Python 3.11 or newer, [`uv`](https://docs.astral.sh/uv/), and a release
build of Moli. From the repository root:

```bash
cargo build --release -p moli
cd moli-benchmark
uv run moli-benchmark run
```

The default `smoke` run exercises startup and deterministic local fixtures. It
does not require the optional comparison browsers.

The command prints its result directory when it finishes. Open `index.html` in
that directory to view the report; no web server is required.

## Common workflows

Run a compact fetch/CDP comparison across the configured engines:

```bash
uv run moli-benchmark run --profile horizontal --timeout 10
```

Run one deterministic case while working on Moli:

```bash
uv run moli-benchmark synthetic \
  --case static-html \
  --runs 5
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

Public-web suites read a `rank,target` CSV seed list. A minimal `sites.csv` looks like:

```csv
rank,target
1,https://example.com/
2,https://www.rust-lang.org/
```

Compare Moli and Chrome on that sample:

```bash
uv run moli-benchmark top-sites \
  --list-path sites.csv \
  --profile quick \
  --target moli \
  --target chrome \
  --timeout 30
```

Compare visible content with Chrome as the baseline:

```bash
uv run moli-benchmark render-compare \
  --list-path sites.csv \
  --profile quick \
  --target moli \
  --baseline-target chrome \
  --timeout 30
```

Run a formal synthetic concurrency matrix:

```bash
uv run moli-benchmark synthetic-matrix \
  --profile formal \
  --timeout 30
```

Every suite has focused help:

```bash
uv run moli-benchmark --help
uv run moli-benchmark top-sites --help
```

## Long-running navigation stress reports

`moli-stress` repeatedly navigates one long-lived CDP target, retains the
100 ms process-tree RSS/PSS/CPU samples, and produces a self-contained D3.js
report. Its default workload matches the sequential-navigation soak shape:
600 navigations across CSDN, SegmentFault, Huaban, and example.com.

From the repository root:

```bash
cargo build --release --locked -p moli
uv sync --project moli-benchmark --locked
uv run --project moli-benchmark --no-sync moli-stress run
```

Results are written under `moli-benchmark/results/stress-TIMESTAMP/` as:

- `result.json`: full navigation and 100 ms resource samples;
- `summary.json`: compact machine-readable metrics;
- `report.html`: offline interactive RSS/PSS/CPU and latency charts.

Choose another exact navigation count or URL sequence with `--navigations`
and repeated `--url`. The navigation count must be divisible by the selected
URL count. An existing retained result can be rendered again without rerunning
the workload:

```bash
uv run --project moli-benchmark --no-sync moli-stress report \
  moli-benchmark/results/stress-TIMESTAMP/result.json
```

The HTML embeds the vendored D3.js runtime, so opening it does not require a
network connection or a local web server.

## Choosing a suite

| Suite | What it measures |
| --- | --- |
| `startup` | Binary/package size, startup latency, readiness, optional first/warm CDP pages, and idle resource use |
| `synthetic` | Correctness and performance on deterministic local HTML, JavaScript, DOM, storage, and event fixtures |
| `synthetic-matrix` | Stability across repeated concurrency levels |
| `synthetic-compare` | The same fetch-style fixture workload across multiple engines |
| `cdp-session` | Repeated navigation through a long-lived CDP page session |
| `agent-episode` | Deterministic, agent-shaped CDP workflows against Moli and Chromium |
| `crawler` / `amiibo-crawler` | Multi-page crawling, including the 933-page Amiibo workload |
| `wild-web` / `top-sites` | Extraction and lifecycle behavior on real public websites |
| `render-compare` | Visible-text similarity against a baseline browser, normally Chrome |
| `cdp-smoke` | Raw CDP, Playwright, and Puppeteer compatibility smoke coverage |
| `wpt` | Selected Web Platform Test compatibility reports |
| `collect-env` | Browser discovery plus environment and version metadata only |

Use `run --suite NAME` to combine supported suites into one report. Repeating
`--suite`, `--target`, or `--case` selects multiple values.

## Targets and browser discovery

The harness distinguishes the browser engine from the way it is driven:

- `moli` uses the normal CLI fetch path.
- `moli-full` uses the same binary with `--layout --resource`.
- Targets ending in `-cdp` use a CDP server instead of the fetch command.
- `lightpanda`, `chrome`, and `obscura` select comparison engines.

Not every suite accepts every target. Its `--help` output lists the valid
choices.

The predefined public-web CSV sources ship under `fixtures/top-sites/`, including
the WebFetch longtail corpus used by `webfetch-mix`. Markdown remains supported
for custom `--list-path` inputs.

Public-web results distinguish an individual attempt from a stable per-site
outcome. `raw-runs.csv` and `runs.json` retain every attempt, while
`site-outcomes.json` groups repeated attempts into `all-pass`, `all-fail`, and
`flaky` sites. Pairwise engine rows are marked `SINGLE SAMPLE` unless the suite
was run with at least three attempts per site; use `--runs 3` or more before
treating an engine-only result as repeat-validated.

Public-web summaries keep three different populations explicit. `raw_*`
metrics include reachable and unreachable observations before cross-engine site
exclusions; the normal pass rate uses counted, comparable attempts; and
`successful_*` latency/memory metrics contain successful attempts only. The
`common_success` cohort contains the exact run/site attempts that succeeded on
every selected target and is the source for cross-engine speed and memory
claims in the HTML report.

Multi-engine public-web runs are scheduled in site-paired groups. Each group
runs the selected targets sequentially, rotates which target goes first, and
allows multiple site groups to run concurrently. Raw rows record the schedule
and target-order indexes, UTC start/finish times, output hashes and samples,
response MIME/body-capture evidence, and the final URL when CDP exposes it.
This makes order effects and response changes auditable without retaining every
successful response body.

When the driver exposes the main-document HTTP status, that status is
authoritative: a rendered 4xx or 5xx error document is a failure even if its
body exceeds the content-size threshold. The report includes HTTP-status
coverage, evidence source, and classification basis for each target. Moli CLI
failures that name a terminal main-document status retain it as
`cli-diagnostic` evidence. CLI fetch drivers that expose neither protocol nor
diagnostic status fall back to conservative error-document markers; for a
protocol-aligned comparison with status coverage, select the `moli-cdp` and
`lightpanda-cdp` targets alongside `chrome`.

A CDP browser reports a binary main-document MIME type before it exposes a
download body. That is retained as `binary-response-headers`, not fabricated
into a successful PDF or archive. Header-only observations are neutral and do
not enter body-success or latency denominators. A CLI transfer that times out
after receiving only part of a binary response remains a transfer failure. For
an explicit `.pdf` main-resource URL, the Moli CLI adapter omits DOM-only page
wait options and counts the result only when the real PDF body is returned.

The built-in `wild-web` targets use the same DOMContentLoaded snapshot boundary
for Moli, Lightpanda, and Chrome: after the adapter observes DOMContentLoaded,
the page event loop must advance for at least 50 ms before the DOM is dumped.
The total readiness deadline still applies to that settle period. Seed
extraction checks require the expected site identity in the title plus a
non-trivial body; the brand name does not need to be repeated in the first
body-text sample.

Moli is discovered from `MOLI_BIN`, `../target/release/moli`, or `PATH`, in
that order. Comparison browsers are optional and can be selected through
`LIGHTPANDA_BIN`, `CHROME_BIN`, and `OBSCURA_BIN` or discovered from `PATH`.
For example:

```bash
MOLI_BIN=/opt/moli/bin/moli \
CHROME_BIN=/usr/bin/chromium \
uv run moli-benchmark run --profile horizontal
```

Unavailable comparison targets remain visible in comparison reports. Most
comparison suites fail the command only when the selected `--gate-target`
fails; the default gate target is Moli.

## Profiles and formal reports

Profiles describe the amount and purpose of work:

- `smoke` is the quick default for local development.
- `horizontal` is a top-level `run` preset for fetch and CDP comparisons.
- `formal` is available on suites with benchmark-standard coverage and uses
  larger run, repeat, or concurrency requirements.

To write a dated report under `benchmarks/results/`, pass `--report-date`:

```bash
uv run moli-benchmark startup \
  --profile formal \
  --report-date 2026-08-11
```

`--report-date` only changes where artifacts are written. It does not make a
smoke workload formal by itself.

## Reading the results

Development runs are written to `moli-benchmark/results/<timestamp>/` by
default. A report contains:

| File | Purpose |
| --- | --- |
| `index.html` | Human-readable offline dashboard |
| `summary.md` / `summary.json` | Compact suite outcomes |
| `publish-readiness.json` | Machine-readable checks that say whether the evidence is publishable or still investigative |
| `report-data.json` | Renderer-independent data behind the dashboard |
| `environment.json` / `versions.json` | Host details and exact browser binaries |
| Suite subdirectories | Raw rows, traces, failures, and suite-specific summaries |

Compare a report with an earlier result directory or `summary.json` using
`--baseline-report`:

```bash
uv run moli-benchmark run \
  --baseline-report ../benchmarks/results/2026-08-01
```

A completed command is not automatically publishable evidence. Smoke and
horizontal runs are normally investigations. Treat a report as formal only
when the required formal workloads were run and `publish-readiness.json`
reports that all gates passed.

Public-web measurements also depend on the network and changing site content.
Use local synthetic suites for deterministic regression checks, and public-web
suites for compatibility evidence rather than exact repeatability.

On Linux, the sampler records process-tree PSS from `/proc` when available and
falls back to RSS otherwise. Startup runs also retain available GNU `time`,
procfs, and cgroup evidence instead of silently inventing missing metrics.

## Spider Bench

`browser-spider-local/` is a separate Node.js/Playwright runner used for
multi-site spider comparisons and pull-request benchmark artifacts. It records
correctness, per-site outcomes, and process-tree resource samples in its own
offline report.

```bash
cd browser-spider-local
npm ci
npm run bench -- --help
```

Run the command with `--help` to see fixture, public-site, sampling, and output
options. The pull-request workflows under `.github/workflows/` are the source
of truth for CI execution and permissions.

## Development

Run the core CLI tests from `moli-benchmark/`:

```bash
uv run python -m unittest discover -s tests -p 'test_cli.py'
```

The complete test suite uses `uv run python -m unittest discover -s tests`; its
curated public-web seed CSV files are versioned under `fixtures/top-sites/`.

Keep benchmark claims tied to archived raw data, exact binary versions, and
the readiness checks. When adding a suite or target, update its CLI help and
report metadata before expanding this overview.

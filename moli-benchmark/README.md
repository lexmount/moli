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

Public-web suites read a Markdown seed list. A minimal `sites.md` looks like:

```markdown
## Top 2

1. `https://example.com/`
2. `https://www.rust-lang.org/`
```

Compare Moli and Chrome on that sample:

```bash
uv run moli-benchmark top-sites \
  --list-path sites.md \
  --profile quick \
  --target moli \
  --target chrome \
  --timeout 30
```

Compare visible content with Chrome as the baseline:

```bash
uv run moli-benchmark render-compare \
  --list-path sites.md \
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

The predefined public-web sources expect curated seed documents under the
repository's `docs/` directory. Those files are not present in every checkout;
use `--list-path` with `top-sites` or `render-compare` when they are unavailable.

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

The complete test suite uses `uv run python -m unittest discover -s tests` and
also expects the curated public-web seed documents described above.

Keep benchmark claims tied to archived raw data, exact binary versions, and
the readiness checks. When adding a suite or target, update its CLI help and
report metadata before expanding this overview.

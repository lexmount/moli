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

### Realtime layout cost against Chromium

On Linux, the standalone probe runs identical geometry and input-state tasks
with release Moli and Chromium. From the repository root:

```bash
uv run --no-project --with websockets==15.0.1 python \
  moli-benchmark/scripts/probe-realtime-chromium.py \
  --moli target/release/moli --moli-sha BUILD_COMMIT \
  --chromium /usr/lib/chromium/chromium \
  --font /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf \
  --nodes 1024 --runs 5 --cpu 0 \
  --output /tmp/moli-current-layout.json
```

Set `--moli-sha` to the commit used to build that binary and provide the actual
Chromium executable so its recorded hash identifies the browser. Both engines
load the same TTF and use a 1280×720 viewport at DPR 1. Each sample starts a fresh
browser with one fixture page; engine order alternates. `--cpu` binds every
browser process and thread; omit it to retain normal scheduling. The driver
and resource sampler remain unbound. `--positioned-blocks` adds relative
positioning to the blocks.

The workload includes clean reads, CSSOM reads, batch writes, interleaved
writes/reads, rAF callbacks, and wheel input. Every sample must return current
geometry. Wheel completion requires the handler, its microtask, the new width,
and the final scroll position. ACK and completion notification timestamps are
recorded separately by the CDP receiver. No screenshot is requested;
presentation latency and matching pixel output need separate workloads.

JSON includes binary, script, fixture and font hashes, process-tree CPU/PSS/RSS,
raw operation timings, and all failed samples. CPU is a lower bound when threads
exit during a phase; the affected identities are recorded. The probe refuses to
overwrite an output file and exits unsuccessfully if any sample fails. Keep
the separate `probe-realtime-layout.py` for Mock/OnDemand regression costs;
those modes retain their existing geometry contract.

Both `probe-realtime-layout.py` and `probe-realtime-chromium.py` accept
`--container blocks|grid|table`. The default block fixture and its measured
tasks retain their original bytes. Grid uses four fractional tracks; table
uses four cells per row with automatic table layout. Both containers have a
definite 1280px width, keeping their layout constraints equal when one engine
reserves viewport space for scrollbars. Each contains the same
number of fixed-size text blocks and runs the same geometry, batch mutation,
interleaved write/read, rAF and wheel tasks. The probes check the actual computed
container display, width and DOM membership before timing; Mock checks the
structure while keeping its approximate geometry contract. Treat each
container as a separate workload, and retain its fixture hash in cost reports.

### Dynamic layout and input against Chromium

The structure probe uses the same Ahem fixture as the PageVm cold, warm and
eviction regression. It runs two rounds of insertion, removal, reordering,
same-shape replacement, text/width changes, wheel scrolling, and list resizing.
Each operation is followed by a real coordinate click on the current row.

```bash
uv run --no-project --with websockets==15.0.1 python \
  moli-benchmark/scripts/probe-realtime-structure.py \
  --moli target/release/moli --moli-sha BUILD_COMMIT \
  --chromium /usr/lib/chromium/chromium \
  --output /tmp/moli-dynamic-structure.json
```

Both engines use the repository Ahem font, 1280×720/DPR1, and hidden scrollbars.
The driver waits for callback/DOM state without requesting screenshots or
adding rAF callbacks, then compares all 65 geometry, hit, identity, observer and
input snapshots. The native regression also checks that inspection does not
repair geometry after the frame and that legal work eviction preserves output.
The probe preserves partial failures and refuses to overwrite its JSON output.
This finite functional comparison does not establish throughput, memory
recovery, starvation freedom, or correctness for arbitrary DOM changes.

### Shared layout retention under pressure

The `layout_working_set_pressure` example exercises two real Realtime Pages in
one Browser through public APIs. It checks actual retained charges, maintenance
refunds, a queued handoff, pressure zero, recovery after a new mutation, and
Page-close cleanup. It uses the default budget timings and waits for the known
protection deadline before competing. A claim that cannot be verified within
the original ticket's conservative time window is reported as unproven.

Build it from a clean commit, embedding that commit's full SHA:

```bash
MOLI_PRESSURE_PROBE_SOURCE_SHA="$(git rev-parse HEAD)" \
  cargo build --release -p moli --example layout_working_set_pressure --locked
```

Freeze independent copies of the binary and example source. Record their
absolute paths and SHA-256 hashes in a build manifest with `source_sha`,
`profile: "release"`, the exact `command`, `binary`, `binary_sha256`,
`probe_source`, `probe_source_sha256`, and `tracked_diff: []`. Also retain the
build log, Rust version, Cargo.lock hash, and build environment. Then run once:

```bash
python3 moli-benchmark/scripts/probe-layout-working-set-pressure.py \
  --build /absolute/pressure-build.json \
  --output /absolute/new-pressure-result.json
```

The driver verifies the frozen artifacts and embedded SHA, preserves JSONL
events and stderr, samples process-tree resources, and rejects existing output
files. It does not invoke Cargo or retry unsuccessful runs. This is a bounded
diagnostic: estimated cache charges are distinct from RSS/PSS, sampled peaks
can miss short allocations, and two static Pages do not establish long-term
fairness or a busy-owner latency bound. Native build/frame/paint counts and
exact per-Page FIFO identities are outside this public harness.

For repeated contention among up to four Pages, use the same frozen build:

```bash
python3 moli-benchmark/scripts/probe-layout-working-set-pressure.py \
  --build /absolute/pressure-build.json \
  --pages 4 --rounds 8 \
  --output /absolute/new-multi-page-pressure-result.json
```

This calibrates a target that fits any one retained Page, rotates the incumbent
and waiter, and checks each original handoff before its claim window expires.
Every round also releases all optional retention under zero pressure, checks
that clean geometry reads do not repopulate it, and warms every Page again only
after a mutation. All Pages must receive a turn in every round and acknowledge
close. Default protection/claim timings remain unchanged; the outer process
deadline scales with the requested number of handoffs. Each handoff has one
waiter, so this does not claim a public census of multi-waiter FIFO identities
or a bound for busy Pages. Freeze and hash the example's module sources along
with its entry point when recording the build. List them as
`probe_modules: [{"path": "/absolute/frozen/mod.rs", "sha256": "..."}]` in the
manifest; repeated scenarios require these module hashes.

### WebMainBench fetch stability CI

The `WebMainBench · 545 pages` job runs on pull requests, pushes to `main`, and
manual CI runs. It reuses the verified HEAD release artifact and replays all
545 frozen HTML pages with `moli fetch --dump markdown --wait done`, a 45-second
page deadline, and a 55-second process watchdog. Cases run serially without
retries. Each page must return nonempty Markdown; panics, crashes, timeouts,
missing results, and unexpected errors fail the job.

The [public dataset](https://huggingface.co/datasets/opendatalab/WebMainBench)
is pinned to revision `5da0972e9b58d0c7891ae75053ced97c268f52e3` and SHA-256
`0efaa4b49a45e320a27fe6e5a0b6aad5b57259fc3321ac3448519cacc74c537e`.
Its original UTF-8 HTML, including inline scripts, is served over local HTTPS.
The runner requires a fresh Linux network namespace with only loopback; missing
fixture paths return 404 and external resources are unreachable. Original URLs
are metadata, not live fetch targets.

One known exception is recorded explicitly: case
`ccb6033c-0a12-4f9c-8c68-794f26129841` redirects to `langrensha.163.com` and may
return that host's DNS resolution error. Only this case and error combination
is accepted; a different failure on that page still fails CI. A successful
result on that page also passes. The gate checks every case, not a minimum
aggregate success count.

To run locally from the repository root, use Python 3.11+, `openssl`, `ip`, and
`unshare` (Debian/Ubuntu packages `iproute2` and `util-linux`):

```bash
export PYTHONPATH="$PWD/moli-benchmark"
python3 -m moli_benchmark.webmainbench download \
  --output target/webmainbench-data/WebMainBench_545.jsonl
unshare --user --map-root-user --net -- \
  python3 -m moli_benchmark.webmainbench run \
  --moli-bin target/release/moli \
  --dataset target/webmainbench-data/WebMainBench_545.jsonl \
  --output target/webmainbench-results \
  --revision "$(git rev-parse HEAD)"
```

Use a new output directory for each run. On systems that disallow unprivileged
user namespaces, use `sudo unshare --net -- env PYTHONPATH="$PYTHONPATH" python3 ...`
as the CI job does. The runner refuses to run against an ordinary network.

The job summary and the existing PR `CI Regression Report` comment report the
verdict, completion/success counts, expected DNS failures, unexpected failures,
panics, crashes, timeouts, and empty outputs. The comment includes bounded
failure details and a link to the source run and artifacts. Missing or malformed
reports remain visibly unavailable. As with the other aggregate checks, comments
are updated for same-repository PRs by the trusted default-branch workflow;
this integration takes effect after these workflow and renderer changes reach
the default branch.

The `webmainbench-results` artifact retains every page's Markdown, stderr, exit
status, elapsed time, and HTML/output hashes for seven days, including when the
check fails. Partial runs retain an incrementally flushed `results.jsonl` for
diagnosis. This job checks fetch stability on Linux. Official content quality
scoring is a separate local evaluation: dynamic page content and the upstream
TEDS adapter issue make the raw composite score unsuitable as a CI threshold.

### Script-authored semantic WPT

The default semantic profile includes both Window and DedicatedWorker variants
of `streams/` and `compression/`, plus their `.window.js` and `.worker.js`
cases. Secure-context `.any.js` cases use the fixture's trustworthy loopback
origin. To run the complete compression suite through the CLI path:

```bash
uv run python -m moli_benchmark.wpt_cross \
  --wpt-root ../../wpt \
  --engine moli --mode cli \
  --dir-prefix compression \
  --output-dir /tmp/moli-compression-wpt
```

Use `--mode cdp` to check the same cases through CDP. Unsupported formats
remain failures in the report; they are not filtered out of this suite.

HTML crashtests, identified by WPT's `-crash` filename flag or a `crashtests`
directory, are also included within the semantic profile's existing filters.
Runs containing crashtests use CDP in `--mode auto`; explicit CLI mode rejects
them. The runner waits for load, fonts, two animation frames, and removal of
`test-wait`, dispatching `TestRendered` when needed. Reports record these as
`crashtest` with zero subtests. An outstanding wait remains a timeout, and a
renderer crash is reported even when the browser process stays alive.

Explicit cases that reference `testdriver.js` use CDP in `--mode auto`.
The CDP runner delivers trusted keyboard and mouse/pen input on an independent
connection, so input can finish while a harness probe awaits a JavaScript
promise. It supports `click`, ASCII and WebDriver special keys in `send_keys`,
and keyboard, mouse/pen, wheel, and pause actions. Unsupported touch sources,
pointer geometry properties, and cross-origin or transformed frame pointer
coordinates reject the automation request. CLI mode cannot run these input
cases. The bridge preserves the engine's DOM geometry and hit-test APIs.

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

## Sampling and failure evidence

Resource samples retain process identities, read errors, and the raw observations
used for CPU and memory totals. Missing or incomplete reads remain unavailable;
they are not reported as zero. Thread CPU deltas and process clock ticks are
recorded separately, including thread retirement and counter precision.

Target shutdown seals the sampler before terminating the engine and records log
reader EOF, cleanup errors, and timeout results. Startup and cancellation failures
retain their original exceptions together with the evidence collected so far.
Sequential CDP receive failures attach already observed messages through
`cdp_receive_evidence`, without consuming extra messages or replacing the error.

These records improve measurement validity and failure diagnosis. They do not
change browser rendering policy or establish a performance improvement.

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

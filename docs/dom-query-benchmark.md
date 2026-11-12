# Geometry and text query benchmark

`scripts/benchmark-dom-queries.mjs` provides a repeatable local workload for
[#1617](https://github.com/lexmount/moli/issues/1617). It measures existing
behavior; it does not change layout policy or implement a performance fix.

Install Node.js 18+ and `playwright-core`, or point `PLAYWRIGHT_CORE_MODULE` at
an existing installation. Start a release build with real layout enabled:

```sh
cargo build --release -p moli
MOLI_LAYOUT=true target/release/moli serve --port 9222
```

In another terminal, run:

```sh
node scripts/benchmark-dom-queries.mjs --cdp http://127.0.0.1:9222 \
  --label moli-before --output target/dom-query-before.json
```

For a Chromium reference, install its matching Playwright browser and run:

```sh
npx playwright-core install chromium
node scripts/benchmark-dom-queries.mjs --chromium \
  --label chromium --output target/dom-query-chromium.json
```

`--executable /path/to/chrome` selects an existing Chromium installation.
`--rows 20 --repeats 1` provides a quick functional check. Default measurements
use 2,000 rows and three samples per case. Each row has four elements:
`div.row > div.c + button + input`; the geometry selector matches 4,000 elements.
The fixture is served on loopback with no external resources or page scripts.

The cases cover:

- `rects`: one `getBoundingClientRect()` per selected element, then the same loop
  again without mutations. This distinguishes first scans from repeated reads.
- `rects-warm-layout`: the same loops after one untimed geometry read initializes
  layout and any query index. The first loop still reads the remaining selected
  elements for the first time.
- `rects-after-screenshot`: the full loops after an untimed screenshot refreshes
  layout. This includes any source-index setup needed by the first geometry read.
- `single-rect-after-screenshot`: one selected element read twice after the same
  screenshot refresh, to expose lookup setup cost for a one-element workload.
- `inner-text`: two reads of `document.body.innerText`.
- `element-inner-text`: two text scans of the first 160 selected elements.
- `observe`: body text plus geometry and rendered text for each selected element.
- `evaluate` and `evaluate-all`: the same text-content checksum through a direct
  selector query and `locator.evaluateAll`, respectively. Neither reads geometry.

Every case and sample uses a fresh page/navigation. Within a case, first and
repeated reads share a page. Sample order alternates to reduce order bias.
All pages use a 1280 × 720 viewport. The script closes its own pages and
disconnects from an attached CDP browser; it does not close existing pages.

Read-case durations use `performance.now()` inside the page. `evaluate` and
`evaluate-all` durations use client wall time, including Playwright/protocol
overhead; callback work time is reported separately. Do not treat those clocks
as interchangeable. Navigation is reported separately and ends at
`DOMContentLoaded`, not first paint. A first read can include initial layout,
style work, and lazy wrapper setup; a repeated read is not proof of a particular
cache implementation.

JSON output contains raw samples, medians, browser/Playwright versions, platform,
viewport, fixture hash, and a user-supplied revision label. Compare release builds
on the same machine with the same fixture, viewport, and layout policy, without
concurrent builds or benchmark runs. The harness validates match counts, stable
checksums, and finite nonempty rectangles. It does not assert cross-engine pixel
equivalence or enforce machine-dependent timing thresholds.

Build different checkouts with separate `CARGO_TARGET_DIR` directories. When
sharing a target directory, clean the workspace's release artifacts before
switching sources. Record the executable SHA-256 and verify that CDP
`Browser.getVersion.revision` matches the expected Git commit before timing;
a revision label alone does not verify which executable actually ran.

For follow-up optimization, retain separate measurements for first scans and
repeated reads. Rect memoization can improve the latter without reducing the
source lookup work of the former. Any retained geometry lookup must follow
published snapshot ownership, including nested documents and snapshot replacement.
Text caches must additionally track the live DOM/style inputs used by `innerText`.

In a local CDP trace with Playwright 1.64.1, `locator.evaluateAll` used one
temporary array handle for both 40 and 4,000 matches. After utility initialization,
each repeated call sent two `Runtime.callFunctionOn` commands and one
`Runtime.releaseObject`; direct `evaluate` sent one `Runtime.callFunctionOn`.
The first locator call also initialized the injected and utility scripts.
This rules out one CDP handle round trip per match in that workload. Selector
work, local DOM wrapper creation, and utility-script overhead still require
separate profiling; this patch does not change the protocol path.

The geometry optimization retains a lazy provenance index with the single
published snapshot, including its member documents. It accelerates individual
rect/quad/range and element-metric reads, and rendered-text fragment membership.
The index is admitted only when its conservative allocation estimate fits beside
the canonical tree in the retention budget. It is replaced with the snapshot;
live DOM/style mutations do not refresh frozen geometry. Budget fallback keeps
the original scan/batch behavior. No subtree text strings are cached.

A [source-verified release comparison](benchmarks/dom-queries-2026-10-17/README.md)
records all samples at 100, 500, and 2,000 rows. At 4,000 matches, repeated
geometry improved from 78.9 to 9.8 ms and repeated combined observation from
169.0 to 21.4 ms. The first single-element read after a screenshot increased
from 0.3 to 3.2 ms because it initializes the source index; cold layout and
protocol overhead remain separate limits.

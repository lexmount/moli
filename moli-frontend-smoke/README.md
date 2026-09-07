# moli-frontend-smoke

This standalone project builds 1020 independent React, Vue, and Angular pages and compares a
deterministic DOM timeline from Chromium with the same timeline exposed by Moli over CDP.

The suite is structural, not visual. Its authoritative observation is
`DOM.enable(includeWhitespace=all)` followed by `DOM.getDocument(depth=-1, pierce=true)`. It
preserves node order, exact text whitespace, comments, namespaces, attributes, template content,
frame documents, and shadow and pseudo-element roots while removing protocol-local node ids.
Before each capture it reads computed styles, including `::before`, `::after`, and `::marker`, so
Chromium materializes the extended tree deterministically.

The one field-level semantic normalization is an inline `style` declaration block. When a block
parses completely into unique, independently-cascaded declarations, the comparison sorts parsed
`(property, token-serialized value, important)` records. Values and priorities remain part of the
comparison. Duplicate properties, shorthands, logical or vendor properties, comments, at-rules,
and parse errors keep their original string and declaration order.

The runner and page use a token handshake: the page pauses at `document`, framework `mounted`,
post-update `ready`, and final `settled` checkpoints while the runner captures the tree. Three
async-timeline cases additionally expose three real `requestAnimationFrame` commits. Any
intermediate-frame difference fails the case; final DOM equality cannot hide a transient bug.
The 210 gallery-derived complex cases also expose an `interaction-1` checkpoint between mounted
and ready. Their two transitions are driven through real React, Vue, or Angular event handlers.
The 30 browsing-context boundary cases expose `boundary-1` and `boundary-2` around frame
navigation, origin, MessagePort, realm, adoption, upgrade, detach, and reconnect transitions.
The advanced platform families expose `platform-1` and `platform-2`; the current 420 cases cover
HTML/XML parsing, fragments, Range/traversal mutation, cross-document adoption/import, text-node
normalization, namespace serialization, detached document construction, MutationObserver record
delivery, custom-element reactions, shadow boundaries, slotchange/microtask ordering, constraint
validation, dynamic form ownership, submitters, successful controls, labels, and datalist
association. They also exercise dedicated and shared workers, rich structured clone values,
ArrayBuffer and MessagePort transfer, classic and module worker graphs, worker fetch streaming,
BroadcastChannel delivery, and worker termination/replacement.
They additionally cover History and Navigation API state, traversal, fragment events,
interception, nested browsing contexts, readable/writable/transform/byte streams, BYOB reads,
streaming text encoding, and Blob/Response body consumption.
They also exercise live URL/URLSearchParams mutation, Blob/File/FileReader and binary transfer,
data URL and Response consumption, propagation/listener options, open and closed shadow retargeting,
slot paths, focus transitions, and event redispatch.
They now also cover service-worker install/activate/controller/update/unregister behavior,
CacheStorage and intercepted fetches, CORS credentials and preflight boundaries, exposed-header
filtering, XHR progress ordering, redirects, opaque responses, and request metadata.
They finally cover parser-blocking/defer/dynamic classic scripts, inert-script cloning,
`document.write` and `document.open`, static and dynamic module graphs, import maps, module failure
recovery, WebSocket text/binary/fragment/close/subprotocol behavior, EventSource multiline events,
reconnect/last-event-id, explicit close, and iframe teardown.

The current catalog is flat:

- React: 340 pages
- Vue: 340 pages
- Angular: 340 pages
- Complexity per framework: 40 simple, 40 medium, 260 complex

Cases have isolated URLs and browser contexts. They do not depend on execution order or state from
another page. Build-time vendor chunks and focused family components are shared where useful.

## Prerequisites

- Node 22 or newer
- Python 3.11 or newer and `uv`
- a Chromium executable
- a built Moli binary

From this directory:

```bash
npm install
npm run check:catalog
npm run test:node
npm run build
uv sync
```

Run the full differential:

```bash
uv run moli-frontend-smoke
```

The default differential is a strict two-phase gate. It completes Chromium readiness and DOM
timeline capture for the entire selected flat list first. Moli is not started unless every
reference case succeeds. Once the reference gate is green, every Moli readiness error,
checkpoint error, CDP error, or normalized tree difference is a failing browser bug; there is no
case-specific known-difference or expected-failure mode.

When the runner starts either browser, it passes port `0` and reads the actual loopback CDP
endpoint from that process's startup announcement before probing `/json/version`. It does not
reserve and release a port ahead of process startup, so concurrent runs do not have a
reserve/close/bind race.

The default is `--jobs 1` because current Moli accepts one browser frontend. A higher value is
an explicit concurrency-boundary probe. Chromium-only baseline calibration can use higher
concurrency, but a release conclusion should also repeat the reference run and compare every frame
hash across scheduling modes.

Each worker keeps one browser-level CDP connection and creates an isolated browser context per
case. CI may pass `--reference-infrastructure-retries 1` to re-observe a case after a Chromium
connection or protocol failure. The retry does not apply to fixture errors, normalization errors,
Moli failures, or DOM differences, and the original failure remains recorded in `summary.json`.

Run Chromium as a reference-only fixture gate:

```bash
uv run moli-frontend-smoke --reference-only
```

List the flat manifest or run focused selections:

```bash
uv run moli-frontend-smoke --list
uv run moli-frontend-smoke --framework react --family events
uv run moli-frontend-smoke --case 'angular/forms/*' --jobs 2
```

Use exact binaries or existing CDP endpoints:

```bash
uv run moli-frontend-smoke \
  --chromium-bin /usr/bin/chromium \
  --moli-bin ../target/release/moli

uv run moli-frontend-smoke \
  --chromium-endpoint http://127.0.0.1:9223 \
  --moli-endpoint http://127.0.0.1:9222
```

For build-path development, a partial fixture build is available:

```bash
node scripts/build.mjs --family text-and-attributes
uv run moli-frontend-smoke \
  --allow-partial-manifest \
  --reference-only \
  --case 'react/text-and-attributes/interpolation-card'
```

Generated files live under `dist/` and `artifacts/` and are ignored. The runner verifies the served
fixture-tree hash against the manifest before starting browsers. A failed case gets its own
artifact directory with final DOM JSON, diagnostics, timeline metadata, normalized DOM JSON for
every captured frame, and per-frame unified diffs when both engines produced a tree.

Validate the expected artifact set, recompute DOM hashes/node counts and diffs, and print
non-gating diagnostic projections:

```bash
uv run moli-frontend-smoke-analyze results /tmp/frontend-smoke-run
```

Require two Chromium runs to have identical manifest, version, diagnostics, case list, and
per-frame signatures:

```bash
uv run moli-frontend-smoke-analyze reference-stability \
  /tmp/frontend-smoke-reference-concurrent \
  /tmp/frontend-smoke-reference-serial
```

Require two full differential runs to have identical Moli timelines and binary hashes while
also proving their Chromium reference phases are identical:

```bash
uv run moli-frontend-smoke-analyze engine-stability moli \
  /tmp/frontend-smoke-differential-first \
  /tmp/frontend-smoke-differential-second
```

The analyzer's projections never change the differential exit status or turn a mismatch into a
pass.

The stable design and current implementation evidence are tracked separately in:

- `../docs/frontend-smoke-design-current.md`
- `../docs/frontend-smoke-implementation-progress-current.md`

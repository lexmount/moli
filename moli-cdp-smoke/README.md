# moli-cdp-smoke

This is a standalone CDP smoke project for moli. The default suite uses Playwright and raw CDP because those dependencies are managed by `uv`; the Puppeteer group uses the pinned `puppeteer-core` development dependency in this directory. Optional agent-browser, chrome-remote-interface, cdp-use, and Stagehand groups exercise their real published clients when the corresponding local dependencies are installed. High-value protocol contracts should also be mirrored as focused Rust tests near their owning `moli-protocol` boundary so they run under `cargo nextest`.

The suite covers real CDP-client workflows: `connect_over_cdp`, concurrent browser and direct-page clients, multiple pages and contexts, navigation, history, `Page.setDocumentContent`, popup target creation, JavaScript dialog events, Network/Fetch routing, workers, WebSocket, file upload, downloads, locator/input flows, DOM/handle flows, Chromium-derived CDP protocol samples, Playwright-upstream-derived route/CDPSession samples, screenshots, `Page.printToPDF`, viewport, and storage/profile behavior. It is not a replacement for unit tests. Its job is to answer whether moli is usable as a Playwright/Puppeteer-style CDP endpoint.

## Chromium Behavior Evidence

CI runs every default group twice per engine: `CDP smoke` uses the HEAD Moli
release and `CDP smoke (Chromium)` uses the Chromium revision bundled with the
Playwright version in `uv.lock`. These are independent executions: passing the
second repetition cannot replace a failed first repetition. Chromium additionally
uses `--retries 3`: each failed group gets up to three retries in fresh worker and
browser processes. Passing groups are not retried. A recovered group is marked
`flaky` in `summary.json`; every execution's log, result, and diagnostics remain
in the artifact. If all four executions fail, the job fails. Moli has no retries.
Both jobs select all default groups, including newly registered groups, without
separate allowlists. The
`chromium-cdp` group is a collection of protocol scenarios; its name alone does
not identify the browser under test.
The existing `CDP smoke` check also requires the Chromium job to succeed, so it
cannot remain green when the Chromium suite fails.

Both jobs use `--strict-full-suite --repeat 2`. Strict mode rejects `--group`,
`--endpoint`, `--fail-fast`, `--list-groups`, and nonempty `MOLI_SMOKE_GROUPS`,
`MOLI_MULTI_PAGE_CASES`, or `MOLI_MULTI_PAGE_SHARD`. A failure does not prevent
the remaining groups from running. Browser crashes, timeouts, failed assertions,
unhandled asyncio callback/task exceptions, incomplete/empty results, and
diagnostic capture failures fail an execution. Exhausting Chromium's per-group
retries, failing any Moli execution, or failing diagnostic uploads fails CI; Chromium failures are
not treated as Moli passes. Classifying a failure as infrastructure trouble or
Chromium instability requires evidence.
New pushes to `main` cancel unfinished CI runs for older `main` commits.
PR revisions and manual runs have independent concurrency groups, so a new
revision does not interrupt their Chromium suites or diagnostic collection.
Once a PR is merged or closed, the trusted `Cancel closed PR CI` workflow
requests cancellation of every unfinished CI run associated with that PR,
including queued runs for older revisions.

Each worker records the actual discovery version, engine, executable command,
and PID for its managed browser and any nested browser launches. Chromium
workers reject Moli server and CLI launches. Empty scenario lists, failed
scenarios, and `not_applicable`/`applicable: false` results fail the supervisor.
Use managed `--browser chromium` runs for the full oracle, including process
resource sampling; sharing an external `--endpoint` does not provide the same
process isolation or owned PID.

## Synchronization and diagnostics

Wait for the condition needed by the assertion, using the identity of the
operation being tested:

| Assertion needs | Synchronization |
| --- | --- |
| A newly loaded Document | Session, main frame, returned loader, commit, and loader-scoped load; `navigation.py` serves raw CDP and Python SDK clients |
| A frontend's response body | That frontend's request/loader and `Network.loadingFinished`; Page load or response headers are insufficient |
| Closing/detaching during evaluation | A unique marker emitted inside the pending evaluation, followed by its completion or expected rejection |
| A worker has initialized | A fixture request/reply, before testing its runtime values |
| An old Document's work is retired | The replacement Document's commit plus the specific pending-operation terminal |

`wait_until` bounds the entire async predicate and propagates protocol, fixture,
and evaluation errors immediately. Such errors must not become `False` followed
by a successful retry. Error documents, downloads, same-document navigations,
and deliberately gated navigations keep their own explicit contracts.

Event collectors have weak callbacks and explicit `close()`/context-manager
cleanup. Automatic cleanup schedules unsubscription on the event loop: a GC
finalizer must never reenter an event emitter's non-reentrant lock. Paused-page
cleanup must still attempt resume, evaluation drain, and debugger disable if
listener removal or assertions fail; those errors remain observable.

Fixed waits in Python groups require an adjacent `# smoke-timing:` explanation,
checked by `test_synchronization_policy.py`. The remaining positive delays are
bounded condition polls, inputs exercising the action-window deadline, or
bounded negative wheel observations with a positive scrolling control.
`sleep(0)` only yields scheduling; it is not evidence of browser readiness.
New readiness tests should use explicit gates/events instead of elapsed time.

Each worker writes `<result>.diagnostics/checkpoint.json` during execution,
containing recent phases, a bounded protocol tail, outstanding commands, and
CPU affinity/cgroup/memory information. Raw connections retain real wire IDs;
Python CDPSession observations use clearly labelled local call IDs. The SDK's
existing Python/driver transport also records its RPC IDs, object GUIDs,
navigation/locator commands and document events, including Page operations that
do not pass through a manual CDPSession. These IDs are not CDP wire IDs. This
small adapter targets the pinned Playwright version and fails explicitly if
that transport changes. Collection adds no browser commands or alternate
readers. A watchdog independently prints
Python thread stacks before the worker deadline, including when the event loop
is blocked. The deadline and failure status do not change.

CPU traces are saved in full before validation, for both successful and failed
workloads. Profile IDs are scoped by process; an expected hot function must be
on an actually sampled stack, not merely declared in the profile's node table.
The closed-page workload duration and its missing-function assertion remain
unchanged. A missing sample remains a failure to investigate with the saved
trace, browser identity, resource evidence, and a minimal executable probe;
it is not proof of either Chromium instability or a Moli adaptation bug.

For local investigation, use filtered groups without `--strict-full-suite`.
Keep every failed execution and report the denominator; a later pass does not
erase earlier failures. Before changing a Chromium-derived expectation, follow
the executable calibration procedure below.

The `Browser.close` lifecycle contract requires an acknowledgment followed by a
natural, zero-status exit of the owned browser process before cleanup. It
observes the parent's return code independently of stdout/stderr EOF: a
descendant may retain an inherited output pipe after the browser has exited,
which can leave `asyncio.subprocess.Process.wait()` pending. Failures identify
the session case and lifecycle stage, including acknowledgment and process exit.
Failed cases retain their close response, completed cases, browser log tail, and
connection state in the worker's `observed` result before any socket or process
cleanup. On Linux they also capture the owned PID's status, thread wait channels
and syscalls, and descendant process states. This distinguishes a live or stuck
browser from an already reaped browser whose descendants still own output pipes;
diagnostic collection does not relax the natural-exit deadline or count forced
cleanup as a successful exit.

Layout and scrollbar probes wait for the requested Document's main-frame
commit and loader-scoped `Page.lifecycleEvent` named `load`. A session-level
`Page.loadEventFired` cannot identify the Document and cannot release that
wait. Reloads must commit a loader different from the previous Document;
events received before the command reply are retained. Navigation failures
record the command, session, frame, loader, commit/load state, and recent wire
messages. Layout-policy scenario failures also retain their original traceback,
and failed initial navigation closes the target it created.

Lifecycle artifacts retain total FDs for both engines. Chromium also records
FDs for regular files inside its owned profile: delayed database initialization
opens these independently of target churn. Its fixed FD budget covers the
remaining descriptors; eventpoll, eventfd, and thread budgets remain separate.
Moli's original all-FD budget is unchanged.

The full-suite calibration on 2026-10-19 used Playwright 1.58.0's Chrome for
Testing 145.0.7632.6, Node 24.15.0, fresh headless profiles, and local fixtures.
The browser command disables the back/forward cache because the history
scenarios explicitly test document requests and their response lifecycle.
Authentication scenarios use fresh browser contexts to prevent cached Basic
credentials from bypassing a later challenge. Worker Network events are
collected from Worker sessions in Chromium and from the owning Page in Moli.

Some existing scenarios test Moli policies or extensions. These keep explicit
expectations for each engine; running all groups does not assert feature parity
or identical scenario counts:

| Boundary | Moli | Pinned Chromium |
| --- | --- | --- |
| Screen layout / wheel scheduling | Frozen layout and fixed action window | Live layout, per-event microtasks and observer updates |
| Scrollbar startup / emulation | CLI/environment matrix; current document updates immediately | Default visible, `--hide-scrollbars`; emulation applies on navigation |
| Locale/timezone across contexts | Shared process defaults and exclusive claims | Independent renderer defaults and claims |
| Fetch body streams | Buffered seeking and explicit rejection of continuation after body take | Sequential reads; EOF may require a separate empty read |
| WebSocket events and URL blocking | Extra generic request events, payload lengths, and WebSocket URL blocking | Dedicated WebSocket events and payload data; URL blocking excludes WebSockets |
| Hosted file URLs | Capability error before navigation/interception | File navigation and interception; missing file returns `ERR_FILE_NOT_FOUND` |
| Other capability boundaries | Stack timing privacy, unsupported CPU throttling, PI rename extension | Observed stack timing, successful throttling, rejected PI rename |
| Response/manifest/idle details | Plaintext TextTrack bodies, document-relative data manifest URLs, same-site idle persistence | Base64 TextTrack bodies, data manifest URL fallbacks, cross-document idle reset |

The `dom-input` text-editing checks were calibrated on 2026-09-12 against
Debian `/usr/bin/chromium` 145.0.7632.116 under xvfb and run unchanged against
Moli. `Input.insertText` on input, textarea and contenteditable elements emits
trusted `InputEvent` instances with `data`, `inputType` and `isComposing`;
textarea Enter and text-control Backspace/Delete have null data and distinct
operation types. `beforeinput` is cancelable; `input` is not. Both bubble and
are composed.

The native drop checks use `Input.dispatchDragEvent` (dragEnter/dragOver/drop)
and were also calibrated on that Chromium binary. Rich contenteditable edits
carry null data and a read-only DataTransfer, shared by beforeinput/input and
readable after dispatch. Text controls carry string data and null DataTransfer.
For plaintext-only contenteditable, Chromium exposes text on beforeinput and
the transfer on input. Cancellation and page-replaced InputEvent constructors
are covered. These checks do not claim full drag-store lifecycle, IME or
target-range support.

Any smoke assertion described as Chromium-compatible must be verified against a
real Chromium binary. Do not infer observable behavior solely from the
specification, Chromium source code, an existing Moli test, or intuition.

Before changing a Chromium-derived expectation:

1. Run a minimal CDP or Playwright probe against a local Chromium build, normally
   `<chromium-src>/out/Default/chrome`.
2. Record the Chromium revision or executable, the exact probe, the observed
   event/result order, and the repetition count when timing is involved.
3. Run the equivalent probe against Moli.
4. Update the smoke assertion only after the two observations explain the
   intended compatibility boundary.

Source inspection remains useful for explaining why Chromium behaves as
observed, but it does not replace the executable probe when one can be run. If
Chromium cannot be tested, document that limitation explicitly instead of
presenting an inferred expectation as verified behavior.

The native drag scenarios were recalibrated on 2026-10-29 against Chrome for
Testing 156.0.8078.73 with its default flags, and with `SetDefaultDropEffect`
and `SuppressPointerStreamAfterDrag` explicitly enabled. Both runs agree.
CI's CDP Chromium job installs this pinned oracle for `dom-input` with
`--chromium-bin-for-group`, while retaining the locked Playwright client and
the existing browser for other groups. All default groups still run under
`--strict-full-suite`. The `dom-input` group checks drag eligibility and
movement thresholds, native `effectAllowed: "uninitialized"`, frontend-session
isolation, cancellation, protected dragenter/dragover data, client-edited replay
payloads, and accepted or rejected drops. Both browsers report all operations
as `-1` at the CDP boundary; Moli keeps the supported mask `19` internally.
Default text drops into writable input, textarea, and
contenteditable targets need no author-canceled dragover. They use copy for a
draggable element source: `copyMove` allows the drop, while `linkMove` rejects it.
The same scenarios run against Moli's current build.

The native source scenarios also compare selected rich text, links and linked
images through `dragstart`, raw `Input.dragIntercepted` data, replay and the
resulting editable HTML. They check URL titles, HTML base URLs, absolute markup
URLs, author replacements and the exact operation mask. Moli retains existing
string-item order when `setData()` replaces a value; Chromium removes and
appends the replacement. Each engine's intercepted order must match its own
`dragstart` string types, and both must clear obsolete native metadata.
Selection HTML preserves selected DOM formatting and excludes hidden elements;
it does not synthesize Chromium's computed-style clipboard annotations.
Native image drags currently supply URL and HTML data, without manufacturing an
image File from decoded pixels. Original encoded response bytes and filename
metadata are required before that payload can be supported.

Cross-document tests cover parent/child and sibling-frame transitions, including
scaled frames. Related targets are retargeted to an iframe host in the receiving
Document, or become null when no such host exists. Moli projects drag coordinates
into the receiving Document, including source `drag` and `dragend`; Chromium 156
uses the destination event manager's client coordinates for those source events.
The shared scenarios assert these values separately instead of treating them as
identical. Source `drag` follows destination updates: enter emits it, the first
following over does not repeat it, and subsequent over updates emit it again.
Intercepted idle time has no autonomous drag timer, and canceling a source
`drag` event does not cancel the session. Like Chromium, source events reuse the
source's protected DataTransfer until dragend; repeated drag callbacks preserve
its identity, original metadata and author-set dropEffect. Target event views
remain independent, and finishing the source disables every retained reference.

Cancellation tests retain Moli's explicit cleanup contract: both intercepted
`Input.cancelDragging` and `dragCancel` leave the target and end the source once.
Chromium 156 leaves intercepted `Input.cancelDragging` idle and sends only
`dragend` for `dragCancel`; the oracle asserts this difference explicitly.

Protected enter/over views start with the preferred operation allowed by the
source; leave uses `none`, and drop receives the negotiated operation. CDP
modifier flags remain observable without overriding this initialization. The
manual port in `fixtures/drag-pointer-suppression.html` preserves the upstream
WPT scenario: pointer cancellation, capture release, and out/ancestor-leave
events must precede leaving the drag source. It also checks resumed pointer
entry while preserving the independent legacy mouse hover state. Returning
from a released capture to its descendant does not leave that ancestor.

Moli's drop-result regressions follow the HTML processing model: a drop must
be handled by script or complete its default action to succeed. Chrome 156
still reports its previous operation for an unhandled generic target and
ignores a drop listener's final operation changes. Those differences are
covered by Moli's specification regressions; the shared Chromium oracle
checks initialization, negotiation, and pointer suppression.

The `Fetch.getResponseBody` text encoding was calibrated on 2026-10-19 against
Debian `/usr/bin/chromium` 145.0.7632.116. A fresh page enabled `Network` and
`Fetch` with `requestStage: "Response"`, navigated to a local UTF-8 HTML fixture,
and read its paused body before continuing the response. Chromium returned
Base64 with `base64Encoded: true`; reading the completed request through
`Network.getResponseBody` returned the original text with `base64Encoded: false`.
The `network` and `multi-context` groups require the encoded Fetch body and flag,
then independently verify the original page-visible text after continuation.
Their Network body and `IO.read` stream assertions retain their own contracts.

The Fetch byte-counter and EventSource-close contracts were probed on 2026-10-20
with Playwright 1.58.0's pinned Chrome for Testing 145.0.7632.6 and local HTTP
fixtures. Of 150 ordinary 45-byte JSON fetches, one reported `dataReceived`
counters `(45, 0)` followed by `(0, 45)`; 146 of 150 gzip responses split their
decoded and encoded counters similarly. The `network` group therefore checks
the totals over all data events, their nonnegative lengths, and their order
between the response and successful terminal. A finite SSE response completed
with `loadingFinished`, while a response with a withheld trailing chunk ended
with `loadingFailed`, `net::ERR_ABORTED`, and `canceled: true` when the message
handler called `close()`. The EventSource scenario accepts those two outcomes
and still rejects other errors, wrong order, and duplicate terminals. The same
probe against the Moli release produced both SSE terminals, with its Fetch byte
counters delivered together. Run
`--browser chromium --group network --group browser-semantics --repeat 20 --jobs 4`
to exercise both groups with fresh browser processes.

The reset-before-response navigation was calibrated on 2026-10-21 with the same
pinned Chrome for Testing 145.0.7632.6. A probe calling
`_verify_failed_main_document_request_extra_info_on_session` on fresh pages
observed one fixture request in 3 of 20 navigations and two requests in the other
17. NetLog showed an internal transport retry only in the two-request case;
both paths exposed one CDP request, one request ExtraInfo, and one
`loadingFailed` with `net::ERR_CONNECTION_RESET`, without response events.
Waiting another 500 ms did not change either fixture count. With four CPU cores
and four workers, the original `chromium-cdp` group failed its fixed two-request
assertion in 5 of 20 runs. The equivalent probe on Moli 1.1.18 observed exactly
one fixture request in all 10 runs. The scenario accepts one or two transport
requests on Chromium, requires one on Moli, and records `fixtureRequestCount`
while keeping the CDP event and error assertions strict. From the repository
root, exercise the scheduling-sensitive case with:

```bash
taskset -c 0-3 uv run --project moli-cdp-smoke --no-sync moli-cdp-smoke \
  --browser chromium --group chromium-cdp --repeat 20 --jobs 4
```

The default `cdp-ordering` group locks down Renderer native and Inspector wire
publication order. It pipelines Page trees/layout, DOMSnapshot, DOM queries and
objects, CSS, AX, and a native backend error before `Runtime.enable`; stylesheet
and isolated-world context notifications must precede their corresponding
responses. Trees must reflect the preceding pipelined history mutation even
when its response has not been consumed. The effective URL includes Chromium's
separate `urlFragment` field; Moli currently includes the fragment in `url`.
Deferred Promise evaluation must allow its later resolver to run,
ordinary pause must allow native queries and synchronous focus callback reentry,
and instrumentation pause must leave Main work pending while IO remains live.
`DOM.getNodeForLocation`, `DOM.getNodeStackTraces`,
`DOM.setNodeStackTracesEnabled`, and `DOM.disable` must also complete through
their frontend native wrappers before the client sends resume. Disabling must
invalidate the original node bindings while the outer evaluation stays paused.
Paused scoped queries also cover selectors, child captures, AX, and geometry;
selector bindings, subtree depth, AX backend identity, and nonempty layout are
checked. Child-node notifications must precede their request's terminal, and
ready Main replies must retain wire order even when awaited in reverse.
All scenarios use local fixture documents, match the navigation's loader and
`DOMContentLoaded` before setup, track the original response session
and exactly one terminal per command, retain the full wire transcript, and use
protocol events/responses instead of sleeps or retries.

Six mutation cases explicitly preserve Moli's current VM entry boundary:
`DOM.setAttributeValue`, `DOM.removeNode`, `DOM.scrollIntoViewIfNeeded`,
`CSS.setStyleSheetText`, `Page.setDocumentContent`, and
`Emulation.setHardwareConcurrencyOverride` wait for resume in Moli but complete
during ordinary pause in Chromium. This is a Moli implementation limitation,
not a CDP requirement; supporting nested mutation later must deliberately update
this expectation. CSS editing checks effective computed color, since Chromium
changes CSSOM without rewriting the style element's DOM text. The group is
calibrated against `/usr/bin/chromium` 145.0.7632.116 and can run unchanged using
`uv run moli-cdp-smoke --group cdp-ordering --endpoint URL`. Run a bounded stress
check with `--group cdp-ordering --repeat 20 --jobs 4`; any failed run fails the
suite.

The `history-worlds` group was calibrated on 2026-09-25 against macOS Google
Chrome 154.0.8037.57 using a fresh headless profile. It uses a real CDP WebSocket
and `Page.createIsolatedWorld` to cover the PR #818 regressions: entry wrapper
identity and expando/method isolation, one `currententrychange` in each world,
isolated `navigate` cancellation, and the realm of History `SecurityError`s.
It also verifies that a later main-world mutation updates the isolated world's
structured state and preserves its local state cache. Event checks cover local
AbortSignal, FormData, File, and DOM wrappers; live FormData mutations; and
synthetic Event/CustomEvent identity, interface type, and shared cancellation.
Abort delivery for a synthetic NavigateEvent is covered by renderer Rust tests:
Chrome 154 crashes when aborting the signal passed to that synthetic event.
Run it with
`uv run moli-cdp-smoke --group history-worlds`, or add `--endpoint URL` to use an
existing browser process.

The detailed `emulation-storage` locale/timezone matrix was calibrated on
2026-09-01 against Debian Chromium 145.0.7632.116. In addition to resolved
locale, timezone, local getters, and formatted strings, the executable probe
now covers Intl and Date subclass `newTarget`, constructor reflection, options
accessor count/receiver identity, winter/summer and DST-gap Date construction,
local versus date-only/offset parsing, local setters with millisecond
preservation, and explicit locale/timezone precedence. The invalid-zone probe
is deliberately performed before installing a valid override: Chromium 145
returns `InvalidParams` on initial admission, while its active override
`change()` path silently retains the old zone for an invalid replacement.
Moli additionally locks down the latter state-preservation case in a focused
Rust protocol test.

The `emulation-storage` cookie expiry case was calibrated against Chrome for
Testing 156.0.8078.47 on 2026-10-28. It sets short-lived, maximum-safe-integer,
and far-future expiries over raw CDP, then checks that every cookie remains
persistent, short expiries stay unchanged, and large expiries are capped at
400 days rather than losing their expiry during timestamp conversion.

The default `locale-timezone-inputs` group adds input-boundary regressions for
PR #369. It was calibrated on 2026-09-10 against `/usr/bin/chromium`
145.0.7632.116 using the group unchanged: four timezone overrides cover Unicode
and lone-surrogate rejection, explicit legacy timezone tokens, ISO date-only
versus local parsing, weekday names and ignored comments, and frozen Intl
options. Option getter order and receiver identity are compared with the
same engine before emulation. Seven ICU locale IDs cover underscore/script
forms, POSIX default normalization, and calendar/collation/numbering keywords;
explicit page locales still use native BCP47 validation. Clearing overrides
must restore the original defaults. The group has its own Moli process because
the Unicode regression previously panicked in a native callback.

For example, Chromium resolves Playwright
`page.goto(..., wait_until="load")` after `Page.loadEventFired` but before the
later `Page.frameStoppedLoading` delivery. A smoke scenario that starts a new
event trace after `goto()` must therefore synchronize with the exact
`frameStartedLoading` / `frameStoppedLoading` pair first; treating the `goto()`
return as that synchronization point is incorrect.

The `url-policy` XHR expectations were calibrated on 2026-08-08 with Debian's
`/usr/bin/chromium` 145.0.7632.116 and raw CDP Network/Fetch observation. The
system binary produced the same synchronous `NetworkError`, `DONE`, reset
response fields, no progress events, and request-to-`loadingFailed` terminal
pair. With Fetch interception enabled, Chromium pauses a blocked-port request
before `net::ERR_UNSAFE_PORT`; Moli intentionally rejects that request at
its earlier admission boundary, so the smoke records no pause as a hosted
security contract rather than claiming identical interception placement. The
local `~/chromium/src/out/Default/chrome` 147 binary remains unusable for this
probe because its V8 snapshot does not match the executable.

The default raw `multi-client` group was calibrated on 2026-08-14 against
Debian `/usr/bin/chromium` 145.0.7632.116 and then run unchanged against Moli.
The group covers 2-, 3-, and 7-client browser and direct-page WebSocket fan-out
against one target. Every connection can reuse the same command ids without
response crossover and observe the same target runtime through distinct
sessions. For each connection, `Target.attachedToTarget` precedes its attach
response and four-command bursts preserve both response order and runtime
side-effect order; ordering between different WebSockets is intentionally
unconstrained. Target discovery enabled on alternating browser connections does
not leak to their peers, and subscribers observe `Target.targetCreated` before
their attach event and response. Chromium rejects a foreign flattened session with
`-32001` and a foreign
`Target.detachFromTarget` session reference with `-32602`. Closing either kind
of peer connection leaves the other client's root and target sessions usable.
The final matrix passed five consecutive runs against one Chromium process and
ten runs against freshly started Moli processes.

The default `xhr-sync-semantics` group was calibrated on 2026-08-09 against the
same executable Chromium and then run unchanged against Moli. It ports 14
Chromium-vendored XHR WPT contracts into one public-process matrix: nine success
variants, ten network/redirect failures, ten document `responseType` cases,
timeout restrictions, repeat-send and reopen/reset behavior, pending-async
cancellation, main-thread blocking, progress totals, upload-event silence, and
the CDP Network success/failure terminal skeleton. The local Chromium source
checkout remains useful for locating the WPT assets but is not counted as an
executable pass while its snapshot and binary are mismatched.

The Puppeteer existing-Page reconnect contract was calibrated on 2026-08-19
with the pinned puppeteer-core 24.30.0 against Debian `/usr/bin/chromium`
145.0.7632.116. One complete Puppeteer-group run created a second Page through
`PUT /json/new`, evaluated in the original Page through one `puppeteer.connect()`
session, disconnected that session, and evaluated the same target and retained
main-world marker through a replacement connection. Chromium returned `42` in
both sessions. The identical probe timed out at the replacement evaluation after
10,000 ms when Moli's parent-session renderer-inspector detach fix was removed,
and passed after the fix was restored.

The Puppeteer DedicatedWorker probe waits for an asserted fixture-message reply
before its first `WebWorker.evaluate()`. `workercreated` and
`Runtime.executionContextCreated` can arrive before worker initialization is
complete. Reading `self.location.pathname` at that point crashed the pinned
Chromium 145.0.7632.6 renderer in 6 of 20 runs with four workers on four CPU
cores, matching the CI `Runtime.callFunctionOn: Target closed` failure. Holding
the worker's `Runtime.runIfWaitingForDebugger` command for 500 ms reproduced
the failure in all three controlled runs; the same runs passed after adding
the fixture round trip. The full Puppeteer group then passed 30 of 30 runs
under the same CPU and worker limits. The original global-value, exact-once
console routing, explicit termination, and navigation teardown checks still
run after readiness is established.

The Puppeteer Shadow DOM accessibility workflow was calibrated on 2026-10-08
with puppeteer-core 24.30.0 against `/usr/bin/chromium` 145.0.7632.116, then run
unchanged against Moli. `puppeteer_shadow_accessibility.mjs` checks default and
unfiltered accessibility snapshots across open, closed, and nested closed shadow
roots, assigned and fallback slots, and a host named from its shadow contents.
ShadowRoot `queryAXTree` requests using `objectId`, `nodeId`, and `backendNodeId`
must return the same backend refs as the full snapshot and stay within the host
subtree. Snapshot refs resolve to the same elements adopted by Puppeteer ARIA
selectors. Three `ElementHandle.click()` calls must emit exactly three trusted
clicks in open/closed/slotted order; updated names preserve AX and backend refs,
and old names stop matching. Removing and restoring a slot assignment must
switch the exposed fallback/assigned controls. The complete `--group puppeteer`
passed one run against each engine; use `--endpoint URL` for the Chromium run.

The Puppeteer accessibility visibility workflow was calibrated on 2026-10-09
with the same client and Chromium executable, then run unchanged against Moli.
`puppeteer_accessibility_visibility.mjs` starts with a fresh Document and checks
an objectId partial AX request before the first full snapshot. It resolves an
external hidden label, observes label text changes, and preserves AX and backend
refs. Both snapshot modes then check computed display, visibility overrides,
HTML inert,
explicit content-visibility:hidden, hidden/until-found CSS overrides, and
hidden ARIA reference names.
Closed shadow styles, :host rules, and assigned-slot visibility participate in
the same checks. Two ARIA-handle clicks must emit exactly two trusted events;
hiding and restoring a control preserves its AX and backend refs, and CSSOM
rule edits update snapshots in both the Document and closed shadow root.
The complete Puppeteer group passed all 24 scenarios against each engine.
The separate Moli layout-policy group passed all 37 scenarios; AX style reads
do not publish or refresh layout. Viewport-dependent content-visibility:auto
display locking remains outside this visibility contract.

The Puppeteer descendant-name workflow was calibrated on 2026-10-10 with the
same client and Chromium executable, then run unchanged against Moli.
`puppeteer_accessibility_names.mjs` checks 28 parent-button name cases from
child SVG/span ARIA alternatives, native image
`alt`, external hidden labels, cycles and shared references, mixed inline text,
visibility overrides, inert/ARIA-hidden descendants, and closed-shadow slots.
It starts with a cold partial AX request, compares both snapshot modes, resolves
the parent button's backend ref through ARIA, and requires two trusted clicks.
Changing or hiding the icon updates its parent's name without changing refs.
Whitespace-only contents preserve tooltip names and inline word boundaries.
The complete Puppeteer group passed all 25 scenarios against each engine; the
separate Moli layout-policy group passed all 37 scenarios. Live Bootstrap 5.3
Modal and Offcanvas pages also matched Chromium for all 22 icon-labelled Copy
buttons and their ARIA backend refs. Each page completed a trusted ARIA-handle
click after explicit scrolling and screenshot layout publication.
AX name reads do not publish layout or change the frozen-layout policy.

The accessibility whitespace workflow was calibrated on 2026-10-11 with
Puppeteer 24.30.0 and Debian Chromium 145.0.7632.116.
`puppeteer_accessibility_whitespace.mjs` covers 31 cases: NBSP within, around,
and alone in names; repeated NBSP; HTML whitespace folding; other Unicode
spaces; and vertical tab. Name sources include contents, ARIA labels and
references, hidden references, native labels, image alternatives, title,
placeholder, input value, descendants, and closed-shadow contents and slots.
Each case checks a cold partial AX request, the full tree, both Puppeteer
snapshot modes, and positive and negative ARIA queries. A separate fixture
keeps ASCII-space and NBSP names side by side and tests exact matching through
`nodeId`, `backendNodeId`, and `objectId`. Repeated NBSP/ASCII-space mutations
must update names and selectors while preserving the target's AX and DOM refs.
The complete Puppeteer group passed all 26 scenarios against both engines.

The `navigation-outcomes` group was calibrated on 2026-08-23 against Debian
`/usr/bin/chromium` 145.0.7632.116 and then run unchanged against Moli. It
directly drives `Page.navigate` and correlates the result with the matching
Document request/response identity. The matrix covers text, renderable HTML,
empty, redirected, and HTTP-error attachments; unsupported binary MIME; an
ordinary HTML 502 document; 204 No Content; 205 Reset Content; and a reset before response
headers. Chromium reports successful attachments as `isDownload=true` plus
`net::ERR_ABORTED` while retaining the old Document, but a 204 reports the same
error with `isDownload=false`; the smoke therefore treats `isDownload`, not the
error string, as authoritative. The 204/205 matrix was expanded on 2026-10-02
against the same executable. Eight raw CDP probes cover ordinary responses,
attachments, request-stage `Fetch.fulfillRequest`, and response-stage
`Fetch.continueResponse`. The expanded group was also run unchanged against
Chromium three times. Ordinary and intercepted 204/205 responses retain the
Document, realm, form values, and navigation history, emit exactly one matching
`Network.loadingFailed` with `net::ERR_ABORTED` and `canceled=true`, and never
emit DOMContentLoaded for the attempted loader. Attachments retain
`isDownload=true` even with status 204/205. Moli still commits binary responses
as external Documents where Chromium retains the old Document; that case
asserts internally coherent response/lifecycle evidence and records the
engine-selected outcome rather than claiming parity. For a 404
attachment, both engines preserve `isDownload` and the HTTP response evidence,
while Chromium commits an error Document with `net::ERR_INVALID_RESPONSE` and
Moli retains the prior Document with `net::ERR_ABORTED`; that difference is
also recorded rather than hidden behind an exact error-string assertion.

The expanded `target-semantics` group was calibrated on 2026-08-26 against
Debian `/usr/bin/chromium` 145.0.7632.116 and then run unchanged against Moli.
In addition to target creation, attachment, close, and browser-context
isolation, its raw WebSocket contracts verify Chromium's TargetHandler access
modes, the browser-level flattened-auto-attach requirement, repeated Tab
auto-attach reconciliation without duplicate sessions, stable Tab-to-Page
ownership across foreground changes, `Target.createTarget` foreground and
background behavior, Page visibility changes, screencast visibility
`true -> false -> true` across foreground changes and fallback selection after
close, and a real
default discovery target that is not recreated after close. It also drives a
real middle-button press/release through `Input.dispatchMouseEvent` and checks
that the trusted `auxclick` anchor default action leaves the source visible and
creates a hidden background Page. The original 12-contract matrix passed three
consecutive fresh Chromium processes and five consecutive fresh Moli processes;
the middle-click contract was then run unchanged against both engines.
Chromium source revision `a03603fe9af6` was also inspected to explain the
access-mode and auto-attach machinery; the executable probe remains the
behavioral authority.

The real-URL debugger-wait lifecycle contract was checked again on 2026-10-20
against Playwright 1.58.0's Chrome for Testing 145.0.7632.6, matching CI.
The probe creates the script-free `/semantic-frames?debugger-wait=1` target
under `waitForDebuggerOnStart`, enables Page lifecycle events and Runtime, and
then sends `Runtime.runIfWaitingForDebugger`. Chromium can emit the requested
main document's `load` before replying to `Runtime.enable`; the attach event's
`waitingForDebugger=true` does not guarantee that this HTML load waits for the
explicit resume. Retain events from both Runtime commands and require the
owning session, main frame, and the committed frame tree's loader. The initial
document load check and child document checks still apply. With four browsers
restricted to two CPUs, discarding the Runtime-enable batch caused 3/300
timeouts; retaining it passed 300/300, including three loads from that batch.
Repeat the whole group with
`--browser chromium --group target-semantics --repeat 30 --jobs 4`.

The expanded `multi-page` group was calibrated again on 2026-08-31 against
Debian `/usr/bin/chromium` 145.0.7632.116 and local Chromium source revision
`a03603fe9af6`. Five new executable contracts cover target-local Debugger
pause/resume, same-origin localStorage fan-out versus target-local
sessionStorage, target-owned navigation-history entry ids, attachment-ordered
union and detach cleanup for `Network.setBlockedURLs`, and OR aggregation for
`Network.setCacheDisabled`. The cache probe also records Chromium's two cache
layers: a bypass load replaces the owning Page's retained resource and the
shared HTTP-cache entry, an already-live peer Page retains its earlier
resource, and a newly created Page sees the replacement. The exact same full
group passed three consecutive fresh Chromium profiles and ten fresh Moli
server processes, with the Moli runs scheduled four at a time to expose
cross-target ownership races. This calibration also corrected two older
assumptions: wire and live UA overrides both follow session attachment order,
and an activation-churn preload probe must enable the Page agent before
registering its new-Document script.

Every `multi-page` case runs even when an earlier case fails, so one race does
not hide later failures. For focused reproduction, set
`MOLI_MULTI_PAGE_CASES` to a comma-separated list of case names. Parallel
workers can partition that selected list with the one-based
`MOLI_MULTI_PAGE_SHARD=INDEX/COUNT` setting.

## Current Coverage

The current suite is a strong core smoke gate, not a complete Playwright compatibility suite.

The `dom-input` keypress contract was calibrated on 2026-09-08 against
headed Debian Chromium 145.0.7632.116 through CDP. It checks the full trusted
`keydown -> keypress -> beforeinput -> input -> keyup` sequence for Playwright
typing, cancellation at keypress, the distinct `rawKeyDown` and `char` paths,
and the absence of keyboard events for `Input.insertText`.

The default `webgl-viewport` group was calibrated on 2026-09-07 against
Debian `/usr/bin/chromium` 145.0.7632.116. It uses the same fixture as the
renderer tests to check WebGL1/2 on HTML and
Offscreen canvases: viewport initialization, setter/conversion errors, copied
Int32Array queries, context isolation, context reacquisition, resize retention,
and clamping to the advertised maximum. It is not a GPU rendering test.

The default `svg-rect` group checks the detached `SVGRect` interface used by
SVG capability detection, sharing the renderer fixture for prototype, identity,
and restricted-float conversion contracts. It was calibrated on 2026-09-07
against Debian `/usr/bin/chromium` 145.0.7632.116 and runs independently of
IndexedDB startup coverage.

The default `target-lifecycle` process group locks down Moli's resource lifetime,
not a Chromium-specific FD count. In one server it closes 800 default-context
targets with `Target.closeTarget`, 800 with `Page.close`, 128 after detaching,
and 64 by disposing explicit contexts. Foreground/background creation alternates.
It waits for each exact `Target.targetDestroyed`, records Linux `/proc/<pid>/fd`
and thread counts every batch, and checks a fixed post-warmup resource budget.
Each phase must still load a real HTTP document and preserve a live peer Page.
This catches closed renderer wakers retaining Tokio I/O drivers without changing
the test to use a fresh context for each default-context Page. Batch progress,
resource samples, and the usual server logs are retained on failure. With an
external endpoint or on non-Linux systems, protocol churn/navigation still run;
the artifact explicitly reports that FD sampling was unavailable. CI uses the
managed Linux server, so the resource assertions are mandatory there.

Covered well:

- The default raw `debugger-breakpoints`, `runtime-exception`, and
  `file-chooser` groups preserve focused Lexbench regressions at the public
  process boundary, including multi-attachment Runtime exception contracts.
  They dispatch `Debugger.getPossibleBreakpoints`,
  `setBreakpoint`, `removeBreakpoint`, and `setBreakpointByUrl` while the Page
  is normally running, require an uncaught timer error to publish
  `Runtime.exceptionThrown` without a follow-up command, verify that each
  Runtime-enabled attachment receives the target-owned exception while a
  disabled peer does not, keep `Runtime.enable` from making Error stack cost
  track JavaScript stack depth, and require a user-gesture file-input activation
  to publish the session-scoped `Page.fileChooserOpened` event.
- The default raw `url-policy` group holds the hosted local-file boundary at the
  public process edge. It requires an exact session-routed `Page.navigate`
  `-32000` error with no lifecycle or document replacement, verifies page
  `fetch()` plus asynchronous/synchronous XHR error surfaces, and requires each
  script request to terminate as `Network.requestWillBeSent` followed by
  `Network.loadingFailed` without `Fetch.requestPaused`, a response, or transport
  completion. The same group exercises non-URL-policy synchronous XHR with a
  blocked port, including the Chromium-shaped `NetworkError`, `DONE`, reset
  response fields, and absence of XHR/upload progress events.
- The default `xhr-sync-semantics` group runs unchanged against Chromium and
  Moli. It covers Content-Length/no-length/204/data responses, POST with
  and without a body, GET/HEAD body suppression, redirects, connection reset,
  malformed data URLs, blocked ports, unsupported redirect schemes, 302/303 DNS
  failures, redirect loops, exact Window timeout/responseType exceptions,
  `open()` state reset, duplicate `send()`, synchronous event order, pending
  asynchronous request cancellation, and suppression of asynchronous events and
  timers while a synchronous request owns the main thread.
- The focused `layout-screenshot` group drives a real raw WebSocket session through target creation/attachment, fixed viewport, lifecycle-gated navigation, DevTools-style PNG capture (`quality: 100`), paint/layout mutations, page clips, `captureBeyondViewport`, and the Chromium DevTools node-screenshot chain (`DOM.getBoxModel` + `Page.getLayoutMetrics` + page clip). Its Moli-only TreeScope fixture also captures 104 open/closed Shadow Roots, nested roots, and 24 roots in an iframe twice, requiring stable pixels and completion within the protocol timeout. The group covers Moli's generation-gated 1 FPS JPEG screencast as well: initial-frame delivery, clean-state frame suppression without ACK backpressure, 400x300 scaling, metadata/session routing, mutation freshness, stop cleanup, and a separately restarted default-Mock boundary without `--layout`. The screenshot surface sequence can also run against Chromium as a coarse reference; the TreeScope, fixed-1-FPS, and default-Mock branches are Moli-only.
- The default `layout-policy` group locks Moli's first-demand screen-layout
  lifecycle through raw CDP. Geometry reads and coordinate input initialize a
  cold Document once, then reuse its snapshot after DOM/CSS/viewport changes.
  Screenshots and actual screencast frames publish fresh screen layout; PDF
  uses temporary print layout without publishing. Plain DOM/Runtime reads,
  computed style (including used size and Grid), stylesheet reads/writes,
  snapshots, and emulation changes neither initialize nor refresh it. The
  matrix also checks live viewport/scroll versus frozen content size, missing
  boxes, invalid zero clips, focus/touch branches that need no layout, and
  four Document replacement paths. Each case uses a new target and mutates a
  known box **before** its assertion reads geometry, proving whether the
  command under test published layout without accidentally warming the page.
  `Performance.getMetrics.LayoutCount` is not used as a layout oracle.
  DOMSnapshot placeholder bounds are not asserted as a stable contract.
  This group tests Moli's policy and explicitly skips other browsers; it does
  not claim Chromium freezes layout after mutations.
- The default `playwright-compat` group takes viewport, clipped, full-page,
  and full-page clipped screenshots through `page.screenshot()` on fresh pages,
  without a geometry probe or raw CDP capture first. DOM geometry reads initialize
  Moli's first layout on demand; subsequent reads consume that published layout
  until a screenshot or new screencast frame refreshes it. The tall-page fixture
  checks the first full-page image's height and bottom pixels, then repeats the
  full-page capture. It sets `scrollbar-width: none` explicitly to match
  Playwright's Chromium `--hide-scrollbars` launch default. With visible native
  scrollbars, both Chromium and Moli produce a 305px-wide full-page capture for
  this narrow-content, 320px-viewport fixture. Empty clips remain invalid
  regardless of layout state.
  CLI `moli fetch --layout --dump screenshot_full` directly requests full-document
  output and works on the first capture without a DOM-size preflight.
  The four first-capture modes were checked with the same test against
  `/usr/bin/chromium` 145.0.7632.116 on 2026-09-27 (one run per mode).
- The default raw `action-window` group holds Moli's on-demand input policy at
  the public CDP boundary. Three acknowledged `Input.dispatchMouseEvent`
  wheel commands remain delayed until one fixed one-second deadline, preserve
  event order, defer their microtask checkpoint, and publish one derived
  IntersectionObserver transition. Coordinate-targeted vertical and horizontal
  wheels must update an inner `overflow: auto` container's `scrollTop` and
  `scrollLeft` without moving the page. `Page.captureScreenshot` must flush
  pending wheel work before paint and retire that window so later input receives
  a fresh deadline. A wheel handler that calls `document.open()` must also stop
  the remainder of its batch from entering the replacement Document. These are
  Moli scheduling contracts rather than Chromium-compatibility claims; the group
  records a non-applicable result when pointed at Chromium.
- The focused `dom-hit-test` group drives `DOM.getNodeForLocation` through the real layout hit index and requires the same option-aware topmost node, backend node id, and frame id shape from Moli and Chromium.
- Node creation stack capture retains at most the newest 1,024 traces per Inspector session; focused renderer tests cover FIFO eviction, shared payloads, document replacement, and session detach while the wire smoke retains enable/disable/session semantics and verifies that `document.open()` preserves capture for replacement-document nodes.
- Optional chrome-remote-interface 0.34.0 and cdp-use 1.4.5 coverage for verified browser/page sessions, multiple targets, local/session storage ownership, history traversal, child-frame isolated worlds, Fetch fulfillment with page-session event routing, complete Network terminals, and the declared position-click boundary.
- Optional Stagehand 3.7.0 deterministic coverage for explicit CDP binding, navigation/evaluate, locator fill, attached-state waiting, shadow piercing, multiple pages and storage ownership, history, frame registry/deep locators, Network extra headers plus page fetch, and the declared position-click boundary. LLM `act`/`extract`/`observe` and a route API are not part of this group.
- Optional agent-browser CLI coverage for explicit CDP binding identity, navigation/read/evaluate, fill and keyboard input, media override, Network route/request observation, tab lifecycle, trace/profiler transport, and the declared position-click boundary. The group uses a unique daemon namespace and an empty config so it cannot silently attach to or launch a different browser.
- CDP discovery and `chromium.connect_over_cdp()`.
- Concurrent 2-, 3-, and 7-client raw browser and direct-page WebSocket fan-out
  against the same target, including colliding per-client command ids, distinct
  session ids, alternating discovery subscriptions, attach-event-before-response
  ordering, four-command per-client FIFO bursts, shared target state without
  response/event crossover, foreign flattened and legacy session rejection,
  and staged peer-disconnect isolation.
- Raw CDP websocket command flow for `Runtime.evaluate(awaitPromise=true)` resolving page `fetch()`, timer-triggered `fetch()`, and WebSocket echo work without any follow-up client command; emitted `Runtime.executionContextCreated.uniqueId` round-tripping through DevTools-shaped `Runtime.evaluate` and `Runtime.callFunctionOn`; Chromium-calibrated pre-commit navigation suspension where DOM/Runtime/Debugger main-thread commands wait while `Performance.getMetrics`, `Runtime.terminateExecution`, and browser commands remain dispatchable; `Debugger.pause` responding before `Debugger.paused`, interrupting an in-flight `Runtime.evaluate`, and resuming that evaluation; commands queued behind a winning `Debugger.resume` completing through normal owner dispatch rather than synthetic cancellation; deterministic nested-function `Debugger.stepOut` response/resumed/caller-pause ordering; browser-global Tracing ownership across independent browser/page WebSocket frontends, including exactly one response for a synchronously completed start and the stop-before-start-ack `end response -> start error -> data -> complete` sequence; shared worker target discovery through `Target.getTargets`, worker-session `Runtime.executionContextCreated` / console log replay, and `Profiler.enable` / `Profiler.start` / `Profiler.stop` through `Target.setAutoAttach`; plus Chromium-calibrated DedicatedWorker target creation/update/attach ordering, exact worker-isolate Runtime/Console routing, `Inspector.workerScriptLoaded`, terminate, and owner-navigation cleanup.
- The default raw `inspector-routing` group is the executable DevToolsSession boundary matrix. It covers per-session Main/IO FIFO and exactly-once completion, IO preemption of non-yielding JavaScript, DedicatedWorker and SharedWorker interrupt overtaking plus FIFO recovery, all 13 methods in Chromium 147's `ShouldSendOnIO`, normal debugger-pause pumping of one mixed V8/Page/DOM Main receiver, instrumentation-pause IO-only behavior, navigation replacement, attached-session detach, BrowserContext teardown with interrupts in flight, and `Page.crash`. Every scenario runs in an isolated target and records its Chromium-derived contract.
- The focused raw `agent-episode` group copies the recorded RL
  `Runtime.evaluate(awaitPromise=true)` observe/fill/click path. It requires the
  action response before destructive cross-document realm events, observes only
  the replacement Document, and then proves that a transport reset commits a
  Runtime-usable error Document instead of returning `Promise was collected`
  or permanent `NoDocumentLoaded`. Same-document fill state is asserted through
  live DOM text; sampled `innerText` is recorded but may remain stale until the
  next layout refresh by design.
- The focused raw `dom-parser-mutations` group holds a parser-blocking head script after an early head-only `DOM.getDocument` and requires Chromium's exact root-agent sequence: commit `DOM.documentUpdated`, parser-tail BODY `DOM.childNodeInserted`, DCL `DOM.documentUpdated`, then `Page.domContentEventFired`. It also proves that the early frontend node id is stale after the DCL barrier and that a refreshed snapshot contains the complete BODY.
- Optional Puppeteer over CDP group for `puppeteer.connect()`, reconnecting an existing background Page and selecting it through a fresh browser session, browser-target and page-target `CDPSession`, `page.goto()`, `page.reload()`, selector-backed DOM activation navigation, same-document hash and History API navigation, `page.evaluate(fetch)`, keyboard input via `page.type()`, CSS / `$eval` / XPath element selection, and DOM interactions across text input, textarea, label/checkbox, radio, select, details/summary, disabled button, and form submission. It also covers `ElementHandle.boundingBox()` / `evaluate()` / `uploadFile()`, DedicatedWorker `workercreated` / `WebWorker.evaluate()` / exact-once worker-session console routing / explicit terminate / navigation-destroy lifecycle, current-viewport `page.screenshot({captureBeyondViewport:false})`, alert and console events, browser-session download behavior/events/artifact with peer-session event isolation, request interception `respond()` / `continue()`, page-scoped `CDPSession` Network event observation, and layout-backed `page.click()` dispatch.
- `browser.new_context()`, `context.new_page()`, multiple pages in one context, target switching, popup-scoped `page.route()` plus `evaluate(fetch)`, popup CDPSession response-stage body / stream / fulfill / fail flows, and held multi-context route / response-stage resume without cross-context Network event bleed.
- Top-level navigation, redirect final URL/response, reload-like click navigation, and history back/forward.
- Reload, same-document hash navigation, `history.pushState()` observation, Playwright `add_init_script()` page/context injection, `page.exposeFunction()` / `page.exposeBinding()` plus context-level exposed functions, and basic `domcontentloaded` / `load` / `networkidle` load-state waits with a parser-discovered delayed image.
- Basic popup target linkage through Playwright's `page.expect_popup()`, anchor `target="_blank"` popup activation, named popup target reuse, reserved-target `window.open(..., "_self")` current-page navigation, basic alert dialog handling through Playwright's `page.expect_event("dialog")` and `dialog.accept()`, and raw CDP prompt event/close shape.
- Basic iframe frame-tree consumption through `Frame.text_content()`.
- `page.wait_for_function()` string predicates, timer readiness, return
  JSHandle values, primitive/ElementHandle arguments, and timeout errors, plus
  Playwright `page.wait_for_selector()` / `locator.wait_for()` attached,
  visible, hidden, detached, and enabled-click auto-wait behavior.
- Chromium inspector-protocol derived samples for `Page.domContentEventFired` before `Page.loadEventFired`, `Page.frameStartedLoading` / `Page.frameStoppedLoading`, non-empty `Page.frameAttached.parentFrameId`, `Page.getFrameTree`, dynamic and nested child-frame event fan-out across attached sessions with session-local lifecycle/disable state, `Page.navigate` fragment navigation, `Page.getAppManifest` default/loading/parsing/error/redirect/dynamic-link contracts, successful-result caching and link invalidation, plus its `Manifest` Network request and terminal lifecycle, `Page.getLayoutMetrics`, `Runtime.executionContextCreated`, `Runtime.evaluate(returnByValue)` and exception details, session-local `Input.setIgnoreInputEvents` aggregation/navigation/detach behavior, `Input.insertText` bypass, idle `Input.cancelDragging`, `Audits.issueAdded` Quirks/CSP shape, replay ordering, navigation storage reset and session-local enable/disable, `Log.entryAdded` network metadata, buffered replay ordering, session-local delivery cursors, target-shared `Log.clear`, violations-report state and validation, `IO.resolveBlob` session-local object resolution and reopenable `blob:<uuid>` streams, session-local `Performance.enable/disable`, strict time-domain transitions, disabled `Performance.getMetrics`, Moli's `Emulation.setCPUThrottlingRate` unsupported-rate errors and neutral resets across pages, `Profiler.start` / `Profiler.stop` CPU profiles and continued recording after rejected CPU throttling, error contracts, attached CDP-session profiler isolation across navigation and detach/reattach, `console.profile` / `console.profileEnd`, precise coverage / best-effort coverage including not-started error, counter reset, and detailed block coverage, `DOM.getAttributes`, `DOM.querySelector(All)` including default-depth node-path publication through ordered `DOM.setChildNodes` events, deep ancestry expansion, repeat suppression, and the chromedp `NodeReady` contract, live `DOMDebugger.getEventListeners`, session-owned event-listener breakpoint pause/re-pause/navigation/detach behavior, session-owned XHR/fetch breakpoint URL matching, synchronous pause data, navigation/child-frame/worker scope, multi-owner sequencing and detach behavior, live-node DOM mutation breakpoints for single-node and multi-child `DocumentFragment` insertion batches, connected-node removal/insertion phases with one pre-pause `DOM.childNodeRemoved`, same-value attributes and node removal with owner/peer data, parser mutation no-pause behavior, unbound-node path ordering, disable/navigation cleanup, and the `DOM.getNodeForLocation` hit-test capability boundary.
- Raw `Input.dispatchKeyEvent` and `Input.dispatchMouseEvent` commands whose DOM handlers initiate a top-level Page replacement; both command responses must remain successful and the replacement Page must immediately accept follow-up CDP work. Focused Rust coverage separately holds the renderer ACK so the cleanup branch itself is deterministic.
- Chromium-calibrated `Tracing` browser-global ownership, duplicate start and peer end errors, data-source start acknowledgement before the `Tracing.start` response, exactly-once synchronous start responses, stop-before-ack response/error ordering, response-before-data ordering, cross-session clock markers, bounded `ReportEvents`, JSON `ReturnAsStream` through `IO.read`, and owner-detach cleanup. The CPU-profiler configuration additionally requires real V8 `Profile` / `ProfileChunk` events, non-empty samples with aligned `timeDeltas`, and named hot functions across navigation replacement, dedicated/shared worker teardown, and closed page targets. The same content contract is applied to the real agent-browser profiler artifact. Proto, gzip, Perfetto, system tracing, and periodic buffer reporting remain explicit unsupported boundaries rather than mock output.
- Chromium-calibrated live DOM mutation mirroring and editing: shallow/deep `DOM.getDocument` projections, `DOM.characterDataModified`, `DOM.childNodeCountUpdated`, `DOM.childNodeInserted`, `DOM.childNodeRemoved`, and event-before-response contracts for `DOM.moveTo`, `DOM.setAttributesAsText`, `DOM.setNodeName`, `DOM.setNodeValue`, and `DOM.setOuterHTML`, including same-value character-data writes, processing-instruction `xml` renaming, and returned frontend node identity.
- Chromium-calibrated Inspector depth-boundary projection: `DOM.getDocument` and `DOM.requestChildNodes` still publish a container's only text child at depth zero, including the common `<title>Example Domain</title>` shape, while containers with multiple children remain collapsed.
- Chromium-calibrated Inspector DOM projection through the focused `dom-whitespace` group. Its fixture preserves the indentation-heavy `widget_plate` / `blog` / `blog_chunk` shape of `ldm0.top`: default `DOM.enable` omits whitespace-only text children, while `includeWhitespace=all` exposes them and keeps `childNodeCount` aligned with the projected tree. It verifies first-enable locking and disable/reset, child-frame default/all projection, visibility-transition insert/remove versus character-data events, and independent-session `pushNodesByBackendIdsToFrontend`, `requestNode`, `describeNode`, and XPath search identity. CSS/manual UA-shadow search is option-gated while XPath remains document-scoped; search results stay `0` before document publication and reuse a positive ID afterwards. The same group proves that node creation stack capture is session-local, non-retroactive, preserved across disable, and applied to fragment-created nodes.
- A focused `computed-style` group runs unchanged against Chromium and Moli. It checks JavaScript computed-style enumeration and `CSS.getComputedStyleForNode` breadth, representative layout-independent values, custom properties, longhand-only unique names, repeated-read stability, and mutation freshness while recording both property counts as diagnostics.
- Chromium-calibrated `DOM.getOuterHTML(includeShadowDOM=true)` through the focused `dom-shadow-outer-html` group: nodeId, backendNodeId, objectId, and a detached object return identical recursive author-shadow markup; omitted/false remain shadow-free; declarative and closed roots are included; user-agent roots and Moli-only shadow-template attributes are excluded; child-frame host/document references retain the option; and serialization emits no DOM mutation events.
- Chromium-calibrated `Autofill.trigger` card behavior on a detected payment form: live values without `value` attribute mutation, `:autofill` state, preserved focus, trusted `input`/`change` ordering, and successful no-op behavior for an ordinary unclassified field.
- Playwright upstream derived samples for route request metadata, `page.route()` precedence over `context.route()`, context fallback, `times`, fallback chaining, terminal fulfill/abort routing, `route.fulfill()` cookies/headers, successful `page.pdf()` stream transport, page/main-frame/browser `CDPSession.send()`, session Network event delivery, unknown command errors, and post-detach command rejection.
- Request interception for main documents, page `fetch()` / `XMLHttpRequest`, worker `fetch()` / `XMLHttpRequest`, page `fetch()` response-stage pause / continue / fulfill / fail, page XHR response-stage `Fetch.getResponseBody`, page fetch response-stage `Fetch.takeResponseBodyAsStream` / `IO.read`, and basic page/worker fetch plus worker XHR `Fetch.authRequired` / `continueWithAuth` flows. `CancelAuth` coverage requires the challenged `401` response and body to remain observable, allows a configured response-stage pause, completes with `Network.loadingFinished` rather than `Network.loadingFailed`, and includes a top-level navigation driven by a separate Playwright CDP session.
- Dedicated worker `postMessage` round trips and same-URL/same-name SharedWorker port reuse, including worker-global identity and connection-count assertions.
- Attached `CDPSession` observation for `Network.requestWillBeSent`, `Network.responseReceived`, `Network.loadingFinished`, and `Network.loadingFailed`, including fetch/XHR POST body, request/response-header fidelity, and redirect-chain event ordering.
- Chromium-derived `Network.requestWillBeSentExtraInfo` / `Network.responseReceivedExtraInfo` correlation for normal HTTP documents, no-cookie redirects, cache revalidation, ordinary page `Fetch.continueRequest`, page and worker Basic auth, and requests sent before the server resets the connection without response bytes, both on the initial request and after a completed redirect hop. Worker coverage includes Fetch and XHR, with XHR held through a response-stage pause; worker `CancelAuth` requires one initial unauthenticated request ExtraInfo and one raw `401` response ExtraInfo. Coverage includes transport-generated `Host` / `Accept-Encoding`, per-hop exchange counts and status sequence, path-mismatched cookies using the public `NotOnPath` blocked reason, auth retries retaining the initial unauthenticated request headers, `Fetch.authRequired` rounds correlated only by `Fetch.requestId` without a non-standard `networkId`, a raw `304` ExtraInfo response paired with the merged cached `200` response, `redirectHasExtraInfo`, and `hasExtraInfo` without assuming an order CDP does not guarantee. Fetch interception also verifies Chromium's shared `XHR` type for page Fetch, XHR, and EventSource requests under each of the `Fetch`, `XHR`, and `EventSource` filters, while Network events retain their distinct high-level types. Reset cases require the complete observed request/redirect ExtraInfo set, no final response event, and a successful `Page.navigate` result carrying `net::ERR_CONNECTION_RESET` in `errorText`; they do not assume ExtraInfo precedes `loadingFailed` or that Chromium will not later finish its internal error document. Response-stage interception also verifies that original transport ExtraInfo is visible before `Fetch.requestPaused`, while a `Fetch.continueResponse` status/header override changes the later `Network.responseReceived` without emitting duplicate ExtraInfo.
- The focused `error-document` group turns the reset-before-response fixture into an end-to-end failed-navigation gate. It checks the direct and redirected event order, the split between internal frame URL and user-visible Target/history URL, `unreachableUrl`, independent loader/realm generations, old-global retirement, consecutive failures, successful recovery, and concurrent target isolation. Every scenario requires Runtime to remain usable after the failure, so the pre-fix permanent `NoDocumentLoaded` state fails the smoke rather than being hidden by a later successful navigation.
- Chromium-derived proxy authentication coverage uses a real local proxy and per-browser-context proxy configuration. HTTP Basic authentication must expose the initial unauthenticated target request headers and final `200` response through ExtraInfo without leaking `Proxy-Authorization`; an HTTPS `CONNECT` challenge canceled through Fetch must expose proxy origin, a `407` response with `hasExtraInfo=false`, and `Network.loadingFailed` without treating CONNECT headers as target-request ExtraInfo.
- Parser-discovered external script, link stylesheet, and parser-created `@import` stylesheet observation plus `Network.getResponseBody`; configured `20,000,000 / 2,000,000` inspector-cache budgets retain a small body while a `2,000,001`-byte body keeps its request identity and repeatedly returns the inspector-cache eviction error.
- Classic WebSocket echo, WebSocket `Network.webSocket*` events, and blocked WebSocket handshake behavior.
- The focused `classic-scrollbar` group compares Chromium and Moli for
  block/flex/grid `stable both-edges` numeric sizing, default-visible root
  stable gutters, body hidden/auto/clip viewport overflow propagation,
  `overflow-body-propagation-003`'s `display:contents` exclusion, and an overlay
  that occludes a lower painted scrollbar. The numeric matrix additionally
  covers padding/border with both box-sizing modes, auto/min/max block sizing,
  aspect-ratio, and physical top/bottom gutters in `vertical-rl`. Block, flex,
  and grid cases also assert that leading `both-edges` gutters neither create a
  phantom `scrollWidth`/`scrollHeight` nor inflate a real overflow range. It
  also verifies Moli's consume-only scrollbar corner and screenshot-triggered layout freshness.
  The final workflow was calibrated on 2026-08-22 against the headed Xvfb
  build `~/chromium/src/out/Default/chrome` (`Chrome/147.0.7709.0`) and then
  extended with the physical-inset matrix on 2026-08-23 against that same
  executable before passing unchanged against Moli. Chromium/Xvfb routes raw-CDP clicks on its
  native scrollbar corner through DOM input, so only that control-consumption
  assertion remains Moli-specific.
- Chromium-calibrated transformed-iframe input routing through raw CDP: exact
  used child viewports, hover/click/wheel targets and child-local coordinates,
  overflow-container scrolling, focus chains, and cross-frame mouseout
  coordinates across both single and nested transformed frames. On 2026-08-18,
  `uv run moli-cdp-smoke --endpoint http://127.0.0.1:9228 --group iframe-input`
  passed three consecutive runs against
  `~/chromium/src/out/Default/chrome` (`Chromium 147.0.7709.0`) under headed
  Xvfb; the same group then passed three consecutive Moli runs. The nested UA
  scrollbar control assertion is Moli-only because Chromium/Xvfb does not route
  raw CDP synthetic mouse input through its platform scrollbar widget.
- Dedicated default `Page.setDocumentContent` replacement coverage through
  Playwright and raw CDP: exact-once, ordered document-open/DOM/load projection
  to two attached sessions while one session has a pending Runtime command;
  session-local lifecycle enable state; preserved Document/realm/history/frame
  identity; repeated replacement and detached `ElementHandle` behavior;
  stylesheet-candidate cleanup; no navigation or realm teardown; deterministic
  parser pause/resume at a body stylesheet; child-frame replacement; and error
  atomicity.
- Dedicated raw CDP DOM replacement coverage through the `dom-snapshot` group:
  `document.open()` replacement must reject stale frontend `nodeId`, keep retained
  old `objectId` detached from the new document, allocate distinct frontend and
  backend node ids for the new live node, and make `DOMSnapshot.captureSnapshot`
  reflect only the current document contents.
- Basic `page.set_content()` static DOM and inline script execution, plus
  Playwright `addScriptTag()` / `addStyleTag()` content, URL, path, and
  missing-URL rejection workflows.
- File upload through `set_input_files`, direct file chooser, scripted `showPicker()`, FileReader content reads, file-name-with-spaces replacement, and repeated input/change events.
- Locator/input workflows for fill/clear, Playwright user-facing `get_by_test_id()` / `get_by_text()` / `get_by_label()` / `get_by_placeholder()` / `get_by_alt_text()` / `get_by_title()` / `get_by_role()` plus role selector selected/checked/pressed/expanded/disabled/level/name/include-hidden filters, Playwright `expect(locator)` text/count/value/attribute/class/visible/hidden/enabled/disabled/checked matchers, `has_text` / `has` / `filter()` locator composition, upstream-derived `first()` / `last()` / `nth()` / `and_()` / `or_()` / Locator-argument composition and `FrameLocator.locator()` workflows, `$eval` / `$$eval` selector evaluation, `page.type()` selection/focus behavior, `fill()` input/change events, `locator.clear()` input event, input type/error/auto-wait behavior, `check()` / `uncheck()` / `setChecked()` state, aria-role, trial, error, and label-retarget behavior, type, press, keyboard modifiers, basic contenteditable editing, hover, checkbox, radio, richer Playwright `selectOption()` value/label/index/handle/multiple/wait behavior, click-triggered navigation, and drag/drop. Raw CDP smoke checks invalid-parameter priority plus explicit layout hit-testing errors for mouse, touch, emulated touch, tap, and drag, and verifies that none dispatch DOM events. Playwright `page.mouse` click/dblclick/buttons/move/wheel workflows enforce the same boundary.
- DOM/handle workflows for `locator.evaluate()`, `ElementHandle` queries, `ElementHandle.content_frame()`, `ElementHandle.wait_for_element_state()` visible/hidden/enabled/editable/timeout/detached behavior, `JSHandle.as_element()`, `JSHandle` property reads, nested/same-handle `evaluate()` arguments, console events with JSHandle args, bounding boxes, owner frames, child-frame evaluation, detached-handle behavior after navigation, and isolated-world DOM resolution through Playwright's injected scripts.
- Download event, downloaded artifact, and download cancellation.
- Direct `Page.navigate` outcome coverage for attachments, empty and redirected
  downloads, download HTTP errors, binary MIME responses, ordinary HTTP error
  Documents, 204 responses, and transport failures, including response/request
  correlation and retained-Document checks.
- Viewport resize, the explicit Playwright-generated screenshot clip boundary, and Chromium-calibrated geolocation
  position/unavailable/clear behavior across navigation and attached CDP sessions.
- localStorage, sessionStorage, basic IndexedDB, multi-context cookie isolation,
  Blob DevTools UUID storage-partition isolation, and browser-context profile
  overrides for user agent, locale, timezone, extra headers, and navigation
  referer.
- Chromium-calibrated `target-semantics` and `browser-semantics` contracts for
  stable Tab/Page ownership, TargetHandler access modes, repeated auto-attach
  reconciliation, foreground/background activation, Page and screencast
  visibility, default-target discovery and close, target
  multi-attach/close/context disposal, real-URL target debugger-wait lifecycle
  correlation, history entry identity and metadata, frame metadata and detach
  order, top-level/popup storage,
  DOMStorage events, resource trees/search, XML, isolated worlds, EventSource,
  HTTP and CacheStorage caches, CSS Typed OM, and View
  Transitions. Each contract records its invariant, source, command chain, and
  observed values independently.

Intentional non-goals:

- The full Web Animations timing/state model, including `finish()`/`cancel()`
  `currentTime` parity. Surface-level API compatibility may remain, but it is
  not a cross-engine smoke gate.

Important gaps:

- Input precision: richer keyboard composition and deeper editable/contenteditable selection defaults remain in scope. Layout-backed single-pointer mouse/touch/tap and raw drag dispatch are covered; Chromium drag interception (`Input.dragIntercepted`), multi-touch dispatch, and mobile/touch emulation remain gaps.
- DOM/handle precision: stronger stale-handle matrix, cross-frame isolated-world handle conversion,
  deeper mutation-tree replacement edge cases, and deeper selector/hit-test accuracy.
- Navigation/lifecycle variants: popup target creation is covered through Target events, Puppeteer `waitForTarget()`, and Playwright `page.expect_popup()`; named popup targets reuse an existing CDP target instead of creating duplicates; reserved `_self` / `_top` / `_parent` popup targets navigate the current page in the current simplified top-level model. `window.open()` now returns a non-null WindowProxy projection. JavaScript dialogs block the invoking script until an automation client accepts or dismisses them, and prompt return values plus opening/closing protocol fields are covered. Remaining gaps are full WindowProxy/opener scripting behavior, background popup document load/activation after named-target reuse, native browser dialog UI, sharper `wait_until` edge cases, same-document history entry precision, and target lifecycle edge cases.
- Navigation response precision: Puppeteer selector-backed DOM activation completes and updates `page.url()`, while the smoke records whether `waitForNavigation()` exposes the committed document response. `page.goto()` and `page.reload()` response objects are required separately. Coordinate `page.click()` is supported when the server runs with `--layout`; Mock policy still fails explicitly.
- Same-document navigation precision: Puppeteer `page.waitForNavigation()` is now covered for hash-only and History API navigation through renderer-produced `Page.navigatedWithinDocument` events. Remaining precision work is deeper history traversal/back-forward entry parity and Navigation API interception-style same-document reasons.
- Screenshot/layout/emulation: raw full-page/clip/node screenshots and raster-backed `Page.printToPDF` are covered, including base64/stream transfer, pagination, page ranges, orientation, print media, and background control. Header/footer templates, CSS page-size preference, tagged PDFs, document outlines, vector/text-preserving output, richer device-scale behavior, and more accurate layout metrics remain gaps. The Chrome `chrome://inspect` command menu is a separate DevTools-frontend concern and does not advertise Moli's screenshot capability.
- Wider network matrix: broader digest/multi-round proxy auth challenge variants, font/preload resource types, parser-discovered resource interception, and broader response-stage coverage beyond the current page/popup fetch/XHR response-stage smoke.
- Storage/profile depth: cookie delete/clear/domain/path/SameSite, persistent profile writeback, storage partition boundaries, and deeper IndexedDB flows.
- Worker/frame target depth: module-worker target lifecycle, nested DedicatedWorker targets, broader worker exception/debugger coverage, iframe subresource attribution, and future per-frame realm boundaries remain. DedicatedWorker now has first-layer CDP target/runtime/console/lifecycle coverage; SharedWorker has first-layer target/runtime/log/profiler and page-side reuse smoke, but both still need richer error matrices.
- Puppeteer parity: the default managed-client group covers the common connect/navigation/reload/same-document/evaluate/keyboard-input/DOM-selector-activation/coordinate-click/handle/current-viewport-screenshot/alert-dialog/popup-target/interception path and DedicatedWorker lifecycle, but full-page/clip screenshots, drag interception, broader dialog edge cases, and richer lifecycle waits remain.

Runner layout:

- `runner.py`: starts the fixture, starts `moli serve`, connects Playwright over CDP, and runs scenario groups.
- `fixture.py`: local HTTP/WebSocket fixture server and fixture routes.
- Rust CDP smoke scaffold: `tests_cdp_smoke_fixture.rs` mirrors the fixture routes for Rust tests, while `tests_cdp_smoke_chromium.rs` and `tests_cdp_smoke_playwright.rs` split Chromium/Playwright-derived CDP contracts into individual libtest cases.
- `serve.py`: `moli serve` process lifecycle and CDP readiness probing.
- `assertions.py`, `helpers.py`, `state.py`, `config.py`: shared runner utilities and state.
- `groups/core.py`: discovery-adjacent page workflows: navigation, iframe, wait, cookies, redirects, and history.
- `groups/protocol.py`: raw CDP protocol workflows that intentionally avoid Playwright helper commands, including shared worker target discovery, Runtime context/log replay, the Chromium/V8 `Debugger.pause` response-event-resume and nested-function `Debugger.stepOut` resume/re-pause sequences, and profiler session state.
- `groups/multi_client.py`, `groups/multi_client_fanout.py`, and
  `groups/multi_client_support.py`: Chromium-calibrated 2/3/7-client
  browser/direct-page WebSocket routing, session ownership, command-id
  collision, per-client FIFO, attach event/response ordering, and staged
  disconnect isolation contracts, split into two-client, fan-out, and shared
  support layers.
- `groups/agent_episode.py`: short raw-CDP RL-shaped observation/action,
  response/realm ordering, and failed-navigation error-Document contract.
- `groups/fetch_runtime_teardown.py`: holds an exact module-fetch lease while CDP disposes its BrowserContext, then verifies that callback-thread cancellation, browser commands, and a replacement context survive teardown.
- `groups/chromium_cdp.py`: Chromium inspector-protocol derived samples for Page, Runtime, Input, IO Blob streams, Performance, Profiler, and DOM contracts.
- `groups/document_content.py`: Playwright and raw-CDP `Page.setDocumentContent` replacement identity, parser pause/resume, child-frame, and error-atomicity workflows.
- `groups/error_document.py`: failed main-document transport error Document identity, lifecycle, realm replacement, recovery, and multi-target isolation.
- `groups/navigation_outcomes.py`: direct Page.navigate download/no-document/error outcomes and matching Network evidence.
- `groups/dom_parser_mutations.py`: cross-engine raw-CDP parser-tail mutation publication and commit/DCL DOM binding barriers.
- `groups/layout_screenshot.py`: raw current-viewport PNG, DevTools parameter compatibility, paint/layout mutation freshness, open/closed Shadow Root and iframe TreeScope stability, generation-gated 1 FPS JPEG screencast/ACK behavior, and Moli default-Mock restart boundary.
- `groups/layout_policy.py`: cold initialization versus warm snapshot reuse,
  non-publishing reads/writes and print, screen publication, validation, and
  Document replacement through raw CDP.
- `groups/action_window.py`: raw wheel admission/deadline batching, screenshot
  flush/reset, derived-effect coalescing, and exact-Document retirement.
- `groups/pdf.py`: raw `Page.printToPDF` base64 and `ReturnAsStream` transport, `IO.read`, pagination, page ranges, orientation, PDF structure, and Chromium-shaped validation errors.
- `groups/dom_snapshot.py`: raw CDP `document.open()` replacement identity and
  `DOMSnapshot.captureSnapshot` freshness workflows, intended to run unchanged
  against a Chromium CDP endpoint before validating moli.
- `groups/playwright_compat.py`: Playwright upstream derived route and CDPSession compatibility samples.
- `groups/chrome_remote_interface.py`, `groups/cdp_use.py`, and `groups/stagehand.py`: optional published-client workflows, with subprocess and JSON-result handling shared by `groups/external_process.py`.
- `groups/puppeteer.py` and `puppeteer_smoke.mjs`: pinned Puppeteer workflows driven from the uv runner through Node. `puppeteer_shadow_accessibility.mjs` covers Shadow DOM snapshots, scoped AX refs, ARIA handle clicks, and slot/name updates. `puppeteer_accessibility_visibility.mjs` covers computed visibility, CSS overrides, stable refs, and Document/shadow CSSOM updates.
- `groups/agent_browser.py`: optional real agent-browser CLI workflows with isolated config, daemon namespace, and endpoint identity verification.
- `groups/network.py`: document/fetch/XHR routing, Network event observation, parser script and stylesheet body capture, WebSocket, and downloads.
- `groups/workers.py`: dedicated worker `postMessage`, SharedWorker port reuse, worker fetch routing/auth, and worker XHR routing.
- `groups/dom_input.py`: `set_content`, file upload, file chooser, scripted picker, click navigation, locator/input workflows, Playwright upstream derived locator composition, `page.type()`, layout-backed mouse/touch/tap/raw-drag input, `fill()`, `check()` / `uncheck()` / `setChecked()`, and `selectOption()` workflows, DOM/handle workflows, plus the explicit high-level drag-interception boundary.
- `groups/iframe_input.py`: a Chromium-calibrated raw-CDP regression for
  transformed single and nested iframe hover, click, wheel, child-local event
  coordinates, exact used frame viewport propagation, and Moli's nested-frame
  UA scrollbar routing boundary.
- `groups/emulation_storage.py`: viewport and Playwright screenshot-clip boundary, detailed geolocation plus locale/timezone runtime overrides, storage/cookie isolation, IndexedDB baseline, and browser-context profile overrides.
- `groups/font_face.py`: process-isolated FontFace payload validation contracts.
- `groups/media_error.py`: process-isolated HTMLMediaElement MediaError lifecycle contracts.
- `groups/target_semantics.py`: raw Target-domain identity, attachment,
  activation, visibility, and lifecycle contracts calibrated against Chromium.
- `groups/browser_semantics.py`: page/runtime cross-engine contracts calibrated
  against Chromium before they are applied to Moli.

Near-term expansion order:

1. Deepen popup and dialog fidelity: popup Target linkage is observable by Playwright/Puppeteer, anchor `target="_blank"` now enters the same popup activation path, named target reuse no longer creates duplicate targets at Target-discovery level, reserved current-context targets no longer create bogus popup targets, `window.open()` returns a WindowProxy projection, and automation-controlled JavaScript dialogs block and resume with the handled value. Remaining work includes fuller WindowProxy/opener behavior, complete background popup document loading after named-target reuse, native browser dialog UI, and Chromium's dialog suppression policies.
2. Add more network/resource-type coverage after that, especially font/preload observation, parser-discovered resource interception, proxy auth, and richer auth variants. Stylesheet link / `@import` has Network smoke coverage without requiring a following parser script, the Chromium-derived matrix covers image/media/text-track/XHR body capture, page `fetch()` response-stage pause / continue has a focused smoke, page XHR response-stage covers `Fetch.getResponseBody`, page fetch response-stage covers `Fetch.takeResponseBodyAsStream` / `IO.read`, and server-auth covers successful credentials plus Chromium-compatible `CancelAuth` for page fetch, worker fetch/XHR, configured response-stage continuation, and top-level navigation.
3. Keep low-frequency CDP methods out of this smoke unless they block a real Playwright workflow. Use focused Rust tests for narrow protocol shape regressions and keep each fixture-derived CDP contract as a separate test.

## Running

For Moli runs, build the binary from the repository root first:

```bash
cargo build -p moli
```

If both debug and release binaries exist, the runner uses the newest one by file modification time. Set `MOLI_BIN` when you need an exact binary.

Then run the smoke project:

```bash
cd moli-cdp-smoke
uv sync
npm ci
uv run moli-cdp-smoke
```

With no `--group`, the supervisor executes every raw, Playwright page, browser,
and repository-managed external-client group, including `inspector-routing`
and the pinned Puppeteer client. Every group runs in a separate worker process
with its own selected browser process, fixture server, cache/profile, Playwright context, and
temporary directories. One failed or wedged group therefore cannot retain
state or child processes for the next group.

The default concurrency is one worker. Use bounded parallelism explicitly;
CI runs the same unfiltered suite with four workers on both engines:

```bash
MOLI_BIN=../target/release/moli uv run moli-cdp-smoke --browser moli --jobs 4
uv run playwright install --with-deps chromium
uv run moli-cdp-smoke --browser chromium --jobs 4
```

Each run writes `summary.json` plus one log and one result JSON per worker under
`target/smoke/cdp/<run-id>/`. Use `--output-dir` for a stable artifact path and
`--timeout` to change the per-worker wall-clock limit. The worker also retains
its own protocol-operation timeouts, so the wall limit is a final process-level
safety boundary rather than the primary synchronization mechanism. Supervised
worker logs include the live browser stdout/stderr stream, so the last
renderer messages survive even when the supervisor terminates a wedged worker.
For failed workers, the supervisor prints the saved error/traceback before the
log tail, so CI output includes the cause even when the worker log only reports
the exception type. The result JSON and summary retain the same diagnostics.

External-client groups whose binaries or dependency environments are not owned
by this project remain explicit integration runs.

List available scenario groups:

```bash
uv run moli-cdp-smoke --list-groups
```

Run a focused subset while developing a CDP area:

```bash
uv run moli-cdp-smoke --group protocol --group network
uv run moli-cdp-smoke --group multi-client
uv run moli-cdp-smoke --group layout-screenshot
uv run moli-cdp-smoke --group layout-policy
uv run moli-cdp-smoke --group action-window
uv run moli-cdp-smoke --group pdf
uv run moli-cdp-smoke --group agent-episode
uv run moli-cdp-smoke --group fetch-runtime-teardown
uv run moli-cdp-smoke --group network-body-cache
uv run moli-cdp-smoke --group dom-input,emulation-storage
uv run moli-cdp-smoke --group font-face,media-error,emulation-storage --jobs 3
uv run moli-cdp-smoke --group document-content
uv run moli-cdp-smoke --group dom-snapshot
uv run moli-cdp-smoke --group dom-whitespace
uv run moli-cdp-smoke --group computed-style
uv run moli-cdp-smoke --group error-document
uv run moli-cdp-smoke --group navigation-outcomes
uv run moli-cdp-smoke --group url-policy
uv run moli-cdp-smoke --group inspector-routing
MOLI_SMOKE_GROUPS=protocol,websocket uv run moli-cdp-smoke
```

The complete Inspector routing group runs by default. Select one or more named
contracts while iterating with
`MOLI_INSPECTOR_ROUTING_SCENARIOS`:

```bash
MOLI_INSPECTOR_ROUTING_SCENARIOS=raw_cdp_active_javascript_main_io_lane_matrix \
  uv run moli-cdp-smoke --group inspector-routing

MOLI_INSPECTOR_ROUTING_SCENARIOS=raw_cdp_nested_v8_main_receiver_matrix,raw_cdp_nested_non_v8_main_receiver_matrix \
  uv run moli-cdp-smoke --group inspector-routing

MOLI_INSPECTOR_ROUTING_SCENARIOS=raw_cdp_dedicated_worker_active_javascript_interrupt,raw_cdp_shared_worker_active_javascript_interrupt \
  uv run moli-cdp-smoke --group inspector-routing
```

The Chromium 147 IO catalog exercised by the complete group is:

```text
Debugger.getPossibleBreakpoints  Debugger.getScriptSource
Debugger.getStackTrace           Debugger.pause
Debugger.removeBreakpoint        Debugger.resume
Debugger.setBreakpoint           Debugger.setBreakpointByUrl
Debugger.setBreakpointsActive    Emulation.setScriptExecutionDisabled
Page.crash                       Performance.getMetrics
Runtime.terminateExecution
```

Main and IO are ordered independently; the smoke never requires relative
ordering between the two routes. A normal debugger pause must pump mixed V8
and non-V8 Main commands from the same session in send order. An
instrumentation pause must leave Main blocked and admit only IO work.

Run the same focused group against an already-running Chromium CDP endpoint.
The Python workers are still separate, but an explicitly supplied external
browser endpoint is shared rather than restarted by the supervisor:

```bash
uv run moli-cdp-smoke \
  --endpoint http://127.0.0.1:9222 \
  --group dom-snapshot
```

The complete `inspector-routing` group was calibrated on 2026-08-16 against
the local Chromium build `Chrome/147.0.7709.0`. Point the same command at a
Chromium remote-debugging endpoint to re-run the oracle before changing a
routing contract. The two Worker active-JavaScript interrupt regressions were
also cross-checked on 2026-08-18 against `Chromium/145.0.7632.116`:

```bash
uv run moli-cdp-smoke \
  --endpoint http://127.0.0.1:9222 \
  --group inspector-routing
```

Run the Puppeteer group:

```bash
npm ci
uv run moli-cdp-smoke --group puppeteer
```

If `puppeteer-core` is installed outside this directory, expose it through Node resolution:

```bash
NODE_PATH=/path/to/node_modules uv run moli-cdp-smoke --group puppeteer
PUPPETEER_CORE_MODULE=/path/to/node_modules/puppeteer-core uv run moli-cdp-smoke --group puppeteer
```

Run the optional agent-browser group:

```bash
AGENT_BROWSER_BIN=/path/to/agent-browser uv run moli-cdp-smoke --group agent-browser
```

The tested CLI must be agent-browser 0.31.1 or newer. The group first runs `connect` and verifies
`get cdp-url` against `/json/version`; it never permits the CLI's browser auto-launch fallback.

Run the optional thin-client and Stagehand groups with the benchmark-pinned clients:

```bash
NODE_PATH=/path/to/node_modules \
CDP_USE_PYTHON=/path/to/cdp-use-venv/bin/python \
uv run moli-cdp-smoke \
  --group chrome-remote-interface,cdp-use,stagehand
```

The calibrated versions are chrome-remote-interface 0.34.0, cdp-use 1.4.5, and
Stagehand 3.7.0. Each group verifies the live CDP product against `/json/version`;
none may launch a fallback browser.

You can also select a specific binary:

```bash
MOLI_BIN=../target/release/moli uv run moli-cdp-smoke
```

Useful environment variables:

- `MOLI_BIN`: path to the `moli` binary under test.
- `CHROMIUM_BIN` / `--chromium-bin`: optional executable override for
  `--browser chromium`; otherwise use the locked Playwright browser.
- `--chromium-bin-for-group GROUP=PATH`: use a Chromium executable for one
  group while preserving the full group selection. May be repeated; each
  group's actual executable is recorded as `chromiumBin` in `summary.json`.
- `MOLI_CDP_PORT`: fixed CDP server port for single-worker debugging. By
  default every worker uses `--port 0` and reads the OS-selected endpoint from
  Moli's bound-listener log. A fixed port is rejected with parallel workers.
- `MOLI_SMOKE_GROUPS`: comma-separated smoke group list. CLI `--group` takes precedence.
- `MOLI_INSPECTOR_ROUTING_SCENARIOS`: comma-separated scenario names within the `inspector-routing` group.
- `MOLI_SMOKE_TRACE=1`: print extra runner-side trace logs.
- `MOLI_SMOKE_TRACE_BG=1`: print background browser logs when invoking a
  worker directly. The supervisor enables this automatically because it
  persists each worker's combined output as a diagnostic artifact.
- `NODE`: Node executable used by optional Node client groups. Defaults to `node`.
- `PUPPETEER_CORE_MODULE`: module name or path used by the Puppeteer group. Defaults to `puppeteer-core`.
- `CHROME_REMOTE_INTERFACE_MODULE`: module name or package directory used by the optional CRI group. Defaults to `chrome-remote-interface`.
- `CHROME_REMOTE_INTERFACE_VERSION`: exact CRI version gate. Defaults to `0.34.0`.
- `CDP_USE_PYTHON`: Python executable containing cdp-use for the optional group. Defaults to the smoke runner's Python.
- `STAGEHAND_MODULE`: module name or package directory used by the optional Stagehand group. Defaults to `@browserbasehq/stagehand`.
- `STAGEHAND_VERSION`: exact Stagehand version gate. Defaults to `3.7.0`.
- `AGENT_BROWSER_BIN`: agent-browser CLI used by the optional external group. Defaults to `agent-browser` on `PATH`.

Moli runs do not need Playwright-managed browser binaries. Chromium runs require
the browser installed by the locked Playwright version, or an explicit override.
The runner starts `moli serve` with `--layout --resource --debugger` so screenshot
coverage uses the real renderer and the Chromium-derived Network matrix can
observe every optional resource family, including images, audio, video, and
text tracks. Debugger scenarios require real pauses, which `moli serve` skips
by default. When testing an external Moli `--endpoint`, start that server with
`--debugger` to run scenarios that assert debugger statements or breakpoints.
These options enable the capabilities exercised by this suite; normal Moli
defaults use Mock layout with optional resources and debugger pauses disabled.

## Relationship to the Node smoke

The repository still keeps `scripts/playwright-cdp-smoke.mjs`. The Python/uv project is the forward maintenance path. The Node script remains temporarily as a reference implementation until the Python suite has proven equivalent coverage.

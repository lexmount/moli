# moli-cdp-smoke

This is a standalone CDP smoke project for moli. The default suite uses Playwright and raw CDP because those dependencies are managed by `uv`; the Puppeteer group uses the pinned `puppeteer-core` development dependency in this directory. Optional agent-browser, chrome-remote-interface, cdp-use, and Stagehand groups exercise their real published clients when the corresponding local dependencies are installed. High-value protocol contracts should also be mirrored as focused Rust tests near their owning `moli-protocol` boundary so they run under `cargo nextest`.

The suite covers real CDP-client workflows: `connect_over_cdp`, concurrent browser and direct-page clients, multiple pages and contexts, navigation, history, `Page.setDocumentContent`, popup target creation, JavaScript dialog events, Network/Fetch routing, workers, WebSocket, file upload, downloads, locator/input flows, DOM/handle flows, Chromium-derived CDP protocol samples, Playwright-upstream-derived route/CDPSession samples, screenshots, `Page.printToPDF`, viewport, and storage/profile behavior. It is not a replacement for unit tests. Its job is to answer whether moli is usable as a Playwright/Puppeteer-style CDP endpoint.

## Chromium Behavior Evidence

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

## Current Coverage

The current suite is a strong core smoke gate, not a complete Playwright compatibility suite.

Covered well:

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
- The default raw `inspector-routing` group is the executable DevToolsSession boundary matrix. It covers per-session Main/IO FIFO and exactly-once completion, IO preemption of non-yielding JavaScript, DedicatedWorker and SharedWorker interrupt overtaking plus FIFO recovery, all 13 methods in Chromium 147's `ShouldSendOnIO`, normal debugger-pause pumping of one mixed V8/Page/DOM Main receiver, instrumentation-pause IO-only behavior, navigation replacement, auxiliary-session detach, BrowserContext teardown with interrupts in flight, and `Page.crash`. Every scenario runs in an isolated target and records its Chromium-derived contract.
- The focused raw `agent-episode` group copies the recorded RL
  `Runtime.evaluate(awaitPromise=true)` observe/fill/click path. It requires the
  action response before destructive cross-document realm events, observes only
  the replacement Document, and then proves that a transport reset commits a
  Runtime-usable error Document instead of returning `Promise was collected`
  or permanent `NoDocumentLoaded`. Same-document fill state is asserted through
  live DOM text; sampled `innerText` is recorded but may remain stale until the
  next layout refresh by design.
- The focused raw `dom-parser-mutations` group holds a parser-blocking head script after an early head-only `DOM.getDocument` and requires Chromium's exact root-agent sequence: commit `DOM.documentUpdated`, parser-tail BODY `DOM.childNodeInserted`, DCL `DOM.documentUpdated`, then `Page.domContentEventFired`. It also proves that the early frontend node id is stale after the DCL barrier and that a refreshed snapshot contains the complete BODY.
- Optional Puppeteer over CDP group for `puppeteer.connect()`, reconnecting an existing parked-then-promoted Page through a fresh browser session, browser-target and page-target `CDPSession`, `page.goto()`, `page.reload()`, selector-backed DOM activation navigation, same-document hash and History API navigation, `page.evaluate(fetch)`, keyboard input via `page.type()`, CSS / `$eval` / XPath element selection, and DOM interactions across text input, textarea, label/checkbox, radio, select, details/summary, disabled button, and form submission. It also covers `ElementHandle.boundingBox()` / `evaluate()` / `uploadFile()`, DedicatedWorker `workercreated` / `WebWorker.evaluate()` / exact-once worker-session console routing / explicit terminate / navigation-destroy lifecycle, current-viewport `page.screenshot({captureBeyondViewport:false})`, alert and console events, browser-session download behavior/events/artifact with peer-session event isolation, request interception `respond()` / `continue()`, page-scoped `CDPSession` Network event observation, and layout-backed `page.click()` dispatch.
- `browser.new_context()`, `context.new_page()`, multiple pages in one context, target switching, popup-scoped `page.route()` plus `evaluate(fetch)`, popup CDPSession response-stage body / stream / fulfill / fail flows, and held multi-context route / response-stage resume without cross-context Network event bleed.
- Top-level navigation, redirect final URL/response, reload-like click navigation, and history back/forward.
- Reload, same-document hash navigation, `history.pushState()` observation, Playwright `add_init_script()` page/context injection, `page.exposeFunction()` / `page.exposeBinding()` plus context-level exposed functions, and basic `domcontentloaded` / `load` / `networkidle` load-state waits with a parser-discovered delayed image.
- Basic popup target linkage through Playwright's `page.expect_popup()`, anchor `target="_blank"` popup activation, named popup target reuse, reserved-target `window.open(..., "_self")` current-page navigation, basic alert dialog handling through Playwright's `page.expect_event("dialog")` and `dialog.accept()`, and raw CDP prompt event/close shape.
- Basic iframe frame-tree consumption through `Frame.text_content()`.
- `page.wait_for_function()` string predicates, timer readiness, return
  JSHandle values, primitive/ElementHandle arguments, and timeout errors, plus
  Playwright `page.wait_for_selector()` / `locator.wait_for()` attached,
  visible, hidden, detached, and enabled-click auto-wait behavior.
- Chromium inspector-protocol derived samples for `Page.domContentEventFired` before `Page.loadEventFired`, `Page.frameStartedLoading` / `Page.frameStoppedLoading`, non-empty `Page.frameAttached.parentFrameId`, `Page.getFrameTree`, dynamic and nested child-frame event fan-out across auxiliary sessions with session-local lifecycle/disable state, `Page.navigate` fragment navigation, `Page.getAppManifest` default/loading/parsing/error/redirect/dynamic-link contracts, successful-result caching and link invalidation, plus its `Manifest` Network request and terminal lifecycle, `Page.getLayoutMetrics`, `Runtime.executionContextCreated`, `Runtime.evaluate(returnByValue)` and exception details, session-local `Input.setIgnoreInputEvents` aggregation/navigation/detach behavior, `Input.insertText` bypass, idle `Input.cancelDragging`, `Audits.issueAdded` Quirks/CSP shape, replay ordering, navigation storage reset and session-local enable/disable, `Log.entryAdded` network metadata, buffered replay ordering, session-local delivery cursors, target-shared `Log.clear`, violations-report state and validation, `IO.resolveBlob` session-local object resolution and reopenable `blob:<uuid>` streams, session-local `Performance.enable/disable`, strict time-domain transitions, disabled `Performance.getMetrics`, `Emulation.setCPUThrottlingRate`, `Profiler.start` / `Profiler.stop` CPU profiles and CPU-throttling profile workflow, error contracts, auxiliary CDP-session profiler isolation across navigation and detach/reattach, `console.profile` / `console.profileEnd`, precise coverage / best-effort coverage including not-started error, counter reset, and detailed block coverage, `DOM.getAttributes`, `DOM.querySelector(All)` including default-depth node-path publication through ordered `DOM.setChildNodes` events, deep ancestry expansion, repeat suppression, and the chromedp `NodeReady` contract, live `DOMDebugger.getEventListeners`, session-owned event-listener breakpoint pause/re-pause/navigation/detach behavior, session-owned XHR/fetch breakpoint URL matching, synchronous pause data, navigation/child-frame/worker scope, multi-owner sequencing and detach behavior, live-node DOM mutation breakpoints for single-node and multi-child `DocumentFragment` insertion batches, connected-node removal/insertion phases with one pre-pause `DOM.childNodeRemoved`, same-value attributes and node removal with owner/peer data, parser mutation no-pause behavior, unbound-node path ordering, disable/navigation cleanup, and the `DOM.getNodeForLocation` hit-test capability boundary.
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
- Auxiliary `CDPSession` observation for `Network.requestWillBeSent`, `Network.responseReceived`, `Network.loadingFinished`, and `Network.loadingFailed`, including fetch/XHR POST body, request/response-header fidelity, and redirect-chain event ordering.
- Chromium-derived `Network.requestWillBeSentExtraInfo` / `Network.responseReceivedExtraInfo` correlation for normal HTTP documents, no-cookie redirects, cache revalidation, ordinary page `Fetch.continueRequest`, page and worker Basic auth, and requests sent before the server resets the connection without response bytes, both on the initial request and after a completed redirect hop. Worker coverage includes Fetch and XHR, with XHR held through a response-stage pause; worker `CancelAuth` requires one initial unauthenticated request ExtraInfo and one raw `401` response ExtraInfo. Coverage includes transport-generated `Host` / `Accept-Encoding`, per-hop exchange counts and status sequence, path-mismatched cookies using the public `NotOnPath` blocked reason, auth retries retaining the initial unauthenticated request headers, `Fetch.authRequired` rounds correlated only by `Fetch.requestId` without a non-standard `networkId`, a raw `304` ExtraInfo response paired with the merged cached `200` response, `redirectHasExtraInfo`, and `hasExtraInfo` without assuming an order CDP does not guarantee. Fetch interception also verifies Chromium's shared `XHR` type for page Fetch, XHR, and EventSource requests under each of the `Fetch`, `XHR`, and `EventSource` filters, while Network events retain their distinct high-level types. Reset cases require the complete observed request/redirect ExtraInfo set, no final response event, and a successful `Page.navigate` result carrying `net::ERR_CONNECTION_RESET` in `errorText`; they do not assume ExtraInfo precedes `loadingFailed` or that Chromium will not later finish its internal error document. Response-stage interception also verifies that original transport ExtraInfo is visible before `Fetch.requestPaused`, while a `Fetch.continueResponse` status/header override changes the later `Network.responseReceived` without emitting duplicate ExtraInfo.
- The focused `error-document` group turns the reset-before-response fixture into an end-to-end failed-navigation gate. It checks the direct and redirected event order, the split between internal frame URL and user-visible Target/history URL, `unreachableUrl`, independent loader/realm generations, old-global retirement, consecutive failures, successful recovery, and concurrent target isolation. Every scenario requires Runtime to remain usable after the failure, so the pre-fix permanent `NoDocumentLoaded` state fails the smoke rather than being hidden by a later successful navigation.
- Chromium-derived proxy authentication coverage uses a real local proxy and per-browser-context proxy configuration. HTTP Basic authentication must expose the initial unauthenticated target request headers and final `200` response through ExtraInfo without leaking `Proxy-Authorization`; an HTTPS `CONNECT` challenge canceled through Fetch must expose proxy origin, a `407` response with `hasExtraInfo=false`, and `Network.loadingFailed` without treating CONNECT headers as target-request ExtraInfo.
- Parser-discovered external script, link stylesheet, and parser-created `@import` stylesheet observation plus `Network.getResponseBody`; configured `20,000,000 / 2,000,000` inspector-cache budgets retain a small body while a `2,000,001`-byte body keeps its request identity and repeatedly returns the inspector-cache eviction error.
- Classic WebSocket echo, WebSocket `Network.webSocket*` events, and blocked WebSocket handshake behavior.
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
- Viewport resize, the explicit Playwright-generated screenshot clip boundary, and Chromium-calibrated geolocation
  position/unavailable/clear behavior across navigation and auxiliary CDP sessions.
- localStorage, sessionStorage, basic IndexedDB, multi-context cookie isolation,
  Blob DevTools UUID storage-partition isolation, and browser-context profile
  overrides for user agent, locale, timezone, extra headers, and navigation
  referer.
- Chromium-calibrated `target-semantics` and `browser-semantics` contracts for
  target multi-attach/close/context disposal, real-URL target debugger-wait
  lifecycle correlation, history entry identity and metadata, frame metadata
  and detach order, top-level/popup storage,
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
- Navigation/lifecycle variants: popup target creation is covered through Target events, Puppeteer `waitForTarget()`, and Playwright `page.expect_popup()`; named popup targets reuse an existing CDP target instead of creating duplicates; reserved `_self` / `_top` / `_parent` popup targets navigate the current page in the current simplified top-level model. `window.open()` now returns a non-null WindowProxy projection. JavaScript dialog protocol shape covers no-dialog rejection plus prompt opening/closing fields. Remaining gaps are full WindowProxy/opener scripting behavior, background popup document load/activation after named-target reuse, confirm/prompt modal return semantics, true script-blocking dialog behavior, sharper `wait_until` edge cases, same-document history entry precision, and target lifecycle edge cases.
- Navigation response precision: Puppeteer selector-backed DOM activation completes and updates `page.url()`, while the smoke records whether `waitForNavigation()` exposes the committed document response. `page.goto()` and `page.reload()` response objects are required separately. Coordinate `page.click()` is supported when the server runs with `--layout`; Mock policy still fails explicitly.
- Same-document navigation precision: Puppeteer `page.waitForNavigation()` is now covered for hash-only and History API navigation through renderer-produced `Page.navigatedWithinDocument` events. Remaining precision work is deeper history traversal/back-forward entry parity and Navigation API interception-style same-document reasons.
- Screenshot/layout/emulation: raw full-page/clip/node screenshots and raster-backed `Page.printToPDF` are covered, including base64/stream transfer, pagination, page ranges, orientation, print media, and background control. Header/footer templates, CSS page-size preference, tagged PDFs, document outlines, vector/text-preserving output, richer device-scale behavior, and more accurate layout metrics remain gaps. The Chrome `chrome://inspect` command menu is a separate DevTools-frontend concern and does not advertise Moli's screenshot capability.
- Wider network matrix: broader digest/multi-round proxy auth challenge variants, font/preload resource types, parser-discovered resource interception, and broader response-stage coverage beyond the current page/popup fetch/XHR response-stage smoke.
- Storage/profile depth: cookie delete/clear/domain/path/SameSite, persistent profile writeback, storage partition boundaries, and deeper IndexedDB flows.
- Worker/frame target depth: module-worker target lifecycle, nested DedicatedWorker targets, broader worker exception/debugger coverage, iframe subresource attribution, and future per-frame realm boundaries remain. DedicatedWorker now has first-layer CDP target/runtime/console/lifecycle coverage; SharedWorker has first-layer target/runtime/log/profiler and page-side reuse smoke, but both still need richer error matrices.
- Puppeteer parity: the default managed-client group covers the common connect/navigation/reload/same-document/evaluate/keyboard-input/DOM-selector-activation/coordinate-click/handle/current-viewport-screenshot/alert-dialog/popup-target/interception path and DedicatedWorker lifecycle, but full-page/clip screenshots, drag interception, confirm/prompt modal return semantics, and richer lifecycle waits remain.

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
- `groups/dom_parser_mutations.py`: cross-engine raw-CDP parser-tail mutation publication and commit/DCL DOM binding barriers.
- `groups/layout_screenshot.py`: raw current-viewport PNG, DevTools parameter compatibility, paint/layout mutation freshness, open/closed Shadow Root and iframe TreeScope stability, generation-gated 1 FPS JPEG screencast/ACK behavior, and Moli default-Mock restart boundary.
- `groups/action_window.py`: raw wheel admission/deadline batching, screenshot
  flush/reset, derived-effect coalescing, and exact-Document retirement.
- `groups/pdf.py`: raw `Page.printToPDF` base64 and `ReturnAsStream` transport, `IO.read`, pagination, page ranges, orientation, PDF structure, and Chromium-shaped validation errors.
- `groups/dom_snapshot.py`: raw CDP `document.open()` replacement identity and
  `DOMSnapshot.captureSnapshot` freshness workflows, intended to run unchanged
  against a Chromium CDP endpoint before validating moli.
- `groups/playwright_compat.py`: Playwright upstream derived route and CDPSession compatibility samples.
- `groups/chrome_remote_interface.py`, `groups/cdp_use.py`, and `groups/stagehand.py`: optional published-client workflows, with subprocess and JSON-result handling shared by `groups/external_process.py`.
- `groups/puppeteer.py` and `puppeteer_smoke.mjs`: pinned Puppeteer workflows driven from the uv runner through Node.
- `groups/agent_browser.py`: optional real agent-browser CLI workflows with isolated config, daemon namespace, and endpoint identity verification.
- `groups/network.py`: document/fetch/XHR routing, Network event observation, parser script and stylesheet body capture, WebSocket, and downloads.
- `groups/workers.py`: dedicated worker `postMessage`, SharedWorker port reuse, worker fetch routing/auth, and worker XHR routing.
- `groups/dom_input.py`: `set_content`, file upload, file chooser, scripted picker, click navigation, locator/input workflows, Playwright upstream derived locator composition, `page.type()`, layout-backed mouse/touch/tap/raw-drag input, `fill()`, `check()` / `uncheck()` / `setChecked()`, and `selectOption()` workflows, DOM/handle workflows, plus the explicit high-level drag-interception boundary.
- `groups/iframe_input.py`: a Chromium-calibrated raw-CDP regression for
  transformed single and nested iframe hover, click, wheel, child-local event
  coordinates, exact used frame viewport propagation, and Moli's nested-frame
  UA scrollbar routing boundary.
- `groups/emulation_storage.py`: viewport and Playwright screenshot-clip boundary, storage/cookie isolation, IndexedDB baseline, and browser-context profile overrides.
- `groups/browser_semantics.py`: raw-target and page/runtime cross-engine
  contracts calibrated against Chromium before they are applied to Moli.

Near-term expansion order:

1. Deepen popup and dialog fidelity: popup Target linkage is observable by Playwright/Puppeteer, anchor `target="_blank"` now enters the same popup activation path, named target reuse no longer creates duplicate targets at Target-discovery level, reserved current-context targets no longer create bogus popup targets, and JavaScript dialog protocol errors/fields now match Chromium more closely, but `window.open()` still returns `null` instead of a WindowProxy, named-target reuse does not yet drive a full background popup document load, and `confirm()` / `prompt()` emit dialog events but do not block and resume the script with the accepted value.
2. Add more network/resource-type coverage after that, especially font/preload observation, parser-discovered resource interception, proxy auth, and richer auth variants. Stylesheet link / `@import` has Network smoke coverage without requiring a following parser script, the Chromium-derived matrix covers image/media/text-track/XHR body capture, page `fetch()` response-stage pause / continue has a focused smoke, page XHR response-stage covers `Fetch.getResponseBody`, page fetch response-stage covers `Fetch.takeResponseBodyAsStream` / `IO.read`, and server-auth covers successful credentials plus Chromium-compatible `CancelAuth` for page fetch, worker fetch/XHR, configured response-stage continuation, and top-level navigation.
3. Keep low-frequency CDP methods out of this smoke unless they block a real Playwright workflow. Use focused Rust tests for narrow protocol shape regressions and keep each fixture-derived CDP contract as a separate test.

## Running

Build moli from the repository root first:

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

With no `--group`, the runner executes every raw, Playwright page, browser, and
repository-managed external-client group, including `inspector-routing` and the
pinned Puppeteer client. CI invokes that same unfiltered default. External-client
groups whose binaries or dependency environments are not owned by this project
remain explicit integration runs.

List available scenario groups:

```bash
uv run moli-cdp-smoke --list-groups
```

Run a focused subset while developing a CDP area:

```bash
uv run moli-cdp-smoke --group protocol --group network
uv run moli-cdp-smoke --group multi-client
uv run moli-cdp-smoke --group layout-screenshot
uv run moli-cdp-smoke --group action-window
uv run moli-cdp-smoke --group pdf
uv run moli-cdp-smoke --group agent-episode
uv run moli-cdp-smoke --group fetch-runtime-teardown
uv run moli-cdp-smoke --group network-body-cache
uv run moli-cdp-smoke --group dom-input,emulation-storage
uv run moli-cdp-smoke --group document-content
uv run moli-cdp-smoke --group dom-snapshot
uv run moli-cdp-smoke --group dom-whitespace
uv run moli-cdp-smoke --group computed-style
uv run moli-cdp-smoke --group error-document
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

Run the same focused group against an already-running Chromium CDP endpoint:

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
- `MOLI_CDP_PORT`: CDP server port. Defaults to a free local port.
- `MOLI_SMOKE_GROUPS`: comma-separated smoke group list. CLI `--group` takes precedence.
- `MOLI_INSPECTOR_ROUTING_SCENARIOS`: comma-separated scenario names within the `inspector-routing` group.
- `MOLI_SMOKE_TRACE=1`: print extra runner-side trace logs.
- `MOLI_SMOKE_TRACE_BG=1`: print background `moli serve` logs.
- `NODE`: Node executable used by optional Node client groups. Defaults to `node`.
- `PUPPETEER_CORE_MODULE`: module name or path used by the Puppeteer group. Defaults to `puppeteer-core`.
- `CHROME_REMOTE_INTERFACE_MODULE`: module name or package directory used by the optional CRI group. Defaults to `chrome-remote-interface`.
- `CHROME_REMOTE_INTERFACE_VERSION`: exact CRI version gate. Defaults to `0.34.0`.
- `CDP_USE_PYTHON`: Python executable containing cdp-use for the optional group. Defaults to the smoke runner's Python.
- `STAGEHAND_MODULE`: module name or package directory used by the optional Stagehand group. Defaults to `@browserbasehq/stagehand`.
- `STAGEHAND_VERSION`: exact Stagehand version gate. Defaults to `3.7.0`.
- `AGENT_BROWSER_BIN`: agent-browser CLI used by the optional external group. Defaults to `agent-browser` on `PATH`.

This smoke connects to moli over CDP, so it does not need Playwright-managed browser binaries.
The runner starts `moli serve` with `--layout --resource` so screenshot
coverage uses the real renderer and the Chromium-derived Network matrix can
observe every optional resource family, including images, audio, video, and
text tracks. These are smoke-only opt-ins; normal moli defaults remain
Mock layout with optional resources disabled.

## Relationship to the Node smoke

The repository still keeps `scripts/playwright-cdp-smoke.mjs`. The Python/uv project is the forward maintenance path. The Node script remains temporarily as a reference implementation until the Python suite has proven equivalent coverage.

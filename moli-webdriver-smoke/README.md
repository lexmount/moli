# moli-webdriver-smoke

This is a standalone WebDriver smoke project for moli. It is a peer of
`moli-cdp-smoke/`: the runner starts a local fixture server, starts
`moli serve`, and exercises the public WebDriver wire surface from outside
the Rust test harness.

The default group uses raw WebDriver Classic HTTP. The optional BiDi group uses
raw WebSocket messages. The optional Selenium group uses Selenium Python's
`webdriver.Remote` client against the same `moli serve` endpoint, so this
project now covers both protocol-level wire shape and a real Selenium client
workflow.

The optional `semantics` group contains isolated cross-engine contracts. It can
run against Moli's native WebDriver endpoint or an external ChromeDriver;
Moli uses standard W3C capabilities without Chrome vendor options.

The optional `url-policy` group is a Moli hosted-product contract. It
uses isolated Classic and BiDi sessions to verify the exact local-file
navigation error envelopes, unchanged browsing contexts, and absence of BiDi
navigation lifecycle events. It is deliberately skipped for desktop Chromium,
whose browser-granted local-file capability is a different product policy.

The optional `navigation-errors` group is a cross-engine Chromium/WPT contract.
It runs the same 37-case public-wire matrix against Moli or ChromeDriver:
ten Classic malformed-body/URL cases, 22 BiDi invalid-argument cases, two
missing-context cases, and three real address failures. It also checks the W3C
HTTP and BiDi error envelopes, unchanged Classic URL, and post-failure session
liveness. The baseline was executed on 2026-08-09 with Debian Chromium
145.0.7632.116 and matching ChromeDriver 145.0.7632.117.

The script-timeout boundary has two deliberately separate optional groups:

- `script-timeout-chromium` is the ChromeDriver 147 oracle. A yielding async
  script times out near its deadline, while a bounded non-yielding loop first
  returns to the renderer and only then produces `script timeout`.
- `script-interrupt` is Moli's renderer-IO extension. It runs actual infinite
  loops through both Classic sync and async execution, requires timeout-driven
  `Runtime.terminateExecution` near the configured deadline, and repeats
  recovery work in the same window after every termination.

The split is intentional: the Moli group proves preemption, while the
ChromeDriver group records that ChromeDriver itself does not offer the same
non-yielding WebDriver timeout behavior.

## Coverage

- WebDriver Classic status/session/delete value envelopes.
- Classic navigation, current URL, title, page source, CSS/XPath element lookup and identity, element
  text/tag/displayed/enabled/rect/attribute/property/computed label/computed
  role, send keys, clear form controls with input/change events, file upload, click, execute script,
  key actions whose handler performs a top-level Page replacement while the action request is completing,
  explicit screenshot unsupported errors, cookies, alerts, unhandled prompt behavior, window-scoped prompt
  switching, shadow roots, page-side SharedWorker probe without polluting window
  handles, `document.open()` replacement stale-element behavior, and headless window state surface.
- WebDriver BiDi `session.status`, `session.new`, `browsingContext.create`,
  `session.subscribe`, `browsingContext.navigate`, DOMContentLoaded lifecycle
  events, `input.performActions`, `input.releaseActions`, key actions that trigger a top-level Page
  replacement and remain successful through the replacement lifecycle, element-origin input
  action routing without asserting layout hit-test effects, `input.setFiles`,
  `network.getData`, `browser.setDownloadBehavior`,
  `browsingContext.downloadWillBegin`/`downloadEnd`, `network.setCacheBehavior`,
  SharedWorker `browsingContext.contextCreated`, live
  `script.realmCreated(type = "shared-worker")`, and `script.getRealms(type =
  "shared-worker")`, user context creation/removal, user-context viewport inheritance,
  explicit `browsingContext.captureScreenshot` unsupported errors,
  `emulation.setUserAgentOverride`, `emulation.setLocaleOverride`,
  `emulation.setTimezoneOverride`, `emulation.setNetworkConditions`, storage
  isolation, `storage.setCookie`, `storage.getCookies`, and `session.end`.
- Selenium Python Remote WebDriver session creation, Selenium BiDi facade
  `webSocketUrl` attachment with `browsing_context` create/navigate/get_tree/
  locate_nodes, browsing context fragment/history event handlers, `script`
  evaluate/console/javascript-error handlers, add/remove preload scripts with
  context filtering, channel arguments, sandbox realms, and userContext
  filtering, `getRealms`, root-owned script handles, `callFunction`, `disown`,
  special number/BigInt serialization, nested handle/local-value arguments,
  `network.add_request_handler()` request interception,
  `network.add_auth_handler()` Basic Auth continuation,
  `browser` user context/client-window,
  and `emulation` user-context UA/locale/timezone/viewport/network-conditions workflows, navigation, title/current
  URL, page source, element text/tag/attribute/property/css/displayed/enabled/
  rect, link text locators, CSS/XPath user-facing locator workflows,
  accessible name, computed ARIA role, label-associated controls including link/img/select/textarea/input-button naming, send keys, clear, execute sync/async scripts with
  primitive/null/array/WebElement results, WebElement arguments, timeout/error recovery, and nested WebElement return values, Selenium client-side
  script pin/list/unpin workflows, Selenium text handling for child/block/inline
  content, whitespace, NBSP, hidden/script text exclusion, Selenium relative locators via `locate_with()`, fetch/XHR through async scripts, localStorage,
  sessionStorage, IndexedDB, Selenium remote downloads via `/se/files`, frame
  switching by index/name/id/WebElement with nested parent/default-content
  recovery and relative lookup errors, deleted-frame recovery, Selenium `Select` single/multi-select
  option lookup, select/deselect, disabled/hidden option errors, form submit
  via Selenium `WebElement.submit()` and submit-button click, checkbox/radio
  selection, text input/textarea typing and clear input/change events, all-printable key input,
  text-control `change` commit on blur,
  WebDriver private-key caret navigation/PageUp/PageDown, arrow-key legacy
  `KeyboardEvent.keyCode`, selection/delete/backspace, numpad, and function-key send keys,
  direct stale element reference errors after navigation/DOM removal/document replacement, implicit waits for single and multiple
  element lookup, WebDriverWait expected
  conditions for presence/visibility/text/clickability/invisibility/staleness/
  selection/value/attribute/frame/alert, W3C keyboard actions, and explicit
  unsupported-operation errors for pointer/wheel actions including
  hover/double-click/context-click/drag-drop, Selenium remote file upload via `/se/file` with FileReader content reads,
  action key input with Shift modifiers, explicit screenshot/element screenshot and print_page unsupported errors, cookie
  management including SameSite, expired cookies, delete-all, and exact-name deletion, window handles/new/switch/close/rect/
  maximize/minimize, anchor/script popup windows, named window target reuse,
  `window.open(..., "_self")`, Selenium `ShadowRoot` lookup/scoped find/
  execute_script equality/no-such-shadow-root errors, alert/confirm/prompt text,
  and alert accept/dismiss/send keys.
- Cross-engine Selenium semantics for capability matching, same-document realm
  and WebElement identity, cross-document stale elements, top-level and popup
  storage ownership, nested frame recovery, shadow-frame Window named access,
  dialog return values, and standard W3C error classes. Each scenario uses an
  independent session and emits a structured contract record.
- Hosted local-file navigation rejection as Classic HTTP `500` with W3C
  `unknown error`, and as a BiDi `unknown error`; both retain `about:blank`, and
  the BiDi path emits no immediate or delayed navigation lifecycle event.
- Chromium/WPT navigation failure shapes: Classic HTTP `400` `invalid argument`
  envelopes for null/missing/non-string/relative/malformed URLs; BiDi `invalid
  argument` for context, URL, and wait validation; `no such frame` for missing
  context ids; `unknown error` for unsupported schemes, DNS failure, and unsafe
  ports; and a usable session after the complete failure matrix.

Known remaining worker-surface gap: the BiDi SharedWorker smoke now requires
target context creation, live realm creation, and `script.getRealms`; richer
lifecycle, error, and teardown coverage is still pending.

## Usage

Build moli first when no binary exists:

```bash
cargo build -p moli
```

Then run the default smoke:

```bash
cd moli-webdriver-smoke
uv sync
uv run moli-webdriver-smoke
```

CI explicitly runs all six groups with `--continue-on-failure`:

```text
classic,bidi,selenium,semantics,url-policy,navigation-errors
```

The local default remains the smaller Classic-only loop for focused development.

List available groups:

```bash
uv run moli-webdriver-smoke --list-groups
```

Run focused groups:

```bash
uv run moli-webdriver-smoke --group classic
uv run moli-webdriver-smoke --group bidi
uv run moli-webdriver-smoke --group selenium
uv run moli-webdriver-smoke --group semantics --continue-on-failure
uv run moli-webdriver-smoke --group url-policy --continue-on-failure
uv run moli-webdriver-smoke --group navigation-errors --continue-on-failure
uv run moli-webdriver-smoke --group script-interrupt
MOLI_WEBDRIVER_SMOKE_GROUPS=classic,bidi,selenium uv run moli-webdriver-smoke
```

Collect all subgroup failures in one diagnostic run:

```bash
uv run moli-webdriver-smoke \
  --group classic,bidi,selenium,semantics,url-policy,navigation-errors \
  --continue-on-failure
```

Run the same `semantics` group against a matching local Chrome/ChromeDriver
build:

```bash
~/chromium/src/out/Default/chromedriver --port=9515

uv run moli-webdriver-smoke \
  --endpoint http://127.0.0.1:9515 \
  --browser-name chrome \
  --browser-binary ~/chromium/src/out/Default/chrome \
  --group semantics --continue-on-failure
```

Run the bounded ChromeDriver script-timeout oracle against the local Chromium
147 pair:

```bash
~/chromium/src/out/Default/chromedriver --port=9515

uv run moli-webdriver-smoke \
  --endpoint http://127.0.0.1:9515 \
  --browser-name chrome \
  --browser-binary ~/chromium/src/out/Default/chrome \
  --group script-timeout-chromium
```

Run Moli's non-yielding sync/async termination contract against `moli serve`:

```bash
uv run moli-webdriver-smoke --group script-interrupt
```

Run the Chromium/WPT navigation failure matrix against any matching local
Chrome/ChromeDriver pair (the driver and browser major versions must agree):

```bash
/path/to/chromedriver-145.0.7632.117 --port=9516

uv run moli-webdriver-smoke \
  --endpoint http://127.0.0.1:9516 \
  --browser-name chrome \
  --browser-binary /usr/bin/chromium \
  --group navigation-errors
```

Useful environment variables:

- `MOLI_BIN`: path to the `moli` binary under test.
- `MOLI_WEBDRIVER_PORT`: WebDriver server port. Defaults to a free local
  port.
- `MOLI_WEBDRIVER_SMOKE_GROUPS`: comma-separated smoke group list. CLI
  `--group` takes precedence.
- `MOLI_WEBDRIVER_SMOKE_CONTINUE_ON_FAILURE=1`: equivalent to
  `--continue-on-failure`.
- `MOLI_WEBDRIVER_SMOKE_TRACE_BG=1`: print background `moli serve`
  logs.

If both debug and release binaries exist, the runner uses the newest one by file
modification time. Set `MOLI_BIN` when you need an exact binary.

# Web fetch recipes

## Contents

- [Output selection](#output-selection)
- [Readiness](#readiness)
- [Dynamic Content and Frames](#dynamic-content-and-frames)
- [Screenshots and PDFs](#screenshots-and-pdfs)
- [Multi-page Retrieval](#multi-page-retrieval)
- [Request State and Policy](#request-state-and-policy)
- [Failure Diagnosis](#failure-diagnosis)

## Output selection

| Need | Command shape | Notes |
| --- | --- | --- |
| Read page content | `--dump markdown` | Default for research and summarization |
| Inspect semantic structure | `--dump semantic_tree_text` | Compact roles, labels, text, and backend node IDs |
| Process semantic structure | `--dump semantic_tree` | Structured accessibility-oriented payload |
| Process stable fields | `--dump json` | Returns `final_url`, `status`, and `html` |
| Inspect exact DOM | `--dump html` | Useful when Markdown loses important structure |
| Diagnose requests | `--dump json --trace-network` | Adds the `network` object |
| Capture the viewport | `--layout --dump screenshot` | Writes PNG bytes to stdout |
| Capture the full document | `--layout --dump screenshot_full` | Writes one full-page PNG to stdout |
| Capture a paginated document | `--layout --dump pdf` | Writes PDF bytes to stdout |

Raw non-HTML responses support only `html` and `json`.

## Readiness

Start with `--wait-until done`. Change or extend it only when the page exposes
a better completion signal:

- `--wait-until domcontentloaded` or `load`: stop at the corresponding browser
  lifecycle event when that event is the contract.
- `--wait-until networkidle`: wait for relevant network activity to become
  quiet on API-driven or lazy-loading pages. Avoid it when polling or streams
  keep the network busy indefinitely.
- `--wait-until domstable`: wait for relevant DOM mutations to settle on
  client-rendered pages. Avoid it when timers, counters, or animations mutate
  the DOM continuously.
- `--wait-selector '<css>'`: wait for a stable content element. Prefer this for
  client-rendered lists, articles, and results.
- `--wait-script '<expression>'`: wait for a JavaScript expression to become
  truthy.
- `--wait-script-file <path>`: use a reusable or multiline condition. It is
  mutually exclusive with `--wait-script`.
- `--wait-response-url <substring>`: wait for an application request whose URL
  contains literal text; use `--wait-response-url-regex <regex>` for a pattern.
- `--wait-response-body <substring>`: require literal text in that response;
  use `--wait-response-body-regex <regex>` for a pattern.
- `--wait-response-json <path=value>`: require an exact JSON field value; use
  `--wait-response-json-regex <path=regex>` for a patterned scalar value. All
  supplied response criteria must match one response. Literal and regex forms
  of the same criterion are mutually exclusive.
- `--delay-ms <ms>`: use only when the site has no observable readiness signal.
- `--timeout <ms>`: bound navigation and explicit waits; the default is 10000.

Examples:

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector "[data-testid='results']" \
  "https://example.com/search?q=moli"

moli fetch \
  --dump json \
  --trace-network \
  --wait-response-url-regex "/api/(search|results)$" \
  --wait-response-json-regex "data.requestId=^req-[0-9]+$" \
  "https://example.com/search"
```

## Dynamic Content and Frames

JavaScript runs by default. `--noscript` strips JavaScript from serialized
output; it does not mean that navigation ran without JavaScript.

If expected text is absent:

1. Confirm the HTTP status and final URL with `--dump json`.
2. Wait for the specific content selector or application response.
3. Try `--dump semantic_tree_text` to separate content from noisy markup.
4. Add `--with-frames` if the content is in an iframe.
5. Enable only the optional resource family that affects the page's behavior.
   Most text retrieval does not need images, fonts, audio, video, or layout.

Use `--with-base` when serialized HTML needs base metadata. Use
`--disable-subframes` when frames must not load. Apply `--strip-mode js`, `ui`,
`css`, or `full` only when the requested output should omit those parts; do not
silently remove behavior or content.

## Screenshots and PDFs

Enable real on-demand layout for binary output and redirect stdout:

```bash
moli fetch \
  --layout \
  --dump screenshot \
  "https://example.com" > viewport.png

moli fetch \
  --layout \
  --dump screenshot_full \
  "https://example.com" > full-page.png

moli fetch \
  --layout \
  --dump pdf \
  "https://example.com" > page.pdf
```

Use `screenshot` for the live viewport and `screenshot_full` for one complete
document raster. A full-document capture does not tile or automatically
downscale: its CSS width and height must each be less than 131,072 pixels, and
device-pixel, encoder, memory, or backend limits can fail earlier.

Use `--image --font` when page appearance depends on external images or fonts.
Use `--resource` only when every optional resource family is needed. Keep stderr
separate from the output file, then validate the file signature and size.

## Multi-page Retrieval

For each queue entry, retain the requested URL, final URL, status, crawl depth,
and parent URL. Canonicalize HTTP(S) links against the final URL, remove
fragments, and maintain a visited set.

Use Markdown for pages selected for reading. Use JSON when code needs redirect
information or exact HTML link extraction. Do not blindly follow every link:
rank links by the user's question and stop once additional pages no longer add
evidence.

For a crawl rather than a single lookup:

- stay within the agreed host and path scope;
- fetch sequentially unless explicit concurrency is justified;
- avoid calendars, faceted-search explosions, session URLs, logout actions,
  and repeated query permutations;
- keep a page and depth limit visible in the work log.

## Request State and Policy

- Add initial navigation headers with repeated `-H 'Name: Value'`.
- Import cookie files with repeated `--cookie-file`.
- Use `--profile-dir` when state must persist across invocations; it also
  provides the default HTTP cache location unless `--http-cache-dir` is set.
- Use `--http-proxy`, `--http-no-proxy`, or
  `--http-host-resolve HOST:PORT:ADDR` when required by the environment.
- Use either `--user-agent` or `--user-agent-suffix`, not both.
- Use `--web-bot-auth-key-file <PKCS8-PEM>` together with
  `--web-bot-auth-domain <DOMAIN>` only when the user supplied an authorized
  bot identity. The key remains local, and Moli signs HTTPS requests only.
- Add `--web-bot-auth-keyid <THUMBPRINT>` to assert the derived JWK thumbprint.
  Keep the default Cloudflare profile unless the receiver explicitly supports
  `--web-bot-auth-profile ietf-01`.
- Use `--document-start-script` or `--document-start-script-file` only when the
  task explicitly requires pre-navigation instrumentation.
- Combine `--block-private-networks` with `--block-cidrs` for untrusted URL
  workloads that need explicit network boundaries.

## Failure Diagnosis

- **Empty or shell-only output:** add a content selector wait, inspect semantic
  output, then check frames and application responses.
- **Timeout:** replace a broad wait with the narrowest observable signal before
  increasing the timeout.
- **401/403/login page:** report the access boundary or use only authorized
  cookies/profile state supplied for the task.
- **Unexpected redirect:** inspect `final_url` and `status` with JSON output.
- **Missing API data:** use response waits; add `--trace-network` to JSON only
  when request diagnostics are needed. Add `--trace-matched-response-body` only
  when the matched response body itself is required.
- **Oversized full-page PNG:** use a viewport capture or deliberately tiled CDP
  clips when one `screenshot_full` raster exceeds a backend or memory boundary.
- **TLS failure:** fix trust or hostname configuration. Use
  `--insecure-disable-tls-host-verification` only after the user explicitly
  accepts that risk.
- **Private-network target:** keep it blocked for untrusted input. Relax that
  boundary only for a clearly authorized private-site task.

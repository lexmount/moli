# CDP server guide

`moli serve` exposes a CDP endpoint for remote automation clients.

## Server

```bash
moli serve
moli serve --layout
moli serve --layout --resource
moli serve --host 127.0.0.1 --port 9333 --layout
```

Defaults:

- Host: `127.0.0.1`
- Port: `9222`
- Server timeout: 10 seconds

## Endpoint map

| Surface | Endpoint |
| --- | --- |
| CDP discovery | `http://127.0.0.1:9222/json/version` |
| CDP targets | `http://127.0.0.1:9222/json/list` |
| CDP protocol | `http://127.0.0.1:9222/json/protocol` |

CDP discovery returns the browser WebSocket URL; prefer discovery over
hard-coding a `/devtools/...` path.

## Client selection

- Use Playwright's `connectOverCDP` / `connect_over_cdp` for existing
  Playwright code.
- Use Puppeteer's `connect` with Moli's browser WebSocket URL for existing
  Puppeteer code.
- Use raw CDP only when the client library cannot express the required command
  or event.

## Runtime options

- Add `--layout` for real geometry, coordinate input, screenshots, PDFs, and
  screencasts.
- Add individual resource flags or `--resource` when visual/media assets must
  load.
- Add `--accept-lang` (or `MOLI_ACCEPT_LANGUAGE`) to pin the browser language;
  see "Language" below.
- Add `--profile-dir` for persistent storage and cookies.
- Add `--cookie` to import cookies.
- Configure proxy and connection controls on `moli serve`.
- Add `--block-private-networks` or `--block-cidrs` for untrusted navigation.
- Keep loopback binding unless remote clients genuinely require exposure.

## Language

One identity field feeds `navigator.language`, `navigator.languages`, and every
outbound `Accept-Language` header. A page that localizes itself from
`navigator.language` therefore changes UI copy, text width, row height, and
auto-sized button geometry when the language changes.

**Default rule: when the instructions do not name a language, Moli sends the
operating system's display language** — the same value a real browser reports.
The correct action is usually to pass nothing.

- macOS: the display language from `AppleLanguages` (for example
  `zh-Hans-CN`, reported as `zh-CN`), then the `AppleLocale` region.
- Windows: the preferred UI language list from `GetUserPreferredUILanguages`.
- Other platforms, and as a last resort: `LANGUAGE` (a `:`-separated
  preference list), then `LC_ALL`, `LC_MESSAGES`, then `LANG`.
- `C` and `POSIX` mean "no preference", so `en-US,en;q=0.9` stays in effect.
  This matters on macOS: a Terminal session normally exports `LANG=C.UTF-8`,
  and the POSIX environment is not a substitute for the display-language
  setting.

Locale subtags are normalized to BCP 47 case, because `navigator.language`
exposes the value verbatim: `EN_us` reaches the page as `en-US`, `ZH-HANS-CN`
as `zh-CN`. HTTP treats the header case-insensitively, but a site that selects
a per-language bundle by string comparison misses the wrong-cased tag. A script
subtag is dropped only when the region already implies it (`zh-Hans-CN` becomes
`zh-CN`); otherwise it is kept so the wrong bundle is never selected
(`zh-Hans-HK` stays `zh-Hans-HK`, `sr-Latn-RS` stays `sr-Latn-RS`). A POSIX
modifier supplies a missing script (`sr_RS@latin` becomes `sr-Latn-RS`).

Confirm the value the browser will report before blaming the page. Omit `--dump`
so the evaluated expression is printed instead of being discarded:

```bash
moli fetch "data:text/html,<p>x" --eval "navigator.languages.join(',')"
# zh-CN,zh
```

`--dump` (including `--dump json`) exports the page state and discards the
expression value, so a verification that uses it prints no language at all.

Pass `--accept-lang` only when the instructions name a language, or when you
must reproduce a baseline that pins one: a zh-CN baseline compared against an
en-US default produces "wrong width" and "missing label" reports that are not
layout defects. Override per session with CDP `Emulation.setUserAgentOverride`
and `acceptLanguage` (pass the current `navigator.userAgent` so the rest of the
identity is unchanged), then reload, because `navigator.language` updates with
the document rather than retroactively. A per-session override wins over
`--accept-lang` for that target only.

## Full-document screenshots

With `--layout`, request an automatic full-document PNG by omitting `clip` and
setting `captureBeyondViewport`:

```json
{
  "method": "Page.captureScreenshot",
  "params": { "format": "png", "captureBeyondViewport": true }
}
```

- Automatic full-document capture requires CSS width and height to each be less
  than 131,072 pixels; equality is rejected.
- An explicit page-coordinate `clip` bypasses that whole-document entry check,
  but must have positive dimensions.
- Moli renders one complete bitmap. It does not stitch tiles or automatically
  downscale, so device-pixel, encoder, backend, and memory limits can fail
  earlier than the CSS boundary.
- Tile explicit clips in the client when one bitmap cannot fit. Keep each clip
  within the active raster and encoder limits.

## Troubleshooting

1. Confirm the Moli process is still running.
2. Probe `/json/version` using the exact host and port.
3. Ensure the client attaches or connects remotely instead of launching a
   bundled browser.
4. Remove Chrome-only launch flags and `executablePath` settings.
5. Enable `--layout` when a failure involves real geometry or visual output.
6. Enable only the resource families the page requires.
7. Check the installed Moli version's `serve --help`.
8. Treat an explicit unsupported protocol error as a capability boundary; do
   not mask it with a synthetic success.

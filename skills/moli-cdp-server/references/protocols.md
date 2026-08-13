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
- CDP active connections: 16
- CDP pending connections: 128

Tune the last two with `--cdp-max-connections` and
`--cdp-max-pending-connections`.

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
- Add `--profile-dir` for persistent storage and cookies.
- Add `--cookie-file` to import cookies.
- Configure proxy and connection controls on `moli serve`.
- Add `--block-private-networks` or `--block-cidrs` for untrusted navigation.
- Keep loopback binding unless remote clients genuinely require exposure.

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

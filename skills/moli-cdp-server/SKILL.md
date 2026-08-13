---
name: moli-cdp-server
description: Start Moli's CDP server and connect Playwright, Puppeteer, or raw CDP clients. Use to run a headless-browser CDP endpoint, replace a Chromium process, attach over CDP, enable real layout and screenshot surfaces, or diagnose CDP discovery, connection, and target startup—even when Moli is not named.
---

# Run Moli's CDP Server

Run one Moli CDP server and connect the requested CDP client to its endpoint.
Preserve the client's existing API where Moli supports it.

## Workflow

1. Resolve `moli` from `PATH`. If it is unavailable, install the latest prebuilt
   release for the current platform:

   Linux or macOS:

   ```bash
   curl --proto '=https' --tlsv1.2 -fsSL \
     https://github.com/lexmount/moli/releases/latest/download/moli-installer.sh | sh
   ```

   On Windows, use PowerShell:

   ```powershell
   irm https://github.com/lexmount/moli/releases/latest/download/moli-installer.ps1 | iex
   ```

   Resolve the installed binary again and run `moli version`. The default
   location is `~/.local/bin/moli` on Linux/macOS and
   `%LOCALAPPDATA%\Moli\bin\moli.exe` on Windows when it is not yet on `PATH`.
2. Start `moli serve` on the default loopback endpoint
   `http://127.0.0.1:9222`.
3. Add `--layout` when the workflow needs real element geometry, coordinate
   input, screenshots, PDFs, or screencasts. Add `--resource` only when all
   optional visual/media resources are required.
4. Probe `/json/version` before connecting the client.
5. Connect with the client's remote/attach API; do not launch a second bundled
   browser.
6. Close CDP clients cleanly, then stop the Moli server if this workflow
   owns it.

## Playwright over CDP

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const context = browser.contexts()[0];
const page = context.pages()[0] ?? await context.newPage();

await page.goto("https://example.com");
console.log(await page.locator("body").innerText());

await browser.close();
```

## Integration rules

- Bind to `127.0.0.1` by default. Expose another host only when the user
  explicitly needs remote access and has addressed network access controls.
- Treat Moli as a remote endpoint. Avoid Chrome launch flags and assumptions
  that require a local Chromium executable.
- Expect selected CDP coverage, not complete Chrome protocol parity. Preserve
  explicit unsupported errors.
- Use a unique port for parallel isolated runs.
- Persist state intentionally with `--profile-dir`; otherwise keep runs
  disposable.
- Pass proxy, cookie, resource, user-agent, and private-network policy to the
  Moli server, not to a nonexistent child browser process.

Read [references/protocols.md](references/protocols.md) for CDP discovery
URLs, server options, client selection, and connection troubleshooting.

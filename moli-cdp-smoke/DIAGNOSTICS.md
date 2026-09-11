# Optional live-site diagnostics

These collectors are **not** CI smoke tests or assertions that a commercial
detector must accept automation. They keep failures and missing verdicts as
evidence instead of retrying until a site returns a desired result.

Run from `moli-cdp-smoke` with its Python dependencies installed (`uv sync`).
Use a fresh output directory for every invocation. Each collector starts its
own browser, uses a disposable context, enables CDP Runtime/Page/Network,
leaves identity and site scripts untouched, and terminates its browser.
Browser traffic is direct, with TLS verification left enabled. On Linux use
`xvfb-run -a` for **headed** Chromium, not `--headless`.

## Fingerprint Pro: a real Windows baseline

Run this on Windows, with a native Chrome installation:

```powershell
uv run python -m moli_cdp_smoke.diagnostics.fingerprint_identity `
  --engine chromium `
  --binary 'C:\Program Files\Google\Chrome\Application\chrome.exe' `
  --require-windows --output windows-fp-baseline
```

`--require-windows` refuses Linux/macOS hosts. Changing a UA on Linux is not
a Windows baseline. The collector records host OS, binary SHA-256, browser
version and native Navigator/UA-CH values; it does not set UA overrides.
Record the Windows version and installed font environment when comparing
results. Browser-version and machine differences are potential confounders,
not evidence that a single identity field caused a commercial verdict.

For Moli, keep the same command but use `--engine moli`, its local binary,
and omit `--require-windows` when running on Linux. Moli retains its own
default identity; the collector never replaces it with the host identity.

`result.json` selects the final event API's bot/tampering/score fields and
an explicit allowlist of device attributes. No API reply within the fixed
12-second post-navigation window means **no valid verdict**. Identity reads
happen after that window. Responses are not intercepted or rewritten.

## Evidence handling

Keep `environment.json` and `result.json` with the investigation. Do not
commit unreviewed raw artifacts: `server.log` can contain URLs and third-party
script diagnostics. Even selected fingerprints describe a device and should
not be published indiscriminately. The collectors omit visitor IDs, cookies,
IP addresses and opaque SDK tokens from their structured reports.

The native Windows guard, result URL selection and structured-report
allowlists have local unit tests. Actual Windows launch and site results must
still be verified on Windows; a passing Linux unit test is not that evidence.

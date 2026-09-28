# Native CDP fingerprint observations

This observational suite visits twelve public fingerprint/bot-test sites. It is
not a performance benchmark or a bot-evasion guarantee. `complete` means the
collector obtained its required fields, **not** that the site judged the browser
normal. Missing, partial, timed-out, network-failed and crashed samples stay in
the report; repetitions are declared before running, never retried to green.

Install the optional client and run from the repository root:

```sh
uv sync --project moli-benchmark --locked --extra fingerprint
xvfb-run -a uv run --project moli-benchmark --locked --extra fingerprint \
  python -m moli_benchmark.fingerprint \
  --moli-bin target/release/moli --chrome-bin /usr/lib/chromium/chromium \
  --output-dir target/benchmark/fingerprint-example
```

Use the native Chromium executable and a real display (or Xvfb), not headless
Chromium. This suite owns fresh loopback CDP servers and browser profiles. It
does not override identity, disable TLS verification or launch Chromium with
`--disable-gpu`. Proxy settings are disabled consistently. Moli uses `serve
--layout --resource`. Both engines enable Runtime, Page and Network via CDP.
Cases run serially with alternating engine order; no resource sampler runs
beside the browser. Default Moli identity remains unchanged.

`--site ID` and `--engine moli|chromium` can repeat to select a subset. `--runs N`
declares 1..10 repetitions; the default is one. Existing output directories are
rejected. Results contain binary, client and collector provenance plus selected
DOM/API values. API bodies, tokens, visitor IDs, IP addresses, console strings
and page HTML are not stored. Script auditing retains only host, hash and size.

## Sampling contract

Each case waits its declared number of seconds after DOMContentLoaded. The
interaction demo types fixed dummy credentials and submits its public demo form;
Pixelscan clicks the public scan link. Both have an explicit post-action window.
These are scripted actions, not human-behavior simulations. The manifest records
all waits and limits; results from different workloads are not interchangeable.

Collectors read scoped DOM `textContent` or selected existing result objects.
They never take a screenshot or force a rendering checkpoint to make a result
appear. Network evidence is frozen before the DOM snapshot: only result bodies
completed by that cutoff qualify. Subsequent audit work cannot fill a missing
sample with a late response. Snapshot start/end and response timing are recorded.

Incolumitas Proxy currently redirects to a host with a certificate mismatch in
the tested environment. No complete result schema has been verified there. Its
collector reports labels/errors as **partial**, never as a successful proxy
check. BrowserLeaks pages are capability reports, not binary bot verdicts.
Pixelscan's summary can complete while individual sections are still collecting;
their separate states remain visible. CreepJS ratings can be incomplete even
when lie/error totals are available. Fingerprint Pro requires a finished,
unambiguous API result; its empty HTML shell does not qualify.

Local real-DOM fixtures, independent of the public services:

```sh
uv run --project moli-benchmark --locked --extra fingerprint python \
  moli-benchmark/tests/check_fingerprint_dom.py --engine moli \
  --binary target/release/moli --output target/benchmark/fingerprint-dom-moli
```

These fixtures originated in the survey's CDP-smoke diagnostics. They test
collector correctness, not new CDP product functionality.

## Reports and history

Every run writes `manifest.json`, `summary.json`, `index.html` and individual
sample results. The HTML is self-contained and shows collection state separately
from the site's selected verdicts. Summaries are saved after each sample, so an
interrupted run retains its completed observations; incomplete matrices are
explicit, not backfilled from another run.

Pass `--baseline-report path/to/previous-run` (or its `summary.json`) to also
write `comparison.json`. It compares each engine/site/repetition and selected
metric, preserving false, zero, null, absent and unsampled as different states.
Numeric changes get deltas; bot verdicts and commercial scores are **not** turned
into a blanket pass-rate or causal regression claim. Different workloads,
environments, matrices or unverified inputs are flagged as incomparable. Browser
and observed script-set changes remain visible. Public services can change
server-side scoring and IP reputation without any browser-code change.

<p align="center">
  <img
    src="assets/moli-browser-banner.jpg"
    alt="Moli Browser — Structure first. Pixels on demand. Open source browser for AI agents."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <strong>English</strong> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.de.md">Deutsch</a> |
  <a href="README.fr.md">Français</a> |
  <a href="README.es.md">Español</a>
</p>

Moli is a production-ready headless browser for AI agents. Its on-demand layout
and rendering design combines a complete browser runtime with a lightweight
resource footprint.

By default, it executes real JavaScript, maintains a real DOM, and exposes real
browser APIs. It computes layout or renders pixels only when they are actually
needed.

Use it through the CLI, CDP, WebDriver Classic, or WebDriver BiDi.

## Quick start

Give this prompt to your AI coding agent:

```text
Install the skills under https://github.com/lexmount/moli/tree/main/skills, follow their instructions to download and install the latest prebuilt Moli binary, then use moli-webfetch to fetch https://example.com and show me the result.
```

## Showcase

<p align="center">
  <a href="assets/moli-game.jpg">
    <img
      src="assets/moli-game.jpg"
      alt="An HTML5 game rendered by Moli and inspected through Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>An HTML5 game rendered by Moli and inspected live through Chrome DevTools.</sub>
</p>

<p align="center">
  <a href="assets/moli-devtools-rust-lang.jpg">
    <img
      src="assets/moli-devtools-rust-lang.jpg"
      alt="rust-lang.org rendered by Moli and inspected through Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>rust-lang.org rendered by Moli, with its live DOM, CSS, and geometry available in Chrome DevTools.</sub>
</p>

## CLI usage

### Extract a page

Render the page as Markdown with Moli's default completion strategy:

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

Or directly return a compact, model-friendly semantic tree:

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

For visual output, enable on-demand layout and write either a viewport PNG screenshot or a paginated PDF:

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

Run `fetch --help` for the complete option list, including output formats,
page-load/response waits, profiles, proxy settings, resource policies, and
tracing options.

### Start the automation server

```bash
# Basic automation server for DOM-first workloads
moli serve

# Enable real geometry, coordinate input, and screenshot/screencast surfaces
moli serve --layout

# Also fetch optional image, font, audio, video, media, and text-track resources
moli serve --layout --resource
```

The same endpoint serves all three protocols: CDP, WebDriver Classic, and
WebDriver BiDi. Playwright can connect directly over CDP:

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const context = browser.contexts()[0];
const page = context.pages()[0] ?? await context.newPage();

await page.goto("https://example.com");
console.log(await page.locator("body").innerText());

await browser.close();
```

## Why Moli

Three qualities matter most for agent workloads, and Moli brings them together:

- **Full-featured** — real JavaScript, DOM, CSS, networking, storage, layout,
  screenshots, and standard automation protocols, all integrated into one
  headless browser.
- **Fast** — most automation requests never need visual rendering, so
  structure-first operations skip layout and paint entirely.
- **Resource-efficient** — layout and pixels are generated only when needed,
  so Moli does not have to continuously maintain and update a fully rendered
  visual state.

What most browser automation tasks actually need is page structure, not a
continuously rendered visual world. Moli treats the native DOM and style state
as the single source of truth, triggering layout or software paint only for
operations that genuinely require them.

| Agent request | What Moli does |
| --- | --- |
| Extract HTML/Markdown, query the DOM, run JS, inspect network/storage | Reads browser runtime state directly — does not trigger layout or paint |
| Read an element's box, hit-test coordinates, send coordinate input | Runs one layout calculation and keeps only the latest geometry snapshot |
| Capture a screenshot or refresh a screencast | Rebuilds from the current DOM/style, renders a fresh frame, and discards it after use |

<p align="center">
  <a href="assets/moli_ondemand_rendering_flow.svg">
    <img
      src="assets/moli_ondemand_rendering_flow.svg"
      alt="How Moli handles a request: DOM-first by default, with layout and paint built fresh only on demand"
      width="680"
    />
  </a>
</p>

Moli still includes the complete set of capabilities: V8, CSS, layout, text
shaping, hit-testing, software paint, and more. The only difference is *when*
visual work runs and *how long* its results are retained. This cost model is
especially well suited to crawling, browser-use agents, retrieval pipelines,
evaluation environments, and reinforcement-learning workloads.

## Current capabilities

- **Complete web runtime** — streaming HTML parsing, native DOM, V8 JavaScript,
  modules/timers/microtasks/events, iframes and workers, CSS cascade,
  Fetch/XHR/WebSocket, cookies, WebCrypto, and profile-scoped storage
  (localStorage, IndexedDB, OPFS).
- **Extraction-optimized outputs** — the CLI directly produces HTML, Markdown,
  JSON, semantic text trees, and frame-aware serialization, with
  selector/script/response waits and network tracing.
- **Unified automation binary** — CDP, WebDriver Classic, and WebDriver BiDi
  share the same kernel and scheduler. No separate ChromeDriver, geckodriver,
  or browser installation is required.
- **Real visual capabilities on demand** — add `--layout` to enable complete box
  construction, Taffy layout, Parley text layout, layout-backed
  hit-testing/input, viewport
  screenshots, and low-frequency CPU-rendered DevTools screencast frames.
- **Controllable operational options** — profiles, cookies, HTTP cache, proxies,
  resource families, connection limits, timeouts, private-network policy,
  user-agent overrides, structured logging, and network diagnostics are all
  available.

## Moli's relationship with Lexmount

Moli is Lexmount's open-source headless browser; Lexmount Browser is the managed
cloud runtime and control plane built around it.

**The open-source headless browser is fully usable without Lexmount Browser.**

## Cost controls

Expensive browser operations in Moli require an explicit opt-in and are never
enabled by default:

| Mode or option | Behavior |
| --- | --- |
| Default | `LayoutPolicy::Mock` — deterministic geometry in a compatible format, with no real layout or paint |
| `--layout` | `LayoutPolicy::OnDemand` — real layout, geometry, hit-testing, coordinate input, screenshots, screencast |
| `--resource` | Fetch all optional visual/media resource families |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Enable one specific optional resource family |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Selectively enable the persistence required by the workload |

Layout is an on-demand snapshot rather than continuously maintained state. The
first geometry request (a cold start) builds one complete layout from the
current DOM/style and retains only the latest `LayoutPassOutput`. After that,
ordinary geometry reads may reuse the snapshot even if the page has changed;
screenshots and screencasts always rebuild and never reuse stale results.

## Architecture

Moli is a standalone browser kernel, not a Chromium wrapper. It is built in
Rust, has its own ownership and lifecycle rules, and relies on:

- `libcurl` — network transport and multi-request runtime
- `html5ever` — HTML parsing
- `rusty_v8` / V8 — JavaScript execution
- Servo/Stylo — selectors, cascade, computed style
- Taffy + Parley — box and text layout
- AnyRender/Vello CPU, `usvg`, and the Rust image ecosystem — software rendering

Document and style have a single source of truth: the native DOM and its Stylo
integration. Every real refresh rebuilds layout from that source, converts the
result into immutable, DOM-independent data, and then discards the temporary
state created during that layout and paint pass. The system has no incremental
layout tree, damage graph, retained display list, GPU compositor, or persistent
window.

## Test data

The following two measured data sets show Moli's current capability envelope.
The tests cover real websites, real automation clients, focused Chromium/WPT
behavior checks, and a large nextest regression suite.

### Mixed public-web crawl test

The test covers 192 public URLs from major Chinese and international sites. A
page only counts as successful if it produces meaningful content after
JavaScript runs — an HTTP 200, challenge page, login wall, empty response, or
shell-only application does not count.

| Browser | Useful pages | Success rate | Median time | Median RSS |
| --- | ---: | ---: | ---: | ---: |
| **Moli** | **103** | **53.6%** | **1.43 s** | **73 MiB** |
| Chrome Headless | 101 | 52.6% | 1.43 s | 773 MiB |
| Lightpanda | 85 | 44.3% | 0.97 s | 40 MiB |
| Obscura | 57 | 29.7% | 1.30 s | 39 MiB |

### Sample agent workload

| Metric | Moli | Chromium |
| --- | ---: | ---: |
| CDP ready | 34.85 ms | 169.37 ms |
| Episode active p50 | 33.40 ms | 57.13 ms |
| Peak PSS | 102.46 MiB | 348.82 MiB |
| Peak processes / threads | 1 / 24 | 11 / 123 |

In the current WPT selection used to validate Moli's agent-browser scope, one
complete run recorded **1.612 million passing tests**.

## Project scope

Within the agent-browser scenarios defined in its documentation, Moli is ready
for production use and remains under active development.

Its current intentional boundaries include:

- No GUI browser, persistent window, GPU compositor, or retained multi-frame
  paint architecture.
- It does not pursue pixel-for-pixel parity with Chrome or provide
  high-fidelity Canvas/WebGL/media playback.
- It covers selected CDP, WebDriver Classic, and WebDriver BiDi functionality
  rather than implementing full protocol parity.
- `--layout` supports software screenshots and raster-backed CDP PDF
  generation, but not every Chrome screenshot or print mode is implemented.
- Resource loading, geometry freshness, and visual rendering cost remain
  explicit policy choices instead of being continuously enabled by default.

Unsupported protocol paths return explicit errors — Moli never pretends that a
browser action, event, network observation, or visual result occurred.

Maintainers can publish a tagged binary release from GitHub Actions by following
the [release guide](RELEASING.md).

## License

Unless a file or directory states otherwise, you may use Moli under either the
[Apache License 2.0](LICENSE-APACHE) or the [MIT License](LICENSE-MIT), at your
option. Separately licensed third-party components and fixtures remain subject
to their own licenses and notices.

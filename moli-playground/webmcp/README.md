# WebMCP demos with Moli

These demos open public WebMCP sites in Moli, discover their native tools over
CDP, invoke them, and check both the tool responses and the page state. No model
account or API key is required: the scripts supply the tool arguments directly.

## Run

From the repository root, build a release Moli binary with native WebMCP support
and run all three demos:

```bash
cargo build --release -p moli
uv run moli-playground/webmcp/webmcp_cdp_demo.py
```

The script starts `moli serve` with a temporary profile and an available local
port, then closes it after the run. It uses `target/release/moli` by default and
rejects binary paths containing a `debug` directory. `uv` installs the script's
Python dependency automatically.

Run a single scenario:

```bash
uv run moli-playground/webmcp/webmcp_cdp_demo.py --demo booking
uv run moli-playground/webmcp/webmcp_cdp_demo.py --demo bistro
uv run moli-playground/webmcp/webmcp_cdp_demo.py --demo pizza
```

| Demo | Tool sequence | What it checks |
| --- | --- | --- |
| `booking` | `getAvailability` → `bookSlot` → `cancelBooking` | Choose an available slot, read its confirmation ID from the structured result, verify the visible confirmation, then cancel and check that the widget resets. |
| `bistro` | `book_table_le_petit_bistro` → regular submit button | Native declarative form filling, `:tool-form-active`, submitter focus, pending result before confirmation, and the response supplied through `SubmitEvent.respondWith()`. |
| `pizza` | `manage_pizza` → `set_pizza_size` → `set_pizza_style` → `add_topping` × 2 | Sequential imperative calls produce a large BBQ pizza with three pineapple and five bacon toppings in the page. |

The bistro uses the site's default form, which waits for confirmation. The demo
first checks the prepared form, then clicks its normal submit button to simulate
the visitor confirming. Dates are computed at runtime; the booking scenario uses
the availability returned by the site.

Progress goes to stderr. The JSON report on stdout includes the discovered tool
schemas, invocation IDs, `WebMCP.toolResponded` events, and the final page state.
Any failed scenario makes the process exit with status 1.

```bash
uv run moli-playground/webmcp/webmcp_cdp_demo.py \
  --output /tmp/moli-webmcp-results.json
```

## Explore other sites

List tools without invoking them:

```bash
uv run moli-playground/webmcp/webmcp_cdp_demo.py --demo pizza --list-tools

uv run moli-playground/webmcp/webmcp_cdp_demo.py --list-tools \
  --url https://microsoftedge.github.io/webmcp-labs/pizza-order/
```

The URL inspector reports tools registered in the page's main frame. It waits
for at least one native tool and fails if the site exposes none within the
timeout. It does not infer tools from a static manifest.

Useful options:

- `--moli-bin /path/to/release/moli` selects a release binary explicitly.
- `--cdp-endpoint http://127.0.0.1:9222` reuses an existing release Moli server;
  the script opens and closes its own tabs and leaves the server running.
- `--http-proxy http://127.0.0.1:7890` configures a server started by the script.
- `--timeout 90` increases command, event, and HTTP timeouts in seconds.
- `--startup-timeout 30` gives a newly started server more time to become ready.

## External WebMCP usage

Research and Moli verification performed on **2026-10-05**. The runtime checks
below used `target/release/moli`, built with `cargo build --release -p moli`. The
[Google Chrome Labs directory](https://github.com/GoogleChromeLabs/webmcp-tools/blob/main/AWESOME_WEBMCP.md)
was a starting point; the links below point to the integrations and their own
documentation or source.

| Integration | Published use | Verification in this playground |
| --- | --- | --- |
| [Google Chrome Labs booking](https://googlechromelabs.github.io/webmcp-tools/demos/explainer/) · [source](https://github.com/GoogleChromeLabs/webmcp-tools/tree/main/demos/explainer) | Availability queries, reservations, and cancellation. | `booking` passed all three native calls and widget checks. |
| [Le Petit Bistro](https://googlechromelabs.github.io/webmcp-tools/demos/french-bistro/) · [source](https://github.com/GoogleChromeLabs/webmcp-tools/tree/main/demos/french-bistro) | Restaurant reservation using HTML form tool attributes. | `bistro` passed native filling, confirmation, and response checks. |
| [Google Chrome Labs pizza maker](https://googlechromelabs.github.io/webmcp-tools/demos/pizza-maker/) · [source](https://github.com/GoogleChromeLabs/webmcp-tools/tree/main/demos/pizza-maker) | Tools update pizza size, style, layers, and toppings. | `pizza` discovered seven tools and passed five native calls and page checks. |
| [Microsoft Edge Contoso Pizza](https://microsoftedge.github.io/webmcp-labs/pizza-order/) · [source](https://github.com/MicrosoftEdge/webmcp-labs) | Menu browsing and cart management; the available tools change with order state. | Native discovery verified: the initial page exposes `browse` and `create-order`. Other order states were not exercised. |
| [BestPrice](https://www.bestprice.gr/mcp#mcp-webmcp) · [tool inventory](https://www.bestprice.gr/webmcp.json) · [source](https://github.com/TheBestCo/bestprice-mcp/tree/main/webmcp) | A shopping comparison site with page tools for product search, listing filters, offers, and price history. | Its published WebMCP documentation and inventory were reviewed; not tested in Moli here. |
| [Open for Agents](https://www.openforagents.com/docs/getting-started) · [demo](https://demo.openforagents.com/) | WordPress and WooCommerce content/product tools, plus form preparation for visitors to confirm. | Its integration documentation was reviewed; not tested in Moli here. |

The three runnable scenarios are public sample apps. BestPrice and Open for
Agents show how the same API is being integrated into shopping and CMS workflows.
Some providers also publish remote MCP endpoints; those are separate from the
browser's WebMCP tool registry.

## Native execution path

The runner enables the `WebMCP` CDP domain, consumes `WebMCP.toolsAdded` and
`WebMCP.toolsRemoved`, calls `WebMCP.invokeTool`, and waits for
`WebMCP.toolResponded`. It also checks that `document.modelContext` is an instance
of the browser's native `ModelContext`.

The bistro and pizza sites ship their own compatibility polyfill, which returns
early when native `document.modelContext` exists. The runner supplies no
polyfill and invokes tools through Moli's registry. `Runtime.evaluate` reads the
page state and performs the bistro's ordinary confirmation click.

These demos need access to the external pages and their dependencies, including
GitHub Pages and the bistro's DOMPurify module on `esm.sh`. A network error or a
changed tool/schema/page structure is reported as a failure. They are manual
playground examples rather than offline CI fixtures.

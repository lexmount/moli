# WebMCP tools and demos with Moli

These demos open public WebMCP sites in Moli, discover their native tools over
CDP, invoke them, and check both the tool responses and the page state. No model
account or API key is required: the scripts supply the tool arguments directly.

## Built-in MCP service

Moli can open a site and expose its native WebMCP tools directly as a
[Streamable HTTP MCP service](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#streamable-http):

```bash
cargo build --release -p moli
target/release/moli webmcp serve \
  https://googlechromelabs.github.io/webmcp-tools/demos/pizza-maker/
```

Connect an MCP client to `http://127.0.0.1:9222/mcp`. CDP is available at
`http://127.0.0.1:9222`, with discovery at `/json/version` and `/json/list`.
Both protocols use the same live page, so calls preserve its state and CDP can
inspect, navigate, or interact with it. Moli implements the MCP tool transport
using its existing Axum, Tokio, and JSON dependencies, with no MCP SDK dependency.
It supports initialization, tool discovery and calls, cancellation, and
list-change notifications. Legacy clients use isolated sessions and the
2025-11-25, 2025-06-18, or 2025-03-26 protocol; sessions expire after 30 minutes
without a request. The 2026-07-28 protocol uses per-request metadata,
`server/discover`, and `subscriptions/listen` for notifications. Closing a modern
call's response stream cancels that invocation; legacy clients cancel explicitly
or terminate their session with HTTP DELETE.

In another terminal, run the pizza demo through the official Python MCP client:

```bash
uv run moli-playground/webmcp/webmcp_mcp_demo.py
# Or inspect the catalog and make a single call:
uv run moli-playground/webmcp/webmcp_mcp_demo.py --list
uv run moli-playground/webmcp/webmcp_mcp_demo.py \
  --tool set_pizza_size --input '{"size":"Small"}'
```

To discover tools without starting a service:

```bash
target/release/moli webmcp list \
  https://googlechromelabs.github.io/webmcp-tools/demos/pizza-maker/
```

`list` prints a JSON array of MCP tool definitions, including full input schemas.
It waits one second after DOM readiness; adjust `--wait-for-tools-ms` for delayed
registrations. An empty catalog prints `[]`. The running service continues to
track tool registrations, removals, navigation, and child-frame detachment.
Names are kept when possible, normalized to MCP's character set, and suffixed
when they collide. `_meta["moli/webmcp"]` identifies the original tool name,
frame, target, and declarative form node. JSON object results are returned as
`structuredContent` with a text representation; other results use text content.
Execution failures set `isError`.

`serve` accepts the normal server options, including `--port`, `--profile-dir`,
`--http-proxy`, and `--layout`. `--timeout` bounds initial navigation and
`--tool-timeout` bounds calls (default: 60 seconds). Cancelled and timed-out
calls abort the native invocation. The service opens one URL; other pages
created through CDP have their own tool catalogs.

Manual declarative forms wait for page confirmation by default. They can be
confirmed through CDP while the MCP call is pending. To submit these forms
automatically through their regular submit button, explicitly start with
`--auto-submit`, for example:

```bash
target/release/moli webmcp serve \
  https://googlechromelabs.github.io/webmcp-tools/demos/french-bistro/ \
  --auto-submit
```

Forms already marked `toolautosubmit` use the site's own behavior. Tool calls
still run the site's native handlers and form validation.

Release verification on 2026-10-05 used the official Python MCP client for all
five pizza calls and both manual and automatic bistro confirmation. CDP checked
the same page state; navigation to an empty page cleared the MCP catalog, and
navigating back registered callable tools again within the same MCP session.

## Operate any WebMCP site

`webmcp.py` accepts a site URL and uses the site's native tool catalog. Build Moli
with `cargo build --release -p moli`, then list a site's tools and full schemas:

```bash
uv run moli-playground/webmcp/webmcp.py \
  https://googlechromelabs.github.io/webmcp-tools/demos/pizza-maker/ list
```

Call a tool with JSON arguments:

```bash
uv run moli-playground/webmcp/webmcp.py \
  https://googlechromelabs.github.io/webmcp-tools/demos/pizza-maker/ \
  call set_pizza_size --input '{"size":"Large"}'
```

`--input @arguments.json` reads a file; `--input -` reads JSON from stdin. Input
must be a JSON object. Tool responses go to stdout as JSON, including the native
status, output, and any execution error. A failed one-shot call exits with status
1. A site with no registered tools produces an empty list.

Each one-shot command opens a new page. To make several calls on the same page,
use the interactive shell:

```bash
uv run moli-playground/webmcp/webmcp.py \
  https://googlechromelabs.github.io/webmcp-tools/demos/pizza-maker/ shell
```

Inside the shell:

```text
tools
schema set_pizza_size
call set_pizza_size {"size":"Large"}
wait <invocationId>
call add_topping {"topping":"🍍","size":"Large","count":3}
wait <invocationId>
quit
```

Use the invocation ID printed by each `call`. `result ID` shows its current
status, `wait ID` waits for completion, and `cancel ID` cancels a pending call.
The shell keeps receiving tool additions, removals, and responses while waiting
for your next command. Commands may also be piped into the shell. Pending calls
started by this session are cancelled when it exits.

For declarative forms that need confirmation, run `call NAME JSON`, inspect the
prepared form in your attached page, then use `confirm ID` and `wait ID`.
Confirmation targets the form identified by the native tool's node, including
forms in child frames. For a one-shot call, `--confirm` explicitly submits the
form after filling. Without it, a manual form call reports that confirmation is
required before starting an invocation.

The catalog includes tools in child frames. If a name occurs in several frames,
select `--frame-id ID` or use `frame ID` in the shell. `frame main` selects the
main frame; `frame all` returns to the complete catalog. Frame IDs belong to the
current page, so use the shell or attach to that page when selecting a child.

To reuse an authenticated or already prepared page in an existing release Moli
server:

```bash
uv run moli-playground/webmcp/webmcp.py - shell \
  --cdp-endpoint http://127.0.0.1:9222 --target-id TID-1
```

The URL `-` preserves the current page without navigating. `--target-id` requires
`--cdp-endpoint`; the script leaves that page and server open on exit. The target
IDs can be obtained from the server's `/json/list` endpoint.

The CLI waits two seconds after page load to collect initial registrations;
increase `--wait-for-tools` for sites that register tools later. It also supports
the release binary, proxy, and timeout options listed below. Sites must expose
native WebMCP tools; a remote MCP endpoint or static inventory alone is not a
browser tool registration.

## Run the preset demos

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

The CLI and demos share [webmcp_client.py](webmcp_client.py), including release
server startup, CDP event handling, and invocation tracking. Run the offline CLI
integration checks with a release binary already built:

```bash
uv run --with websockets python3 -m unittest discover \
  -s moli-playground/webmcp -p test_webmcp.py -v
```

The checks cover shared page state, duplicate names across frames, dynamic tool
registration/removal, cancellation, exact form confirmation with two pending
forms, execution errors, and preserving an attached page.

These demos need access to the external pages and their dependencies, including
GitHub Pages and the bistro's DOMPurify module on `esm.sh`. A network error or a
changed tool/schema/page structure is reported as a failure. They are manual
playground examples rather than offline CI fixtures.

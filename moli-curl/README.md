# Native HTTP and WebSocket runtime

## Module boundaries

| Module | Responsibility |
| --- | --- |
| `runtime.rs`, `runtime/owner.rs` | Runtime ownership and the single native thread: submit handling, shutdown, `perform`, completion dispatch and `poll` |
| `http.rs`, `http/registry.rs`, `http/scheduling.rs` | HTTP request capability, priority/origin admission, DNS/deadlines and transfer completion |
| `websocket/connector.rs`, `websocket/connection.rs` | WebSocket submission capability and the caller/owner I/O endpoints, including independent send completion |
| `websocket/registry.rs`, `websocket/session.rs` | Resident native connections, handshake and frame I/O |
| `websocket/readiness.rs` | I/O admission, extra socket interests and applying the shared poll's readiness results |
| `websocket/standalone.rs` | Convenience owner using the same runtime for standalone callers |
| `dns_adapter.rs`, `tls.rs` | Shared DNS and TLS configuration |

The owner dispatches to concrete HTTP and WebSocket registries. HTTP completion
removes a transfer; WebSocket handshake completion keeps its handle resident.
Both registries preserve the easy handle's error details when interpreting
`CURLMSG_DONE`. Only `runtime/owner.rs` calls `perform`, `messages` and `poll`.
Read that file first when investigating scheduling or thread shutdown.

Public exports remain at `moli_curl::*` for HTTP/runtime and
`moli_curl::websocket::*` for WebSocket. Neither protocol's request capability
owns the native thread. Shared owner counters live in `runtime/diagnostics.rs`;
the diagnostics switch and log fields below remain compatible.

`moli-curl` owns native connections and frame I/O. Browser message assembly,
UTF-8 checks, command admission and the close handshake live in `moli-websocket`.
The native owner keeps reads and writes independent and parks I/O after AGAIN
until the corresponding socket is signalled. It shares one spare receive Vec;
successful reads transfer that Vec to the event consumer.

## Enable counters

Set `MOLI_CURL_WEBSOCKET_DIAGNOSTICS=1` before creating the runtime and enable
INFO logging for the `moli_curl_websocket` tracing target. The owner emits an
aggregate about once per second and a final partial window when it exits.
Diagnostics are disabled by default; disabled diagnostics do not request
per-turn or per-poll timestamps. Counters are maintained on the owner thread.

| Fields | Meaning |
| --- | --- |
| `owner_thread` | Identifies the owner when more than one runtime emits reports |
| `turns`, `progressed_turns` | Owner rounds and rounds with native WebSocket I/O progress |
| `sessions` | Attached sessions, including opening handshakes |
| `read_bytes`, `written_bytes` | Payload bytes returned by receive / consumed by native send |
| `read_frames`, `written_frames` | Completed native frames; a fragmented browser message has multiple frames |
| `read_again`, `write_again` | Native operations that returned AGAIN |
| `receive_allocations` | Allocations of the owner's receive buffer |
| `polls`, `zero_timeout_polls` | All poll calls and calls explicitly requested with zero timeout |
| `requested_wait_ns`, `actual_wait_ns`, `max_wait_ns` | Sum of requested waits, sum of actual poll durations, maximum actual duration |
| `fast_no_io_polls`, `max_fast_no_io_streak` | Positive-timeout polls returning in less than 100 microseconds after a round without WebSocket I/O progress; total and longest consecutive streak |

Zero-timeout polls are expected while draining buffered work. A long sequence
of fast positive-timeout polls together with AGAIN and no byte/frame progress
is useful evidence for investigating a busy loop. Command and handshake
wakeups can also return early, so this counter is a diagnostic hint rather
than an automatic failure condition. Streaks carry over between log windows.
Actual poll duration includes time executing the poll call and time descheduled;
it is not a measurement of sleeping time alone. Compare it with owner CPU time.

Logs contain aggregate counts, with no URLs, request headers or payloads.
The current binding does not expose a TLS operation's actual cross-direction
wait. Ordinary WSS/backpressure coverage does not prove that all TLS wait
directions have been exercised.

## Local workload

Build the standalone probe once:

```sh
cargo build --release -p moli-curl --example websocket_owner_probe
```

Run individual workloads using the same duration:

```sh
target/release/examples/websocket_owner_probe --scenario idle --idle 128 --seconds 3
target/release/examples/websocket_owner_probe --scenario active --idle 1 --seconds 3
target/release/examples/websocket_owner_probe --scenario active --idle 32 --seconds 3
target/release/examples/websocket_owner_probe --scenario active --idle 128 --seconds 3
target/release/examples/websocket_owner_probe --scenario slow-read --idle 0 --seconds 3
target/release/examples/websocket_owner_probe --scenario wss --idle 0 --seconds 3
MOLI_CURL_WEBSOCKET_DIAGNOSTICS=1 target/release/examples/websocket_owner_probe --scenario active --idle 128 --seconds 3
```

`--idle` counts quiet connections; active scenarios add one connection. The
total must be between 1 and 255. The peer and owner run in separate threads
inside the probe. `slow-read` spaces peer reads by 2 ms to create send
backpressure. `wss` pauses native reading for 100 ms during the measured window,
then resumes it; its generated certificate is used only for the local fixture.

The probe validates payload content and reports elapsed time, transferred
payload bytes, MiB/s and owner-thread CPU time. On Linux, CPU time comes from
the owner's `/proc/self/task/*/schedstat`; on other platforms it is unavailable.
It excludes a fixed warmup and connection teardown. Native send completion
counts local consumption, not acknowledgement by the peer.

For comparisons, alternate binaries/configurations under the same conditions
and record several runs. Avoid concurrent builds or test suites while measuring.
CPU/throughput measurements are observations, not CI pass thresholds. Deterministic
CI regressions cover idle receive attempts, buffer allocation counts, retained
Chunk contents, WSS recovery, peer EOF and send progress under backpressure.

## Shared HTTP and WebSocket runtime

`CurlMultiRuntime::websocket_connector()` admits WebSockets to the same native
owner, Multi and poll loop as HTTP. HTTP completions remove an easy handle;
WebSocket handshake completion leaves it attached until the connection ends.
`CurlWebSocketRuntime` is a standalone wrapper around this same driver.

The connector is a request capability: retaining it does not retain the runtime
owner, and an explicitly supplied closed connector fails without a standalone
fallback. Fetch, Page and Worker use the connector from their network runtime.
The Fetch semantic thread and WebSocket Tokio session thread remain separate.

HTTP/1, HTTP/2 and WebSocket sockets share the Multi's default connection pool
and its host/total connection limits. A live WebSocket consumes one connection;
additional HTTP/2 streams can reuse an existing socket. Work requiring a new
connection waits when the pool is full, and that wait counts towards its HTTP
request or WebSocket handshake deadline. No protocol has reserved connections.
Long-lived WebSockets can therefore keep new HTTP connections waiting until
their deadline. Established WebSockets must never be evicted as idle HTTP
connections; the pinned curl fork protects them in both eviction paths.

The native WebSocket admission bound (255 pending/open sessions), event queue
and frame/message memory limits remain separate work and memory bounds. They
do not grant additional sockets beyond the shared pool limits. WebSocket
transport still uses HTTP/1.1 Upgrade; it does not implement RFC 8441.

In shared mode, poll/turn counters include HTTP work and HTTP wakeups. A fast
poll with no WebSocket byte progress can therefore reflect useful HTTP work.

Compare a shared owner with two separate owners in the same release binary:

```sh
target/release/examples/websocket_owner_probe --http separate --scenario active --idle 32 --seconds 5
target/release/examples/websocket_owner_probe --http shared --scenario active --idle 32 --seconds 5
```

`--http` defaults to `off`. Both enabled modes use an HTTP/1.1 keepalive peer,
16 warmup requests and a sequential offered load of up to 100 requests/s.
They report HTTP request count and p50/p95 latency, WebSocket throughput,
native owner count and the sum of native owner CPU time. The HTTP producer,
local peer and Tokio executor CPU are excluded. This measures native scheduling
coexistence, not HTTP/2 throughput or end-to-end browser page performance.
The Fetch regression tests separately verify real TLS HTTP/2 multiplexing while
WS/WSS reads are paused and after the WebSocket close handshake.

HTTP uses `runtime.http_sender().submit(job)` and WebSocket uses
`runtime.websocket_connector().connect(request)`. Only the runtime owns shutdown
and join; neither request handle does, and the runtime itself is not cloneable.

The browser WebSocket API takes the connector as an explicit spawn argument.
`ConnectOptions` contains only request configuration. Callers without a network
owner must choose `spawn_standalone_connection` (or its handshake-pause variant)
explicitly; scoped connection failures never select another owner.

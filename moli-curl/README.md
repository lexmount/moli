# Native WebSocket owner diagnostics

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

# Inspector JSON

`InspectorJson` carries V8 Inspector envelopes through CDP without decoding their
JavaScript string payloads into Rust UTF-8 strings. It validates JSON syntax with
`serde_json::RawValue`, shares immutable fields across clones, and rewrites the
outer command ID and session independently. Payload escapes, including unpaired
UTF-16 surrogates, retain their wire representation.

`value()` provides a cached UTF-8 business view; existing Value API access through
`Deref` requests the same view. Unpaired surrogates become U+FFFD in that view.
Reading it does not change serialization. `value_mut()` explicitly switches the
modified copy to `Parsed(Value)`. Other copies retain their original payloads.

Equality compares the emitted wire representation, including payload whitespace
and escape spelling. Raw/Raw comparison uses shared-storage and field-byte checks,
without creating a business view. Heterogeneous equality with Value is omitted
to keep its business semantics separate. Compare `message.value()` explicitly when lossy
UTF-8 business equality is intended.

Raw construction rejects the reserved top-level parameter names used for Moli
host metadata, including Unicode-escaped spellings. Host-generated messages use
`Parsed(Value)` and the existing output filter. Other parameter names, including
unpaired surrogate escapes, and names inside user objects remain opaque. The
reserved-name check uses serde_json's byte decoder instead of requiring UTF-8
field names. `Fetch.requestPaused` retains its existing Parsed-only interceptor
projection policy.

The structured view has the Value parser's depth limit. If a valid raw payload
exceeds it, `value()` returns a synthetic `-32603` error view; CDP still serializes
the original raw response. SessionSink settlement checks the raw `error` field,
and completion matching reads the raw call ID without requesting a business
view. A synthetic view error cannot turn a successful session response into a
failed settlement. This compatibility API is not a fallible, lossless JSON
accessor. The raw fields and cached Value coexist until their owners drop.

This is validated raw-field transport, not zero-copy or zero-scan forwarding.
Remote-object ownership, lifecycle handling, debugger state, and automation
adapters may still need structured data. The CDP writer retains its existing
message-size and pending-byte limits.

## Benchmark

Capture actual Inspector replies using Node.js 22+ with no npm dependencies:

```sh
node scripts/benchmark-inspector-json.mjs \
  --capture /path/to/moli --output target/inspector-json-capture
cargo run --release -p moli-inspector-json --example benchmark -- \
  --fixtures target/inspector-json-capture/fixtures --rounds 5
```

The example compares parsing, two clones, ID/session updates, and serialization.
`raw-view` also materializes a business view. It checks semantic equivalence before
timing, warms each group, and rotates mode order. These are carrier measurements,
not whole-browser speedups. The example uses the system allocator; Moli on Linux
x86_64 uses its V8 PartitionAlloc shim.

Five-round medians in microseconds, Rust 1.96.1, Linux x86_64,
Intel Core Ultra 7 270K Plus, pinned to CPU 0:

| Actual Inspector reply | Bytes | Value | Raw | Raw + view |
| --- | ---: | ---: | ---: | ---: |
| evaluate 1 KiB string | 1,101 | 1.058 | 0.399 | 0.898 |
| evaluate 100 KiB string | 102,477 | 108.336 | 10.374 | 72.835 |
| evaluate 1 MiB string | 1,048,653 | 1,517.392 | 772.093 | 1,133.940 |
| getProperties, 1,000 properties | 159,077 | 962.984 | 63.864 | 448.546 |

The script also supports paired end-to-end measurements and verifies exact UTF-16
units through Window, Worker, notifications, and non-flat CDP sessions:

```sh
node scripts/benchmark-inspector-json.mjs \
  --before /path/to/moli-before --after /path/to/moli-after \
  --output target/inspector-json-e2e --rounds 5

# Check wire semantics without running timings.
node scripts/benchmark-inspector-json.mjs \
  --verify /path/to/moli-after --output target/inspector-json-verification
```

The output records wall time, server CPU ticks on Linux, request counts, and wire
sizes. Compare binaries built with the same profile and flags. A debug-binary run
does not establish a production release speedup; allocation counts and RSS need
separate measurements.

A local five-round alternating A/B check against `57b2b17658`, using matching
debug builds and CPU affinity 0–7, measured these per-request wall-time medians:

| Request | Before, ms | After, ms |
| --- | ---: | ---: |
| evaluate 1 KiB | 1.078 | 1.066 |
| evaluate 100 KiB | 3.497 | 2.307 |
| evaluate 1 MiB | 30.977 | 18.869 |
| getProperties | 23.003 | 22.835 |

Both large-string cases improved in all five pairs. The small reply and
getProperties results do not show a stable performance improvement. In
particular, remote-object ownership still consumes structured property data;
the carrier microbenchmark overstates the benefit for that complete workflow.
No release end-to-end speedup or RSS improvement is established by this run.

## Retained-memory model

The `retained` release example holds 16 independently parsed copies of a captured
Inspector reply, with one or two queued clones per copy. It samples its own Linux `smaps_rollup` while
the queue is live. All three modes use the system allocator. It does not run the
other modes' validation pipelines first, which would contaminate allocation
and peak-heap measurements.

Five rotated rounds per configuration, using glibc `libmemusage.so` for allocation
counts (no failed allocations), produced these median PSS values in MiB:

| Reply | Clones per reply | Value | Raw | Raw + view |
| --- | ---: | ---: | ---: | ---: |
| 1 MiB string | 1 | 18.659 | 17.715 | 33.812 |
| 1 MiB string | 2 | 34.699 | 17.716 | 33.809 |
| 1,000 properties | 1 | 29.642 | 3.238 | 30.427 |
| 1,000 properties | 2 | 56.789 | 3.238 | 30.422 |

For one queued clone per reply, the corresponding complete-process counts of
malloc/realloc/calloc calls were 422 / 214 / 422 for the string fixture, and
416,902 / 214 / 208,726 for the property fixture (Value / Raw / Raw + view).

The single-clone string case demonstrates the memory cost of retaining both raw
bytes and a parsed copy. Sharing benefits depend on the number of live holders
and how often they request a view. These measurements do not settle whole-browser
PSS: they use a synthetic retained queue and the system allocator, rather than
Moli's allocator and actual message lifetimes. These clones keep identical
envelope metadata; copy-on-write routing changes can require independent views.
They do not validate or dismiss a Spider memory-regression observation.

The [raw CSV](benchmarks/retained-2026-11-08.csv) contains all 120 runs, including
1 KiB and 100 KiB fixtures, live PSS/RSS, peak requested heap bytes, and allocation
counts. Allocation hooks perturb timing; no timing results are derived from
these runs. Example command on Linux/glibc:

```sh
cargo build --release -p moli-inspector-json --example retained
LD_PRELOAD=/usr/lib/x86_64-linux-gnu/libmemusage.so \
  target/release/examples/retained \
  --fixture target/inspector-json-capture/fixtures/evaluate-1m.json \
  --mode raw-view --messages 16 --clones 1
```

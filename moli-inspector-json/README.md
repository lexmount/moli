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

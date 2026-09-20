# moli-disk-pool

Thread-safe storage for multiple resources in one anonymous temporary file.
Moli shares a pool between [parkable images](../moli-parkable-image/README.md) and generic response bodies.

## What it does

- Stores each payload in an immutable `(offset, len)` extent. Reservations and stored-data handles keep the pool alive and return their space when dropped.
- Reuses free extents with exact-fit, then worst-fit allocation, merging adjacent holes on release. Offset and size indexes make allocation and release O(log F) for F free extents.
- Provides offset-based reads and writes using standard platform file APIs. The file is created with `tempfile`; no `positioned_io` dependency is needed.

## Trade-offs

- One file avoids a separate file per resource. The two indexes cost extra metadata, and fragmentation can still prevent a contiguous allocation.
- Released space is reusable, but the file is not compacted or truncated. The optional capacity limit bounds the logical high-water mark, not guaranteed filesystem space.
- I/O is synchronous; async callers must arrange blocking execution. Storage failures are returned to the caller, which owns any memory fallback.
- This is temporary backing storage, not a persistent cache. It provides no URL index, deduplication, or durability guarantee.

## Examples

- [`basic.rs`](examples/basic.rs): write, read a range, and reuse an extent.
- [`capacity.rs`](examples/capacity.rs): handle capacity exhaustion and cancel a reservation.
- [`shared_readers.rs`](examples/shared_readers.rs): share stored data across threads.

Run from the repository root:

```sh
cargo run -p moli-disk-pool --example basic
cargo run -p moli-disk-pool --example capacity
cargo run -p moli-disk-pool --example shared_readers
```

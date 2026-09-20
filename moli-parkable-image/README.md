# moli-parkable-image

Shared storage for completed encoded image bytes.
It releases idle buffers from memory by parking them in [`moli-disk-pool`](../moli-disk-pool/README.md), then restores them when a reader needs them.
The renderer shares this backing between image consumers and network records.

## What it does

- `from_frozen_bytes(Vec<u8>)` takes ownership of a completed buffer. Snapshots provide read-only access and prevent its memory from being discarded while readers are active.
- Parking writes one extent per image. Reading a parked image restores its bytes; the disk backup remains, so parking again needs no additional write.
- A manager tracks deadlines and separates resident from parked images. Defaults are a 1 KiB minimum size, a 30-second initial delay, and a 2-second delay after the last snapshot is released.
- `park_images()` respects those delays; `force_park_images()` bypasses them and immediately retries parking. Neither waits for a deadline or discards bytes held by live snapshots.
- The owning runtime drives scheduling. This crate creates no threads or timers; constructing a manager alone does not start automatic parking.

## Trade-offs

- `Arc<Vec<u8>>` adopts the receive buffer without a full payload copy, but retains its spare capacity until parking.
- Only completed encoded bytes are managed. Receive buffers and decoded RGBA/SVG data remain outside this crate.
- Unparking synchronously reads the whole image, even for a partial read. Retaining the disk backup uses space but makes later parking cheap.
- Failed parking keeps the bytes in memory; failed unparking returns an I/O error. Releasing a heap buffer does not guarantee an immediate RSS reduction, particularly with allocator retention or tmpfs.

## Examples

These use synthetic bytes; no decoder is involved.

- [`park_unpark.rs`](examples/park_unpark.rs): park manually, then restore through a snapshot.
- [`reader_leases.rs`](examples/reader_leases.rs): show why snapshots prevent parking but cloned image handles do not.
- [`deadline.rs`](examples/deadline.rs): customize the policy and drive one deadline. This is a single-threaded demonstration, not a concurrent scheduler.

Run from the repository root:

```sh
cargo run -p moli-parkable-image --example park_unpark
cargo run -p moli-parkable-image --example reader_leases
cargo run -p moli-parkable-image --example deadline
```

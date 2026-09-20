//! Retain bytes when capacity is exhausted and cancel an unwritten reservation.
//!
//! Run with `cargo run -p moli-disk-pool --example capacity`.

use moli_disk_pool::DiskPool;

fn main() -> std::io::Result<()> {
    let pool = DiskPool::new(Some(8))?;
    let pending = pool
        .try_reserve_chunk(8)
        .expect("reserve the entire capacity");
    let bytes = b"payload".to_vec();

    let stored = pool.store(&bytes)?;
    assert!(stored.is_none());
    assert!(pool.may_write()); // Capacity exhaustion does not disable allocation.
    assert_eq!(bytes, b"payload"); // The caller still owns its fallback bytes.

    drop(pending); // No file write was needed to release this reservation.
    let stored = pool.store(&bytes)?.expect("the released range now fits");
    assert_eq!(stored.to_vec()?, bytes);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, 8);
    println!(
        "Stored {} bytes after cancelling the reservation; logical high-water mark: {} bytes.",
        bytes.len(),
        pool.diagnostics().disk_footprint_bytes,
    );
    Ok(())
}

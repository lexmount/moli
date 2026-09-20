//! Show that snapshots pin synthetic image bytes, but cloned image handles do not.
//!
//! Run with `cargo run -p moli-parkable-image --example reader_leases`.

use moli_disk_pool::DiskPool;
use moli_parkable_image::{ParkableImageManager, ParkableImagePolicy};

fn main() -> std::io::Result<()> {
    let manager =
        ParkableImageManager::new(Some(DiskPool::new(None)?), ParkableImagePolicy::default());
    let image = manager.from_frozen_bytes(vec![7; 4096]);
    let other_handle = image.clone();
    assert!(image.shares_storage_with(&other_handle));

    let first = image.snapshot()?;
    let second = first.clone();
    manager.force_park_images();
    assert_eq!(manager.diagnostics().parked_count, 0);

    drop(first);
    manager.force_park_images();
    assert_eq!(manager.diagnostics().parked_count, 0); // One reader remains.

    drop(second);
    manager.force_park_images();
    assert_eq!(manager.diagnostics().parked_count, 1);
    assert_eq!(other_handle.retained_memory_bytes(), 0);
    println!("Released both snapshot leases and parked the image with another handle still alive.");
    Ok(())
}

//! Park synthetic image bytes and restore them through a snapshot.
//!
//! Run with `cargo run -p moli-parkable-image --example park_unpark`.

use moli_disk_pool::DiskPool;
use moli_parkable_image::{ParkableImageManager, ParkableImagePolicy};

fn main() -> std::io::Result<()> {
    let pool = DiskPool::new(None)?;
    let manager = ParkableImageManager::new(Some(pool), ParkableImagePolicy::default());
    let image = manager.from_frozen_bytes(vec![42; 4096]);

    {
        let snapshot = image.snapshot()?;
        assert_eq!(snapshot[0], 42);
        // The image cannot discard its resident bytes while this snapshot exists.
    }

    manager.force_park_images();
    assert_eq!(manager.diagnostics().parked_count, 1);

    let snapshot = image.snapshot()?; // Synchronous unpark; keep the disk backup.
    assert_eq!(snapshot.len(), 4096);
    println!(
        "Restored {} bytes; resident images with a disk backup: {}.",
        snapshot.len(),
        manager.diagnostics().resident_with_disk_backup_count,
    );
    Ok(())
}

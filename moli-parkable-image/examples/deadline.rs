//! Drive one parking deadline with a custom policy and synthetic image bytes.
//!
//! Run with `cargo run -p moli-parkable-image --example deadline`.
//!
//! This single-threaded, one-shot wait is not a concurrent scheduler.
//! Concurrent owners must also react to new_with_schedule_wakeup() notifications.

use std::time::{Duration, Instant};

use moli_disk_pool::DiskPool;
use moli_parkable_image::{ParkableImageManager, ParkableImagePolicy};

fn main() -> std::io::Result<()> {
    let policy = ParkableImagePolicy {
        parking_delay: Duration::from_millis(10),
        ..ParkableImagePolicy::default()
    };
    let manager = ParkableImageManager::new(Some(DiskPool::new(None)?), policy);
    let image = manager.from_frozen_bytes(vec![3; 4096]);

    let deadline = manager.next_parking_deadline().expect("image is eligible");
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    manager.park_images();

    assert_eq!(image.retained_memory_bytes(), 0);
    assert_eq!(manager.diagnostics().parked_count, 1);
    assert!(manager.next_parking_deadline().is_none());
    println!("Parked the image at its deadline; no further deadline is scheduled.");
    Ok(())
}

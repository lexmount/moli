//! Share one disk extent across threads, even after dropping the pool handle.
//!
//! Run with `cargo run -p moli-disk-pool --example shared_readers`.

use std::sync::Arc;

use moli_disk_pool::DiskPool;

fn main() -> std::io::Result<()> {
    let pool = DiskPool::new(None)?;
    let data = pool
        .store(b"shared payload")?
        .expect("a fresh pool has capacity");
    let shared = Arc::new(data);
    let reader = Arc::clone(&shared);
    drop(pool);

    let worker = std::thread::spawn(move || reader.to_vec());
    let local_bytes = shared.to_vec()?;
    let remote_bytes = worker.join().expect("reader should not panic")?;
    assert_eq!(local_bytes, remote_bytes);
    assert_eq!(local_bytes, b"shared payload");

    drop(shared); // The worker is done, so the final data owner is released.
    println!(
        "Both readers recovered the same {} bytes.",
        local_bytes.len()
    );
    Ok(())
}

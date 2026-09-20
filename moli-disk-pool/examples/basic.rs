//! Store bytes, read a range, and reuse a released extent.
//!
//! Run with `cargo run -p moli-disk-pool --example basic`.

use moli_disk_pool::DiskPool;

fn main() -> std::io::Result<()> {
    let pool = DiskPool::new(Some(1024 * 1024))?;
    let data = pool
        .store(b"hello world")?
        .expect("the payload fits this fresh pool");

    let mut suffix = [0; 5];
    data.read_exact_at(6, &mut suffix)?;
    assert_eq!(&suffix, b"world");

    let original_offset = data.offset();
    drop(data); // Return the extent; the pool file remains available for reuse.

    let reserved = pool
        .try_reserve_chunk(11)
        .expect("the released extent is reusable");
    assert_eq!(reserved.offset(), original_offset);
    let replacement = pool.write(reserved, b"other bytes")?;
    assert_eq!(replacement.to_vec()?, b"other bytes");
    println!("Stored, read, and reused an extent at offset {original_offset}.");
    Ok(())
}

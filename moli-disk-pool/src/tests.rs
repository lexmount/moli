use std::sync::Arc;

use crate::{DiskData, DiskPool, DiskPoolDiagnostics};

fn store(pool: &DiskPool, bytes: &[u8]) -> DiskData {
    pool.store(bytes)
        .expect("disk write should succeed")
        .expect("disk pool should have capacity")
}

#[test]
fn reserved_chunk_is_released_on_drop() {
    let pool = DiskPool::new(None).unwrap();
    let first = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(first.offset(), 0);
    let second = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(second.offset(), 100);
    drop(second);

    let reused = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(reused.offset(), 100);
}

#[test]
fn writes_and_reads_without_a_shared_file_cursor() {
    let pool = DiskPool::new(None).unwrap();
    let left = store(&pool, b"left body");
    let right = store(&pool, b"right body");
    assert_eq!(left.offset(), 0);
    assert_eq!(right.offset(), 9);

    let mut middle = [0; 5];
    right.read_exact_at(1, &mut middle).unwrap();
    assert_eq!(&middle, b"ight ");
    assert_eq!(left.to_vec().unwrap(), b"left body");
    assert_eq!(right.to_vec().unwrap(), b"right body");
}

#[test]
fn allocation_prefers_exact_fit_then_worst_fit() {
    let pool = DiskPool::new(None).unwrap();
    let exact_hole = store(&pool, &[1; 100]);
    let separator = store(&pool, &[2; 10]);
    let larger_hole = store(&pool, &[3; 200]);
    let tail = store(&pool, &[4; 10]);
    let exact_offset = exact_hole.offset();
    let larger_offset = larger_hole.offset();
    drop(exact_hole);
    drop(larger_hole);

    let exact = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(exact.offset(), exact_offset);
    drop(exact);

    let worst_fit = pool.try_reserve_chunk(99).unwrap();
    assert_eq!(worst_fit.offset(), larger_offset);
    drop((separator, tail, worst_fit));
}

#[test]
fn adjacent_free_chunks_are_coalesced() {
    let pool = DiskPool::new(None).unwrap();
    let first = store(&pool, &[1; 100]);
    let second = store(&pool, &[2; 100]);
    let third = store(&pool, &[3; 100]);
    let fourth = store(&pool, &[4; 100]);

    drop(first);
    drop(third);
    assert_eq!(pool.diagnostics().free_chunk_count, 2);
    drop(second);
    assert_eq!(pool.diagnostics().free_chunk_count, 1);
    assert_eq!(pool.diagnostics().free_bytes, 300);
    drop(fourth);

    assert_eq!(
        pool.diagnostics(),
        DiskPoolDiagnostics {
            may_write: true,
            disk_footprint_bytes: 400,
            free_bytes: 400,
            free_chunk_count: 1,
        }
    );
}

#[test]
fn capacity_reuses_holes_but_does_not_extend_tail() {
    let pool = DiskPool::new(Some(100)).unwrap();
    let full = store(&pool, &[7; 100]);
    assert!(pool.try_reserve_chunk(1).is_none());
    drop(full);

    let reused = pool.try_reserve_chunk(60).unwrap();
    assert_eq!(reused.offset(), 0);
    assert!(pool.try_reserve_chunk(41).is_none());
    assert!(pool.try_reserve_chunk(40).is_some());
    assert_eq!(pool.diagnostics().disk_footprint_bytes, 100);
}

#[test]
fn stored_data_keeps_pool_alive_and_supports_concurrent_reads() {
    let pool = DiskPool::new(None).unwrap();
    let diagnostics = pool.clone();
    let data = Arc::new(store(&pool, &[9; 128 * 1024]));
    drop(pool);

    let readers: Vec<_> = (0..4)
        .map(|_| {
            let data = Arc::clone(&data);
            std::thread::spawn(move || assert_eq!(data.to_vec().unwrap(), vec![9; data.len()]))
        })
        .collect();
    for reader in readers {
        reader.join().unwrap();
    }
    assert_eq!(diagnostics.diagnostics().free_bytes, 0);
    drop(data);
    assert_eq!(diagnostics.diagnostics().free_bytes, 128 * 1024);
}

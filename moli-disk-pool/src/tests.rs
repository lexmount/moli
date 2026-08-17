use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::Arc,
};

use crate::{
    DiskData, DiskPool, DiskPoolDiagnostics,
    allocation::{AllocationState, Extent, find_free_chunk, release_chunk},
};

fn store(pool: &DiskPool, bytes: &[u8]) -> DiskData {
    pool.store(bytes)
        .expect("disk write should succeed")
        .expect("disk pool should have capacity")
}

fn store_filled(pool: &DiskPool, len: usize, value: u8) -> DiskData {
    store(pool, &vec![value; len])
}

fn reference_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| u8::try_from((offset * 37) % 251).unwrap())
        .collect()
}

fn allocate_equal(pool: &DiskPool, len: usize, count: usize) -> Vec<DiskData> {
    (0..count)
        .map(|index| {
            let data = store_filled(pool, len, u8::try_from(index).unwrap());
            assert_eq!(
                data.offset(),
                u64::try_from(index.checked_mul(len).unwrap()).unwrap()
            );
            data
        })
        .collect()
}

fn assert_free_chunks(pool: &DiskPool, expected: &[(u64, usize)]) {
    assert_eq!(pool.free_chunks_for_test(), expected);
    let diagnostics = pool.diagnostics();
    assert_eq!(diagnostics.free_chunk_count, expected.len());
    assert_eq!(
        diagnostics.free_bytes,
        expected.iter().map(|(_, len)| len).sum::<usize>()
    );
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

    let larger = pool.try_reserve_chunk(300).unwrap();
    assert_eq!(larger.offset(), 200);
    drop(larger);

    let split_reuse = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(split_reuse.offset(), 200);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, 500);
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
fn exact_end_reads_succeed_but_extent_escape_and_overflow_are_rejected() {
    let pool = DiskPool::new(None).unwrap();
    let data = store(&pool, b"abcdef");
    let neighbor = store(&pool, b"neighbor");

    let mut last = [0];
    data.read_exact_at(data.len() - 1, &mut last).unwrap();
    assert_eq!(last, [b'f']);
    data.read_exact_at(data.len(), &mut []).unwrap();

    for offset in [data.len(), data.len() + 1, usize::MAX] {
        let error = data.read_exact_at(offset, &mut [0]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    let error = data.read_exact_at(data.len() + 1, &mut []).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(neighbor.to_vec().unwrap(), b"neighbor");
}

#[test]
fn reused_extent_overwrites_old_bytes_without_touching_neighbors() {
    let pool = DiskPool::new(None).unwrap();
    let old = store(&pool, b"old-body");
    let neighbor = store(&pool, b"neighbor");
    let old_offset = old.offset();
    drop(old);

    let replacement = store(&pool, b"new-body");

    assert_eq!(replacement.offset(), old_offset);
    assert_eq!(replacement.to_vec().unwrap(), b"new-body");
    assert_eq!(neighbor.to_vec().unwrap(), b"neighbor");
}

#[test]
fn write_to_crosses_internal_buffer_boundaries() {
    let pool = DiskPool::new(None).unwrap();
    let expected = reference_bytes(2 * 64 * 1024 + 17);
    let data = store(&pool, &expected);
    let mut output = Vec::new();

    data.write_to(&mut output).unwrap();

    assert_eq!(output, expected);
}

#[test]
fn write_to_propagates_writer_failure_without_releasing_the_extent() {
    struct FailAfter {
        remaining: usize,
    }

    impl Write for FailAfter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected writer failure",
                ));
            }
            let written = buffer.len().min(self.remaining);
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let pool = DiskPool::new(None).unwrap();
    let expected = reference_bytes(64 * 1024 + 32);
    let data = store(&pool, &expected);
    let mut writer = FailAfter {
        remaining: 64 * 1024 + 7,
    };

    let error = data.write_to(&mut writer).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(pool.diagnostics().free_bytes, 0);
    assert_eq!(data.to_vec().unwrap(), expected);
}

#[test]
fn reads_and_discards_multiple_extents_in_independent_orders() {
    let pool = DiskPool::new(None).unwrap();
    let sizes = [137, 991, 256, 743, 1000, 411, 829, 173, 619, 347];
    let mut file_tail = 0_u64;
    let mut entries = sizes
        .into_iter()
        .enumerate()
        .map(|(index, len)| {
            let bytes = (0..len)
                .map(|offset| u8::try_from((index * 37 + offset) % 251).unwrap())
                .collect::<Vec<_>>();
            let data = store(&pool, &bytes);
            assert_eq!(data.offset(), file_tail);
            file_tail += u64::try_from(len).unwrap();
            Some((data, bytes))
        })
        .collect::<Vec<_>>();

    for index in [7, 2, 9, 0, 5, 1, 8, 4, 6, 3] {
        let (data, expected) = entries[index].as_ref().unwrap();
        assert_eq!(data.to_vec().unwrap(), *expected);
    }

    let mut released_bytes = 0;
    for index in [4, 1, 8, 0, 9, 3, 6, 2, 7, 5] {
        released_bytes += entries[index].as_ref().unwrap().0.len();
        drop(entries[index].take());
        assert_eq!(pool.diagnostics().free_bytes, released_bytes);
    }

    assert_eq!(pool.diagnostics().disk_footprint_bytes, file_tail);
    assert_free_chunks(&pool, &[(0, usize::try_from(file_tail).unwrap())]);
}

#[test]
fn freed_middle_extent_is_reused_before_growing_the_file_tail() {
    const CHUNK_SIZE: usize = 1024;
    let pool = DiskPool::new(None).unwrap();
    let mut chunks = allocate_equal(&pool, CHUNK_SIZE, 10)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    let reused_offset = chunks[4].as_ref().unwrap().offset();
    drop(chunks[4].take());

    let replacement = store_filled(&pool, CHUNK_SIZE, 42);
    assert_eq!(replacement.offset(), reused_offset);
    assert_eq!(
        pool.diagnostics().disk_footprint_bytes,
        u64::try_from(CHUNK_SIZE * 10).unwrap()
    );
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
fn equal_sized_candidates_preserve_lowest_offset_tie_breaking() {
    let pool = DiskPool::new(None).unwrap();
    let first_hole = store(&pool, &[1; 200]);
    let first_separator = store(&pool, &[2; 10]);
    let second_hole = store(&pool, &[3; 200]);
    let second_separator = store(&pool, &[4; 10]);
    let first_offset = first_hole.offset();
    drop(first_hole);
    drop(second_hole);

    let exact = pool.try_reserve_chunk(200).unwrap();
    assert_eq!(exact.offset(), first_offset);
    drop(exact);

    let chosen = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(chosen.offset(), first_offset);
    drop((chosen, first_separator, second_separator));
}

#[test]
fn free_extent_indexes_stay_consistent_through_split_and_merge() {
    fn assert_consistent(state: &AllocationState) {
        let expected_by_size = state
            .free_chunks_by_offset
            .iter()
            .map(|(&offset, &len)| (len, offset))
            .collect::<BTreeSet<_>>();
        assert_eq!(state.free_chunks_by_size, expected_by_size);
        assert_eq!(
            state.free_bytes,
            state.free_chunks_by_offset.values().copied().sum::<usize>()
        );
    }

    let mut state = AllocationState::default();
    release_chunk(
        &mut state,
        Extent {
            offset: 0,
            len: 100,
        },
    );
    release_chunk(
        &mut state,
        Extent {
            offset: 200,
            len: 200,
        },
    );
    assert_consistent(&state);

    let chosen = find_free_chunk(&mut state, 50).unwrap();
    assert_eq!(chosen.offset, 200);
    assert_consistent(&state);

    release_chunk(&mut state, chosen);
    release_chunk(
        &mut state,
        Extent {
            offset: 100,
            len: 100,
        },
    );
    assert_consistent(&state);
    assert_eq!(state.free_chunks_by_offset, BTreeMap::from([(0, 400)]));
}

#[test]
fn dual_indexes_match_chromium_linear_policy_across_many_operations() {
    #[derive(Default)]
    struct LinearReference {
        file_tail: u64,
        free_chunks: Vec<Extent>,
    }

    impl LinearReference {
        fn reserve(&mut self, size: usize) -> Extent {
            let mut chosen_index = None;
            let mut worst_fit_size = 0;
            for (index, extent) in self.free_chunks.iter().enumerate() {
                if extent.len == size {
                    chosen_index = Some(index);
                    break;
                }
                if extent.len > size && extent.len > worst_fit_size {
                    chosen_index = Some(index);
                    worst_fit_size = extent.len;
                }
            }

            if let Some(index) = chosen_index {
                let mut chosen = self.free_chunks.remove(index);
                if chosen.len > size {
                    self.free_chunks.push(Extent {
                        offset: chosen
                            .offset
                            .checked_add(u64::try_from(size).unwrap())
                            .unwrap(),
                        len: chosen.len - size,
                    });
                    self.free_chunks
                        .sort_unstable_by_key(|extent| extent.offset);
                    chosen.len = size;
                }
                return chosen;
            }

            let extent = Extent {
                offset: self.file_tail,
                len: size,
            };
            self.file_tail = self
                .file_tail
                .checked_add(u64::try_from(size).unwrap())
                .unwrap();
            extent
        }

        fn release(&mut self, extent: Extent) {
            self.free_chunks.push(extent);
            self.free_chunks
                .sort_unstable_by_key(|extent| extent.offset);

            let mut coalesced: Vec<Extent> = Vec::with_capacity(self.free_chunks.len());
            for extent in self.free_chunks.drain(..) {
                if let Some(left) = coalesced.last_mut() {
                    let left_end = left
                        .offset
                        .checked_add(u64::try_from(left.len).unwrap())
                        .unwrap();
                    assert!(left_end <= extent.offset);
                    if left_end == extent.offset {
                        left.len = left.len.checked_add(extent.len).unwrap();
                        continue;
                    }
                }
                coalesced.push(extent);
            }
            self.free_chunks = coalesced;
        }
    }

    fn reserve(state: &mut AllocationState, size: usize) -> Extent {
        find_free_chunk(state, size).unwrap_or_else(|| {
            let extent = Extent {
                offset: state.file_tail,
                len: size,
            };
            state.file_tail = state
                .file_tail
                .checked_add(u64::try_from(size).unwrap())
                .unwrap();
            extent
        })
    }

    fn assert_matches(state: &AllocationState, reference: &LinearReference) {
        let expected_by_offset = reference
            .free_chunks
            .iter()
            .map(|extent| (extent.offset, extent.len))
            .collect::<BTreeMap<_, _>>();
        let expected_by_size = reference
            .free_chunks
            .iter()
            .map(|extent| (extent.len, extent.offset))
            .collect::<BTreeSet<_>>();
        assert_eq!(state.file_tail, reference.file_tail);
        assert_eq!(state.free_chunks_by_offset, expected_by_offset);
        assert_eq!(state.free_chunks_by_size, expected_by_size);
        assert_eq!(
            state.free_bytes,
            reference
                .free_chunks
                .iter()
                .map(|extent| extent.len)
                .sum::<usize>()
        );
    }

    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    let mut state = AllocationState::default();
    let mut reference = LinearReference::default();
    let mut live = Vec::new();
    let mut random_state = 0x4d59_5df4_d0f3_3173;

    for _ in 0..5_000 {
        let random = next_random(&mut random_state);
        let should_reserve = live.is_empty() || (live.len() < 128 && random % 100 < 58);
        if should_reserve {
            let size = match random % 5 {
                0 => 1024,
                1 => 4096,
                _ => usize::try_from(random % 8192).unwrap() + 1,
            };
            let actual = reserve(&mut state, size);
            let expected = reference.reserve(size);
            assert_eq!(actual, expected);
            live.push(actual);
        } else {
            let index = usize::try_from(random % u64::try_from(live.len()).unwrap()).unwrap();
            let extent = live.swap_remove(index);
            release_chunk(&mut state, extent);
            reference.release(extent);
        }
        assert_matches(&state, &reference);
    }

    for extent in live {
        release_chunk(&mut state, extent);
        reference.release(extent);
        assert_matches(&state, &reference);
    }
    assert_eq!(
        reference.free_chunks,
        vec![Extent {
            offset: 0,
            len: usize::try_from(reference.file_tail).unwrap(),
        }]
    );
}

#[test]
fn free_chunks_merge_from_the_left_right_and_both_sides() {
    const CHUNK_SIZE: usize = 100;

    let pool = DiskPool::new(None).unwrap();
    let mut chunks = allocate_equal(&pool, CHUNK_SIZE, 4)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    drop(chunks[0].take());
    assert_free_chunks(&pool, &[(0, 100)]);
    drop(chunks[1].take());
    assert_free_chunks(&pool, &[(0, 200)]);
    drop(chunks[2].take());
    assert_free_chunks(&pool, &[(0, 300)]);
    drop(chunks[3].take());
    assert_free_chunks(&pool, &[(0, 400)]);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, 400);

    let pool = DiskPool::new(None).unwrap();
    let mut chunks = allocate_equal(&pool, CHUNK_SIZE, 4)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    drop(chunks[3].take());
    assert_free_chunks(&pool, &[(300, 100)]);
    drop(chunks[2].take());
    assert_free_chunks(&pool, &[(200, 200)]);
    drop(chunks[0].take());
    assert_free_chunks(&pool, &[(0, 100), (200, 200)]);
    drop(chunks[1].take());
    assert_free_chunks(&pool, &[(0, 400)]);

    let pool = DiskPool::new(None).unwrap();
    let mut chunks = allocate_equal(&pool, CHUNK_SIZE, 4)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    drop(chunks[0].take());
    drop(chunks[2].take());
    assert_free_chunks(&pool, &[(0, 100), (200, 100)]);
    drop(chunks[1].take());
    assert_free_chunks(&pool, &[(0, 300)]);
    drop(chunks[3].take());
    assert_eq!(
        pool.diagnostics(),
        DiskPoolDiagnostics {
            may_write: true,
            disk_footprint_bytes: 400,
            free_bytes: 400,
            free_chunk_count: 1,
        }
    );
    assert_free_chunks(&pool, &[(0, 400)]);
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
fn zero_length_and_over_capacity_requests_are_side_effect_free() {
    const CAPACITY: usize = 8;
    let pool = DiskPool::new(Some(CAPACITY as u64)).unwrap();
    let initial = pool.diagnostics();

    assert!(pool.try_reserve_chunk(0).is_none());
    assert!(pool.store(&[]).unwrap().is_none());
    assert_eq!(pool.diagnostics(), initial);

    let full = store(&pool, b"12345678");
    let at_capacity = pool.diagnostics();
    assert_eq!(at_capacity.disk_footprint_bytes, CAPACITY as u64);
    assert!(pool.try_reserve_chunk(1).is_none());
    assert!(pool.try_reserve_chunk(usize::MAX).is_none());
    assert_eq!(pool.diagnostics(), at_capacity);

    drop(full);
    let replacement = store(&pool, b"abcdefgh");
    assert_eq!(replacement.offset(), 0);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, CAPACITY as u64);
}

#[test]
fn reservation_cannot_be_written_by_another_pool() {
    let source = DiskPool::new(None).unwrap();
    let target = DiskPool::new(None).unwrap();
    let chunk = source.try_reserve_chunk(4).unwrap();

    let error = target.write(chunk, b"data").unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_free_chunks(&source, &[(0, 4)]);
    assert_eq!(target.diagnostics().disk_footprint_bytes, 0);
    assert_eq!(target.diagnostics().free_bytes, 0);
    assert!(target.may_write());
    assert_eq!(source.try_reserve_chunk(4).unwrap().offset(), 0);
}

#[test]
fn limited_capacity_does_not_combine_disjoint_free_space() {
    const CAPACITY: usize = 1024 * 1024;
    let pool = DiskPool::new(Some(u64::try_from(CAPACITY).unwrap())).unwrap();

    {
        let full = pool.try_reserve_chunk(CAPACITY).unwrap();
        assert!(pool.try_reserve_chunk(1).is_none());
        drop(full);
    }

    // Layout: | allocated (capacity - 1000) | free 500 |
    //         | allocated 100 | free 400 |
    let first = store_filled(&pool, CAPACITY - 1000, 1);
    let middle_hole = store_filled(&pool, 500, 2);
    let separator = store_filled(&pool, 100, 3);
    drop(middle_hole);
    assert_free_chunks(
        &pool,
        &[
            (u64::try_from(CAPACITY - 1000).unwrap(), 500),
            (u64::try_from(CAPACITY - 400).unwrap(), 400),
        ],
    );

    let reserved = pool.try_reserve_chunk(450).unwrap();
    assert_eq!(reserved.offset(), u64::try_from(CAPACITY - 1000).unwrap());
    assert!(pool.try_reserve_chunk(450).is_none());
    assert_eq!(
        pool.diagnostics().disk_footprint_bytes,
        u64::try_from(CAPACITY).unwrap()
    );
    drop((first, separator, reserved));
}

#[test]
fn write_failure_releases_the_extent_and_disables_later_writes() {
    let pool = DiskPool::new(None).unwrap();
    let chunk = pool.try_reserve_chunk(256).unwrap();
    pool.fail_next_write_for_test();

    let error = pool.write(chunk, &[7; 256]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert!(!pool.may_write());
    assert!(pool.try_reserve_chunk(1).is_none());
    assert_eq!(
        pool.diagnostics(),
        DiskPoolDiagnostics {
            may_write: false,
            disk_footprint_bytes: 256,
            free_bytes: 256,
            free_chunk_count: 1,
        }
    );
    assert_free_chunks(&pool, &[(0, 256)]);
}

#[test]
fn invalid_write_lengths_release_the_reservation_without_disabling_the_pool() {
    let pool = DiskPool::new(None).unwrap();
    for data_len in [99, 101] {
        let chunk = pool.try_reserve_chunk(100).unwrap();
        let error = pool.write(chunk, &vec![1; data_len]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(pool.may_write());
        assert_free_chunks(&pool, &[(0, 100)]);
    }

    let reused = pool.try_reserve_chunk(100).unwrap();
    assert_eq!(reused.offset(), 0);
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

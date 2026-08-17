use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub(crate) struct AllocationState {
    pub(crate) may_write: bool,
    pub(crate) file_tail: u64,
    pub(crate) free_chunks_by_offset: BTreeMap<u64, usize>,
    pub(crate) free_chunks_by_size: BTreeSet<(usize, u64)>,
    pub(crate) free_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Extent {
    pub(crate) offset: u64,
    pub(crate) len: usize,
}

impl AllocationState {
    fn insert_free_chunk(&mut self, extent: Extent) {
        let previous = self.free_chunks_by_offset.insert(extent.offset, extent.len);
        debug_assert!(previous.is_none());
        let inserted = self.free_chunks_by_size.insert((extent.len, extent.offset));
        debug_assert!(inserted);
    }

    fn remove_free_chunk(&mut self, extent: Extent) {
        let removed = self.free_chunks_by_offset.remove(&extent.offset);
        debug_assert_eq!(removed, Some(extent.len));
        let removed = self
            .free_chunks_by_size
            .remove(&(extent.len, extent.offset));
        debug_assert!(removed);
    }
}

pub(crate) fn find_free_chunk(state: &mut AllocationState, size: usize) -> Option<Extent> {
    let exact_fit = state
        .free_chunks_by_size
        .range((size, 0)..=(size, u64::MAX))
        .next()
        .copied();
    let (chunk_size, offset) = if let Some(exact_fit) = exact_fit {
        exact_fit
    } else {
        let &(largest_size, _) = state.free_chunks_by_size.last()?;
        if largest_size <= size {
            return None;
        }
        // The previous offset-ordered scan kept the lowest offset when
        // multiple worst-fit chunks had the same size. Preserve that policy.
        state
            .free_chunks_by_size
            .range((largest_size, 0)..=(largest_size, u64::MAX))
            .next()
            .copied()
            .expect("largest free chunk size should remain indexed")
    };

    let mut chosen = Extent {
        offset,
        len: chunk_size,
    };
    let allocation_len = u64::try_from(size).ok()?;
    let remainder_offset = chosen
        .offset
        .checked_add(allocation_len)
        .expect("free disk extent should not overflow");
    state.remove_free_chunk(chosen);
    state.free_bytes -= size;
    if chosen.len > size {
        state.insert_free_chunk(Extent {
            offset: remainder_offset,
            len: chosen.len - size,
        });
        chosen.len = size;
    }
    Some(chosen)
}

pub(crate) fn release_chunk(state: &mut AllocationState, extent: Extent) {
    let original_len = extent.len;
    let mut merged = extent;

    if let Some((&left_offset, &left_len)) = state
        .free_chunks_by_offset
        .range(..merged.offset)
        .next_back()
    {
        let left_end = left_offset
            .checked_add(u64::try_from(left_len).expect("extent length should fit u64"))
            .expect("free disk extent should not overflow");
        debug_assert!(left_end <= merged.offset);
        if left_end == merged.offset {
            state.remove_free_chunk(Extent {
                offset: left_offset,
                len: left_len,
            });
            merged.offset = left_offset;
            merged.len = merged
                .len
                .checked_add(left_len)
                .expect("merged disk extent should fit usize");
        }
    }

    if let Some((&right_offset, &right_len)) =
        state.free_chunks_by_offset.range(merged.offset..).next()
    {
        let merged_end = merged
            .offset
            .checked_add(u64::try_from(merged.len).expect("extent length should fit u64"))
            .expect("free disk extent should not overflow");
        debug_assert!(merged_end <= right_offset);
        if merged_end == right_offset {
            state.remove_free_chunk(Extent {
                offset: right_offset,
                len: right_len,
            });
            merged.len = merged
                .len
                .checked_add(right_len)
                .expect("merged disk extent should fit usize");
        }
    }

    state.insert_free_chunk(merged);
    state.free_bytes = state
        .free_bytes
        .checked_add(original_len)
        .expect("free disk bytes should fit usize");
}

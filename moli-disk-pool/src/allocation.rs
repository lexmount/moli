use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(crate) struct AllocationState {
    pub(crate) may_write: bool,
    pub(crate) file_tail: u64,
    pub(crate) free_chunks: BTreeMap<u64, usize>,
    pub(crate) free_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Extent {
    pub(crate) offset: u64,
    pub(crate) len: usize,
}

pub(crate) fn find_free_chunk(state: &mut AllocationState, size: usize) -> Option<Extent> {
    let mut chosen = None;
    let mut worst_fit_size = 0;
    for (&offset, &chunk_size) in &state.free_chunks {
        if chunk_size == size {
            chosen = Some(Extent {
                offset,
                len: chunk_size,
            });
            break;
        }
        if chunk_size > size && chunk_size > worst_fit_size {
            chosen = Some(Extent {
                offset,
                len: chunk_size,
            });
            worst_fit_size = chunk_size;
        }
    }

    let mut chosen = chosen?;
    state.free_chunks.remove(&chosen.offset);
    state.free_bytes -= size;
    if chosen.len > size {
        let remainder_offset = chosen
            .offset
            .checked_add(u64::try_from(size).ok()?)
            .expect("free disk extent should not overflow");
        let previous = state
            .free_chunks
            .insert(remainder_offset, chosen.len - size);
        debug_assert!(previous.is_none());
        chosen.len = size;
    }
    Some(chosen)
}

pub(crate) fn release_chunk(state: &mut AllocationState, extent: Extent) {
    let original_len = extent.len;
    let mut merged = extent;

    if let Some((&left_offset, &left_len)) = state.free_chunks.range(..merged.offset).next_back() {
        let left_end = left_offset
            .checked_add(u64::try_from(left_len).expect("extent length should fit u64"))
            .expect("free disk extent should not overflow");
        debug_assert!(left_end <= merged.offset);
        if left_end == merged.offset {
            state.free_chunks.remove(&left_offset);
            merged.offset = left_offset;
            merged.len = merged
                .len
                .checked_add(left_len)
                .expect("merged disk extent should fit usize");
        }
    }

    if let Some((&right_offset, &right_len)) = state.free_chunks.range(merged.offset..).next() {
        let merged_end = merged
            .offset
            .checked_add(u64::try_from(merged.len).expect("extent length should fit u64"))
            .expect("free disk extent should not overflow");
        debug_assert!(merged_end <= right_offset);
        if merged_end == right_offset {
            state.free_chunks.remove(&right_offset);
            merged.len = merged
                .len
                .checked_add(right_len)
                .expect("merged disk extent should fit usize");
        }
    }

    let previous = state.free_chunks.insert(merged.offset, merged.len);
    debug_assert!(previous.is_none());
    state.free_bytes = state
        .free_bytes
        .checked_add(original_len)
        .expect("free disk bytes should fit usize");
}

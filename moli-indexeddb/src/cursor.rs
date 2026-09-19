use std::{cmp::Ordering as CmpOrdering, str::FromStr};

use crate::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum CursorDirection {
    Next,
    NextUnique,
    Prev,
    PrevUnique,
}

impl CursorDirection {
    pub const fn default_next() -> Self {
        Self::Next
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub const fn is_reverse(self) -> bool {
        matches!(self, Self::Prev | Self::PrevUnique)
    }

    pub const fn is_unique(self) -> bool {
        matches!(self, Self::NextUnique | Self::PrevUnique)
    }
}

pub fn apply_cursor_direction_by_key<T>(
    mut entries: Vec<T>,
    direction: CursorDirection,
    mut key_for: impl FnMut(&T) -> &Key,
) -> Vec<T> {
    // Unique iteration always selects the lowest primary key for each index
    // key. Deduplicate ascending entries before reversing the distinct keys.
    if direction.is_unique() {
        entries.dedup_by(|right, left| key_for(left) == key_for(right));
    }
    if direction.is_reverse() {
        entries.reverse();
    }
    entries
}

pub fn apply_collection_direction<T>(mut entries: Vec<T>, direction: CursorDirection) -> Vec<T> {
    if direction.is_reverse() {
        entries.reverse();
    }
    entries
}

pub fn compare_cursor_direction(
    direction: CursorDirection,
    candidate: &Key,
    target: &Key,
) -> CmpOrdering {
    let ordering = candidate.cmp(target);
    if direction.is_reverse() {
        ordering.reverse()
    } else {
        ordering
    }
}

pub fn compare_cursor_tuple_direction(
    direction: CursorDirection,
    candidate_key: &Key,
    candidate_primary_key: &Key,
    target_key: &Key,
    target_primary_key: &Key,
) -> CmpOrdering {
    let ordering = candidate_key
        .cmp(target_key)
        .then_with(|| candidate_primary_key.cmp(target_primary_key));
    if direction.is_reverse() {
        ordering.reverse()
    } else {
        ordering
    }
}

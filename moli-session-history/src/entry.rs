use crate::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};

/// Browser supplied position. It includes steps outside the current renderer's
/// Navigation API view, including entries in other Documents and origins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionHistoryPosition {
    pub(crate) index: usize,
    pub(crate) length: usize,
}

impl SessionHistoryPosition {
    pub const INITIAL: Self = Self {
        index: 0,
        length: 1,
    };

    pub fn new(index: usize, length: usize) -> Option<Self> {
        (index < length).then_some(Self { index, length })
    }

    pub fn index(self) -> usize {
        self.index
    }

    pub fn length(self) -> usize {
        self.length
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryEntry {
    pub key: NavigationHistoryEntryKey,
    pub document: NavigationHistoryDocumentId,
}

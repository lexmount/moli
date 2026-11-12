// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Model for font data.

use crate::AtomicCounter;
use core::sync::atomic::Ordering;
use linebender_resource_handle::Blob;
#[cfg(feature = "std")]
use {
    hashbrown::HashMap,
    std::{path::Path, sync::Arc},
};

/// Unique identifier for a font source.
///
/// See [`SourceInfo`].
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct SourceId(u64);

impl SourceId {
    /// Creates a new unique identifier.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static ID_COUNTER: AtomicCounter = AtomicCounter::new(1);
        #[allow(clippy::useless_conversion, reason = "Not useless on 32-bit platforms")]
        Self(ID_COUNTER.fetch_add(1, Ordering::Relaxed).into())
    }

    /// Returns the underlying integer value.
    pub fn to_u64(self) -> u64 {
        self.0
    }
}

/// Handle that associates font data with a unique identifier.
#[derive(Clone, Debug)]
pub struct SourceInfo {
    pub id: SourceId,
    pub kind: SourceKind,
}

impl SourceInfo {
    /// Creates a new source with the given identifier and kind.
    pub fn new(id: SourceId, kind: SourceKind) -> Self {
        Self { id, kind }
    }

    /// Returns the unique identifier for the source.
    pub fn id(&self) -> SourceId {
        self.id
    }

    /// Returns the kind of the source.
    pub fn kind(&self) -> &SourceKind {
        &self.kind
    }
}

/// Font data that is either a path to the font file or shared data in memory.
///
/// See [`SourceInfo`].
#[derive(Clone)]
pub enum SourceKind {
    /// Shared data containing the content of the font file.
    Memory(Blob<u8>),
    #[cfg(feature = "std")]
    /// Path to a font file.
    Path(Arc<Path>),
}

impl core::fmt::Debug for SourceKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(feature = "std")]
            Self::Path(path) => {
                write!(f, "Path({path:?})")
            }
            Self::Memory(_) => {
                write!(f, "Data([..])")
            }
        }
    }
}

#[cfg(feature = "std")]
/// Map for deduplicating font data file paths.
#[derive(Default)]
pub struct SourcePathMap {
    map: HashMap<Arc<Path>, SourceInfo>,
}

#[cfg(feature = "std")]
impl SourcePathMap {
    /// Converts a path string into a font data object, creating it if it
    /// doesn't already exist.
    pub fn get_or_insert(&mut self, path: &Path) -> SourceInfo {
        if let Some(source) = self.map.get(path) {
            source.clone()
        } else {
            let path: Arc<Path> = path.into();
            let source = SourceInfo::new(SourceId::new(), SourceKind::Path(path.clone()));
            self.map.insert(path, source.clone());
            source
        }
    }
}

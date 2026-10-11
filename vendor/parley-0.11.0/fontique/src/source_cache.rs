// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cache for font data.

#[cfg(feature = "std")]
use super::source::SourceId;
use super::source::{SourceInfo, SourceKind};
#[cfg(feature = "std")]
use hashbrown::HashMap;
use linebender_resource_handle::Blob;
#[cfg(feature = "std")]
use linebender_resource_handle::WeakBlob;
#[cfg(feature = "std")]
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// Options for a [source cache].
///
/// [source cache]: SourceCache
#[derive(Copy, Clone, Default, Debug)]
pub struct SourceCacheOptions {
    #[cfg(feature = "std")]
    /// If true, the source cache will use a secondary shared cache
    /// guaranteeing that all clones will use the same backing store.
    ///
    /// This is useful for ensuring that only one copy of font data is
    /// loaded into memory in multi-threaded scenarios.
    ///
    /// The default value is `false`.
    pub shared: bool,
}

/// Cache for font data loaded from the file system.
#[derive(Clone, Default)]
pub struct SourceCache {
    #[cfg(feature = "std")]
    cache: HashMap<SourceId, Entry<Blob<u8>>>,
    #[cfg(feature = "std")]
    serial: u64,
    #[cfg(feature = "std")]
    shared: Option<Arc<Mutex<Shared>>>,
}

impl SourceCache {
    /// Creates an empty cache with the given [options].
    ///
    /// [options]: SourceCacheOptions
    #[cfg_attr(not(feature = "std"), allow(unused))]
    pub fn new(options: SourceCacheOptions) -> Self {
        #[cfg(feature = "std")]
        if options.shared {
            return Self {
                cache: HashMap::default(),
                serial: 0,
                shared: Some(Arc::new(Mutex::new(Shared::default()))),
            };
        }
        Self::default()
    }

    /// Creates an empty cache that is suitable for multi-threaded use.
    ///
    /// A cache created with this function maintains a synchronized internal
    /// store that is shared among all clones.
    ///
    /// This is the same as calling [`SourceCache::new`] with an options
    /// struct where `shared = true`.
    #[cfg(feature = "std")]
    pub fn new_shared() -> Self {
        Self {
            cache: HashMap::default(),
            serial: 0,
            shared: Some(Arc::new(Mutex::new(Shared::default()))),
        }
    }

    /// Turns an unshared cache into a shared cache that can used to ensure that fonts only get loaded once
    /// even when they are loaded across multiple threads.
    #[cfg(feature = "std")]
    pub fn make_shared(&mut self) {
        if self.shared.is_none() {
            self.shared = Some(Arc::new(Mutex::new(Shared::from_local(&self.cache))));
        }
    }

    /// Returns the [blob] for the given font data, attempting to load
    /// it from the file system if not already present.
    ///
    /// Returns `None` if loading failed.
    ///
    /// [blob]: Blob
    pub fn get(&mut self, source: &SourceInfo) -> Option<Blob<u8>> {
        match &source.kind {
            SourceKind::Memory(memory) => Some(memory.clone()),
            #[cfg(feature = "std")]
            SourceKind::Path(path) => {
                use hashbrown::hash_map::Entry as HashEntry;
                match self.cache.entry(source.id()) {
                    HashEntry::Vacant(vacant) => {
                        if let Some(mut shared) =
                            self.shared.as_ref().and_then(|shared| shared.lock().ok())
                        {
                            // If we have a backing cache, try to load it there first
                            // and then propagate the result here.
                            if let Some(blob) = shared.get(source.id(), path) {
                                vacant.insert(Entry::Loaded(EntryData {
                                    font_data: blob.clone(),
                                    serial: self.serial,
                                }));
                                Some(blob)
                            } else {
                                vacant.insert(Entry::Failed);
                                None
                            }
                        } else {
                            // Otherwise, load it ourselves.
                            if let Some(blob) = load_blob(path) {
                                vacant.insert(Entry::Loaded(EntryData {
                                    font_data: blob.clone(),
                                    serial: self.serial,
                                }));
                                Some(blob)
                            } else {
                                vacant.insert(Entry::Failed);
                                None
                            }
                        }
                    }
                    HashEntry::Occupied(mut occupied) => {
                        let entry = occupied.get_mut();
                        match entry {
                            Entry::Loaded(data) => {
                                data.serial = self.serial;
                                Some(data.font_data.clone())
                            }
                            Entry::Failed => None,
                        }
                    }
                }
            }
        }
    }

    /// Removes all cached blobs that have not been accessed in the last
    /// `max_age` times `prune` has been called.
    #[cfg_attr(not(feature = "std"), allow(unused))]
    pub fn prune(&mut self, max_age: u64, prune_failed: bool) {
        #[cfg(feature = "std")]
        {
            self.cache.retain(|_, entry| match entry {
                Entry::Failed => !prune_failed,
                Entry::Loaded(data) => self.serial.saturating_sub(data.serial) < max_age,
            });
            self.serial = self.serial.saturating_add(1);
        }
    }
}

/// Shared backing store for a font data cache.
#[cfg(feature = "std")]
#[derive(Default)]
struct Shared {
    cache: HashMap<SourceId, Entry<WeakBlob<u8>>>,
}

#[cfg(feature = "std")]
impl Shared {
    /// Bootstrap a shared cache from a local one
    fn from_local(unshared: &HashMap<SourceId, Entry<Blob<u8>>>) -> Self {
        let shared_cache: HashMap<SourceId, Entry<WeakBlob<u8>>> = unshared
            .iter()
            .map(|(key, value)| (*key, value.into()))
            .collect();
        Self {
            cache: shared_cache,
        }
    }

    pub fn get(&mut self, id: SourceId, path: &Path) -> Option<Blob<u8>> {
        use hashbrown::hash_map::Entry as HashEntry;
        match self.cache.entry(id) {
            HashEntry::Vacant(vacant) => {
                if let Some(blob) = load_blob(path) {
                    vacant.insert(Entry::Loaded(EntryData {
                        font_data: blob.downgrade(),
                        serial: 0,
                    }));
                    Some(blob)
                } else {
                    vacant.insert(Entry::Failed);
                    None
                }
            }
            HashEntry::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                match entry {
                    Entry::Loaded(data) => {
                        if let Some(blob) = data.font_data.upgrade() {
                            // The weak ref is still valid.
                            Some(blob)
                        } else if let Some(blob) = load_blob(path) {
                            // Otherwise, try to reload it.
                            data.font_data = blob.downgrade();
                            Some(blob)
                        } else {
                            // We failed for some reason.. don't try again.
                            *entry = Entry::Failed;
                            None
                        }
                    }
                    Entry::Failed => None,
                }
            }
        }
    }
}

#[cfg(feature = "std")]
#[derive(Clone, Default)]
enum Entry<T> {
    Loaded(EntryData<T>),
    #[default]
    Failed,
}

#[cfg(feature = "std")]
#[derive(Clone)]
struct EntryData<T> {
    font_data: T,
    serial: u64,
}

#[cfg(feature = "std")]
impl<T> From<&Entry<Blob<T>>> for Entry<WeakBlob<T>> {
    fn from(value: &Entry<Blob<T>>) -> Self {
        match value {
            Entry::Loaded(entry_data) => Self::Loaded(EntryData {
                font_data: entry_data.font_data.downgrade(),
                serial: entry_data.serial,
            }),
            Entry::Failed => Self::Failed,
        }
    }
}

#[cfg(feature = "std")]
pub(crate) fn load_blob(path: &Path) -> Option<Blob<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mapped = unsafe { memmap2::Mmap::map(&file).ok()? };
    Some(Blob::new(Arc::new(mapped)))
}

use std::{
    collections::BTreeMap,
    collections::{HashMap, VecDeque},
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use anyhow::{Context, Result};
use moli_storage_key::{
    MoliStorageKey, storage_key_for_origin_and_top_level_site, storage_key_prefix_for_origin,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use typed_num::Num;

use crate::util::{
    string_from_utf16_units_lossy, utf16_units, utf16_units_contain_unpaired_surrogate,
};

const WEB_STORAGE_QUOTA_BYTES: usize = 5 * 1024 * 1024;
type WebStorageJsonVersion = Num<1>;
type SerializedWebStorageAreas = BTreeMap<String, Arc<RawValue>>;

pub fn web_storage_partitioned_area_key(origin: &str, top_level_site: &str) -> String {
    storage_key_for_origin_and_top_level_site(origin, top_level_site)
}

pub fn web_storage_area_key_for_storage_key(storage_key: &MoliStorageKey) -> String {
    storage_key.serialized_storage_key()
}

fn web_storage_area_key_prefix_for_origin(origin: &str) -> String {
    storage_key_prefix_for_origin(origin)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WebStorageString {
    units: Vec<u16>,
}

#[derive(Debug, Default, Clone)]
struct MemoryWebStorageArea {
    values: HashMap<WebStorageString, WebStorageString>,
    size: usize,
}

#[derive(Debug, Default, Clone)]
struct MemoryWebStorageBackend {
    origins: HashMap<String, MemoryWebStorageArea>,
}

#[derive(Debug, Clone)]
struct JsonWebStorageBackend {
    path: PathBuf,
    memory: MemoryWebStorageBackend,
    serialized_areas: SerializedWebStorageAreas,
}

enum WebStorageBackend {
    Memory(MemoryWebStorageBackend),
    Json(JsonWebStorageBackend),
}

pub struct WebStorageStore {
    backend: WebStorageBackend,
    mutation_subscribers: Vec<WebStorageMutationSubscriber>,
}

pub type SharedWebStorageStore = Arc<Mutex<WebStorageStore>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebStorageMutation {
    ItemAdded {
        area_key: String,
        key: WebStorageString,
        value: WebStorageString,
    },
    ItemUpdated {
        area_key: String,
        key: WebStorageString,
        old_value: WebStorageString,
        new_value: WebStorageString,
    },
    ItemRemoved {
        area_key: String,
        key: WebStorageString,
        old_value: WebStorageString,
    },
    ItemsCleared {
        area_key: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebStorageAreaKind {
    Local,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebStorageMutationRecord {
    pub area_kind: WebStorageAreaKind,
    pub mutation: WebStorageMutation,
}

struct WebStorageMutationSubscriber {
    area_kind: WebStorageAreaKind,
    queue: Weak<Mutex<VecDeque<WebStorageMutationRecord>>>,
}

#[derive(Clone)]
pub struct WebStorageMutationSubscription {
    queue: Arc<Mutex<VecDeque<WebStorageMutationRecord>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebStorageMutationError {
    QuotaExceeded,
    Persistence(String),
}

impl fmt::Display for WebStorageMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuotaExceeded => f.write_str("localStorage quota exceeded"),
            Self::Persistence(message) => write!(f, "localStorage persistence failed: {message}"),
        }
    }
}

impl std::error::Error for WebStorageMutationError {}

pub fn new_shared_web_storage_store() -> SharedWebStorageStore {
    Arc::new(Mutex::new(WebStorageStore::default()))
}

pub fn deep_clone_shared_web_storage_store(
    source: &SharedWebStorageStore,
) -> SharedWebStorageStore {
    let memory = match &source.lock().backend {
        WebStorageBackend::Memory(memory) => memory.clone(),
        WebStorageBackend::Json(json) => json.memory.clone(),
    };
    Arc::new(Mutex::new(WebStorageStore {
        backend: WebStorageBackend::Memory(memory),
        mutation_subscribers: Vec::new(),
    }))
}

pub fn new_shared_json_web_storage_store(path: impl AsRef<Path>) -> Result<SharedWebStorageStore> {
    let backend = JsonWebStorageBackend::open(path.as_ref())?;
    Ok(Arc::new(Mutex::new(WebStorageStore {
        backend: WebStorageBackend::Json(backend),
        mutation_subscribers: Vec::new(),
    })))
}

impl Default for WebStorageStore {
    fn default() -> Self {
        Self {
            backend: WebStorageBackend::Memory(MemoryWebStorageBackend::default()),
            mutation_subscribers: Vec::new(),
        }
    }
}

impl fmt::Debug for WebStorageStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.backend {
            WebStorageBackend::Memory(memory) => f
                .debug_struct("WebStorageStore")
                .field("backend", &"memory")
                .field("origins", &memory.origins.keys().collect::<Vec<_>>())
                .field("mutation_subscribers", &self.mutation_subscribers.len())
                .finish(),
            WebStorageBackend::Json(json) => f
                .debug_struct("WebStorageStore")
                .field("backend", &"json")
                .field("path", &json.path)
                .field("origins", &json.memory.origins.keys().collect::<Vec<_>>())
                .field("mutation_subscribers", &self.mutation_subscribers.len())
                .finish(),
        }
    }
}

impl fmt::Debug for WebStorageMutationSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebStorageMutationSubscription")
            .field("pending", &self.queue.lock().len())
            .finish()
    }
}

impl PartialEq for WebStorageMutationSubscription {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.queue, &other.queue)
    }
}

impl Eq for WebStorageMutationSubscription {}

impl WebStorageMutationSubscription {
    pub fn drain(&self) -> Vec<WebStorageMutationRecord> {
        self.queue.lock().drain(..).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.lock().is_empty()
    }
}

impl WebStorageString {
    pub fn from_utf16_units(units: impl Into<Vec<u16>>) -> Self {
        Self {
            units: units.into(),
        }
    }

    pub fn from_utf8(value: &str) -> Self {
        Self::from_utf16_units(utf16_units(value))
    }

    pub fn as_units(&self) -> &[u16] {
        &self.units
    }

    pub fn to_string_lossy(&self) -> String {
        string_from_utf16_units_lossy(&self.units)
    }

    fn contains_unpaired_surrogate(&self) -> bool {
        utf16_units_contain_unpaired_surrogate(&self.units)
    }

    fn usage_bytes(&self) -> usize {
        if self.contains_unpaired_surrogate() {
            return self.units.len().saturating_mul(2);
        }
        String::from_utf16(&self.units)
            .map_or_else(|_| self.units.len().saturating_mul(2), |value| value.len())
    }
}

impl From<&str> for WebStorageString {
    fn from(value: &str) -> Self {
        Self::from_utf8(value)
    }
}

impl MemoryWebStorageArea {
    fn get_item(&self, key: &str) -> Option<String> {
        self.get_item_utf16(utf16_units(key).as_slice())
            .map(|value| string_from_utf16_units_lossy(&value))
    }

    fn get_item_utf16(&self, key: &[u16]) -> Option<Vec<u16>> {
        self.values
            .get(&WebStorageString::from_utf16_units(key.to_vec()))
            .map(|value| value.as_units().to_vec())
    }

    fn set_item_utf16(&mut self, key: &[u16], value: &[u16]) -> bool {
        let key = WebStorageString::from_utf16_units(key.to_vec());
        let value = WebStorageString::from_utf16_units(value.to_vec());
        let next_size = updated_storage_size(self.size, self.values.get(&key), value.usage_bytes());
        if next_size > WEB_STORAGE_QUOTA_BYTES {
            return false;
        }
        self.size = next_size;
        self.values.insert(key, value);
        true
    }

    fn remove_item_utf16(&mut self, key: &[u16]) -> bool {
        let Some(previous) = self
            .values
            .remove(&WebStorageString::from_utf16_units(key.to_vec()))
        else {
            return false;
        };
        self.size = self.size.saturating_sub(previous.usage_bytes());
        true
    }

    fn key(&self, index: usize) -> Option<String> {
        self.key_utf16(index)
            .map(|key| string_from_utf16_units_lossy(&key))
    }

    fn key_utf16(&self, index: usize) -> Option<Vec<u16>> {
        let mut keys = self.values.keys().collect::<Vec<_>>();
        keys.sort();
        keys.get(index).map(|key| key.as_units().to_vec())
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn contains_key(&self, key: &str) -> bool {
        self.contains_key_utf16(&utf16_units(key))
    }

    fn contains_key_utf16(&self, key: &[u16]) -> bool {
        self.values
            .contains_key(&WebStorageString::from_utf16_units(key.to_vec()))
    }

    fn usage_bytes(&self) -> usize {
        self.size
    }

    fn sorted_keys(&self) -> Vec<String> {
        self.sorted_keys_utf16()
            .into_iter()
            .map(|key| string_from_utf16_units_lossy(&key))
            .collect()
    }

    fn sorted_keys_utf16(&self) -> Vec<Vec<u16>> {
        let mut keys = self.values.keys().collect::<Vec<_>>();
        keys.sort();
        keys.into_iter()
            .map(|key| key.as_units().to_vec())
            .collect()
    }
}

fn updated_storage_size(
    current_size: usize,
    previous_value: Option<&WebStorageString>,
    new_value_size: usize,
) -> usize {
    current_size
        .saturating_sub(previous_value.map_or(0, WebStorageString::usage_bytes))
        .saturating_add(new_value_size)
}

impl WebStorageStore {
    pub fn subscribe_mutations(
        &mut self,
        area_kind: WebStorageAreaKind,
    ) -> WebStorageMutationSubscription {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let subscription = WebStorageMutationSubscription { queue };
        self.add_mutation_subscription(area_kind, &subscription);
        subscription
    }

    pub fn add_mutation_subscription(
        &mut self,
        area_kind: WebStorageAreaKind,
        subscription: &WebStorageMutationSubscription,
    ) {
        self.mutation_subscribers.retain(|subscriber| {
            let Some(queue) = subscriber.queue.upgrade() else {
                return false;
            };
            !(subscriber.area_kind == area_kind && Arc::ptr_eq(&queue, &subscription.queue))
        });
        self.mutation_subscribers
            .push(WebStorageMutationSubscriber {
                area_kind,
                queue: Arc::downgrade(&subscription.queue),
            });
    }

    pub fn get_item(&mut self, origin: &str, key: &str) -> Option<String> {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.get_item(origin, key),
            WebStorageBackend::Json(json) => json.memory.get_item(origin, key),
        }
    }

    pub fn get_item_utf16(&mut self, origin: &str, key: &[u16]) -> Option<Vec<u16>> {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.get_item_utf16(origin, key),
            WebStorageBackend::Json(json) => json.memory.get_item_utf16(origin, key),
        }
    }

    pub fn set_item(&mut self, origin: &str, key: &str, value: &str) -> bool {
        self.try_set_item(origin, key, value).unwrap_or(false)
    }

    pub fn try_set_item(
        &mut self,
        origin: &str,
        key: &str,
        value: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_set_item_utf16(origin, &utf16_units(key), &utf16_units(value))
    }

    pub fn set_item_utf16(&mut self, origin: &str, key: &[u16], value: &[u16]) -> bool {
        self.try_set_item_utf16(origin, key, value).unwrap_or(false)
    }

    pub fn try_set_item_utf16(
        &mut self,
        origin: &str,
        key: &[u16],
        value: &[u16],
    ) -> std::result::Result<bool, WebStorageMutationError> {
        let previous = self.get_item_utf16(origin, key);
        if previous.as_deref() == Some(value) {
            return Ok(true);
        }
        let updated = match &mut self.backend {
            WebStorageBackend::Memory(memory) => {
                if memory.set_item_utf16(origin, key, value) {
                    Ok(true)
                } else {
                    Err(WebStorageMutationError::QuotaExceeded)
                }
            }
            WebStorageBackend::Json(json) => json.update(&[origin], |memory| {
                memory.set_item_utf16(origin, key, value)
            }),
        }?;
        if updated {
            let key = WebStorageString::from_utf16_units(key.to_vec());
            let value = WebStorageString::from_utf16_units(value.to_vec());
            let mutation = match previous {
                Some(previous) => WebStorageMutation::ItemUpdated {
                    area_key: origin.to_owned(),
                    key,
                    old_value: WebStorageString::from_utf16_units(previous),
                    new_value: value,
                },
                None => WebStorageMutation::ItemAdded {
                    area_key: origin.to_owned(),
                    key,
                    value,
                },
            };
            self.publish_mutation(mutation);
        }
        Ok(updated)
    }

    pub fn remove_item(&mut self, origin: &str, key: &str) -> bool {
        self.try_remove_item(origin, key).unwrap_or(false)
    }

    pub fn try_remove_item(
        &mut self,
        origin: &str,
        key: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_remove_item_utf16(origin, &utf16_units(key))
    }

    pub fn remove_item_utf16(&mut self, origin: &str, key: &[u16]) -> bool {
        self.try_remove_item_utf16(origin, key).unwrap_or(false)
    }

    pub fn try_remove_item_utf16(
        &mut self,
        origin: &str,
        key: &[u16],
    ) -> std::result::Result<bool, WebStorageMutationError> {
        let Some(previous) = self.get_item_utf16(origin, key) else {
            return Ok(false);
        };
        let removed = match &mut self.backend {
            WebStorageBackend::Memory(memory) => Ok(memory.remove_item_utf16(origin, key)),
            WebStorageBackend::Json(json) => {
                json.update(&[origin], |memory| memory.remove_item_utf16(origin, key))
            }
        }?;
        if removed {
            self.publish_mutation(WebStorageMutation::ItemRemoved {
                area_key: origin.to_owned(),
                key: WebStorageString::from_utf16_units(key.to_vec()),
                old_value: WebStorageString::from_utf16_units(previous),
            });
        }
        Ok(removed)
    }

    pub fn clear(&mut self, origin: &str) {
        let _ = self.try_clear(origin);
    }

    pub fn try_clear(
        &mut self,
        origin: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        let had_items = self.len(origin) != 0;
        let cleared = match &mut self.backend {
            WebStorageBackend::Memory(memory) => Ok(memory.clear_origin(origin)),
            WebStorageBackend::Json(json) => {
                json.update(&[origin], |memory| memory.clear_origin(origin))
            }
        }?;
        if cleared && had_items {
            self.publish_mutation(WebStorageMutation::ItemsCleared {
                area_key: origin.to_owned(),
            });
        }
        Ok(cleared)
    }

    pub fn key(&mut self, origin: &str, index: usize) -> Option<String> {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.key(origin, index),
            WebStorageBackend::Json(json) => json.memory.key(origin, index),
        }
    }

    pub fn key_utf16(&mut self, origin: &str, index: usize) -> Option<Vec<u16>> {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.key_utf16(origin, index),
            WebStorageBackend::Json(json) => json.memory.key_utf16(origin, index),
        }
    }

    pub fn len(&mut self, origin: &str) -> usize {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.len(origin),
            WebStorageBackend::Json(json) => json.memory.len(origin),
        }
    }

    pub fn contains_key(&mut self, origin: &str, key: &str) -> bool {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.contains_key(origin, key),
            WebStorageBackend::Json(json) => json.memory.contains_key(origin, key),
        }
    }

    pub fn contains_key_utf16(&mut self, origin: &str, key: &[u16]) -> bool {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.contains_key_utf16(origin, key),
            WebStorageBackend::Json(json) => json.memory.contains_key_utf16(origin, key),
        }
    }

    pub fn sorted_keys(&mut self, origin: &str) -> Vec<String> {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.sorted_keys(origin),
            WebStorageBackend::Json(json) => json.memory.sorted_keys(origin),
        }
    }

    pub fn sorted_keys_utf16(&mut self, origin: &str) -> Vec<Vec<u16>> {
        match &mut self.backend {
            WebStorageBackend::Memory(memory) => memory.sorted_keys_utf16(origin),
            WebStorageBackend::Json(json) => json.memory.sorted_keys_utf16(origin),
        }
    }

    pub fn usage_bytes(&self, origin: &str) -> usize {
        match &self.backend {
            WebStorageBackend::Memory(memory) => memory.usage_bytes(origin),
            WebStorageBackend::Json(json) => json.memory.usage_bytes(origin),
        }
    }

    pub fn usage_bytes_for_origin_areas(&self, origin: &str) -> usize {
        match &self.backend {
            WebStorageBackend::Memory(memory) => memory.usage_bytes_for_origin_areas(origin),
            WebStorageBackend::Json(json) => json.memory.usage_bytes_for_origin_areas(origin),
        }
    }

    pub fn clear_origin(&mut self, origin: &str) {
        self.clear(origin);
    }

    pub fn try_clear_origin(
        &mut self,
        origin: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_clear(origin)
    }

    pub fn clear_origin_areas(&mut self, origin: &str) {
        let _ = self.try_clear_origin_areas(origin);
    }

    pub fn try_clear_origin_areas(
        &mut self,
        origin: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        let (area_keys, nonempty_area_keys) = match &self.backend {
            WebStorageBackend::Memory(memory) => (
                memory.area_keys_for_origin(origin),
                memory.nonempty_area_keys_for_origin(origin),
            ),
            WebStorageBackend::Json(json) => (
                json.memory.area_keys_for_origin(origin),
                json.memory.nonempty_area_keys_for_origin(origin),
            ),
        };
        if area_keys.is_empty() {
            return Ok(false);
        }
        let cleared = match &mut self.backend {
            WebStorageBackend::Memory(memory) => Ok(memory.clear_origin_areas(origin)),
            WebStorageBackend::Json(json) => {
                let affected_areas = area_keys.iter().map(String::as_str).collect::<Vec<_>>();
                json.update(&affected_areas, |memory| memory.clear_origin_areas(origin))
            }
        }?;
        if cleared {
            for area_key in nonempty_area_keys {
                self.publish_mutation(WebStorageMutation::ItemsCleared { area_key });
            }
        }
        Ok(cleared)
    }

    fn publish_mutation(&mut self, mutation: WebStorageMutation) {
        self.mutation_subscribers.retain(|subscriber| {
            let Some(queue) = subscriber.queue.upgrade() else {
                return false;
            };
            queue.lock().push_back(WebStorageMutationRecord {
                area_kind: subscriber.area_kind,
                mutation: mutation.clone(),
            });
            true
        });
    }
}

impl MemoryWebStorageBackend {
    fn get_item(&self, origin: &str, key: &str) -> Option<String> {
        self.origins.get(origin).and_then(|area| area.get_item(key))
    }

    fn get_item_utf16(&self, origin: &str, key: &[u16]) -> Option<Vec<u16>> {
        self.origins
            .get(origin)
            .and_then(|area| area.get_item_utf16(key))
    }

    fn set_item_utf16(&mut self, origin: &str, key: &[u16], value: &[u16]) -> bool {
        self.origins
            .entry(origin.to_owned())
            .or_default()
            .set_item_utf16(key, value)
    }

    fn remove_item_utf16(&mut self, origin: &str, key: &[u16]) -> bool {
        let (removed, clear_origin) = {
            let Some(area) = self.origins.get_mut(origin) else {
                return false;
            };
            let removed = area.remove_item_utf16(key);
            (removed, area.len() == 0)
        };
        if clear_origin {
            self.origins.remove(origin);
        }
        removed
    }

    fn clear_origin(&mut self, origin: &str) -> bool {
        self.origins.remove(origin).is_some()
    }

    fn key(&self, origin: &str, index: usize) -> Option<String> {
        self.origins.get(origin).and_then(|area| area.key(index))
    }

    fn key_utf16(&self, origin: &str, index: usize) -> Option<Vec<u16>> {
        self.origins
            .get(origin)
            .and_then(|area| area.key_utf16(index))
    }

    fn len(&self, origin: &str) -> usize {
        self.origins
            .get(origin)
            .map_or(0, MemoryWebStorageArea::len)
    }

    fn contains_key(&self, origin: &str, key: &str) -> bool {
        self.origins
            .get(origin)
            .is_some_and(|area| area.contains_key(key))
    }

    fn contains_key_utf16(&self, origin: &str, key: &[u16]) -> bool {
        self.origins
            .get(origin)
            .is_some_and(|area| area.contains_key_utf16(key))
    }

    fn sorted_keys(&self, origin: &str) -> Vec<String> {
        self.origins
            .get(origin)
            .map_or_else(Vec::new, MemoryWebStorageArea::sorted_keys)
    }

    fn sorted_keys_utf16(&self, origin: &str) -> Vec<Vec<u16>> {
        self.origins
            .get(origin)
            .map_or_else(Vec::new, MemoryWebStorageArea::sorted_keys_utf16)
    }

    fn usage_bytes(&self, origin: &str) -> usize {
        self.origins
            .get(origin)
            .map_or(0, MemoryWebStorageArea::usage_bytes)
    }

    fn usage_bytes_for_origin_areas(&self, origin: &str) -> usize {
        let area_prefix = web_storage_area_key_prefix_for_origin(origin);
        self.origins
            .iter()
            .filter(|(area_key, _)| area_key.starts_with(&area_prefix))
            .map(|(_, area)| area.usage_bytes())
            .sum()
    }

    fn clear_origin_areas(&mut self, origin: &str) -> bool {
        let area_keys = self.area_keys_for_origin(origin);
        let removed = !area_keys.is_empty();
        for area_key in area_keys {
            self.origins.remove(&area_key);
        }
        removed
    }

    fn area_keys_for_origin(&self, origin: &str) -> Vec<String> {
        let area_prefix = web_storage_area_key_prefix_for_origin(origin);
        self.origins
            .keys()
            .filter(|area_key| area_key.starts_with(&area_prefix))
            .cloned()
            .collect()
    }

    fn nonempty_area_keys_for_origin(&self, origin: &str) -> Vec<String> {
        self.area_keys_for_origin(origin)
            .into_iter()
            .filter(|area_key| self.len(area_key) > 0)
            .collect()
    }
}

impl JsonWebStorageBackend {
    fn open(path: &Path) -> Result<Self> {
        let (memory, serialized_areas) = load_json_web_storage(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            memory,
            serialized_areas,
        })
    }

    fn update(
        &mut self,
        affected_areas: &[&str],
        update: impl FnOnce(&mut MemoryWebStorageBackend) -> bool,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        // A Chrome import can contain thousands of unrelated storage areas.
        // Stage only the affected areas, retaining rollback on persistence
        // failure without cloning and re-encoding the entire profile per key.
        let mut next = MemoryWebStorageBackend {
            origins: affected_areas
                .iter()
                .filter_map(|origin| {
                    self.memory
                        .origins
                        .get(*origin)
                        .map(|area| ((*origin).to_owned(), area.clone()))
                })
                .collect(),
        };
        if !update(&mut next) {
            return Ok(false);
        }
        let mut serialized_areas = self.serialized_areas.clone();
        for origin in affected_areas {
            if let Some(area) = next.origins.get(*origin) {
                let serialized =
                    serde_json::value::to_raw_value(&JsonWebStorageAreaFile::from(area))
                        .map_err(|error| WebStorageMutationError::Persistence(error.to_string()))?;
                serialized_areas.insert((*origin).to_owned(), Arc::from(serialized));
            } else {
                serialized_areas.remove(*origin);
            }
        }
        persist_json_web_storage(&self.path, &serialized_areas)
            .map_err(|error| WebStorageMutationError::Persistence(error.to_string()))?;
        for origin in affected_areas {
            if let Some(area) = next.origins.remove(*origin) {
                self.memory.origins.insert((*origin).to_owned(), area);
            } else {
                self.memory.origins.remove(*origin);
            }
        }
        self.serialized_areas = serialized_areas;
        Ok(true)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonWebStorageFile<Area = JsonWebStorageAreaFile> {
    version: WebStorageJsonVersion,
    origins: BTreeMap<String, Area>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonWebStorageAreaFile {
    entries: Vec<JsonWebStorageEntry>,
}

impl From<&MemoryWebStorageArea> for JsonWebStorageAreaFile {
    fn from(area: &MemoryWebStorageArea) -> Self {
        let mut pairs = area.values.iter().collect::<Vec<_>>();
        pairs.sort_by_key(|(key, _)| *key);
        Self {
            entries: pairs
                .into_iter()
                .map(|(key, value)| JsonWebStorageEntry {
                    key: JsonWebStorageDomString::from_web_storage_string(key),
                    value: JsonWebStorageDomString::from_web_storage_string(value),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonWebStorageEntry {
    key: JsonWebStorageDomString,
    value: JsonWebStorageDomString,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum JsonWebStorageDomString {
    Text(String),
    Utf16 { utf16: Vec<u16> },
}

impl JsonWebStorageDomString {
    fn from_web_storage_string(value: &WebStorageString) -> Self {
        if value.contains_unpaired_surrogate() {
            Self::Utf16 {
                utf16: value.as_units().to_vec(),
            }
        } else {
            Self::Text(value.to_string_lossy())
        }
    }

    fn into_web_storage_string(self) -> WebStorageString {
        match self {
            Self::Text(value) => WebStorageString::from_utf8(&value),
            Self::Utf16 { utf16 } => WebStorageString::from_utf16_units(utf16),
        }
    }
}

fn load_json_web_storage(
    path: &Path,
) -> Result<(MemoryWebStorageBackend, SerializedWebStorageAreas)> {
    if !path.exists() {
        return Ok(Default::default());
    }
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read localStorage store `{}`", path.display()))?;
    if bytes.is_empty() {
        return Ok(Default::default());
    }
    let file: JsonWebStorageFile<Box<RawValue>> = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse localStorage store `{}`", path.display()))?;
    let mut memory = MemoryWebStorageBackend::default();
    let mut serialized_areas = BTreeMap::new();
    for (origin, serialized) in file.origins {
        let area: JsonWebStorageAreaFile = serde_json::from_str(serialized.get())
            .with_context(|| format!("failed to parse localStorage store `{}`", path.display()))?;
        let values = area
            .entries
            .into_iter()
            .map(|entry| {
                (
                    entry.key.into_web_storage_string(),
                    entry.value.into_web_storage_string(),
                )
            })
            .collect::<HashMap<_, _>>();
        let size = values.values().map(WebStorageString::usage_bytes).sum();
        memory
            .origins
            .insert(origin.clone(), MemoryWebStorageArea { values, size });
        serialized_areas.insert(origin, Arc::from(serialized));
    }
    Ok((memory, serialized_areas))
}

fn persist_json_web_storage(path: &Path, areas: &SerializedWebStorageAreas) -> Result<()> {
    let file = JsonWebStorageFile {
        version: WebStorageJsonVersion::default(),
        origins: areas
            .iter()
            .map(|(origin, area)| (origin.clone(), area.as_ref()))
            .collect(),
    };
    let bytes =
        serde_json::to_vec_pretty(&file).context("failed to serialize localStorage store")?;

    moli_browser_profile::write_profile_file(path, &bytes, "localStorage store")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        MemoryWebStorageArea, WEB_STORAGE_QUOTA_BYTES, WebStorageAreaKind, WebStorageBackend,
        WebStorageMutation, WebStorageMutationError, WebStorageMutationRecord, WebStorageStore,
        WebStorageString, deep_clone_shared_web_storage_store, new_shared_json_web_storage_store,
        new_shared_web_storage_store, web_storage_partitioned_area_key,
    };

    struct TempPath {
        path: PathBuf,
    }

    impl TempPath {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-web-storage-{name}-{}-{nonce}.json",
                std::process::id()
            ));
            Self { path }
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn first_party_area_key(origin: &str) -> String {
        web_storage_partitioned_area_key(origin, origin)
    }

    #[test]
    fn memory_web_storage_preserves_sorted_key_and_quota_behavior() {
        let mut store = WebStorageStore::default();
        let area_key = first_party_area_key("https://a.test");
        assert!(store.set_item(&area_key, "b", "2"));
        assert!(store.set_item(&area_key, "a", "1"));
        assert_eq!(store.len(&area_key), 2);
        assert_eq!(store.key(&area_key, 0), Some("a".to_owned()));
        assert_eq!(store.sorted_keys(&area_key), vec!["a", "b"]);
        assert_eq!(store.get_item(&area_key, "b"), Some("2".to_owned()));
        assert!(store.contains_key(&area_key, "a"));
        assert!(store.remove_item(&area_key, "a"));
        assert!(!store.contains_key(&area_key, "a"));

        let large = "x".repeat(WEB_STORAGE_QUOTA_BYTES + 1);
        assert!(!store.set_item(&area_key, "large", &large));
        assert_eq!(
            store.try_set_item(&area_key, "large", &large),
            Err(WebStorageMutationError::QuotaExceeded)
        );
    }

    #[test]
    fn mutation_subscription_reports_only_successful_observable_changes() {
        let mut store = WebStorageStore::default();
        let area_key = first_party_area_key("https://a.test");
        let subscription = store.subscribe_mutations(WebStorageAreaKind::Local);

        assert!(store.set_item(&area_key, "a", "1"));
        assert!(store.set_item(&area_key, "a", "1"));
        assert!(store.set_item(&area_key, "a", "2"));
        assert!(!store.remove_item(&area_key, "missing"));
        assert!(store.remove_item(&area_key, "a"));
        assert!(!store.try_clear(&area_key).expect("empty clear should work"));
        assert!(store.set_item(&area_key, "b", "3"));
        assert!(
            store
                .try_clear(&area_key)
                .expect("nonempty clear should work")
        );

        assert_eq!(
            subscription.drain(),
            vec![
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemAdded {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("a"),
                        value: WebStorageString::from_utf8("1"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemUpdated {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("a"),
                        old_value: WebStorageString::from_utf8("1"),
                        new_value: WebStorageString::from_utf8("2"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemRemoved {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("a"),
                        old_value: WebStorageString::from_utf8("2"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemAdded {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("b"),
                        value: WebStorageString::from_utf8("3"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemsCleared {
                        area_key: area_key.clone(),
                    },
                },
            ]
        );
        assert!(subscription.is_empty());

        let empty_area_key = first_party_area_key("https://empty.test");
        match &mut store.backend {
            WebStorageBackend::Memory(memory) => {
                memory
                    .origins
                    .insert(empty_area_key.clone(), MemoryWebStorageArea::default());
            }
            WebStorageBackend::Json(_) => panic!("default store should use the memory backend"),
        }
        assert!(
            store
                .try_clear(&empty_area_key)
                .expect("empty existing area should clear")
        );
        let WebStorageBackend::Memory(memory) = &store.backend else {
            panic!("default store should use the memory backend");
        };
        assert!(
            !memory.origins.contains_key(&empty_area_key),
            "clear must preserve the existing area cleanup semantics"
        );
        assert!(
            subscription.is_empty(),
            "clearing an empty existing area must not publish a mutation"
        );

        drop(subscription);
        assert!(store.set_item(&area_key, "after-drop", "4"));
        assert!(store.mutation_subscribers.is_empty());
    }

    #[test]
    fn one_mutation_subscription_preserves_cross_store_order_and_deduplicates_registration() {
        let area_key = first_party_area_key("https://a.test");
        let mut local = WebStorageStore::default();
        let mut session = WebStorageStore::default();
        let subscription = local.subscribe_mutations(WebStorageAreaKind::Local);
        local.add_mutation_subscription(WebStorageAreaKind::Local, &subscription);
        session.add_mutation_subscription(WebStorageAreaKind::Session, &subscription);

        assert!(local.set_item(&area_key, "local-first", "1"));
        assert!(session.set_item(&area_key, "session-second", "2"));
        assert!(local.set_item(&area_key, "local-third", "3"));

        let records = subscription.drain();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records
                .iter()
                .map(|record| record.area_kind)
                .collect::<Vec<_>>(),
            vec![
                WebStorageAreaKind::Local,
                WebStorageAreaKind::Session,
                WebStorageAreaKind::Local,
            ]
        );
    }

    #[test]
    fn deep_clone_copies_all_areas_without_sharing_subsequent_mutations() {
        let source = new_shared_web_storage_store();
        let a_area_key = first_party_area_key("https://a.test");
        let b_area_key = first_party_area_key("https://b.test");
        {
            let mut source = source.lock();
            assert!(source.set_item(&a_area_key, "shared", "source"));
            assert!(source.set_item(&b_area_key, "other", "copied"));
        }
        let subscription = source
            .lock()
            .subscribe_mutations(WebStorageAreaKind::Session);

        let cloned = deep_clone_shared_web_storage_store(&source);
        {
            let mut cloned = cloned.lock();
            assert_eq!(
                cloned.get_item(&a_area_key, "shared"),
                Some("source".to_owned())
            );
            assert_eq!(
                cloned.get_item(&b_area_key, "other"),
                Some("copied".to_owned())
            );
            assert!(cloned.set_item(&a_area_key, "shared", "clone"));
            assert!(cloned.set_item(&a_area_key, "clone-only", "yes"));
        }
        assert!(
            subscription.is_empty(),
            "a deep-cloned sessionStorage namespace must not retain opener mutation subscribers"
        );
        {
            let mut source = source.lock();
            assert_eq!(
                source.get_item(&a_area_key, "shared"),
                Some("source".to_owned())
            );
            assert_eq!(source.get_item(&a_area_key, "clone-only"), None);
            assert!(source.set_item(&b_area_key, "source-only", "yes"));
        }
        assert_eq!(cloned.lock().get_item(&b_area_key, "source-only"), None);
    }

    #[test]
    fn json_web_storage_persists_values_by_storage_key() {
        let temp = TempPath::new("persist");
        let a_area_key = first_party_area_key("https://a.test");
        let b_area_key = first_party_area_key("https://b.test");
        {
            let store = new_shared_json_web_storage_store(&temp.path)
                .expect("json web storage should open");
            let mut store = store.lock();
            assert!(store.set_item(&a_area_key, "b", "2"));
            assert!(store.set_item(&a_area_key, "a", "1"));
            assert!(store.set_item(&b_area_key, "a", "other"));
            assert_eq!(store.sorted_keys(&a_area_key), vec!["a", "b"]);
        }

        let store =
            new_shared_json_web_storage_store(&temp.path).expect("json web storage should reopen");
        let mut store = store.lock();
        assert_eq!(store.get_item(&a_area_key, "a"), Some("1".to_owned()));
        assert_eq!(store.get_item(&b_area_key, "a"), Some("other".to_owned()));
        assert_eq!(store.len(&a_area_key), 2);
        store.clear_origin(&a_area_key);
        assert_eq!(store.len(&a_area_key), 0);
        assert_eq!(store.get_item(&b_area_key, "a"), Some("other".to_owned()));
    }

    #[test]
    fn json_web_storage_reports_persistence_error_without_mutating_memory() {
        let temp = TempPath::new("persist-error-dir");
        let area_key = first_party_area_key("https://a.test");
        let other_area_key = first_party_area_key("https://b.test");
        let shared =
            new_shared_json_web_storage_store(&temp.path).expect("json web storage should open");
        let mut store = shared.lock();
        assert!(store.set_item(&area_key, "a", "1"));
        assert!(store.set_item(&other_area_key, "b", "2"));
        fs::remove_file(&temp.path).expect("storage file should be removed");
        fs::create_dir_all(&temp.path).expect("storage path directory should be created");
        let subscription = store.subscribe_mutations(WebStorageAreaKind::Local);

        for result in [
            store.try_set_item(&area_key, "a", "changed"),
            store.try_remove_item(&area_key, "a"),
            store.try_clear_origin(&area_key),
        ] {
            assert!(matches!(
                result,
                Err(WebStorageMutationError::Persistence(_))
            ));
        }
        assert_eq!(store.get_item(&area_key, "a"), Some("1".to_owned()));
        assert!(subscription.is_empty());
        fs::remove_dir_all(&temp.path).expect("storage path directory should be removed");

        assert!(store.set_item(&other_area_key, "b", "recovered"));
        let reopened = new_shared_json_web_storage_store(&temp.path)
            .expect("json web storage should reopen after recovery");
        let mut reopened = reopened.lock();
        assert_eq!(reopened.get_item(&area_key, "a"), Some("1".to_owned()));
        assert_eq!(
            reopened.get_item(&other_area_key, "b"),
            Some("recovered".to_owned())
        );
    }

    #[test]
    fn json_web_storage_updates_and_origin_clears_preserve_unrelated_imported_areas() {
        let temp = TempPath::new("imported-areas");
        let origin = "https://cdn.example.test";
        let first_party = first_party_area_key(origin);
        let partitioned = web_storage_partitioned_area_key(origin, "https://top.example.test");
        let unrelated = first_party_area_key("https://other.example.test");
        let surrogate_key = [0xD800];
        let surrogate_value = [0xDC00, u16::from(b'"'), u16::from(b'\\')];
        {
            let shared =
                new_shared_json_web_storage_store(&temp.path).expect("storage should open");
            let mut store = shared.lock();
            assert!(store.set_item(&first_party, "existing", "first"));
            assert!(store.set_item(&partitioned, "existing", "partitioned"));
            assert!(store.set_item_utf16(&unrelated, &surrogate_key, &surrogate_value));
        }
        {
            let shared =
                new_shared_json_web_storage_store(&temp.path).expect("storage should reopen");
            let mut store = shared.lock();
            assert!(store.set_item(&first_party, "existing", "updated"));
            assert!(store.set_item(&first_party, "new", "added"));
            assert!(store.remove_item(&first_party, "existing"));
            assert_eq!(
                store.get_item(&partitioned, "existing"),
                Some("partitioned".to_owned())
            );
            assert!(
                store
                    .try_clear_origin_areas(origin)
                    .expect("origin areas should clear")
            );
        }
        let shared = new_shared_json_web_storage_store(&temp.path)
            .expect("storage should reopen after clearing");
        let mut store = shared.lock();
        assert_eq!(store.len(&first_party), 0);
        assert_eq!(store.len(&partitioned), 0);
        assert_eq!(
            store.get_item_utf16(&unrelated, &surrogate_key),
            Some(surrogate_value.to_vec())
        );
        assert!(store.set_item(&first_party, "new", "restored"));
        drop(store);
        let reopened = new_shared_json_web_storage_store(&temp.path)
            .expect("storage should reopen after restoring");
        assert_eq!(
            reopened.lock().get_item(&first_party, "new"),
            Some("restored".to_owned())
        );
        assert_eq!(
            reopened.lock().get_item_utf16(&unrelated, &surrogate_key),
            Some(surrogate_value.to_vec())
        );
    }

    #[test]
    fn json_web_storage_persists_unpaired_surrogates_losslessly() {
        let temp = TempPath::new("persist-utf16");
        let area_key = first_party_area_key("https://a.test");
        let key = vec![0xD800];
        let value = vec![0xDC00];
        {
            let store = new_shared_json_web_storage_store(&temp.path)
                .expect("json web storage should open");
            let mut store = store.lock();
            assert!(store.set_item_utf16(&area_key, &key, &value));
        }

        let persisted = fs::read_to_string(&temp.path).expect("json storage should be persisted");
        assert!(
            persisted.contains(r#""entries""#),
            "lossless storage should use entry form: {persisted}"
        );
        assert!(
            persisted.contains(r#""utf16""#),
            "lossless storage should encode unpaired surrogates explicitly: {persisted}"
        );
        assert!(
            persisted.contains("55296") && persisted.contains("56320"),
            "persisted JSON should contain the raw UTF-16 units: {persisted}"
        );

        let store =
            new_shared_json_web_storage_store(&temp.path).expect("json web storage should reopen");
        let mut store = store.lock();
        assert_eq!(store.get_item_utf16(&area_key, &key), Some(value.clone()));
        assert_eq!(store.key_utf16(&area_key, 0), Some(key));
    }

    #[test]
    fn web_storage_usage_tracks_value_bytes_by_storage_key() {
        let mut store = WebStorageStore::default();
        let a_area_key = first_party_area_key("https://a.test");
        let b_area_key = first_party_area_key("https://b.test");
        assert_eq!(store.usage_bytes(&a_area_key), 0);

        assert!(store.set_item(&a_area_key, "first", "abc"));
        assert!(store.set_item(&a_area_key, "second", "defg"));
        assert!(store.set_item(&b_area_key, "other", "xxxx"));
        assert_eq!(store.usage_bytes(&a_area_key), 7);
        assert_eq!(store.usage_bytes(&b_area_key), 4);

        assert!(store.set_item(&a_area_key, "first", "z"));
        assert_eq!(store.usage_bytes(&a_area_key), 5);

        assert!(store.remove_item(&a_area_key, "second"));
        assert_eq!(store.usage_bytes(&a_area_key), 1);

        store.clear_origin(&a_area_key);
        assert_eq!(store.usage_bytes(&a_area_key), 0);
        assert_eq!(store.usage_bytes(&b_area_key), 4);
    }

    #[test]
    fn origin_area_usage_and_clear_include_partitioned_area_keys() {
        let origin = "https://cdn.example.test";
        let first_party = first_party_area_key(origin);
        let top_a = web_storage_partitioned_area_key(origin, "https://top-a.example.test");
        let top_b = web_storage_partitioned_area_key(origin, "https://top-b.example.test");
        let sibling = web_storage_partitioned_area_key(
            "https://sibling.example.test",
            "https://top-a.example.test",
        );
        let mut store = WebStorageStore::default();
        assert!(store.set_item(&first_party, "fp", "a"));
        assert!(store.set_item(&top_a, "a", "bb"));
        assert!(store.set_item(&top_b, "b", "ccc"));
        assert!(store.set_item(&sibling, "s", "dddd"));

        assert_eq!(store.usage_bytes(&first_party), 1);
        assert_eq!(store.usage_bytes_for_origin_areas(origin), 6);
        assert_eq!(
            store.usage_bytes_for_origin_areas("https://sibling.example.test"),
            4
        );

        store.clear_origin(&first_party);
        assert_eq!(store.usage_bytes_for_origin_areas(origin), 5);
        assert_eq!(store.get_item(&top_a, "a"), Some("bb".to_owned()));

        store.clear_origin_areas(origin);
        assert_eq!(store.usage_bytes_for_origin_areas(origin), 0);
        assert_eq!(store.get_item(&top_a, "a"), None);
        assert_eq!(store.get_item(&top_b, "b"), None);
        assert_eq!(store.get_item(&sibling, "s"), Some("dddd".to_owned()));
    }
}

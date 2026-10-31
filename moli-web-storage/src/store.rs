use super::*;

pub(super) struct Edit<R> {
    pub(super) result: R,
    pub(super) mutation: Option<WebStorageMutation>,
}
impl<R> Edit<R> {
    fn unchanged(result: R) -> Self {
        Self {
            result,
            mutation: None,
        }
    }
}

pub fn new_shared_web_storage_store() -> SharedWebStorageStore {
    Arc::new(WebStorageStore::default())
}
pub fn deep_clone_shared_web_storage_store(
    source: &SharedWebStorageStore,
) -> SharedWebStorageStore {
    // This API implements browsing-context sessionStorage cloning. A local
    // SQLite store is not a sessionStorage namespace and cannot be copied here.
    let memory = match &source.backend {
        WebStorageBackend::Memory(memory) => memory.lock().clone(),
        WebStorageBackend::Sqlite(_) => {
            panic!("sessionStorage cloning requires an in-memory namespace")
        }
    };
    Arc::new(WebStorageStore {
        backend: WebStorageBackend::Memory(Mutex::new(memory)),
        mutation_subscribers: Mutex::new(Vec::new()),
    })
}
impl Default for WebStorageStore {
    fn default() -> Self {
        Self {
            backend: WebStorageBackend::Memory(Mutex::new(MemoryWebStorageBackend::default())),
            mutation_subscribers: Mutex::new(Vec::new()),
        }
    }
}
impl fmt::Debug for WebStorageStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.backend {
            WebStorageBackend::Memory(_) => "memory",
            WebStorageBackend::Sqlite(_) => "sqlite",
        };
        f.debug_struct("WebStorageStore")
            .field("backend", &kind)
            .finish_non_exhaustive()
    }
}
impl WebStorageStore {
    fn read<R>(
        &self,
        origin: &str,
        read: impl FnOnce(Option<&MemoryWebStorageArea>) -> R,
    ) -> std::result::Result<R, WebStorageMutationError> {
        match &self.backend {
            WebStorageBackend::Memory(memory) => Ok(read(memory.lock().origins.get(origin))),
            WebStorageBackend::Sqlite(sqlite) => sqlite.read(origin, read),
        }
    }
    fn edit<R>(
        &self,
        origin: &str,
        edit: impl FnOnce(
            &mut MemoryWebStorageArea,
        ) -> std::result::Result<Edit<R>, WebStorageMutationError>,
    ) -> std::result::Result<R, WebStorageMutationError> {
        let publish = |edit: Edit<R>| {
            if let Some(mutation) = edit.mutation {
                self.publish_mutation(mutation);
            }
            edit.result
        };
        match &self.backend {
            WebStorageBackend::Memory(memory) => {
                let mut memory = memory.lock();
                let result = edit(memory.origins.entry(origin.to_owned()).or_default())?;
                if memory
                    .origins
                    .get(origin)
                    .is_some_and(|area| area.values.is_empty())
                {
                    memory.origins.remove(origin);
                }
                Ok(publish(result))
            }
            WebStorageBackend::Sqlite(sqlite) => sqlite.edit(origin, edit, publish),
        }
    }
    pub fn subscribe_mutations(
        &self,
        area_kind: WebStorageAreaKind,
    ) -> WebStorageMutationSubscription {
        let subscription = WebStorageMutationSubscription {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        };
        self.add_mutation_subscription(area_kind, &subscription);
        subscription
    }
    pub fn add_mutation_subscription(
        &self,
        area_kind: WebStorageAreaKind,
        subscription: &WebStorageMutationSubscription,
    ) {
        let mut subscribers = self.mutation_subscribers.lock();
        subscribers.retain(|subscriber| {
            subscriber.queue.upgrade().is_some_and(|queue| {
                !(subscriber.area_kind == area_kind && Arc::ptr_eq(&queue, &subscription.queue))
            })
        });
        subscribers.push(WebStorageMutationSubscriber {
            area_kind,
            queue: Arc::downgrade(&subscription.queue),
        });
    }
    fn publish_mutation(&self, mutation: WebStorageMutation) {
        self.mutation_subscribers.lock().retain(|subscriber| {
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
    pub fn try_get_item_utf16(
        &self,
        origin: &str,
        key: &[u16],
    ) -> std::result::Result<Option<Vec<u16>>, WebStorageMutationError> {
        self.read(origin, |area| {
            area.and_then(|area| area.get_item_utf16(key))
        })
    }
    pub fn get_item_utf16(&self, origin: &str, key: &[u16]) -> Option<Vec<u16>> {
        self.try_get_item_utf16(origin, key).ok().flatten()
    }
    pub fn get_item(&self, origin: &str, key: &str) -> Option<String> {
        self.get_item_utf16(origin, &utf16_units(key))
            .map(|v| String::from_utf16_lossy(&v))
    }
    pub fn try_set_change_utf16(
        &self,
        origin: &str,
        key: &[u16],
        value: &[u16],
    ) -> std::result::Result<Option<WebStorageMutation>, WebStorageMutationError> {
        self.edit(origin, |area| {
            let key = WebStorageString::from_utf16_units(key.to_vec());
            let value = WebStorageString::from_utf16_units(value.to_vec());
            let previous = area.values.get(&key);
            if previous == Some(&value) {
                return Ok(Edit::unchanged(None));
            }
            let size = updated_storage_size(area.size, &key, previous, &value);
            if size > area.size && size > WEB_STORAGE_QUOTA_BYTES {
                return Err(WebStorageMutationError::QuotaExceeded);
            }
            let previous = area.values.insert(key.clone(), value.clone());
            area.size = size;
            let mutation = match previous {
                Some(old_value) => WebStorageMutation::ItemUpdated {
                    area_key: origin.to_owned(),
                    key,
                    old_value,
                    new_value: value,
                },
                None => WebStorageMutation::ItemAdded {
                    area_key: origin.to_owned(),
                    key,
                    value,
                },
            };
            Ok(Edit {
                result: Some(mutation.clone()),
                mutation: Some(mutation),
            })
        })
    }
    pub fn try_set_item_utf16(
        &self,
        origin: &str,
        key: &[u16],
        value: &[u16],
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_set_change_utf16(origin, key, value).map(|_| true)
    }
    pub fn try_set_item(
        &self,
        origin: &str,
        key: &str,
        value: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_set_item_utf16(origin, &utf16_units(key), &utf16_units(value))
    }
    pub fn set_item(&self, origin: &str, key: &str, value: &str) -> bool {
        self.try_set_item(origin, key, value).unwrap_or(false)
    }
    pub fn set_item_utf16(&self, origin: &str, key: &[u16], value: &[u16]) -> bool {
        self.try_set_item_utf16(origin, key, value).unwrap_or(false)
    }
    pub fn try_remove_change_utf16(
        &self,
        origin: &str,
        key: &[u16],
    ) -> std::result::Result<Option<WebStorageMutation>, WebStorageMutationError> {
        self.edit(origin, |area| {
            let key = WebStorageString::from_utf16_units(key.to_vec());
            let Some(old_value) = area.values.remove(&key) else {
                return Ok(Edit::unchanged(None));
            };
            area.size -= storage_item_size(&key, &old_value);
            let mutation = WebStorageMutation::ItemRemoved {
                area_key: origin.to_owned(),
                key,
                old_value,
            };
            Ok(Edit {
                result: Some(mutation.clone()),
                mutation: Some(mutation),
            })
        })
    }
    pub fn try_remove_item_utf16(
        &self,
        origin: &str,
        key: &[u16],
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_remove_change_utf16(origin, key)
            .map(|change| change.is_some())
    }
    pub fn try_remove_item(
        &self,
        origin: &str,
        key: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_remove_item_utf16(origin, &utf16_units(key))
    }
    pub fn remove_item(&self, origin: &str, key: &str) -> bool {
        self.try_remove_item(origin, key).unwrap_or(false)
    }
    pub fn remove_item_utf16(&self, origin: &str, key: &[u16]) -> bool {
        self.try_remove_item_utf16(origin, key).unwrap_or(false)
    }
    pub fn try_clear_change(
        &self,
        origin: &str,
    ) -> std::result::Result<Option<WebStorageMutation>, WebStorageMutationError> {
        self.edit(origin, |area| {
            if area.values.is_empty() {
                return Ok(Edit::unchanged(None));
            }
            area.values.clear();
            area.size = 0;
            let mutation = WebStorageMutation::ItemsCleared {
                area_key: origin.to_owned(),
            };
            Ok(Edit {
                result: Some(mutation.clone()),
                mutation: Some(mutation),
            })
        })
    }
    pub fn try_clear(&self, origin: &str) -> std::result::Result<bool, WebStorageMutationError> {
        // Legacy native callers distinguish removal of an existing empty area.
        match &self.backend {
            WebStorageBackend::Memory(memory) => {
                let mut memory = memory.lock();
                let removed = memory.origins.remove(origin);
                if removed.as_ref().is_some_and(|area| !area.values.is_empty()) {
                    self.publish_mutation(WebStorageMutation::ItemsCleared {
                        area_key: origin.to_owned(),
                    });
                }
                Ok(removed.is_some())
            }
            WebStorageBackend::Sqlite(_) => {
                self.try_clear_change(origin).map(|change| change.is_some())
            }
        }
    }
    pub fn clear(&self, origin: &str) {
        let _ = self.try_clear(origin);
    }
    pub fn clear_origin(&self, origin: &str) {
        self.clear(origin);
    }
    pub fn try_clear_origin(
        &self,
        origin: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.try_clear(origin)
    }
    pub fn try_key_utf16(
        &self,
        origin: &str,
        index: usize,
    ) -> std::result::Result<Option<Vec<u16>>, WebStorageMutationError> {
        self.read(origin, |area| area.and_then(|area| area.key_utf16(index)))
    }
    pub fn key_utf16(&self, origin: &str, index: usize) -> Option<Vec<u16>> {
        self.try_key_utf16(origin, index).ok().flatten()
    }
    pub fn key(&self, origin: &str, index: usize) -> Option<String> {
        self.key_utf16(origin, index)
            .map(|v| String::from_utf16_lossy(&v))
    }
    pub fn try_items_utf16(
        &self,
        origin: &str,
    ) -> std::result::Result<Vec<(Vec<u16>, Vec<u16>)>, WebStorageMutationError> {
        self.read(origin, |area| {
            let Some(area) = area else {
                return Vec::new();
            };
            let mut items = area.values.iter().collect::<Vec<_>>();
            items.sort_by_key(|(key, _)| *key);
            items
                .into_iter()
                .map(|(key, value)| (key.as_units().to_vec(), value.as_units().to_vec()))
                .collect()
        })
    }
    pub fn try_len(&self, origin: &str) -> std::result::Result<usize, WebStorageMutationError> {
        self.read(origin, |area| area.map_or(0, |a| a.values.len()))
    }
    pub fn len(&self, origin: &str) -> usize {
        self.try_len(origin).unwrap_or(0)
    }
    pub fn try_contains_key_utf16(
        &self,
        origin: &str,
        key: &[u16],
    ) -> std::result::Result<bool, WebStorageMutationError> {
        self.read(origin, |area| {
            area.is_some_and(|area| area.contains_key_utf16(key))
        })
    }
    pub fn contains_key_utf16(&self, origin: &str, key: &[u16]) -> bool {
        self.try_contains_key_utf16(origin, key).unwrap_or(false)
    }
    pub fn contains_key(&self, origin: &str, key: &str) -> bool {
        self.contains_key_utf16(origin, &utf16_units(key))
    }
    pub fn try_sorted_keys_utf16(
        &self,
        origin: &str,
    ) -> std::result::Result<Vec<Vec<u16>>, WebStorageMutationError> {
        self.read(origin, |area| {
            area.map_or_else(Vec::new, |area| area.sorted_keys_utf16())
        })
    }
    pub fn sorted_keys_utf16(&self, origin: &str) -> Vec<Vec<u16>> {
        self.try_sorted_keys_utf16(origin).unwrap_or_default()
    }
    pub fn sorted_keys(&self, origin: &str) -> Vec<String> {
        self.sorted_keys_utf16(origin)
            .into_iter()
            .map(|v| String::from_utf16_lossy(&v))
            .collect()
    }
    pub fn try_usage_bytes(
        &self,
        origin: &str,
    ) -> std::result::Result<usize, WebStorageMutationError> {
        if let WebStorageBackend::Sqlite(sqlite) = &self.backend {
            return sqlite.usage_bytes(origin);
        }
        self.read(origin, |area| area.map_or(0, |a| a.size))
    }
    pub fn usage_bytes(&self, origin: &str) -> usize {
        self.try_usage_bytes(origin).unwrap_or(0)
    }
    pub fn try_clear_origin_areas(
        &self,
        origin: &str,
    ) -> std::result::Result<bool, WebStorageMutationError> {
        let publish = |keys: Vec<String>| {
            let changed = !keys.is_empty();
            for area_key in keys {
                self.publish_mutation(WebStorageMutation::ItemsCleared { area_key });
            }
            changed
        };
        match &self.backend {
            WebStorageBackend::Memory(memory) => {
                let mut memory = memory.lock();
                let keys = memory.area_keys_for_origin(origin);
                let nonempty = keys
                    .iter()
                    .filter(|key| memory.len(key) > 0)
                    .cloned()
                    .collect::<Vec<_>>();
                for key in &keys {
                    memory.origins.remove(key);
                }
                publish(nonempty);
                Ok(!keys.is_empty())
            }
            WebStorageBackend::Sqlite(sqlite) => {
                sqlite.clear_origin_areas(origin, |mutation| self.publish_mutation(mutation))
            }
        }
    }
    pub fn clear_origin_areas(&self, origin: &str) {
        let _ = self.try_clear_origin_areas(origin);
    }
    pub fn flush(&self) -> std::result::Result<(), WebStorageMutationError> {
        match &self.backend {
            WebStorageBackend::Sqlite(sqlite) => sqlite.client.flush().map_err(Into::into),
            _ => Ok(()),
        }
    }
}

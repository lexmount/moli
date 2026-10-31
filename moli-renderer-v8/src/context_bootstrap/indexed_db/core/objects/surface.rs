use super::*;
use crate::util::{get_private_value, set_private_value, v8_string_from_utf16_units};

const OBJECT_STORE_CACHE_SLOT: &str = "moli.IndexedDb.objectStoreHandles";
const INDEX_CACHE_SLOT: &str = "moli.IndexedDb.indexHandles";

pub(in crate::context_bootstrap::indexed_db) fn release_schema_handle_caches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(stores) = get_private_value(scope, transaction, OBJECT_STORE_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    else {
        return;
    };
    let entries = stores.as_array(scope);
    for index in (1..entries.length()).step_by(2) {
        if let Some(store) = entries
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && let Some(indexes) = get_private_value(scope, store, INDEX_CACHE_SLOT)
                .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        {
            indexes.clear();
        }
    }
    // A terminal transaction cannot expose these handles through lookup again.
    // Existing author references keep their own native metadata alive.
    stores.clear();
}

pub(in crate::context_bootstrap::indexed_db) fn invalidate_index_handle_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    name: &moli_indexeddb::IndexedDbName,
) {
    if let Some(cache) = get_private_value(scope, store, INDEX_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        && let Some(key) = v8_string_from_utf16_units(scope, name.as_utf16())
    {
        let _ = cache.delete(scope, key.into());
    }
}

fn cached_schema_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &'static str,
    name: &moli_indexeddb::IndexedDbName,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = get_private_value(scope, owner, slot)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())?;
    let key = v8_string_from_utf16_units(scope, name.as_utf16())?;
    v8::Local::<v8::Object>::try_from(cache.get(scope, key.into())?).ok()
}

fn cache_schema_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &'static str,
    name: &moli_indexeddb::IndexedDbName,
    handle: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let context = owner.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let cache = get_private_value(scope, owner, slot)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .unwrap_or_else(|| {
            let cache = v8::Map::new(scope);
            set_private_value(scope, owner, slot, cache.into());
            cache
        });
    let key = v8_string_from_utf16_units(scope, name.as_utf16())?;
    cache.set(scope, key.into(), handle.into())?;
    Some(())
}

mod database;
mod index;
mod object_store;

pub(in crate::context_bootstrap::indexed_db) use self::database::*;
pub(in crate::context_bootstrap::indexed_db) use self::index::*;
pub(in crate::context_bootstrap::indexed_db) use self::object_store::*;

use super::store::{WebStorageMutationError, WebStorageStore};
use super::*;
use crate::document_runtime::DomHandle;
use crate::util::{get_private_value, set_private_value};
use crate::{
    native_bridge::{OwnerDispatchScope, WindowTaskTarget},
    page_task_queue::RendererPageStorageEventData,
};
use moli_webapi_declare::WebApiObject;

pub(super) const STORAGE_PROTOTYPE_INDEXED_DESCRIPTORS_SLOT: &str =
    "__moliStoragePrototypeIndexedDescriptors";
pub(super) const STORAGE_KIND_INTERNAL_FIELD_INDEX: usize = 0;
pub(super) const STORAGE_OWNER_INTERNAL_FIELD_INDEX: usize = 1;
pub(super) const STORAGE_OWNER_CHILD_HANDLE_SLOT: &str = "__moliStorageOwnerChildHandle";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", allow_empty)]
struct StoragePrototypeIndexDescriptorStoreDeclaration {}

const STORAGE_INTERNAL_NAME_UNITS: &[u16] = &[
    b'_' as u16,
    b'_' as u16,
    b'l' as u16,
    b'm' as u16,
    b'S' as u16,
    b't' as u16,
    b'o' as u16,
    b'r' as u16,
    b'a' as u16,
    b'g' as u16,
    b'e' as u16,
    b'K' as u16,
    b'i' as u16,
    b'n' as u16,
    b'd' as u16,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WebStorageOwner {
    ActiveDocument,
    Child(DomHandle),
    LightweightPopup(u64),
}

pub(super) fn web_storage_owner_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> WebStorageOwner {
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, window) {
        return WebStorageOwner::LightweightPopup(popup_id);
    }
    super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(scope, window)
        .map(WebStorageOwner::Child)
        .unwrap_or(WebStorageOwner::ActiveDocument)
}

pub(super) fn storage_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage: v8::Local<'_, v8::Object>,
) -> WebStorageOwner {
    let storage = v8::Local::new(scope, v8::Global::new(scope, storage));
    if let Some(handle) = get_private_value(scope, storage, STORAGE_OWNER_CHILD_HANDLE_SLOT)
        .and_then(|value| {
            super::super::navigation_window::dom_handle_from_marker_value(scope, value)
        })
    {
        return WebStorageOwner::Child(handle);
    }
    storage
        .get_internal_field(scope, STORAGE_OWNER_INTERNAL_FIELD_INDEX)
        .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
        .and_then(|v| v8::Local::<v8::BigInt>::try_from(v).ok())
        .and_then(|value| {
            let (popup_id, lossless) = value.u64_value();
            (lossless && popup_id != 0).then_some(WebStorageOwner::LightweightPopup(popup_id))
        })
        .unwrap_or(WebStorageOwner::ActiveDocument)
}

pub(super) fn set_storage_owner_child_handle(
    scope: &mut v8::PinScope<'_, '_>,
    storage: v8::Local<'_, v8::Object>,
    handle: DomHandle,
) {
    let value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
    set_private_value(
        scope,
        storage,
        STORAGE_OWNER_CHILD_HANDLE_SLOT,
        value.into(),
    );
}

pub(super) fn storage_access_allows_web_storage_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, window) {
        let runtime = unsafe { &mut *host_ptr };
        let Some(storage_context) = runtime.storage_context_for_lightweight_popup(popup_id) else {
            return false;
        };
        return !moli_storage_key::serialized_storage_key_has_opaque_origin(
            &storage_context.storage_key().serialized_storage_key(),
        );
    }

    let owner = super::super::navigation_window::runtime_window_owner(scope, window);
    let child_handle =
        super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(
            scope, window,
        )
        .or_else(|| {
            super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(
                scope, owner,
            )
        });
    let runtime = unsafe { &mut *host_ptr };
    let context = if let Some(handle) = child_handle {
        let Some(context) = runtime.storage_context_for_child_browsing_context(handle) else {
            return false;
        };
        context
    } else {
        runtime.top_document_storage_context()
    };
    !moli_storage_key::serialized_storage_key_has_opaque_origin(
        &context.storage_key().serialized_storage_key(),
    )
}

pub(super) fn current_storage_host_for_owner(
    scope: &mut v8::PinScope<'_, '_>,
    owner: WebStorageOwner,
) -> Option<(String, String, *mut crate::native_bridge::JsContextHost)> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &mut *host_ptr };
    let (origin, area_key) = match owner {
        WebStorageOwner::ActiveDocument => runtime
            .top_document_storage_context()
            .into_origin_and_area_key(),
        WebStorageOwner::Child(handle) => runtime
            .storage_context_for_child_browsing_context(handle)?
            .into_origin_and_area_key(),
        WebStorageOwner::LightweightPopup(popup_id) => runtime
            .storage_context_for_lightweight_popup(popup_id)?
            .into_origin_and_area_key(),
    };
    Some((origin, area_key, host_ptr))
}

pub(super) fn this_storage_kind(
    scope: &mut v8::PinScope<'_, '_>,
    this: v8::Local<'_, v8::Object>,
) -> bool {
    this.get_internal_field(scope, STORAGE_KIND_INTERNAL_FIELD_INDEX)
        .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
        .and_then(|v| v.to_string(scope))
        .is_some_and(|s| s.to_rust_string_lossy(scope) == "session")
}

pub(super) fn with_storage_store<R>(
    scope: &mut v8::PinScope<'_, '_>,
    storage: v8::Local<'_, v8::Object>,
    f: impl FnOnce(&mut WebStorageStore, &str) -> R,
) -> Option<R> {
    let is_session = this_storage_kind(scope, storage);
    let owner = storage_owner(scope, storage);
    let (_, area_key, host_ptr) = current_storage_host_for_owner(scope, owner)?;
    let host = unsafe { &*host_ptr };
    let store = if is_session {
        match owner {
            WebStorageOwner::ActiveDocument | WebStorageOwner::Child(_) => {
                host.session_storage_store()
            }
            WebStorageOwner::LightweightPopup(popup_id) => {
                host.lightweight_popup_session_storage_store(popup_id)?
            }
        }
    } else {
        host.web_storage_store()
    };
    let mut guard = store.lock();
    Some(f(&mut guard, &area_key))
}

pub(super) fn storage_put_utf16(
    scope: &mut v8::PinScope<'_, '_>,
    storage: v8::Local<'_, v8::Object>,
    key: &[u16],
    value: &[u16],
) -> bool {
    if storage_internal_name_utf16(key) {
        return false;
    }
    let is_session = this_storage_kind(scope, storage);
    let owner = storage_owner(scope, storage);
    let previous = with_storage_store(scope, storage, |store, origin| {
        store.get_item_utf16(origin, key)
    })
    .flatten();
    if previous.as_deref() == Some(value) {
        return true;
    }
    let result = with_storage_store(scope, storage, |store, origin| {
        store.try_set_item_utf16(origin, key, value)
    });
    let Some(result) = result else {
        return false;
    };
    let inserted = match result {
        Ok(inserted) => inserted,
        Err(WebStorageMutationError::QuotaExceeded) => {
            throw_storage_quota_exceeded(scope);
            return false;
        }
        Err(WebStorageMutationError::Persistence(message)) => {
            throw_storage_persistence_error(scope, &message);
            return false;
        }
    };
    queue_storage_event(
        scope,
        owner,
        is_session,
        Some(key.to_vec()),
        previous,
        Some(value.to_vec()),
    );
    inserted
}

pub(super) fn storage_remove_utf16(
    scope: &mut v8::PinScope<'_, '_>,
    storage: v8::Local<'_, v8::Object>,
    key: &[u16],
) -> bool {
    if storage_internal_name_utf16(key) {
        return false;
    }
    let is_session = this_storage_kind(scope, storage);
    let owner = storage_owner(scope, storage);
    let previous = with_storage_store(scope, storage, |store, origin| {
        store.get_item_utf16(origin, key)
    })
    .flatten();
    if previous.is_none() {
        return false;
    }
    let result = with_storage_store(scope, storage, |store, origin| {
        store.try_remove_item_utf16(origin, key)
    });
    let Some(result) = result else {
        return false;
    };
    let removed = match result {
        Ok(removed) => removed,
        Err(WebStorageMutationError::QuotaExceeded) => false,
        Err(WebStorageMutationError::Persistence(message)) => {
            throw_storage_persistence_error(scope, &message);
            return false;
        }
    };
    if removed {
        queue_storage_event(scope, owner, is_session, Some(key.to_vec()), previous, None);
    }
    removed
}

pub(super) fn storage_clear(scope: &mut v8::PinScope<'_, '_>, storage: v8::Local<'_, v8::Object>) {
    let is_session = this_storage_kind(scope, storage);
    let owner = storage_owner(scope, storage);
    let had_items =
        with_storage_store(scope, storage, |store, origin| store.len(origin)).unwrap_or(0) > 0;
    let result = with_storage_store(scope, storage, |store, origin| store.try_clear(origin));
    let Some(result) = result else {
        return;
    };
    let cleared = match result {
        Ok(cleared) => cleared,
        Err(WebStorageMutationError::QuotaExceeded) => false,
        Err(WebStorageMutationError::Persistence(message)) => {
            throw_storage_persistence_error(scope, &message);
            return;
        }
    };
    if had_items && cleared {
        queue_storage_event(scope, owner, is_session, None, None, None);
    }
}

fn queue_storage_event(
    scope: &mut v8::PinScope<'_, '_>,
    owner: WebStorageOwner,
    is_session: bool,
    key: Option<Vec<u16>>,
    old_value: Option<Vec<u16>>,
    new_value: Option<Vec<u16>>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let (source_scope, origin, area_key, url) = match owner {
        WebStorageOwner::LightweightPopup(popup_id) => {
            let Some(storage_context) = host.storage_context_for_lightweight_popup(popup_id) else {
                return;
            };
            let Some(document_url) = host.lightweight_popup_document_url(popup_id) else {
                return;
            };
            (
                OwnerDispatchScope::LightweightPopup(popup_id),
                storage_context.origin().to_owned(),
                storage_context.web_storage_area_key().to_owned(),
                document_url.to_string(),
            )
        }
        WebStorageOwner::Child(handle) => {
            let Some(storage_context) = host.storage_context_for_child_browsing_context(handle)
            else {
                return;
            };
            let url = host
                .child_browsing_context_current_url(handle)
                .map(|url| url.to_string())
                .unwrap_or_else(|| host.document_url().to_string());
            (
                OwnerDispatchScope::Child(handle),
                storage_context.origin().to_owned(),
                storage_context.web_storage_area_key().to_owned(),
                url,
            )
        }
        WebStorageOwner::ActiveDocument => {
            let storage_context = host.top_document_storage_context();
            (
                OwnerDispatchScope::Top,
                storage_context.origin().to_owned(),
                storage_context.web_storage_area_key().to_owned(),
                host.document_url().to_string(),
            )
        }
    };
    let Some(source_owner) = host.current_window_execution_context_owner(source_scope) else {
        return;
    };
    let source = WindowTaskTarget::new(source_scope, source_owner);
    let data = RendererPageStorageEventData::new(url, is_session, key, old_value, new_value);
    host.queue_storage_event_deliveries(source, &origin, &area_key, data);
}

pub(super) fn storage_internal_name_utf16(key: &[u16]) -> bool {
    key == STORAGE_INTERNAL_NAME_UNITS
}

pub(super) fn storage_key_is_shadowed_by_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    storage: v8::Local<'_, v8::Object>,
    key: v8::Local<'_, v8::String>,
) -> bool {
    let indexed_key = storage_array_index_key(&key.to_rust_string_lossy(scope));
    let mut prototype = storage.get_prototype(scope);
    while let Some(value) = prototype {
        if value.is_null_or_undefined() {
            return false;
        }
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            return false;
        };
        // Numeric property access reaches Storage through V8's indexed
        // interceptor. Use indexed lookup for the prototype side as well so
        // element-backed properties like Storage.prototype[9] shadow stored
        // items the same way named properties do.
        if let Some(index) = indexed_key
            && (object.has_index(scope, index).unwrap_or(false)
                || storage_prototype_index_descriptor(scope, index).is_some())
        {
            return true;
        }
        if object.has_own_property(scope, key.into()).unwrap_or(false) {
            return true;
        }
        prototype = object.get_prototype(scope);
    }
    false
}

fn storage_array_index_key(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index.to_string() == key).then_some(index)
}

pub(super) fn storage_prototype_index_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let store = storage_prototype_index_descriptor_store(scope, false)?;
    let key = v8_string(scope, &index.to_string())?;
    store
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn set_storage_prototype_index_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    descriptor: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(store) = storage_prototype_index_descriptor_store(scope, true) else {
        return false;
    };
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return false;
    };
    store
        .set(scope, key.into(), descriptor.into())
        .unwrap_or(false)
}

pub(super) fn remove_storage_prototype_index_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
) -> bool {
    let Some(store) = storage_prototype_index_descriptor_store(scope, false) else {
        return true;
    };
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return false;
    };
    store.delete(scope, key.into()).unwrap_or(true)
}

fn storage_prototype_index_descriptor_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    create: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(value) =
        get_private_value(scope, global, STORAGE_PROTOTYPE_INDEXED_DESCRIPTORS_SLOT)
        && let Ok(store) = v8::Local::<v8::Object>::try_from(value)
    {
        return Some(store);
    }
    if !create {
        return None;
    }
    let store = StoragePrototypeIndexDescriptorStoreDeclaration::default()
        .bind(scope)
        .ok()?;
    set_private_value(
        scope,
        global,
        STORAGE_PROTOTYPE_INDEXED_DESCRIPTORS_SLOT,
        store.into(),
    );
    Some(store)
}

fn throw_storage_quota_exceeded(scope: &mut v8::PinScope<'_, '_>) {
    let exception = crate::context_bootstrap::new_quota_exceeded_error_value(
        scope,
        "Failed to execute 'setItem' on 'Storage': QuotaExceededError",
        None,
        None,
    );
    scope.throw_exception(exception);
}

fn throw_storage_persistence_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let message = format!("Failed to persist localStorage data: {message}");
    crate::context_bootstrap::throw_dom_exception_value(scope, &message, "UnknownError");
}

#[cfg(test)]
mod tests {
    #[test]
    fn storage_array_index_key_accepts_canonical_u32_strings() {
        assert_eq!(super::storage_array_index_key("9"), Some(9));
        assert_eq!(super::storage_array_index_key("0"), Some(0));
        assert_eq!(super::storage_array_index_key("09"), None);
        assert_eq!(super::storage_array_index_key("x"), None);
    }
}

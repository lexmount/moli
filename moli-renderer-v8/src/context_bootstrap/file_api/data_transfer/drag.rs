use super::*;
#[cfg(any(test, feature = "devtools"))]
use crate::runtime::RendererDragDataItem;
use crate::runtime::{RendererDragData, RendererDraggedDirectory, RendererDraggedFile};
use moli_file_api::data_transfer::child_entry_full_path;
use moli_file_api::data_transfer::drag_effect_allowed_from_mask;
#[cfg(any(test, feature = "devtools"))]
use moli_file_api::data_transfer::drag_operations_mask_from_effect_allowed;

use moli_webapi::drag_data_values::{
    DataTransferDirectoryItemObjectDeclaration, FileSystemDirectoryEntryObjectDeclaration,
};

pub(super) fn set_data_transfer_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mode: DragDataStoreMode,
) {
    let previous = data_transfer_mode(scope, object);
    if previous == mode || previous == DragDataStoreMode::Disabled {
        return;
    }
    let value = match mode {
        DragDataStoreMode::ReadWrite => 0.0,
        DragDataStoreMode::ReadOnly => 1.0,
        DragDataStoreMode::Protected => 2.0,
        DragDataStoreMode::Disabled => 3.0,
    };
    set_private_number(scope, object, DATA_TRANSFER_MODE_SLOT, value);
    if previous.can_read_types() != mode.can_read_types() {
        set_private_value(
            scope,
            object,
            DATA_TRANSFER_TYPES_SLOT,
            v8::undefined(scope).into(),
        );
    }
    if let Some(items) = get_private_object(scope, object, DATA_TRANSFER_ITEMS_SLOT) {
        refresh_data_transfer_item_views(scope, items);
    }
}

/// Native drag-session data. Its private backing is never a JavaScript
/// DataTransfer; event views have their own wrappers and access lifetime.
pub(crate) struct NativeDragDataStore {
    backing: v8::Global<v8::Object>,
}

impl NativeDragDataStore {
    pub(super) fn local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        v8::Local::new(scope, &self.backing)
    }
}

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::DataTransfer, require_prototype)]
struct DataTransferShellDeclaration {
    #[webapi(slot = DATA_TRANSFER_FILES_SLOT, init = "null")]
    files: (),
    #[webapi(slot = DATA_TRANSFER_ITEMS_SLOT, init = "null")]
    items: (),
    #[webapi(slot = DATA_TRANSFER_TYPES_SLOT, init = "undefined")]
    types: (),
    #[webapi(slot = DATA_TRANSFER_DROP_EFFECT_SLOT, init = string("none"))]
    drop_effect: (),
    #[webapi(slot = DATA_TRANSFER_EFFECT_ALLOWED_SLOT, init = string("uninitialized"))]
    effect_allowed: (),
}

#[cfg(feature = "devtools")]
pub(crate) fn native_data_transfer_has_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
    mime_type: &str,
) -> bool {
    data_transfer_mode(scope, data_transfer).can_read_types()
        && DataTransferItemStore::for_owner(scope, data_transfer)
            .is_some_and(|store| store.contains_string_type(scope, mime_type))
}

#[cfg(feature = "devtools")]
/// Counts file items without exposing their contents in protected mode.
pub(crate) fn native_data_transfer_file_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
) -> usize {
    let Some(items) = get_private_object(scope, data_transfer, DATA_TRANSFER_ITEMS_SLOT)
        .and_then(|list| item_list_array(scope, list))
    else {
        return 0;
    };
    (0..items.length())
        .filter(|index| {
            let Some(value) = items.get_index(scope, *index) else {
                return false;
            };
            let Ok(item) = v8::Local::<v8::Object>::try_from(value) else {
                return false;
            };
            let Some(record) = item_record(scope, item) else {
                return false;
            };
            item_kind(scope, item).as_deref() == Some("file")
                && get_private_object(scope, record, DATA_TRANSFER_ITEM_FILE_SLOT).is_some()
        })
        .count()
}

#[cfg(all(test, feature = "devtools"))]
struct DragDataReadCount(std::cell::Cell<usize>);

#[cfg(all(test, feature = "devtools"))]
pub(crate) fn drag_data_read_count_for_test(scope: &mut v8::PinScope<'_, '_>) -> usize {
    scope
        .get_current_context()
        .get_slot::<DragDataReadCount>()
        .map_or(0, |count| count.0.get())
}

/// Reads the string items and `effectAllowed` that `dragstart` listeners left
/// in the shared store, in the shape CDP reports as `Input.DragData`.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn read_string_drag_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: &NativeDragDataStore,
) -> (Vec<RendererDragDataItem>, i32) {
    #[cfg(all(test, feature = "devtools"))]
    {
        let context = scope.get_current_context();
        let count = context.get_slot::<DragDataReadCount>().unwrap_or_else(|| {
            let count = std::rc::Rc::new(DragDataReadCount(std::cell::Cell::new(0)));
            context.set_slot(count.clone());
            count
        });
        count.0.set(count.0.get() + 1);
    }
    let backing = store.local(scope);
    let mask = drag_operations_mask_from_effect_allowed(
        &private_string_property(scope, backing, DATA_TRANSFER_EFFECT_ALLOWED_SLOT)
            .unwrap_or_else(|| "uninitialized".to_owned()),
    );
    let mut items = Vec::new();
    let Some(item_array) =
        private_property_as_array(scope, backing, DATA_TRANSFER_STORE_ITEMS_SLOT)
    else {
        return (items, mask);
    };
    for index in 0..item_array.length() {
        let Some(item) = item_array
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if item_kind(scope, item).as_deref() != Some("string") {
            continue;
        }
        let mime_type = item_type(scope, item).unwrap_or_default();
        let base_url = private_string_property(scope, item, DATA_TRANSFER_ITEM_BASE_URL_SLOT)
            .or_else(|| (mime_type == "text/html").then(|| "about:blank#blocked".to_owned()));
        items.push(RendererDragDataItem {
            mime_type,
            // Native interception serializes the store, independently of the
            // access granted to the last JavaScript event listener.
            data: private_string_property(scope, item, DATA_TRANSFER_ITEM_STRING_SLOT)
                .unwrap_or_default(),
            title: private_string_property(scope, item, DATA_TRANSFER_ITEM_TITLE_SLOT),
            base_url,
        });
    }
    (items, mask)
}

#[cfg(any(test, feature = "devtools"))]
fn data_transfer_effect_allowed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_property(scope, object, DATA_TRANSFER_EFFECT_ALLOWED_SLOT)
}

pub(crate) fn create_native_drag_data_store(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<NativeDragDataStore> {
    let backing = new_drag_data_store_backing(scope, "uninitialized");
    Some(NativeDragDataStore {
        backing: v8::Global::new(scope, backing),
    })
}

pub(crate) fn create_external_drag_data_store(
    scope: &mut v8::PinScope<'_, '_>,
    data: &RendererDragData,
) -> Option<NativeDragDataStore> {
    let store = create_native_drag_data_store(scope)?;
    populate_native_drag_data_store(scope, &store, data)?;
    update_drag_data_store_operations_mask(scope, &store, data.drag_operations_mask);
    Some(store)
}

pub(crate) fn create_drag_data_transfer_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: &NativeDragDataStore,
    mode: DragDataStoreMode,
    drop_effect: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner = DataTransferShellDeclaration::default().bind(scope).ok()?;
    let backing = store.local(scope);
    initialize_data_transfer_object(scope, owner, backing, mode, drop_effect)?;
    Some(owner)
}

#[cfg(any(test, feature = "devtools"))]
pub(crate) fn finish_drag_data_transfer_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) -> String {
    let effect = native_data_transfer_drop_effect(scope, view);
    set_data_transfer_mode(scope, view, DragDataStoreMode::Disabled);
    effect
}

/// The source keeps readable item metadata after dragstart, without regaining
/// access to payloads or allowing a finished view to become active again.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn protect_native_drag_source_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) {
    if data_transfer_mode(scope, view) == DragDataStoreMode::ReadWrite {
        set_data_transfer_mode(scope, view, DragDataStoreMode::Protected);
    }
}

/// Editing events retain readable data independently of the drag views' access
/// lifetime and later mutations of their source item records.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn readonly_data_transfer_for_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !data_transfer_mode(scope, source).can_read_data() {
        return None;
    }
    let source_list = get_private_object(scope, source, DATA_TRANSFER_ITEMS_SLOT)?;
    let source_items = item_list_array(scope, source_list)?;
    let effect = native_data_transfer_effect_allowed(scope, source);
    let backing = new_drag_data_store_backing(scope, &effect);
    let records = private_property_as_array(scope, backing, DATA_TRANSFER_STORE_ITEMS_SLOT)?;
    for index in 0..source_items.length() {
        let Some(item) = source_items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        let mime_type = item_type(scope, item).unwrap_or_default();
        let copy = if let Some(text) = item_string_value(scope, item) {
            build_data_transfer_item_for_string(scope, &mime_type, &text)
        } else if let Some(entry) = item_entry_object(scope, item) {
            let record = crate::util::new_null_prototype_object(scope);
            if let Some(file) = item_file_object(scope, item) {
                DataTransferFileItemObjectDeclaration::new(&mime_type, file, entry)
                    .initialize(scope, record)
                    .ok()?;
            } else {
                DataTransferDirectoryItemObjectDeclaration::new(entry)
                    .initialize(scope, record)
                    .ok()?;
            }
            Some(record)
        } else {
            None
        };
        if let Some(copy) = copy {
            append_item(scope, records, copy)?;
        }
    }
    let owner = DataTransferShellDeclaration::default().bind(scope).ok()?;
    initialize_data_transfer_object(scope, owner, backing, DragDataStoreMode::ReadOnly, "none")?;
    Some(owner)
}

fn append_drag_data_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    records: v8::Local<'s, v8::Array>,
    data: &RendererDragData,
) -> Option<()> {
    for item in &data.items {
        let record = append_string_item(
            scope,
            records,
            &normalize_drag_data_type(&item.mime_type),
            &item.data,
        )?;
        if let Some(title) = &item.title {
            set_private_string(scope, record, DATA_TRANSFER_ITEM_TITLE_SLOT, title);
        }
        if let Some(base_url) = &item.base_url {
            set_private_string(scope, record, DATA_TRANSFER_ITEM_BASE_URL_SLOT, base_url);
        }
    }
    for file in &data.files {
        append_file_item(scope, records, file)?;
    }
    for directory in &data.directories {
        append_directory_item(scope, records, directory)?;
    }
    Some(())
}

/// Native session writes happen between event scopes, never by reviving an old
/// JavaScript DataTransfer view.
pub(crate) fn populate_native_drag_data_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: &NativeDragDataStore,
    data: &RendererDragData,
) -> Option<()> {
    let backing = store.local(scope);
    let records = private_property_as_array(scope, backing, DATA_TRANSFER_STORE_ITEMS_SLOT)?;
    append_drag_data_records(scope, records, data)
}

#[cfg(feature = "devtools")]
pub(crate) fn replace_drag_data_store_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: &NativeDragDataStore,
    data: &RendererDragData,
) -> Option<()> {
    let backing = store.local(scope);
    let next = v8::Array::new(scope, 0);
    append_drag_data_records(scope, next, data)?;
    // Existing source views retain their original item metadata. Target views
    // have their own lifetime and cannot read these records after they finish.
    set_private_value(scope, backing, DATA_TRANSFER_STORE_ITEMS_SLOT, next.into());
    update_drag_data_store_operations_mask(scope, store, data.drag_operations_mask);
    Some(())
}

pub(crate) fn update_drag_data_store_operations_mask(
    scope: &mut v8::PinScope<'_, '_>,
    store: &NativeDragDataStore,
    mask: i32,
) {
    let backing = store.local(scope);
    set_private_string(
        scope,
        backing,
        DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        drag_effect_allowed_from_mask(mask),
    );
}

#[cfg(any(test, feature = "devtools"))]
pub(crate) fn native_data_transfer_effect_allowed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) -> String {
    data_transfer_effect_allowed(scope, view).unwrap_or_else(|| "none".to_owned())
}

#[cfg(any(test, feature = "devtools"))]
pub(crate) fn native_data_transfer_drop_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) -> String {
    private_string_property(scope, view, DATA_TRANSFER_DROP_EFFECT_SLOT)
        .unwrap_or_else(|| "none".to_owned())
}

fn build_dragged_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: &RendererDraggedFile,
) -> Option<v8::Local<'s, v8::Object>> {
    let object =
        moli_webapi::file::file_object_with_metadata(scope, &file.name, file.last_modified)?;
    moli_webapi::blob::init_blob_object(scope, object, file.bytes.clone(), file.mime_type.clone());
    Some(object)
}

fn build_data_transfer_item_for_directory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    directory: &RendererDraggedDirectory,
) -> Option<v8::Local<'s, v8::Object>> {
    let entry = build_file_system_directory_entry(scope, directory, None)?;
    let record = crate::util::new_null_prototype_object(scope);
    DataTransferDirectoryItemObjectDeclaration::new(entry)
        .initialize(scope, record)
        .ok()?;
    Some(record)
}

fn build_file_system_directory_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    directory: &RendererDraggedDirectory,
    parent_path: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let full_path = child_entry_full_path(parent_path, &directory.name);
    let filesystem = build_file_system_object(scope)?;
    let child_entries = build_file_system_directory_child_entries(scope, directory, &full_path)?;
    FileSystemDirectoryEntryObjectDeclaration::new(
        filesystem,
        &full_path,
        &directory.name,
        child_entries,
    )
    .bind(scope)
    .ok()
}

fn build_file_system_directory_child_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    directory: &RendererDraggedDirectory,
    directory_path: &str,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let mut entries = Vec::new();
    for file in &directory.files {
        let file_object = build_dragged_file_object(scope, file)?;
        let full_path = child_entry_full_path(Some(directory_path), &file.name);
        let entry = build_file_system_file_entry_with_full_path(scope, file_object, &full_path)?;
        entries.push(entry);
    }
    for child_directory in &directory.directories {
        let entry =
            build_file_system_directory_entry(scope, child_directory, Some(directory_path))?;
        entries.push(entry);
    }
    Some(entries)
}

fn append_file_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_array: v8::Local<'s, v8::Array>,
    file: &RendererDraggedFile,
) -> Option<v8::Local<'s, v8::Object>> {
    let file_object = build_dragged_file_object(scope, file)?;
    let item = build_data_transfer_item_for_file(scope, file_object)?;
    append_item(scope, item_array, item)
}

fn append_directory_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_array: v8::Local<'s, v8::Array>,
    directory: &RendererDraggedDirectory,
) -> Option<v8::Local<'s, v8::Object>> {
    let item = build_data_transfer_item_for_directory(scope, directory)?;
    append_item(scope, item_array, item)
}

#[cfg(any(test, feature = "devtools"))]
pub(crate) fn native_data_transfer_files<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    if !data_transfer_mode(scope, view).can_read_data() {
        return Vec::new();
    }
    let Some(items) = get_private_object(scope, view, DATA_TRANSFER_ITEMS_SLOT)
        .and_then(|list| item_list_array(scope, list))
    else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for index in 0..items.length() {
        if let Some(item) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && let Some(file) = item_file_object(scope, item)
        {
            files.push(file);
        }
    }
    files
}

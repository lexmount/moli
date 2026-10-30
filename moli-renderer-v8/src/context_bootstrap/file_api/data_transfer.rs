use super::*;
use crate::blob;
use crate::native_bridge::throw_dom_exception;
use crate::runtime::{
    RendererDragData, RendererDragDataItem, RendererDraggedDirectory, RendererDraggedFile,
};
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_file_api::data_transfer::{
    DataTransferItemSummary, child_entry_full_path, clear_data_removes_item,
    contains_string_item_type, data_transfer_types_from_items, drag_effect_allowed_from_mask,
    drag_operations_mask_from_effect_allowed, normalize_drag_data_type, valid_drop_effect,
    valid_effect_allowed,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub(crate) use moli_file_api::data_transfer::DragDataStoreMode;

mod clipboard;
pub(crate) use clipboard::{
    build_clipboard_data_transfer, clipboard_data_transfer_contents,
    disable_clipboard_data_transfer,
};

const DATA_TRANSFER_FILES_SLOT: &str = "__lmDataTransferFiles";
const DATA_TRANSFER_ITEMS_SLOT: &str = "__lmDataTransferItems";
const DATA_TRANSFER_TYPES_SLOT: &str = "__lmDataTransferTypes";
const DATA_TRANSFER_DROP_EFFECT_SLOT: &str = "__lmDataTransferDropEffect";
const DATA_TRANSFER_EFFECT_ALLOWED_SLOT: &str = "__lmDataTransferEffectAllowed";
const DATA_TRANSFER_STORE_SLOT: &str = "__lmDataTransferStore";
const DATA_TRANSFER_STORE_ITEMS_SLOT: &str = "__lmDragDataStoreItems";
const DATA_TRANSFER_CLIPBOARD_SLOT: &str = "__lmDataTransferClipboard";
const DATA_TRANSFER_MODE_SLOT: &str = "__lmDataTransferMode";
const DATA_TRANSFER_ITEM_OWNER_SLOT: &str = "__lmDataTransferItemOwner";
const DATA_TRANSFER_ITEM_RECORD_SLOT: &str = "__lmDataTransferItemRecord";
const DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT: &str = "__lmDataTransferItemArray";
const DATA_TRANSFER_ITEM_LIST_OWNER_SLOT: &str = "__lmDataTransferOwner";
const DATA_TRANSFER_ITEM_LIST_INDEXED_LENGTH_SLOT: &str = "__lmDataTransferItemListIndexedLength";
const DATA_TRANSFER_ITEM_KIND_SLOT: &str = "__lmDataTransferItemKind";
const DATA_TRANSFER_ITEM_TYPE_SLOT: &str = "__lmDataTransferItemType";
const DATA_TRANSFER_ITEM_FILE_SLOT: &str = "__lmDataTransferItemFile";
const DATA_TRANSFER_ITEM_ENTRY_SLOT: &str = "__lmDataTransferItemEntry";
const DATA_TRANSFER_ITEM_STRING_SLOT: &str = "__lmDataTransferItemString";
const DATA_TRANSFER_ITEM_TITLE_SLOT: &str = "__lmDataTransferItemTitle";
const DATA_TRANSFER_ITEM_BASE_URL_SLOT: &str = "__lmDataTransferItemBaseURL";
const FILE_SYSTEM_ENTRY_FILESYSTEM_SLOT: &str = "__lmFileSystemEntryFilesystem";
const FILE_SYSTEM_ENTRY_FULL_PATH_SLOT: &str = "__lmFileSystemEntryFullPath";
const FILE_SYSTEM_ENTRY_IS_DIRECTORY_SLOT: &str = "__lmFileSystemEntryIsDirectory";
const FILE_SYSTEM_ENTRY_IS_FILE_SLOT: &str = "__lmFileSystemEntryIsFile";
const FILE_SYSTEM_ENTRY_NAME_SLOT: &str = "__lmFileSystemEntryName";
const FILE_SYSTEM_FILE_ENTRY_FILE_SLOT: &str = "__lmFileSystemFileEntryFile";
const FILE_SYSTEM_DIRECTORY_ENTRY_ENTRIES_SLOT: &str = "__lmFileSystemDirectoryEntryEntries";
pub(super) const FILE_SYSTEM_DIRECTORY_READER_ENTRIES_SLOT: &str =
    "__lmFileSystemDirectoryReaderEntries";
pub(super) const FILE_SYSTEM_DIRECTORY_READER_OFFSET_SLOT: &str =
    "__lmFileSystemDirectoryReaderOffset";
pub(super) const FILE_SYSTEM_DIRECTORY_READER_ACTIVE_REQUEST_SLOT: &str =
    "__lmFileSystemDirectoryReaderActiveRequest";
pub(super) const FILE_SYSTEM_DIRECTORY_READER_DONE_SLOT: &str = "__lmFileSystemDirectoryReaderDone";
pub(super) const FILE_SYSTEM_DIRECTORY_READER_ERROR_SLOT: &str =
    "__lmFileSystemDirectoryReaderError";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DataTransfer, require_prototype)]
struct DataTransferObjectDeclaration<'s> {
    #[webapi(slot = DATA_TRANSFER_FILES_SLOT)]
    files: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA_TRANSFER_ITEMS_SLOT)]
    items: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA_TRANSFER_STORE_SLOT)]
    store: v8::Local<'s, v8::Object>,
    // `undefined` means that no FrozenArray snapshot has been published for the current
    // item-list generation. The getter replaces it with the cached snapshot on first read.
    #[webapi(slot = DATA_TRANSFER_TYPES_SLOT, init = "undefined")]
    types: (),
    #[webapi(slot = DATA_TRANSFER_DROP_EFFECT_SLOT, constructor_default = "none")]
    drop_effect: &'static str,
    #[webapi(slot = DATA_TRANSFER_EFFECT_ALLOWED_SLOT, constructor_default = "none")]
    effect_allowed: &'static str,
    #[webapi(slot = DATA_TRANSFER_MODE_SLOT, constructor_default = 0.0)]
    mode: f64,
    #[webapi(slot = DATA_TRANSFER_CLIPBOARD_SLOT, init = false)]
    clipboard: (),
}

/// Native drag-session data. Its private backing is never a JavaScript
/// DataTransfer; event views have their own wrappers and access lifetime.
pub(crate) struct NativeDragDataStore {
    backing: v8::Global<v8::Object>,
}

impl NativeDragDataStore {
    fn local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        v8::Local::new(scope, &self.backing)
    }
}

pub(crate) fn is_branded_data_transfer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::DataTransfer::is_instance(scope, object)
}

fn data_transfer_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> DragDataStoreMode {
    match private_number_property(scope, object, DATA_TRANSFER_MODE_SLOT) {
        Some(1.0) => DragDataStoreMode::ReadOnly,
        Some(2.0) => DragDataStoreMode::Protected,
        Some(3.0) => DragDataStoreMode::Disabled,
        _ => DragDataStoreMode::ReadWrite,
    }
}

fn set_data_transfer_mode<'s>(
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

fn item_data_transfer_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> DragDataStoreMode {
    if let Some(owner) = get_private_object(scope, item, DATA_TRANSFER_ITEM_OWNER_SLOT) {
        return data_transfer_mode(scope, owner);
    }
    if web_api_interfaces::DataTransferItem::is_instance(scope, item) {
        DragDataStoreMode::Disabled
    } else {
        DragDataStoreMode::ReadWrite
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

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DataTransfer, receiver)]
struct DataTransferPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = data_transfer_files_getter, enumerable)]
    files: (),
    #[webapi(accessor_property, getter = data_transfer_items_getter, enumerable)]
    items: (),
    #[webapi(accessor_property, getter = data_transfer_types_getter, enumerable)]
    types: (),
    #[webapi(
        accessor_property = "dropEffect",
        getter = data_transfer_drop_effect_getter,
        setter = data_transfer_drop_effect_setter,
        enumerable
    )]
    drop_effect: (),
    #[webapi(
        accessor_property = "effectAllowed",
        getter = data_transfer_effect_allowed_getter,
        setter = data_transfer_effect_allowed_setter,
        enumerable
    )]
    effect_allowed: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DataTransferItemList, require_prototype)]
struct DataTransferItemListObjectDeclaration<'s> {
    #[webapi(
        slot = DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT,
        constructor_default = Vec::new()
    )]
    items: Vec<v8::Local<'s, v8::Value>>,
    #[webapi(slot = DATA_TRANSFER_ITEM_LIST_OWNER_SLOT)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(
        slot = DATA_TRANSFER_ITEM_LIST_INDEXED_LENGTH_SLOT,
        constructor_default = 0.0
    )]
    indexed_length: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DataTransferItemList, receiver)]
struct DataTransferItemListPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = data_transfer_item_list_length_getter, enumerable)]
    length: (),
}

#[derive(WebApiObject)]
#[webapi(plain, scope_lifetime = 's)]
struct DataTransferFileItemObjectDeclaration<'s, 'item_type> {
    #[webapi(slot = DATA_TRANSFER_ITEM_KIND_SLOT, constructor_default = "file")]
    kind: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_TYPE_SLOT)]
    item_type: &'item_type str,
    #[webapi(slot = DATA_TRANSFER_ITEM_FILE_SLOT)]
    file: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA_TRANSFER_ITEM_ENTRY_SLOT)]
    entry: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DataTransferDirectoryItemObjectDeclaration<'s> {
    #[webapi(slot = DATA_TRANSFER_ITEM_KIND_SLOT, constructor_default = "file")]
    kind: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_TYPE_SLOT, constructor_default = "")]
    item_type: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_ENTRY_SLOT)]
    entry: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DataTransferStringItemObjectDeclaration<'item_type, 'data> {
    #[webapi(slot = DATA_TRANSFER_ITEM_KIND_SLOT, constructor_default = "string")]
    kind: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_TYPE_SLOT)]
    item_type: &'item_type str,
    #[webapi(slot = DATA_TRANSFER_ITEM_STRING_SLOT)]
    data: &'data str,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DataTransferItem, require_prototype)]
struct DataTransferItemViewDeclaration<'s> {
    #[webapi(slot = DATA_TRANSFER_ITEM_OWNER_SLOT)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA_TRANSFER_ITEM_RECORD_SLOT)]
    record: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DataTransferItem, receiver)]
struct DataTransferItemPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = data_transfer_item_kind_getter, enumerable)]
    kind: (),
    #[webapi(accessor_property = "type", getter = data_transfer_item_type_getter, enumerable)]
    item_type: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FileSystem, require_prototype)]
struct FileSystemObjectDeclaration<'s> {
    #[webapi(data_property, constructor_default = "")]
    name: &'static str,
    #[webapi(data_property)]
    root: v8::Local<'s, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::FileSystemFileEntry,
    scope_lifetime = 's,
    require_prototype
)]
struct FileSystemFileEntryObjectDeclaration<'s, 'full_path, 'name> {
    #[webapi(slot = FILE_SYSTEM_ENTRY_FILESYSTEM_SLOT)]
    filesystem: v8::Local<'s, v8::Object>,
    #[webapi(slot = FILE_SYSTEM_ENTRY_FULL_PATH_SLOT)]
    full_path: &'full_path str,
    #[webapi(slot = FILE_SYSTEM_ENTRY_IS_DIRECTORY_SLOT, constructor_default = false)]
    is_directory: bool,
    #[webapi(slot = FILE_SYSTEM_ENTRY_IS_FILE_SLOT, constructor_default = true)]
    is_file: bool,
    #[webapi(slot = FILE_SYSTEM_ENTRY_NAME_SLOT)]
    name: &'name str,
    #[webapi(slot = FILE_SYSTEM_FILE_ENTRY_FILE_SLOT)]
    file: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::FileSystemDirectoryEntry,
    scope_lifetime = 's,
    require_prototype
)]
struct FileSystemDirectoryEntryObjectDeclaration<'s, 'full_path, 'name> {
    #[webapi(slot = FILE_SYSTEM_ENTRY_FILESYSTEM_SLOT)]
    filesystem: v8::Local<'s, v8::Object>,
    #[webapi(slot = FILE_SYSTEM_ENTRY_FULL_PATH_SLOT)]
    full_path: &'full_path str,
    #[webapi(slot = FILE_SYSTEM_ENTRY_IS_DIRECTORY_SLOT, constructor_default = true)]
    is_directory: bool,
    #[webapi(slot = FILE_SYSTEM_ENTRY_IS_FILE_SLOT, constructor_default = false)]
    is_file: bool,
    #[webapi(slot = FILE_SYSTEM_ENTRY_NAME_SLOT)]
    name: &'name str,
    #[webapi(slot = FILE_SYSTEM_DIRECTORY_ENTRY_ENTRIES_SLOT)]
    entries: Vec<v8::Local<'s, v8::Object>>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FileSystemEntry)]
struct FileSystemEntryPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = file_system_entry_filesystem_getter, enumerable)]
    filesystem: (),
    #[webapi(
        accessor_property = "fullPath",
        getter = file_system_entry_full_path_getter,
        enumerable
    )]
    full_path: (),
    #[webapi(
        accessor_property = "isDirectory",
        getter = file_system_entry_is_directory_getter,
        enumerable
    )]
    is_directory: (),
    #[webapi(
        accessor_property = "isFile",
        getter = file_system_entry_is_file_getter,
        enumerable
    )]
    is_file: (),
    #[webapi(accessor_property, getter = file_system_entry_name_getter, enumerable)]
    name: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FileSystemDirectoryReader, require_prototype)]
struct FileSystemDirectoryReaderObjectDeclaration<'s> {
    #[webapi(slot = FILE_SYSTEM_DIRECTORY_READER_ENTRIES_SLOT)]
    entries: v8::Local<'s, v8::Array>,
    #[webapi(slot = FILE_SYSTEM_DIRECTORY_READER_OFFSET_SLOT, constructor_default = 0.0)]
    offset: f64,
    #[webapi(slot = FILE_SYSTEM_DIRECTORY_READER_ACTIVE_REQUEST_SLOT)]
    active_request: v8::Local<'s, v8::Value>,
    #[webapi(slot = FILE_SYSTEM_DIRECTORY_READER_DONE_SLOT, constructor_default = false)]
    done: bool,
    #[webapi(slot = FILE_SYSTEM_DIRECTORY_READER_ERROR_SLOT)]
    error: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransfer.getData")]
struct DataTransferGetDataArgs {
    #[webidl(required, name = "format")]
    format: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransfer.setData")]
struct DataTransferSetDataArgs {
    #[webidl(required, name = "format")]
    format: String,
    #[webidl(required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransfer.clearData")]
struct DataTransferClearDataArgs {
    #[webidl(index = 0)]
    format: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransferItemList.add")]
struct DataTransferItemListAddArgs<'s> {
    #[webidl(index = 0, converter = "raw")]
    data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(index = 1)]
    item_type: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransferItemList.item")]
struct DataTransferItemListItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransferItemList.remove")]
struct DataTransferItemListRemoveArgs {
    #[webidl(required)]
    index: u32,
}

pub(super) fn install_data_transfer_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "DataTransfer" => {
            DataTransferPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "DataTransferItem" => {
            DataTransferItemPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "DataTransferItemList" => {
            DataTransferItemListPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "FileSystemEntry" => {
            FileSystemEntryPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(crate) fn native_data_transfer_has_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
    mime_type: &str,
) -> bool {
    data_transfer_mode(scope, data_transfer).can_read_types()
        && DataTransferItemStore::for_owner(scope, data_transfer)
            .is_some_and(|store| store.contains_string_type(scope, mime_type))
}

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

pub(crate) fn native_set_drag_drop_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
    effect: &str,
) {
    if valid_drop_effect(effect) {
        set_private_string(scope, data_transfer, DATA_TRANSFER_DROP_EFFECT_SLOT, effect);
    }
}

/// Reads the string items and `effectAllowed` that `dragstart` listeners left
/// in the shared store, in the shape CDP reports as `Input.DragData`.
pub(crate) fn read_string_drag_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: &NativeDragDataStore,
) -> (Vec<RendererDragDataItem>, i32) {
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

fn private_string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn private_number_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, object, slot)?.number_value(scope)
}

fn private_bool_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    get_private_value(scope, object, slot).map(|value| value.boolean_value(scope))
}

fn private_property_as_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn set_private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: &str,
) {
    let Some(value) = v8_string(scope, value) else {
        return;
    };
    set_private_value(scope, object, slot, value.into());
}

fn set_private_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

fn data_transfer_effect_allowed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_property(scope, object, DATA_TRANSFER_EFFECT_ALLOWED_SLOT)
}

fn item_list_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    private_property_as_array(scope, item_list, DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT)
}

fn item_list_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, item_list, DATA_TRANSFER_ITEM_LIST_OWNER_SLOT)
}

fn item_list_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let owner = item_list_owner(scope, item_list)?;
    let store = get_private_object(scope, owner, DATA_TRANSFER_STORE_SLOT)?;
    private_property_as_array(scope, store, DATA_TRANSFER_STORE_ITEMS_SLOT)
}

fn set_item_list_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_list: v8::Local<'s, v8::Object>,
    records: v8::Local<'s, v8::Array>,
) {
    if let Some(owner) = item_list_owner(scope, item_list)
        && let Some(store) = get_private_object(scope, owner, DATA_TRANSFER_STORE_SLOT)
    {
        set_private_value(scope, store, DATA_TRANSFER_STORE_ITEMS_SLOT, records.into());
    }
}

fn item_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::DataTransferItem::is_instance(scope, item) {
        get_private_object(scope, item, DATA_TRANSFER_ITEM_RECORD_SLOT)
    } else {
        Some(item)
    }
}

fn detach_data_transfer_item_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) {
    let null = v8::null(scope);
    set_private_value(scope, item, DATA_TRANSFER_ITEM_RECORD_SLOT, null.into());
    set_private_value(scope, item, DATA_TRANSFER_ITEM_OWNER_SLOT, null.into());
}

// JavaScript writes to shared records run inside DataTransferItemStore::mutate;
// its one observer refreshes only this view and its cached `types`. Native drag
// commands update the store between event scopes, before creating a new view.
enum DataTransferItemListMutation<T> {
    Unchanged(T),
    Changed(T),
}

#[derive(Clone, Copy)]
struct DataTransferItemStore<'s> {
    item_list: v8::Local<'s, v8::Object>,
}

impl<'s> DataTransferItemStore<'s> {
    fn for_owner(
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        get_private_object(scope, owner, DATA_TRANSFER_ITEMS_SLOT).map(Self::new)
    }

    fn new(item_list: v8::Local<'s, v8::Object>) -> Self {
        Self { item_list }
    }

    fn mutate<'a, T>(
        self,
        scope: &mut v8::PinScope<'s, 'a>,
        mutation: impl FnOnce(
            &mut v8::PinScope<'s, 'a>,
            v8::Local<'s, v8::Array>,
        ) -> DataTransferItemListMutation<T>,
    ) -> Option<T> {
        let owner = item_list_owner(scope, self.item_list)?;
        if !data_transfer_mode(scope, owner).can_write_data() {
            return None;
        }
        let item_array = item_list_records(scope, self.item_list)?;
        let (value, changed) = match mutation(scope, item_array) {
            DataTransferItemListMutation::Unchanged(value) => (value, false),
            DataTransferItemListMutation::Changed(value) => (value, true),
        };
        if changed {
            data_transfer_item_list_did_change(scope, self.item_list);
        }
        Some(value)
    }

    fn contains_string_type(self, scope: &mut v8::PinScope<'s, '_>, mime_type: &str) -> bool {
        item_list_records(scope, self.item_list).is_some_and(|item_array| {
            contains_string_item_type(&item_summaries_from_array(scope, item_array), mime_type)
        })
    }

    fn append(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        record: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.mutate(scope, |scope, item_array| {
            if append_item(scope, item_array, record).is_some() {
                DataTransferItemListMutation::Changed(true)
            } else {
                DataTransferItemListMutation::Unchanged(false)
            }
        })?
        .then_some(())?;
        let views = item_list_array(scope, self.item_list)?;
        find_item_view(scope, views, record)
    }

    fn set_string_data(self, scope: &mut v8::PinScope<'s, '_>, mime_type: &str, data: &str) {
        let _ = self.mutate(scope, |scope, item_array| {
            for index in 0..item_array.length() {
                let Some(existing) = item_array.get_index(scope, index) else {
                    continue;
                };
                let Ok(existing) = v8::Local::<v8::Object>::try_from(existing) else {
                    continue;
                };
                if item_kind(scope, existing).as_deref() != Some("string")
                    || item_type(scope, existing).as_deref() != Some(mime_type)
                {
                    continue;
                }
                set_private_string(scope, existing, DATA_TRANSFER_ITEM_STRING_SLOT, data);
                // Author replacement supplies a new string, not the native
                // source's URL label or HTML provenance.
                for slot in [
                    DATA_TRANSFER_ITEM_TITLE_SLOT,
                    DATA_TRANSFER_ITEM_BASE_URL_SLOT,
                ] {
                    set_private_value(scope, existing, slot, v8::undefined(scope).into());
                }
                return DataTransferItemListMutation::Changed(());
            }

            if append_string_item(scope, item_array, mime_type, data).is_some() {
                DataTransferItemListMutation::Changed(())
            } else {
                DataTransferItemListMutation::Unchanged(())
            }
        });
    }

    fn clear_string_data(self, scope: &mut v8::PinScope<'s, '_>, target_type: Option<&str>) {
        let _ = self.mutate(scope, |scope, item_array| {
            let next = v8::Array::new(scope, 0);
            let mut next_index = 0u32;
            let mut removed_item = false;
            for index in 0..item_array.length() {
                let Some(value) = item_array.get_index(scope, index) else {
                    continue;
                };
                let Ok(item) = v8::Local::<v8::Object>::try_from(value) else {
                    continue;
                };
                if clear_data_removes_item(&item_summary(scope, item), target_type) {
                    disable_data_transfer_item(scope, item);
                    removed_item = true;
                    continue;
                }
                define_index_data_property(scope, next.into(), next_index, item.into());
                next_index += 1;
            }
            if !removed_item {
                return DataTransferItemListMutation::Unchanged(());
            }
            set_item_list_records(scope, self.item_list, next);
            DataTransferItemListMutation::Changed(())
        });
    }

    fn remove(self, scope: &mut v8::PinScope<'s, '_>, index: u32) {
        let _ = self.mutate(scope, |scope, item_array| {
            if index >= item_array.length() {
                return DataTransferItemListMutation::Unchanged(());
            }

            let next = v8::Array::new(scope, 0);
            let mut next_index = 0u32;
            for current in 0..item_array.length() {
                let Some(value) = item_array.get_index(scope, current) else {
                    continue;
                };
                if current == index {
                    if let Ok(item) = v8::Local::<v8::Object>::try_from(value) {
                        disable_data_transfer_item(scope, item);
                    }
                    continue;
                }
                define_index_data_property(scope, next.into(), next_index, value);
                next_index += 1;
            }
            set_item_list_records(scope, self.item_list, next);
            DataTransferItemListMutation::Changed(())
        });
    }

    fn clear(self, scope: &mut v8::PinScope<'s, '_>) {
        let _ = self.mutate(scope, |scope, item_array| {
            if item_array.length() == 0 {
                return DataTransferItemListMutation::Unchanged(());
            }
            for index in 0..item_array.length() {
                let Some(value) = item_array.get_index(scope, index) else {
                    continue;
                };
                if let Ok(item) = v8::Local::<v8::Object>::try_from(value) {
                    disable_data_transfer_item(scope, item);
                }
            }
            set_item_list_records(scope, self.item_list, v8::Array::new(scope, 0));
            DataTransferItemListMutation::Changed(())
        });
    }
}

pub(super) fn item_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !item_data_transfer_mode(scope, item).can_read_types() {
        return None;
    }
    let record = item_record(scope, item)?;
    private_string_property(scope, record, DATA_TRANSFER_ITEM_KIND_SLOT)
}

fn item_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !item_data_transfer_mode(scope, item).can_read_types() {
        return None;
    }
    let record = item_record(scope, item)?;
    private_string_property(scope, record, DATA_TRANSFER_ITEM_TYPE_SLOT)
}

pub(super) fn item_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !item_data_transfer_mode(scope, item).can_read_data() {
        return None;
    }
    let record = item_record(scope, item)?;
    private_string_property(scope, record, DATA_TRANSFER_ITEM_STRING_SLOT)
}

fn item_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !item_data_transfer_mode(scope, item).can_read_data() {
        return None;
    }
    let record = item_record(scope, item)?;
    let file = get_private_object(scope, record, DATA_TRANSFER_ITEM_FILE_SLOT)?;
    let file = crate::context_bootstrap::world_wrappers::owner(scope, file);
    let context = item.get_creation_context(scope)?;
    if file.get_creation_context(scope) == Some(context) {
        return Some(file);
    }
    if let Some(wrapper) =
        crate::context_bootstrap::world_wrappers::get_in_context(scope, file, context)
    {
        return Some(wrapper);
    }
    // Keep the same Blob identity and payload; only its File wrapper and
    // metadata belong to this item view's receiving Window.
    let scope = &mut v8::ContextScope::new(scope, context);
    let wrapper = blob::world_wrapper(scope, file)?;
    super::file::bind_file_world_wrapper(scope, file, wrapper)?;
    crate::context_bootstrap::world_wrappers::insert(scope, file, wrapper);
    Some(wrapper)
}

fn item_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !item_data_transfer_mode(scope, item).can_read_data() {
        return None;
    }
    let record = item_record(scope, item)?;
    get_private_object(scope, record, DATA_TRANSFER_ITEM_ENTRY_SLOT)
}

fn item_summary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> DataTransferItemSummary {
    DataTransferItemSummary::new(
        item_kind(scope, item).unwrap_or_default(),
        item_type(scope, item).unwrap_or_default(),
    )
}

fn disable_data_transfer_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) {
    set_private_string(scope, item, DATA_TRANSFER_ITEM_KIND_SLOT, "");
    set_private_string(scope, item, DATA_TRANSFER_ITEM_TYPE_SLOT, "");
    set_private_string(scope, item, DATA_TRANSFER_ITEM_STRING_SLOT, "");
    let null = v8::null(scope);
    set_private_value(scope, item, DATA_TRANSFER_ITEM_FILE_SLOT, null.into());
    set_private_value(scope, item, DATA_TRANSFER_ITEM_ENTRY_SLOT, null.into());
}

fn item_summaries_from_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_array: v8::Local<'s, v8::Array>,
) -> Vec<DataTransferItemSummary> {
    let mut summaries = Vec::with_capacity(item_array.length() as usize);
    for index in 0..item_array.length() {
        let Some(value) = item_array.get_index(scope, index) else {
            continue;
        };
        let Ok(item) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        summaries.push(item_summary(scope, item));
    }
    summaries
}

fn new_drag_data_store_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    effect_allowed: &str,
) -> v8::Local<'s, v8::Object> {
    let backing = crate::util::new_null_prototype_object(scope);
    set_private_value(
        scope,
        backing,
        DATA_TRANSFER_STORE_ITEMS_SLOT,
        v8::Array::new(scope, 0).into(),
    );
    set_private_string(
        scope,
        backing,
        DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        effect_allowed,
    );
    backing
}

fn initialize_data_transfer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    backing: v8::Local<'s, v8::Object>,
    mode: DragDataStoreMode,
    drop_effect: &str,
) -> Option<()> {
    let items = build_data_transfer_item_list_object(scope, owner)?;
    let empty_files = build_file_list_object(scope, &[])?;
    DataTransferObjectDeclaration::new(empty_files, items, backing)
        .initialize(scope, owner)
        .ok()?;
    let effect_allowed = private_string_property(scope, backing, DATA_TRANSFER_EFFECT_ALLOWED_SLOT)
        .unwrap_or_else(|| "none".to_owned());
    set_private_string(
        scope,
        owner,
        DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        &effect_allowed,
    );
    native_set_drag_drop_effect(scope, owner, drop_effect);
    let value = match mode {
        DragDataStoreMode::ReadWrite => 0.0,
        DragDataStoreMode::ReadOnly => 1.0,
        DragDataStoreMode::Protected => 2.0,
        DragDataStoreMode::Disabled => 3.0,
    };
    set_private_number(scope, owner, DATA_TRANSFER_MODE_SLOT, value);
    refresh_data_transfer_item_views(scope, items);
    Some(())
}

pub(in crate::context_bootstrap) fn data_transfer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DataTransfer': Please use the 'new' operator.",
        );
        return;
    }
    let backing = new_drag_data_store_backing(scope, "none");
    if initialize_data_transfer_object(
        scope,
        args.this(),
        backing,
        DragDataStoreMode::ReadWrite,
        "none",
    )
    .is_none()
    {
        return;
    }
    rv.set(args.this().into());
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

pub(crate) fn native_data_transfer_effect_allowed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) -> String {
    data_transfer_effect_allowed(scope, view).unwrap_or_else(|| "none".to_owned())
}

pub(crate) fn native_data_transfer_drop_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
) -> String {
    private_string_property(scope, view, DATA_TRANSFER_DROP_EFFECT_SLOT)
        .unwrap_or_else(|| "none".to_owned())
}

fn build_data_transfer_item_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    DataTransferItemListObjectDeclaration::new(owner)
        .bind(scope)
        .ok()
}

fn build_dragged_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: &RendererDraggedFile,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = super::file::file_object_with_metadata(scope, &file.name, file.last_modified)?;
    blob::init_blob_object(scope, object, file.bytes.clone(), file.mime_type.clone());
    Some(object)
}

fn build_data_transfer_item_for_file<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mime_type = blob::blob_mime_type_from_object(scope, file).unwrap_or_default();
    let entry = build_file_system_file_entry(scope, file)?;
    let record = crate::util::new_null_prototype_object(scope);
    DataTransferFileItemObjectDeclaration::new(&mime_type, file, entry)
        .initialize(scope, record)
        .ok()?;
    Some(record)
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

fn build_data_transfer_item_for_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mime_type: &str,
    data: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let record = crate::util::new_null_prototype_object(scope);
    DataTransferStringItemObjectDeclaration::new(mime_type, data)
        .initialize(scope, record)
        .ok()?;
    Some(record)
}

fn build_file_system_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    FileSystemObjectDeclaration::new(v8::null(scope).into())
        .bind(scope)
        .ok()
}

fn build_file_system_file_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let name = file_name_from_object(scope, file).unwrap_or_default();
    let full_path = format!("/{name}");
    build_file_system_file_entry_with_full_path(scope, file, &full_path)
}

fn build_file_system_file_entry_with_full_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    file: v8::Local<'s, v8::Object>,
    full_path: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let name = file_name_from_object(scope, file).unwrap_or_default();
    let filesystem = build_file_system_object(scope)?;
    FileSystemFileEntryObjectDeclaration::new(filesystem, full_path, &name, file)
        .bind(scope)
        .ok()
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

fn build_file_system_directory_reader<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    let no_active_request = v8::null(scope);
    let no_error = v8::null(scope);
    FileSystemDirectoryReaderObjectDeclaration::new(
        entries,
        no_active_request.into(),
        no_error.into(),
    )
    .bind(scope)
    .ok()
}

fn clone_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Array> {
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return v8::Array::new(scope, 0);
    };
    let clone = v8::Array::new(scope, array.length() as i32);
    for index in 0..array.length() {
        if let Some(item) = array.get_index(scope, index) {
            let _ = clone.set_index(scope, index, item);
        }
    }
    clone
}

fn append_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_array: v8::Local<'s, v8::Array>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let next_index = item_array.length();
    define_index_data_property(scope, item_array.into(), next_index, item.into()).then_some(item)
}

fn define_index_data_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let key = v8::Integer::new_from_unsigned(scope, index)
        .to_string(scope)
        .expect("native array index should stringify");
    object.create_data_property(scope, key.into(), value) == Some(true)
}

fn append_string_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_array: v8::Local<'s, v8::Array>,
    mime_type: &str,
    data: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let item = build_data_transfer_item_for_string(scope, mime_type, data)?;
    append_item(scope, item_array, item)
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

fn data_transfer_item_list_did_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_list: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = item_list_owner(scope, item_list) else {
        return;
    };
    // Invalidate the cached FrozenArray before refreshing live views so a re-entrant getter
    // cannot observe the previous item-list generation.
    set_private_value(
        scope,
        owner,
        DATA_TRANSFER_TYPES_SLOT,
        v8::undefined(scope).into(),
    );
    refresh_data_transfer_item_views(scope, item_list);
}

fn find_item_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    views: v8::Local<'s, v8::Array>,
    record: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    for index in 0..views.length() {
        let Some(view) = views
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if get_private_object(scope, view, DATA_TRANSFER_ITEM_RECORD_SLOT)
            .is_some_and(|existing| existing.strict_equals(record.into()))
        {
            return Some(view);
        }
    }
    None
}

fn refresh_data_transfer_item_views<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item_list: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = item_list_owner(scope, item_list) else {
        return;
    };
    let mode = data_transfer_mode(scope, owner);
    let Some(previous_views) = item_list_array(scope, item_list) else {
        return;
    };
    let previous_length = private_number_property(
        scope,
        item_list,
        DATA_TRANSFER_ITEM_LIST_INDEXED_LENGTH_SLOT,
    )
    .unwrap_or(0.0) as u32;
    for index in 0..previous_length {
        let _ = item_list.delete_index(scope, index);
    }
    let mut views = Vec::new();
    let mut files = Vec::new();
    if mode.can_read_types()
        && let Some(records) = item_list_records(scope, item_list)
    {
        for index in 0..records.length() {
            let Some(record) = records
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            else {
                continue;
            };
            let view = find_item_view(scope, previous_views, record).or_else(|| {
                DataTransferItemViewDeclaration::new(owner, record)
                    .bind(scope)
                    .ok()
            });
            let Some(view) = view else {
                continue;
            };
            define_index_data_property(scope, item_list, views.len() as u32, view.into());
            views.push(view.into());
            if item_kind(scope, view).as_deref() == Some("file")
                && let Some(file) = item_file_object(scope, view)
            {
                files.push(file);
            }
        }
    }
    for index in 0..previous_views.length() {
        if let Some(previous) = previous_views
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && !views.iter().any(|view| previous.strict_equals(*view))
        {
            detach_data_transfer_item_view(scope, previous);
        }
    }
    let view_array = v8::Array::new_with_elements(scope, &views);
    set_private_value(
        scope,
        item_list,
        DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT,
        view_array.into(),
    );
    set_private_number(
        scope,
        item_list,
        DATA_TRANSFER_ITEM_LIST_INDEXED_LENGTH_SLOT,
        views.len() as f64,
    );
    if let Some(file_list) = get_private_object(scope, owner, DATA_TRANSFER_FILES_SLOT) {
        sync_file_list_contents(scope, file_list, &files);
    }
    if mode == DragDataStoreMode::Disabled {
        // Ending one event view must not keep the shared drag payload alive or
        // clear the store needed by later events and independent editing copies.
        set_private_value(
            scope,
            owner,
            DATA_TRANSFER_STORE_SLOT,
            v8::null(scope).into(),
        );
    }
}

fn ensure_data_transfer_types_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(snapshot) =
        get_private_value(scope, owner, DATA_TRANSFER_TYPES_SLOT).filter(|value| value.is_array())
    {
        return Some(snapshot);
    }

    let item_list = get_private_object(scope, owner, DATA_TRANSFER_ITEMS_SLOT)?;
    let item_array = item_list_array(scope, item_list)?;
    let types = if data_transfer_mode(scope, owner).can_read_types() {
        data_transfer_types_from_items(&item_summaries_from_array(scope, item_array))
    } else {
        Vec::new()
    };
    let types_array = crate::util::serialize_v8_array(scope, types.as_slice())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let _ = types_array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    set_private_value(scope, owner, DATA_TRANSFER_TYPES_SLOT, types_array.into());
    Some(types_array.into())
}

fn data_transfer_files_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), DATA_TRANSFER_FILES_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn data_transfer_items_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), DATA_TRANSFER_ITEMS_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn data_transfer_types_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = ensure_data_transfer_types_snapshot(scope, args.this())
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn data_transfer_drop_effect_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = private_string_property(scope, args.this(), DATA_TRANSFER_DROP_EFFECT_SLOT)
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "none").into());
    rv.set(value);
}

fn data_transfer_drop_effect_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        args.get(0),
        webidl::Context::argument("DataTransfer.dropEffect", 1),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !private_bool_property(scope, args.this(), DATA_TRANSFER_CLIPBOARD_SLOT).unwrap_or(false) {
        native_set_drag_drop_effect(scope, args.this(), &value);
    }
}

fn data_transfer_effect_allowed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = private_string_property(scope, args.this(), DATA_TRANSFER_EFFECT_ALLOWED_SLOT)
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "uninitialized").into());
    rv.set(value);
}

fn data_transfer_effect_allowed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        args.get(0),
        webidl::Context::argument("DataTransfer.effectAllowed", 1),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if valid_effect_allowed(&value)
        && data_transfer_mode(scope, args.this()).can_write_data()
        && !private_bool_property(scope, args.this(), DATA_TRANSFER_CLIPBOARD_SLOT).unwrap_or(false)
    {
        set_private_string(
            scope,
            args.this(),
            DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
            &value,
        );
        if let Some(store) = get_private_object(scope, args.this(), DATA_TRANSFER_STORE_SLOT) {
            set_private_string(scope, store, DATA_TRANSFER_EFFECT_ALLOWED_SLOT, &value);
        }
    }
}

fn data_transfer_item_kind_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = item_kind(scope, args.this())
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

fn data_transfer_item_type_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = item_type(scope, args.this())
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

fn data_transfer_item_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let length = item_list_owner(scope, args.this())
        .filter(|owner| data_transfer_mode(scope, *owner).can_read_types())
        .and_then(|_| item_list_array(scope, args.this()))
        .map(|items| items.length())
        .unwrap_or(0);
    rv.set(v8::Number::new(scope, length as f64).into());
}

fn file_system_entry_filesystem_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), FILE_SYSTEM_ENTRY_FILESYSTEM_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn file_system_entry_full_path_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = private_string_property(scope, args.this(), FILE_SYSTEM_ENTRY_FULL_PATH_SLOT)
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

fn file_system_entry_is_directory_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        private_bool_property(scope, args.this(), FILE_SYSTEM_ENTRY_IS_DIRECTORY_SLOT)
            .unwrap_or(false),
    );
}

fn file_system_entry_is_file_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        private_bool_property(scope, args.this(), FILE_SYSTEM_ENTRY_IS_FILE_SLOT).unwrap_or(false),
    );
}

fn file_system_entry_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = private_string_property(scope, args.this(), FILE_SYSTEM_ENTRY_NAME_SLOT)
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

/// Reads an event view without consulting author-visible methods or properties.
/// The view's read permissions still apply to these native default actions.
pub(crate) fn native_data_transfer_get_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::Object>,
    format: &str,
) -> String {
    if !data_transfer_mode(scope, view).can_read_data() {
        return String::new();
    }
    let normalized_type = normalize_drag_data_type(format);
    let Some(items) = get_private_object(scope, view, DATA_TRANSFER_ITEMS_SLOT)
        .and_then(|list| item_list_array(scope, list))
    else {
        return String::new();
    };
    for index in 0..items.length() {
        let Some(item) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if item_kind(scope, item).as_deref() == Some("string")
            && item_type(scope, item).as_deref() == Some(normalized_type.as_str())
        {
            return item_string_value(scope, item).unwrap_or_default();
        }
    }
    String::new()
}

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

pub(crate) fn data_transfer_get_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferGetDataArgs>(scope, &args) else {
        return;
    };
    let value = native_data_transfer_get_data(scope, args.this(), &parsed.format);
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(crate) fn data_transfer_set_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferSetDataArgs>(scope, &args) else {
        return;
    };
    let normalized_type = normalize_drag_data_type(&parsed.format);
    let value = parsed.data;
    let Some(store) = DataTransferItemStore::for_owner(scope, args.this()) else {
        return;
    };
    store.set_string_data(scope, &normalized_type, &value);
}

pub(crate) fn data_transfer_clear_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferClearDataArgs>(scope, &args) else {
        return;
    };
    let target_type = parsed.format.as_deref().map(normalize_drag_data_type);
    let Some(store) = DataTransferItemStore::for_owner(scope, args.this()) else {
        return;
    };
    store.clear_string_data(scope, target_type.as_deref());
}

pub(crate) fn data_transfer_item_list_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferItemListAddArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(data) = parsed.data else {
        rv.set(v8::null(scope).into());
        return;
    };
    let store = DataTransferItemStore::new(args.this());

    let item = if let Some(item_type) = parsed.item_type {
        let mime_type = normalize_drag_data_type(&item_type);
        let data = match webidl::convert::<webidl::DomString>(
            scope,
            data,
            webidl::Context::argument("DataTransferItemList.add", 1),
        ) {
            Ok(value) => value.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
        if !item_list_owner(scope, args.this())
            .is_some_and(|owner| data_transfer_mode(scope, owner).can_write_data())
        {
            rv.set(v8::null(scope).into());
            return;
        }
        if store.contains_string_type(scope, &mime_type) {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "DataTransferItemList already contains a string item with this type.",
            );
            return;
        }
        build_data_transfer_item_for_string(scope, &mime_type, &data)
    } else {
        if !item_list_owner(scope, args.this())
            .is_some_and(|owner| data_transfer_mode(scope, owner).can_write_data())
        {
            rv.set(v8::null(scope).into());
            return;
        }
        v8::Local::<v8::Object>::try_from(data)
            .ok()
            .filter(|file| selected_file_from_object(scope, *file).is_some())
            .and_then(|file| build_data_transfer_item_for_file(scope, file))
    };

    let Some(item) = item else {
        rv.set(v8::null(scope).into());
        return;
    };
    let added_item = store.append(scope, item);
    rv.set(
        added_item
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn data_transfer_item_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferItemListItemArgs>(scope, &args) else {
        return;
    };
    if !item_list_owner(scope, args.this())
        .is_some_and(|owner| data_transfer_mode(scope, owner).can_read_types())
    {
        rv.set(v8::null(scope).into());
        return;
    }
    let Some(value) = args.this().get_index(scope, parsed.index) else {
        rv.set(v8::null(scope).into());
        return;
    };
    if value.is_undefined() {
        rv.set(v8::null(scope).into());
        return;
    }
    rv.set(value);
}

pub(crate) fn data_transfer_item_list_remove_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferItemListRemoveArgs>(scope, &args) else {
        return;
    };
    if !item_list_owner(scope, args.this())
        .is_some_and(|owner| data_transfer_mode(scope, owner).can_write_data())
    {
        throw_dom_exception(scope, "InvalidStateError", 11, "The list is not writable.");
        return;
    }
    DataTransferItemStore::new(args.this()).remove(scope, parsed.index);
}

pub(crate) fn data_transfer_item_list_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    DataTransferItemStore::new(args.this()).clear(scope);
}

pub(crate) fn data_transfer_item_get_as_file_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(file) = item_file_object(scope, args.this()) {
        rv.set(file.into());
    } else {
        rv.set(v8::null(scope).into());
    }
}

pub(crate) fn data_transfer_item_webkit_get_as_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(entry) = item_entry_object(scope, args.this()) {
        rv.set(entry.into());
        return;
    }
    let Some(file) = item_file_object(scope, args.this()) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(entry) = build_file_system_file_entry(scope, file) else {
        rv.set(v8::null(scope).into());
        return;
    };
    rv.set(entry.into());
}

pub(super) fn file_system_file_entry_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, entry, FILE_SYSTEM_FILE_ENTRY_FILE_SLOT)
}

pub(crate) fn file_system_directory_entry_create_reader_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let entries = get_private_value(scope, args.this(), FILE_SYSTEM_DIRECTORY_ENTRY_ENTRIES_SLOT)
        .map(|value| clone_array(scope, value))
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let Some(reader) = build_file_system_directory_reader(scope, entries) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    rv.set(reader.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_transfer_scope(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        initialize_transfer_test_interfaces(scope);
        test(scope);
    }

    fn initialize_transfer_test_interfaces(scope: &mut v8::PinScope<'_, '_>) {
        let global = scope.get_current_context().global(scope);
        for name in [
            "DataTransfer",
            "DataTransferItemList",
            "DataTransferItem",
            "FileList",
        ] {
            let constructor = v8::Object::new(scope);
            let prototype = v8::Object::new(scope);
            assert_eq!(
                constructor.create_data_property(
                    scope,
                    v8str(scope, "prototype").into(),
                    prototype.into()
                ),
                Some(true)
            );
            assert_eq!(
                global.create_data_property(scope, v8str(scope, name).into(), constructor.into()),
                Some(true)
            );
        }
    }

    fn first_item<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        view: v8::Local<'s, v8::Object>,
    ) -> v8::Local<'s, v8::Object> {
        let list = get_private_object(scope, view, DATA_TRANSFER_ITEMS_SLOT).unwrap();
        let array = item_list_array(scope, list).unwrap();
        v8::Local::<v8::Object>::try_from(array.get_index(scope, 0).unwrap()).unwrap()
    }

    #[test]
    fn native_drag_metadata_roundtrips_and_author_replacement_clears_its_provenance() {
        with_transfer_scope(|scope| {
            let data = RendererDragData {
                items: vec![
                    RendererDragDataItem {
                        mime_type: "text/uri-list".to_owned(),
                        data: "https://source.test/link".to_owned(),
                        title: Some("Source label".to_owned()),
                        base_url: None,
                    },
                    RendererDragDataItem {
                        mime_type: "text/html".to_owned(),
                        data: "<b>source</b>".to_owned(),
                        title: None,
                        base_url: Some("https://source.test/".to_owned()),
                    },
                ],
                files: Vec::new(),
                directories: Vec::new(),
                drag_operations_mask: 19,
            };
            let store = create_native_drag_data_store(scope).unwrap();
            populate_native_drag_data_store(scope, &store, &data).unwrap();
            let (items, mask) = read_string_drag_data(scope, &store);
            assert_eq!(items, data.items);
            assert_eq!(mask, 19);
            let source =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadWrite, "none")
                    .unwrap();
            let items = DataTransferItemStore::for_owner(scope, source).unwrap();
            items.set_string_data(scope, "text/uri-list", "https://author.test/new");
            items.set_string_data(scope, "text/html", "<i>author</i>");
            let (items, _) = read_string_drag_data(scope, &store);
            assert_eq!(items[0].data, "https://author.test/new");
            assert_eq!(items[0].title, None);
            assert_eq!(items[1].data, "<i>author</i>");
            assert_eq!(items[1].base_url.as_deref(), Some("about:blank#blocked"));
        });
    }

    #[test]
    fn event_views_share_records_without_reviving_closed_view_permissions() {
        with_transfer_scope(|scope| {
            let store = create_native_drag_data_store(scope).unwrap();
            let backing = store.local(scope);
            assert!(!is_branded_data_transfer_object(scope, backing));
            let source =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadWrite, "none")
                    .unwrap();
            DataTransferItemStore::for_owner(scope, source)
                .unwrap()
                .set_string_data(scope, "text/plain", "payload");
            let source_item = first_item(scope, source);
            assert_eq!(
                item_string_value(scope, source_item).as_deref(),
                Some("payload")
            );
            let file = crate::util::new_null_prototype_object(scope);
            let entry = crate::util::new_null_prototype_object(scope);
            let record = crate::util::new_null_prototype_object(scope);
            DataTransferFileItemObjectDeclaration::new("application/octet-stream", file, entry)
                .initialize(scope, record)
                .unwrap();
            let source_file_item = DataTransferItemStore::for_owner(scope, source)
                .unwrap()
                .append(scope, record)
                .unwrap();
            assert!(item_file_object(scope, source_file_item).is_some());
            assert_eq!(native_data_transfer_files(scope, source).len(), 1);
            protect_native_drag_source_view(scope, source);
            assert_eq!(item_kind(scope, source_item).as_deref(), Some("string"));
            assert_eq!(item_type(scope, source_item).as_deref(), Some("text/plain"));
            assert_eq!(item_kind(scope, source_file_item).as_deref(), Some("file"));
            assert_eq!(
                item_type(scope, source_file_item).as_deref(),
                Some("application/octet-stream")
            );
            assert_eq!(item_string_value(scope, source_item), None);
            assert!(native_data_transfer_get_data(scope, source, "text/plain").is_empty());
            assert!(item_file_object(scope, source_file_item).is_none());
            assert!(item_entry_object(scope, source_file_item).is_none());
            assert!(native_data_transfer_files(scope, source).is_empty());

            let protected =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::Protected, "none")
                    .unwrap();
            let protected_item = first_item(scope, protected);
            assert!(!source.strict_equals(protected.into()));
            assert!(!source_item.strict_equals(protected_item.into()));
            assert!(native_data_transfer_has_type(
                scope,
                protected,
                "text/plain"
            ));
            assert_eq!(item_string_value(scope, protected_item), None);
            finish_drag_data_transfer_view(scope, protected);

            let drop =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadOnly, "copy")
                    .unwrap();
            let drop_item = first_item(scope, drop);
            assert!(!drop_item.strict_equals(protected_item.into()));
            assert_eq!(
                item_string_value(scope, drop_item).as_deref(),
                Some("payload")
            );
            DataTransferItemStore::for_owner(scope, drop)
                .unwrap()
                .set_string_data(scope, "text/plain", "forbidden");
            assert_eq!(
                item_string_value(scope, drop_item).as_deref(),
                Some("payload")
            );
            assert_eq!(item_kind(scope, source_item).as_deref(), Some("string"));
            assert_eq!(item_kind(scope, protected_item), None);
            update_drag_data_store_operations_mask(scope, &store, 2);
            assert_eq!(
                private_string_property(scope, backing, DATA_TRANSFER_EFFECT_ALLOWED_SLOT)
                    .as_deref(),
                Some("link")
            );
            assert_eq!(
                native_data_transfer_effect_allowed(scope, source),
                "uninitialized"
            );
            assert_eq!(finish_drag_data_transfer_view(scope, drop), "copy");
            assert_eq!(item_kind(scope, drop_item), None);

            replace_drag_data_store_contents(
                scope,
                &store,
                &RendererDragData {
                    items: Vec::new(),
                    files: Vec::new(),
                    directories: Vec::new(),
                    drag_operations_mask: 1,
                },
            )
            .unwrap();
            assert_eq!(
                private_property_as_array(scope, backing, DATA_TRANSFER_STORE_ITEMS_SLOT)
                    .unwrap()
                    .length(),
                0
            );
            assert_eq!(item_kind(scope, source_item).as_deref(), Some("string"));
            assert_eq!(item_type(scope, source_item).as_deref(), Some("text/plain"));
            assert_eq!(item_kind(scope, source_file_item).as_deref(), Some("file"));
            let source_types = ensure_data_transfer_types_snapshot(scope, source).unwrap();
            let source_types = v8::Local::<v8::Array>::try_from(source_types).unwrap();
            assert_eq!(source_types.length(), 2);
            assert_eq!(item_kind(scope, protected_item), None);
            assert_eq!(item_kind(scope, drop_item), None);
            finish_drag_data_transfer_view(scope, source);
            assert_eq!(item_kind(scope, source_item), None);
            assert_eq!(item_kind(scope, source_file_item), None);
            protect_native_drag_source_view(scope, source);
            assert_eq!(
                data_transfer_mode(scope, source),
                DragDataStoreMode::Disabled
            );
            assert_eq!(item_kind(scope, source_item), None);
        });
    }

    #[test]
    fn finished_views_disconnect_records_without_invalidating_readonly_editing_copies() {
        with_transfer_scope(|scope| {
            let store = create_native_drag_data_store(scope).unwrap();
            let source =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadWrite, "none")
                    .unwrap();
            let items = DataTransferItemStore::for_owner(scope, source).unwrap();
            items.set_string_data(scope, "text/plain", "captured payload");
            let source_string = first_item(scope, source);
            let file = crate::util::new_null_prototype_object(scope);
            let entry = crate::util::new_null_prototype_object(scope);
            set_private_value(scope, entry, FILE_SYSTEM_FILE_ENTRY_FILE_SLOT, file.into());
            let record = crate::util::new_null_prototype_object(scope);
            DataTransferFileItemObjectDeclaration::new("text/plain", file, entry)
                .initialize(scope, record)
                .unwrap();
            let source_file = items.append(scope, record).unwrap();
            protect_native_drag_source_view(scope, source);
            assert!(get_private_object(scope, source, DATA_TRANSFER_STORE_SLOT).is_some());
            assert!(item_record(scope, source_file).is_some());

            let target =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadOnly, "copy")
                    .unwrap();
            let target_string = first_item(scope, target);
            let admitted_string = item_string_value(scope, target_string).unwrap();
            let target_files = get_private_object(scope, target, DATA_TRANSFER_FILES_SLOT).unwrap();
            let captured_file = native_data_transfer_files(scope, target)[0];
            let snapshot = readonly_data_transfer_for_input(scope, target).unwrap();
            let snapshot_string = first_item(scope, snapshot);
            assert!(!snapshot_string.strict_equals(target_string.into()));
            assert!(
                !item_record(scope, snapshot_string)
                    .unwrap()
                    .strict_equals(item_record(scope, target_string).unwrap().into())
            );

            finish_drag_data_transfer_view(scope, target);
            assert!(get_private_object(scope, target, DATA_TRANSFER_STORE_SLOT).is_none());
            assert!(
                get_private_object(scope, target_string, DATA_TRANSFER_ITEM_OWNER_SLOT).is_none()
            );
            assert!(item_record(scope, target_string).is_none());
            assert_eq!(
                item_data_transfer_mode(scope, target_string),
                DragDataStoreMode::Disabled
            );
            assert!(
                file_list_files_from_object(scope, target_files)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(item_kind(scope, source_string).as_deref(), Some("string"));
            assert!(item_record(scope, source_file).is_some());
            let next =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadOnly, "copy")
                    .unwrap();
            assert_eq!(
                native_data_transfer_get_data(scope, next, "text/plain"),
                "captured payload"
            );
            finish_drag_data_transfer_view(scope, next);
            finish_drag_data_transfer_view(scope, source);
            assert!(get_private_object(scope, source, DATA_TRANSFER_STORE_SLOT).is_none());
            for item in [source_string, source_file] {
                assert!(get_private_object(scope, item, DATA_TRANSFER_ITEM_OWNER_SLOT).is_none());
                assert!(item_record(scope, item).is_none());
                assert_eq!(item_kind(scope, item), None);
            }
            assert_eq!(admitted_string, "captured payload");
            assert!(captured_file.strict_equals(file.into()));
            assert!(
                file_system_file_entry_file_object(scope, entry)
                    .unwrap()
                    .strict_equals(file.into())
            );
            assert_eq!(
                native_data_transfer_get_data(scope, snapshot, "text/plain"),
                "captured payload"
            );
            assert!(native_data_transfer_files(scope, snapshot)[0].strict_equals(file.into()));

            // A later mutable view may delete its records without deleting the
            // independent records owned by the retained editing snapshot.
            let mutable =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadWrite, "none")
                    .unwrap();
            let removed_item = first_item(scope, mutable);
            DataTransferItemStore::for_owner(scope, mutable)
                .unwrap()
                .clear(scope);
            assert!(item_record(scope, removed_item).is_none());
            assert!(
                get_private_object(scope, removed_item, DATA_TRANSFER_ITEM_OWNER_SLOT).is_none()
            );
            assert_eq!(
                native_data_transfer_get_data(scope, snapshot, "text/plain"),
                "captured payload"
            );
            assert!(native_data_transfer_files(scope, snapshot)[0].strict_equals(file.into()));
        });
    }

    #[test]
    fn finished_views_allow_backing_collection_while_wrappers_and_captured_files_survive() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let context;
        let source;
        let retained;
        let backing;
        let record;
        let file;
        let captured_file;
        let captured_entry;
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let local = v8::Context::new(scope, Default::default());
            context = v8::Global::new(scope, local);
            let scope = &mut v8::ContextScope::new(scope, local);
            initialize_transfer_test_interfaces(scope);
            let store = create_native_drag_data_store(scope).unwrap();
            let store_backing = store.local(scope);
            backing = v8::Weak::new(scope, store_backing);
            let source_view =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadWrite, "none")
                    .unwrap();
            let file_object = crate::util::new_null_prototype_object(scope);
            file = v8::Weak::new(scope, file_object);
            captured_file = v8::Global::new(scope, file_object);
            let entry = crate::util::new_null_prototype_object(scope);
            set_private_value(
                scope,
                entry,
                FILE_SYSTEM_FILE_ENTRY_FILE_SLOT,
                file_object.into(),
            );
            captured_entry = v8::Global::new(scope, entry);
            let raw_record = crate::util::new_null_prototype_object(scope);
            DataTransferFileItemObjectDeclaration::new("text/plain", file_object, entry)
                .initialize(scope, raw_record)
                .unwrap();
            record = v8::Weak::new(scope, raw_record);
            let source_item = DataTransferItemStore::for_owner(scope, source_view)
                .unwrap()
                .append(scope, raw_record)
                .unwrap();
            protect_native_drag_source_view(scope, source_view);
            let target =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::ReadOnly, "copy")
                    .unwrap();
            let target_item = first_item(scope, target);
            let mut held = vec![target, source_item, target_item];
            for view in [source_view, target] {
                let list = get_private_object(scope, view, DATA_TRANSFER_ITEMS_SLOT).unwrap();
                held.push(list);
                held.push(get_private_object(scope, view, DATA_TRANSFER_FILES_SLOT).unwrap());
                held.push(item_list_array(scope, list).unwrap().into());
            }
            retained = held
                .into_iter()
                .map(|object| v8::Global::new(scope, object))
                .collect::<Vec<_>>();
            source = v8::Global::new(scope, source_view);
            finish_drag_data_transfer_view(scope, target);
        }
        isolate.clear_kept_objects();
        isolate.low_memory_notification();
        assert!(
            !backing.is_empty(),
            "Protected source must retain its store until dragend"
        );
        assert!(!record.is_empty());
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let local = v8::Local::new(scope, &context);
            let scope = &mut v8::ContextScope::new(scope, local);
            let source_view = v8::Local::new(scope, &source);
            finish_drag_data_transfer_view(scope, source_view);
        }
        isolate.clear_kept_objects();
        isolate.low_memory_notification();
        assert!(
            backing.is_empty(),
            "retained finished wrappers must not root the drag store"
        );
        assert!(
            record.is_empty(),
            "retained item views must not root shared item records"
        );
        assert!(
            !file.is_empty(),
            "an explicitly captured File must remain usable"
        );
        drop(captured_file);
        isolate.clear_kept_objects();
        isolate.low_memory_notification();
        assert!(!file.is_empty(), "a captured entry still owns its File");
        drop(captured_entry);
        isolate.clear_kept_objects();
        isolate.low_memory_notification();
        assert!(
            file.is_empty(),
            "finished FileLists must not keep uncaptured files alive"
        );
        drop(retained);
    }

    #[test]
    fn drop_effect_initialization_belongs_to_the_event_view() {
        with_transfer_scope(|scope| {
            let store = create_native_drag_data_store(scope).unwrap();
            let first =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::Protected, "copy")
                    .unwrap();
            assert_eq!(native_data_transfer_drop_effect(scope, first), "copy");
            native_set_drag_drop_effect(scope, first, "none");
            assert_eq!(finish_drag_data_transfer_view(scope, first), "none");
            let next =
                create_drag_data_transfer_view(scope, &store, DragDataStoreMode::Protected, "move")
                    .unwrap();
            assert_eq!(native_data_transfer_drop_effect(scope, next), "move");
            native_set_drag_drop_effect(scope, next, "invalid");
            assert_eq!(finish_drag_data_transfer_view(scope, next), "move");
        });
    }
}

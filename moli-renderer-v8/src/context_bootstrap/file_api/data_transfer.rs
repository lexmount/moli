use super::*;
use crate::blob;
use crate::native_bridge::throw_dom_exception;
use crate::runtime::{RendererDragData, RendererDraggedDirectory, RendererDraggedFile};
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_file_api::data_transfer::{
    DataTransferItemSummary, child_entry_full_path, clear_data_removes_item,
    contains_string_item_type, data_transfer_types_from_items, drag_effect_allowed_from_mask,
    drop_effect_allowed_by_effect_allowed, modifier_drop_effect, normalize_drag_data_type,
    preferred_drop_effect_from_mask, valid_drop_effect, valid_effect_allowed,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const DATA_TRANSFER_FILES_SLOT: &str = "__lmDataTransferFiles";
const DATA_TRANSFER_ITEMS_SLOT: &str = "__lmDataTransferItems";
const DATA_TRANSFER_TYPES_SLOT: &str = "__lmDataTransferTypes";
const DATA_TRANSFER_DROP_EFFECT_SLOT: &str = "__lmDataTransferDropEffect";
const DATA_TRANSFER_EFFECT_ALLOWED_SLOT: &str = "__lmDataTransferEffectAllowed";
const DATA_TRANSFER_READ_ONLY_SLOT: &str = "__lmDataTransferReadOnly";
const DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT: &str = "__lmDataTransferItemArray";
const DATA_TRANSFER_ITEM_LIST_OWNER_SLOT: &str = "__lmDataTransferOwner";
const DATA_TRANSFER_ITEM_LIST_INDEXED_LENGTH_SLOT: &str = "__lmDataTransferItemListIndexedLength";
const DATA_TRANSFER_ITEM_KIND_SLOT: &str = "__lmDataTransferItemKind";
const DATA_TRANSFER_ITEM_TYPE_SLOT: &str = "__lmDataTransferItemType";
const DATA_TRANSFER_ITEM_FILE_SLOT: &str = "__lmDataTransferItemFile";
const DATA_TRANSFER_ITEM_ENTRY_SLOT: &str = "__lmDataTransferItemEntry";
const DATA_TRANSFER_ITEM_STRING_SLOT: &str = "__lmDataTransferItemString";
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
    #[webapi(slot = DATA_TRANSFER_READ_ONLY_SLOT, constructor_default = false)]
    read_only: bool,
    #[webapi(slot = DATA_TRANSFER_FILES_SLOT)]
    files: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA_TRANSFER_ITEMS_SLOT)]
    items: v8::Local<'s, v8::Object>,
    // `undefined` means that no FrozenArray snapshot has been published for the current
    // item-list generation. The getter replaces it with the cached snapshot on first read.
    #[webapi(slot = DATA_TRANSFER_TYPES_SLOT, init = "undefined")]
    types: (),
    #[webapi(slot = DATA_TRANSFER_DROP_EFFECT_SLOT, constructor_default = "none")]
    drop_effect: &'static str,
    #[webapi(slot = DATA_TRANSFER_EFFECT_ALLOWED_SLOT, constructor_default = "none")]
    effect_allowed: &'static str,
}

pub(crate) fn is_branded_data_transfer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::DataTransfer::is_instance(scope, object)
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
#[webapi(interface = web_api_interfaces::DataTransfer)]
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
#[webapi(interface = web_api_interfaces::DataTransferItemList)]
struct DataTransferItemListPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = data_transfer_item_list_length_getter, enumerable)]
    length: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DataTransferItem, scope_lifetime = 's, require_prototype)]
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
#[webapi(interface = web_api_interfaces::DataTransferItem, require_prototype)]
struct DataTransferDirectoryItemObjectDeclaration<'s> {
    #[webapi(slot = DATA_TRANSFER_ITEM_KIND_SLOT, constructor_default = "file")]
    kind: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_TYPE_SLOT, constructor_default = "")]
    item_type: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_ENTRY_SLOT)]
    entry: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DataTransferItem, require_prototype)]
struct DataTransferStringItemObjectDeclaration<'item_type, 'data> {
    #[webapi(slot = DATA_TRANSFER_ITEM_KIND_SLOT, constructor_default = "string")]
    kind: &'static str,
    #[webapi(slot = DATA_TRANSFER_ITEM_TYPE_SLOT)]
    item_type: &'item_type str,
    #[webapi(slot = DATA_TRANSFER_ITEM_STRING_SLOT)]
    data: &'data str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DataTransferItem)]
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

pub(crate) fn apply_drag_modifier_drop_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
    modifiers: u8,
) {
    let Some(drop_effect) = modifier_drop_effect(modifiers) else {
        return;
    };
    let effect_allowed = data_transfer_effect_allowed(scope, data_transfer);
    if !drop_effect_allowed_by_effect_allowed(effect_allowed.as_deref(), drop_effect) {
        return;
    }
    set_private_string(
        scope,
        data_transfer,
        DATA_TRANSFER_DROP_EFFECT_SLOT,
        drop_effect,
    );
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

fn data_transfer_can_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    private_bool_property(scope, object, DATA_TRANSFER_READ_ONLY_SLOT) == Some(false)
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

// Every write to the canonical item array runs inside DataTransferItemStore::mutate. The
// outcome controls the single observer notification and, consequently, the cached `types`
// identity required by Web IDL's CachedAttribute semantics.
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

    fn can_write(self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        item_list_owner(scope, self.item_list)
            .is_some_and(|owner| data_transfer_can_write(scope, owner))
    }

    fn mutate<'a, T>(
        self,
        scope: &mut v8::PinScope<'s, 'a>,
        mutation: impl FnOnce(
            &mut v8::PinScope<'s, 'a>,
            v8::Local<'s, v8::Array>,
        ) -> DataTransferItemListMutation<T>,
    ) -> Option<T> {
        if !self.can_write(scope) {
            return None;
        }
        let item_array = item_list_array(scope, self.item_list)?;
        let (value, changed) = match mutation(scope, item_array) {
            DataTransferItemListMutation::Unchanged(value) => (value, false),
            DataTransferItemListMutation::Changed(value) => (value, true),
        };
        if changed {
            data_transfer_item_list_did_change(scope, self.item_list);
        }
        Some(value)
    }

    fn append_drag_data(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        data: &RendererDragData,
    ) -> Option<()> {
        self.mutate(scope, |scope, item_array| {
            let mut changed = false;
            for item in &data.items {
                let normalized_type = normalize_drag_data_type(&item.mime_type);
                changed |=
                    append_string_item(scope, item_array, &normalized_type, &item.data).is_some();
            }
            for file in &data.files {
                changed |= append_file_item(scope, item_array, file).is_some();
            }
            for directory in &data.directories {
                changed |= append_directory_item(scope, item_array, directory).is_some();
            }
            if changed {
                DataTransferItemListMutation::Changed(())
            } else {
                DataTransferItemListMutation::Unchanged(())
            }
        })
    }

    fn contains_string_type(self, scope: &mut v8::PinScope<'s, '_>, mime_type: &str) -> bool {
        item_list_array(scope, self.item_list).is_some_and(|item_array| {
            contains_string_item_type(&item_summaries_from_array(scope, item_array), mime_type)
        })
    }

    fn append(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        item: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.mutate(scope, |scope, item_array| {
            if append_item(scope, item_array, item).is_some() {
                DataTransferItemListMutation::Changed(Some(item))
            } else {
                DataTransferItemListMutation::Unchanged(None)
            }
        })
        .flatten()
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
                let _ = next.set_index(scope, next_index, item.into());
                next_index += 1;
            }
            if !removed_item {
                return DataTransferItemListMutation::Unchanged(());
            }
            set_private_value(
                scope,
                self.item_list,
                DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT,
                next.into(),
            );
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
                let _ = next.set_index(scope, next_index, value);
                next_index += 1;
            }
            set_private_value(
                scope,
                self.item_list,
                DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT,
                next.into(),
            );
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
            set_private_value(
                scope,
                self.item_list,
                DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT,
                v8::Array::new(scope, 0).into(),
            );
            DataTransferItemListMutation::Changed(())
        });
    }
}

pub(super) fn item_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_property(scope, item, DATA_TRANSFER_ITEM_KIND_SLOT)
}

fn item_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_property(scope, item, DATA_TRANSFER_ITEM_TYPE_SLOT)
}

pub(super) fn item_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<String> {
    private_string_property(scope, item, DATA_TRANSFER_ITEM_STRING_SLOT)
}

fn item_file_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, item, DATA_TRANSFER_ITEM_FILE_SLOT)
}

fn item_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, item, DATA_TRANSFER_ITEM_ENTRY_SLOT)
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

fn initialize_data_transfer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let items = build_data_transfer_item_list_object(scope, owner)?;
    let empty_files = build_file_list_object(scope, &[])?;

    DataTransferObjectDeclaration::new(empty_files, items)
        .initialize(scope, owner)
        .ok()?;
    Some(items)
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

    if initialize_data_transfer_object(scope, args.this()).is_none() {
        rv.set(v8::undefined(scope).into());
        return;
    }
    rv.set(args.this().into());
}

pub(crate) fn build_data_transfer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: &RendererDragData,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = DataTransferShellDeclaration::default().bind(scope).ok()?;
    let item_list = initialize_data_transfer_object(scope, object)?;

    set_private_string(
        scope,
        object,
        DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        drag_effect_allowed_from_mask(data.drag_operations_mask),
    );
    set_private_string(
        scope,
        object,
        DATA_TRANSFER_DROP_EFFECT_SLOT,
        preferred_drop_effect_from_mask(data.drag_operations_mask),
    );

    DataTransferItemStore::new(item_list).append_drag_data(scope, data)?;
    Some(object)
}

/// Editing InputEvents keep their own readable data beyond the drag dispatch.
/// Copy native item state, not page-visible getters or the mutable drag list.
pub(crate) fn readonly_data_transfer_for_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source_list = get_private_object(scope, source, DATA_TRANSFER_ITEMS_SLOT)?;
    let source_items = item_list_array(scope, source_list)?;
    let object = DataTransferShellDeclaration::default().bind(scope).ok()?;
    let item_list = initialize_data_transfer_object(scope, object)?;
    let effect = data_transfer_effect_allowed(scope, source)?;
    set_private_string(scope, object, DATA_TRANSFER_EFFECT_ALLOWED_SLOT, &effect);
    DataTransferItemStore::new(item_list).mutate(scope, |scope, items| {
        for index in 0..source_items.length() {
            let Some(item) = source_items
                .get_index(scope, index)
                .and_then(|item| v8::Local::<v8::Object>::try_from(item).ok())
            else {
                continue;
            };
            let mime_type = item_type(scope, item).unwrap_or_default();
            let copy = if let Some(text) = item_string_value(scope, item) {
                build_data_transfer_item_for_string(scope, &mime_type, &text)
            } else if let Some(entry) = item_entry_object(scope, item) {
                if let Some(file) = item_file_object(scope, item) {
                    DataTransferFileItemObjectDeclaration::new(&mime_type, file, entry)
                        .bind(scope)
                        .ok()
                } else {
                    DataTransferDirectoryItemObjectDeclaration::new(entry)
                        .bind(scope)
                        .ok()
                }
            } else {
                None
            };
            if let Some(copy) = copy {
                let _ = append_item(scope, items, copy);
            }
        }
        DataTransferItemListMutation::Changed(())
    })?;
    set_private_value(
        scope,
        object,
        DATA_TRANSFER_READ_ONLY_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    Some(object)
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
    DataTransferFileItemObjectDeclaration::new(&mime_type, file, entry)
        .bind(scope)
        .ok()
}

fn build_data_transfer_item_for_directory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    directory: &RendererDraggedDirectory,
) -> Option<v8::Local<'s, v8::Object>> {
    let entry = build_file_system_directory_entry(scope, directory, None)?;
    DataTransferDirectoryItemObjectDeclaration::new(entry)
        .bind(scope)
        .ok()
}

fn build_data_transfer_item_for_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mime_type: &str,
    data: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    DataTransferStringItemObjectDeclaration::new(mime_type, data)
        .bind(scope)
        .ok()
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
    (item_array.set_index(scope, next_index, item.into()) == Some(true)).then_some(item)
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
    let Some(item_array) = item_list_array(scope, item_list) else {
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

    let mut files = Vec::new();
    for index in 0..item_array.length() {
        let Some(value) = item_array.get_index(scope, index) else {
            continue;
        };
        let Ok(item) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let _ = item_list.set_index(scope, index, item.into());
        if item_kind(scope, item).as_deref() == Some("file")
            && let Some(file) = item_file_object(scope, item)
        {
            files.push(file);
        }
    }

    set_private_number(
        scope,
        item_list,
        DATA_TRANSFER_ITEM_LIST_INDEXED_LENGTH_SLOT,
        item_array.length() as f64,
    );

    if let Some(file_list) = get_private_object(scope, owner, DATA_TRANSFER_FILES_SLOT) {
        sync_file_list_contents(scope, file_list, &files);
    } else if let Some(file_list) = build_file_list_object(scope, &files) {
        set_private_value(scope, owner, DATA_TRANSFER_FILES_SLOT, file_list.into());
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
    let types = data_transfer_types_from_items(&item_summaries_from_array(scope, item_array));
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
    if valid_drop_effect(&value) {
        set_private_string(scope, args.this(), DATA_TRANSFER_DROP_EFFECT_SLOT, &value);
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
    if valid_effect_allowed(&value) && data_transfer_can_write(scope, args.this()) {
        set_private_string(
            scope,
            args.this(),
            DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
            &value,
        );
    }
}

fn data_transfer_item_kind_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = private_string_property(scope, args.this(), DATA_TRANSFER_ITEM_KIND_SLOT)
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

fn data_transfer_item_type_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = private_string_property(scope, args.this(), DATA_TRANSFER_ITEM_TYPE_SLOT)
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

fn data_transfer_item_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let length = item_list_array(scope, args.this())
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

pub(crate) fn data_transfer_get_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferGetDataArgs>(scope, &args) else {
        return;
    };
    let normalized_type = normalize_drag_data_type(&parsed.format);
    let Some(item_list) = get_private_object(scope, args.this(), DATA_TRANSFER_ITEMS_SLOT) else {
        rv.set(v8str(scope, "").into());
        return;
    };
    let Some(item_array) = item_list_array(scope, item_list) else {
        rv.set(v8str(scope, "").into());
        return;
    };

    for index in 0..item_array.length() {
        let Some(value) = item_array.get_index(scope, index) else {
            continue;
        };
        let Ok(item) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        if item_kind(scope, item).as_deref() != Some("string") {
            continue;
        }
        if item_type(scope, item).as_deref() != Some(normalized_type.as_str()) {
            continue;
        }
        let value = item_string_value(scope, item).unwrap_or_default();
        rv.set(
            v8_string(scope, &value)
                .unwrap_or_else(|| v8str(scope, ""))
                .into(),
        );
        return;
    }

    rv.set(v8str(scope, "").into());
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
        if !store.can_write(scope) {
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
        if !store.can_write(scope) {
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
    let store = DataTransferItemStore::new(args.this());
    if !store.can_write(scope) {
        throw_dom_exception(scope, "InvalidStateError", 11, "The list is not writable.");
        return;
    }
    store.remove(scope, parsed.index);
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

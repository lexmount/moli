//! File and directory entry objects and item records for drag-data snapshots.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, new_null_prototype_object, v8_string, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub const DATA_TRANSFER_ITEM_ENTRY_SLOT: &str = "__lmDataTransferItemEntry";
pub const DATA_TRANSFER_ITEM_FILE_SLOT: &str = "__lmDataTransferItemFile";
pub const DATA_TRANSFER_ITEM_KIND_SLOT: &str = "__lmDataTransferItemKind";
pub const DATA_TRANSFER_ITEM_OWNER_SLOT: &str = "__lmDataTransferItemOwner";
pub const DATA_TRANSFER_ITEM_RECORD_SLOT: &str = "__lmDataTransferItemRecord";
pub const DATA_TRANSFER_ITEM_STRING_SLOT: &str = "__lmDataTransferItemString";
pub const DATA_TRANSFER_ITEM_TYPE_SLOT: &str = "__lmDataTransferItemType";
pub const FILE_SYSTEM_DIRECTORY_ENTRY_ENTRIES_SLOT: &str = "__lmFileSystemDirectoryEntryEntries";
pub const FILE_SYSTEM_DIRECTORY_READER_ACTIVE_REQUEST_SLOT: &str =
    "__lmFileSystemDirectoryReaderActiveRequest";
pub const FILE_SYSTEM_DIRECTORY_READER_DONE_SLOT: &str = "__lmFileSystemDirectoryReaderDone";
pub const FILE_SYSTEM_DIRECTORY_READER_ENTRIES_SLOT: &str = "__lmFileSystemDirectoryReaderEntries";
pub const FILE_SYSTEM_DIRECTORY_READER_ERROR_SLOT: &str = "__lmFileSystemDirectoryReaderError";
pub const FILE_SYSTEM_DIRECTORY_READER_OFFSET_SLOT: &str = "__lmFileSystemDirectoryReaderOffset";
const FILE_SYSTEM_ENTRY_FILESYSTEM_SLOT: &str = "__lmFileSystemEntryFilesystem";
const FILE_SYSTEM_ENTRY_FULL_PATH_SLOT: &str = "__lmFileSystemEntryFullPath";
const FILE_SYSTEM_ENTRY_IS_DIRECTORY_SLOT: &str = "__lmFileSystemEntryIsDirectory";
const FILE_SYSTEM_ENTRY_IS_FILE_SLOT: &str = "__lmFileSystemEntryIsFile";
const FILE_SYSTEM_ENTRY_NAME_SLOT: &str = "__lmFileSystemEntryName";
pub const FILE_SYSTEM_FILE_ENTRY_FILE_SLOT: &str = "__lmFileSystemFileEntryFile";

#[derive(WebApiObject)]
#[webapi(plain, scope_lifetime = 's)]
pub struct DataTransferFileItemObjectDeclaration<'s, 'item_type> {
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
pub struct DataTransferDirectoryItemObjectDeclaration<'s> {
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
pub struct DataTransferItemViewDeclaration<'s> {
    #[webapi(slot = DATA_TRANSFER_ITEM_OWNER_SLOT)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA_TRANSFER_ITEM_RECORD_SLOT)]
    record: v8::Local<'s, v8::Object>,
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
pub struct FileSystemFileEntryObjectDeclaration<'s, 'full_path, 'name> {
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
pub struct FileSystemDirectoryEntryObjectDeclaration<'s, 'full_path, 'name> {
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

#[derive(WebApiFunctionTemplate)]
// The legacy entry getters return empty values when the receiver has no entry.
#[webapi(interface = web_api_interfaces::FileSystemEntry, receiver = false)]
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

pub fn build_data_transfer_item_for_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mime_type: &str,
    data: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let record = new_null_prototype_object(scope);
    DataTransferStringItemObjectDeclaration::new(mime_type, data)
        .initialize(scope, record)
        .ok()?;
    Some(record)
}

pub fn build_file_system_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    FileSystemObjectDeclaration::new(v8::null(scope).into())
        .bind(scope)
        .ok()
}

pub fn build_file_system_directory_reader<'s>(
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

pub fn install_file_system_entry_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    FileSystemEntryPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
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
    let value = get_private_value(scope, args.this(), FILE_SYSTEM_ENTRY_FULL_PATH_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
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
        get_private_value(scope, args.this(), FILE_SYSTEM_ENTRY_IS_DIRECTORY_SLOT)
            .map(|value| value.boolean_value(scope))
            .unwrap_or(false),
    );
}

fn file_system_entry_is_file_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        get_private_value(scope, args.this(), FILE_SYSTEM_ENTRY_IS_FILE_SLOT)
            .map(|value| value.boolean_value(scope))
            .unwrap_or(false),
    );
}

fn file_system_entry_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), FILE_SYSTEM_ENTRY_NAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| v8_string(scope, &value).map(Into::into))
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

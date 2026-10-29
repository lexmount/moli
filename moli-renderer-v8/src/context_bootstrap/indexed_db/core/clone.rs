use super::*;
use crate::structured_clone::{
    HOST_OBJECT_TAG_DOM_EXCEPTION, HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR,
    read_dom_exception_payload, read_quota_exceeded_error_payload, write_dom_exception_payload,
    write_quota_exceeded_error_payload,
};
use crate::{
    context_bootstrap::dom_rect_clone_data,
    structured_clone::{HOST_OBJECT_TAG_GEOMETRY, read_dom_rect_payload, write_dom_rect_payload},
};
use crate::{
    context_bootstrap::{
        FileSystemHandleDurablePayload, build_file_system_handle_from_durable_payload,
        file_system_handle_clone_payload_from_object,
        file_system_handle_durable_payload_from_object, image_data_clone_payload_from_object,
    },
    dom::native::SelectedFile,
    structured_clone::{
        BlobClonePayload, HOST_OBJECT_TAG_BLOB, HOST_OBJECT_TAG_CRYPTO_KEY,
        HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE, HOST_OBJECT_TAG_IMAGE_DATA,
        blob_clone_payload_from_object, build_blob_object_from_clone_payload,
        read_crypto_key_payload, read_image_data_payload, write_crypto_key_payload,
        write_image_data_payload,
    },
};
use moli_indexeddb::{IndexedDbFileSystemHandleBucket, IndexedDbFileSystemHandleKind};
use std::{cell::RefCell, rc::Rc};

struct IndexedDbStructuredCloneSerializer {
    external_objects: Rc<RefCell<Vec<IndexedDbExternalObject>>>,
}

impl v8::ValueSerializerImpl for IndexedDbStructuredCloneSerializer {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let message = message.to_rust_string_lossy(scope);
        let exception = dom_exception_value(scope, &message, "DataCloneError");
        scope.throw_exception(exception);
    }

    fn has_custom_host_object(&self, _isolate: &v8::Isolate) -> bool {
        true
    }

    fn is_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<bool> {
        Some(moli_webapi_declare::web_api_object_type(scope, object).is_some())
    }

    fn write_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        serializer: &dyn v8::ValueSerializerHelper,
    ) -> Option<bool> {
        match moli_webapi_declare::web_api_object_type(scope, object).map(|kind| kind.name()) {
            Some("DOMException")
                if write_dom_exception_payload(scope, object, serializer).is_some() =>
            {
                return Some(true);
            }
            Some("QuotaExceededError")
                if write_quota_exceeded_error_payload(scope, object, serializer).is_some() =>
            {
                return Some(true);
            }
            Some("DOMRect" | "DOMRectReadOnly") => {
                if let Some((mutable, values)) = dom_rect_clone_data(scope, object) {
                    write_dom_rect_payload(serializer, mutable, values);
                    return Some(true);
                }
            }
            Some("EncodedVideoChunk") => {
                let exception = dom_exception_value(
                    scope,
                    "EncodedVideoChunk cannot be stored.",
                    "DataCloneError",
                );
                scope.throw_exception(exception);
                return None;
            }
            Some("CryptoKey") => {
                if write_crypto_key_payload(scope, object, serializer).is_some() {
                    return Some(true);
                }
            }
            Some("Blob" | "File") => {
                if let Some(payload) = blob_clone_payload_from_object(scope, object) {
                    let mut external_objects = self.external_objects.borrow_mut();
                    let Ok(index) = u32::try_from(external_objects.len()) else {
                        drop(external_objects);
                        let exception = dom_exception_value(
                            scope,
                            "Too many external objects in IndexedDB structured clone.",
                            "DataCloneError",
                        );
                        scope.throw_exception(exception);
                        return None;
                    };
                    external_objects.push(indexed_db_external_object_from_blob_payload(payload));
                    serializer.write_uint32(HOST_OBJECT_TAG_BLOB);
                    serializer.write_uint32(index);
                    return Some(true);
                }
            }
            Some("ImageData") => {
                if let Some(payload) = image_data_clone_payload_from_object(scope, object) {
                    write_image_data_payload(serializer, payload);
                    return Some(true);
                }
            }
            Some("FileSystemFileHandle" | "FileSystemDirectoryHandle")
                if file_system_handle_clone_payload_from_object(scope, object).is_some() =>
            {
                let Some(payload) = file_system_handle_durable_payload_from_object(scope, object)
                else {
                    let exception = dom_exception_value(
                        scope,
                        "FileSystemHandle is not authorized for this IndexedDB storage scope.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                };
                let mut external_objects = self.external_objects.borrow_mut();
                let Ok(index) = u32::try_from(external_objects.len()) else {
                    drop(external_objects);
                    let exception = dom_exception_value(
                        scope,
                        "Too many external objects in IndexedDB structured clone.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                };
                external_objects.push(indexed_db_external_object_from_file_system_handle(payload));
                serializer.write_uint32(HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE);
                serializer.write_uint32(index);
                return Some(true);
            }
            _ => {}
        }
        let exception = dom_exception_value(
            scope,
            "Unsupported host object during IndexedDB structured clone.",
            "DataCloneError",
        );
        scope.throw_exception(exception);
        None
    }

    fn get_shared_array_buffer_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        _buffer: v8::Local<'s, v8::SharedArrayBuffer>,
    ) -> Option<u32> {
        let exception = dom_exception_value(
            scope,
            "SharedArrayBuffer could not be cloned.",
            "DataCloneError",
        );
        scope.throw_exception(exception);
        None
    }

    fn get_wasm_module_transfer_id(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        _module: v8::Local<v8::WasmModuleObject>,
    ) -> Option<u32> {
        let exception = dom_exception_value(
            scope,
            "A WebAssembly.Module can not be serialized for storage.",
            "DataCloneError",
        );
        scope.throw_exception(exception);
        None
    }
}

struct IndexedDbStructuredCloneDeserializer {
    external_objects: Vec<IndexedDbExternalObject>,
}

impl v8::ValueDeserializerImpl for IndexedDbStructuredCloneDeserializer {
    fn read_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        deserializer: &dyn v8::ValueDeserializerHelper,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let mut tag = 0;
        if !deserializer.read_uint32(&mut tag) {
            let exception = dom_exception_value(
                scope,
                "Failed to deserialize IndexedDB structured clone host object.",
                "DataCloneError",
            );
            scope.throw_exception(exception);
            return None;
        }
        match tag {
            HOST_OBJECT_TAG_DOM_EXCEPTION => read_dom_exception_payload(scope, deserializer),
            HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR => {
                read_quota_exceeded_error_payload(scope, deserializer)
            }
            HOST_OBJECT_TAG_GEOMETRY => read_dom_rect_payload(scope, deserializer),
            HOST_OBJECT_TAG_IMAGE_DATA => read_image_data_payload(scope, deserializer),
            HOST_OBJECT_TAG_CRYPTO_KEY => {
                read_crypto_key_payload(scope, deserializer).or_else(|| {
                    let exception = dom_exception_value(
                        scope,
                        "Failed to deserialize IndexedDB CryptoKey.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    None
                })
            }
            HOST_OBJECT_TAG_BLOB => {
                let mut index = 0;
                if !deserializer.read_uint32(&mut index) {
                    let exception = dom_exception_value(
                        scope,
                        "Failed to deserialize IndexedDB Blob index.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                }
                let Some(payload) = self
                    .external_objects
                    .get(index as usize)
                    .and_then(blob_payload_from_indexed_db_external_object)
                else {
                    let exception = dom_exception_value(
                        scope,
                        "Missing external Blob during IndexedDB structured clone.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                };
                build_blob_object_from_clone_payload(scope, &payload)
            }
            HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE => {
                let mut index = 0;
                if !deserializer.read_uint32(&mut index) {
                    let exception = dom_exception_value(
                        scope,
                        "Failed to deserialize IndexedDB FileSystemHandle index.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                }
                let Some(payload) = self
                    .external_objects
                    .get(index as usize)
                    .and_then(file_system_handle_payload_from_indexed_db_external_object)
                else {
                    let exception = dom_exception_value(
                        scope,
                        "Missing external FileSystemHandle during IndexedDB structured clone.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                };
                build_file_system_handle_from_durable_payload(scope, &payload).or_else(|| {
                    let exception = dom_exception_value(
                        scope,
                        "FileSystemHandle is not authorized for this IndexedDB storage scope.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    None
                })
            }
            _ => {
                let exception = dom_exception_value(
                    scope,
                    "Unsupported IndexedDB structured clone host object.",
                    "DataCloneError",
                );
                scope.throw_exception(exception);
                None
            }
        }
    }
}

fn indexed_db_external_object_from_blob_payload(
    payload: BlobClonePayload,
) -> IndexedDbExternalObject {
    match payload {
        BlobClonePayload::Blob { bytes, mime_type } => {
            IndexedDbExternalObject::Blob { bytes, mime_type }
        }
        BlobClonePayload::File { file, .. } => IndexedDbExternalObject::File {
            bytes: file.bytes,
            mime_type: file.mime_type,
            name: file.name,
            last_modified: file.last_modified,
        },
    }
}

fn blob_payload_from_indexed_db_external_object(
    object: &IndexedDbExternalObject,
) -> Option<BlobClonePayload> {
    match object {
        IndexedDbExternalObject::Blob { bytes, mime_type } => Some(BlobClonePayload::Blob {
            bytes: bytes.clone(),
            mime_type: mime_type.clone(),
        }),
        IndexedDbExternalObject::File {
            bytes,
            mime_type,
            name,
            last_modified,
        } => Some(BlobClonePayload::File {
            file: SelectedFile {
                bytes: bytes.clone(),
                mime_type: mime_type.clone(),
                name: name.clone(),
                last_modified: *last_modified,
            },
            // IndexedDB external objects durably materialize the File bytes.
            // A later read is no longer backed by the source OPFS entry.
            opfs_snapshot: None,
        }),
        IndexedDbExternalObject::FileSystemHandle { .. } => None,
    }
}

fn indexed_db_external_object_from_file_system_handle(
    payload: FileSystemHandleDurablePayload,
) -> IndexedDbExternalObject {
    let kind = match payload.kind {
        moli_storage_service::EntryKind::File => IndexedDbFileSystemHandleKind::File,
        moli_storage_service::EntryKind::Directory => IndexedDbFileSystemHandleKind::Directory,
    };
    let bucket = payload
        .bucket_id
        .map(|bucket_id| IndexedDbFileSystemHandleBucket::Named { bucket_id })
        .unwrap_or(IndexedDbFileSystemHandleBucket::Default);
    IndexedDbExternalObject::FileSystemHandle {
        kind,
        bucket,
        path: payload.path,
    }
}

fn file_system_handle_payload_from_indexed_db_external_object(
    object: &IndexedDbExternalObject,
) -> Option<FileSystemHandleDurablePayload> {
    let IndexedDbExternalObject::FileSystemHandle { kind, bucket, path } = object else {
        return None;
    };
    let kind = match kind {
        IndexedDbFileSystemHandleKind::File => moli_storage_service::EntryKind::File,
        IndexedDbFileSystemHandleKind::Directory => moli_storage_service::EntryKind::Directory,
    };
    let bucket_id = match bucket {
        IndexedDbFileSystemHandleBucket::Default => None,
        IndexedDbFileSystemHandleBucket::Named { bucket_id } => Some(*bucket_id),
    };
    Some(FileSystemHandleDurablePayload {
        bucket_id,
        path: path.clone(),
        kind,
    })
}

pub(in crate::context_bootstrap::indexed_db) fn serialize_js_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<IndexedDbValue> {
    let external_objects = Rc::new(RefCell::new(Vec::new()));
    let serialized = {
        let context = scope.get_current_context();
        let serializer = v8::ValueSerializer::new(
            scope,
            Box::new(IndexedDbStructuredCloneSerializer {
                external_objects: Rc::clone(&external_objects),
            }),
        );
        serializer.write_header();
        match serializer.write_value(context, value) {
            Some(true) => Some(serializer.release()),
            _ => None,
        }
    };
    if let Some(wire_bytes) = serialized {
        let external_objects = external_objects.borrow().clone();
        return Some(IndexedDbValue::new(wire_bytes, external_objects));
    }
    None
}

/// Serialization invokes getters while the transaction cannot accept requests.
/// Restore its state before propagating the original exception, including throws
/// of null or undefined. An explicit abort during serialization stays finished.
pub(in crate::context_bootstrap::indexed_db) fn serialize_js_value_for_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<IndexedDbValue> {
    let inactive = v8::Boolean::new(scope, false).into();
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
        inactive,
    );
    v8::tc_scope!(let scope, scope);
    let serialized = serialize_js_value(scope, value);
    if scope.can_continue() {
        let exception = scope.exception();
        scope.reset();
        if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
        {
            let active = v8::Boolean::new(scope, true).into();
            set_indexed_db_slot_value(
                scope,
                transaction,
                INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
                active,
            );
        }
        if let Some(exception) = exception {
            scope.throw_exception(exception);
        }
    }
    if scope.has_caught() {
        scope.rethrow();
    }
    serialized
}

pub(in crate::context_bootstrap::indexed_db) fn deserialize_js_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &IndexedDbValue,
) -> Option<v8::Local<'s, v8::Value>> {
    let context = scope.get_current_context();
    let deserializer = v8::ValueDeserializer::new(
        scope,
        Box::new(IndexedDbStructuredCloneDeserializer {
            external_objects: value.external_objects().to_vec(),
        }),
        value.wire_bytes(),
    );
    let Some(true) = deserializer.read_header(context) else {
        return None;
    };
    deserializer.read_value(context)
}

use super::*;
use crate::{
    context_bootstrap::{
        FileSystemHandleDurablePayload, build_file_list_object,
        build_file_system_handle_from_durable_payload, build_geometry_object_from_clone_payload,
        file_list_files_from_object, file_system_handle_clone_payload_from_object,
        file_system_handle_durable_payload_from_object, geometry_clone_payload_from_object,
        image_data_clone_payload_from_object, is_file_list_object,
    },
    dom::native::SelectedFile,
    structured_clone::{
        BlobClonePayload, HOST_OBJECT_TAG_BLOB, HOST_OBJECT_TAG_CRYPTO_KEY,
        HOST_OBJECT_TAG_FILE_LIST, HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE, HOST_OBJECT_TAG_GEOMETRY,
        HOST_OBJECT_TAG_IMAGE_DATA, blob_clone_payload_from_object,
        build_blob_object_from_clone_payload, read_crypto_key_payload, read_geometry_clone_payload,
        read_image_data_payload, write_crypto_key_payload, write_geometry_clone_payload,
        write_image_data_payload,
    },
};
use moli_indexeddb::{IndexedDbFileSystemHandleBucket, IndexedDbFileSystemHandleKind};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

struct IndexedDbStructuredCloneSerializer {
    external_objects: Rc<RefCell<Vec<IndexedDbExternalObject>>>,
    external_object_sources: Rc<RefCell<Vec<IndexedDbExternalObjectSource>>>,
}

struct IndexedDbExternalObjectSource {
    index: u32,
    object: v8::Global<v8::Object>,
}

impl IndexedDbStructuredCloneSerializer {
    fn store_blob_external_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        payload: BlobClonePayload,
    ) -> Option<u32> {
        if let Some(source) = self
            .external_object_sources
            .borrow()
            .iter()
            .find(|source| v8::Local::new(scope, &source.object).strict_equals(object.into()))
        {
            return Some(source.index);
        }

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
        drop(external_objects);
        self.external_object_sources
            .borrow_mut()
            .push(IndexedDbExternalObjectSource {
                index,
                object: v8::Global::new(scope, object),
            });
        Some(index)
    }
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
        Some(
            crate::context_bootstrap::is_crypto_key_object(scope, object)
                || image_data_clone_payload_from_object(scope, object).is_some()
                || geometry_clone_payload_from_object(scope, object).is_some()
                || crate::blob::is_blob_object(scope, object)
                || is_file_list_object(scope, object)
                || file_system_handle_clone_payload_from_object(scope, object).is_some()
                || moli_webapi_declare::is_web_api_platform_object(scope, object),
        )
    }

    fn write_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        serializer: &dyn v8::ValueSerializerHelper,
    ) -> Option<bool> {
        if write_crypto_key_payload(scope, object, serializer).is_some() {
            return Some(true);
        }
        if let Some(payload) = image_data_clone_payload_from_object(scope, object) {
            write_image_data_payload(serializer, payload);
            return Some(true);
        }
        if let Some(payload) = geometry_clone_payload_from_object(scope, object) {
            write_geometry_clone_payload(serializer, payload);
            return Some(true);
        }
        if is_file_list_object(scope, object) {
            let Some(files) = file_list_files_from_object(scope, object) else {
                let exception = dom_exception_value(
                    scope,
                    "Invalid FileList during IndexedDB structured clone.",
                    "DataCloneError",
                );
                scope.throw_exception(exception);
                return None;
            };
            let Ok(length) = u32::try_from(files.len()) else {
                let exception = dom_exception_value(
                    scope,
                    "Too many Files in IndexedDB structured clone FileList.",
                    "DataCloneError",
                );
                scope.throw_exception(exception);
                return None;
            };
            let mut indices = Vec::with_capacity(files.len());
            for file in files {
                let Some(payload @ BlobClonePayload::File { .. }) =
                    blob_clone_payload_from_object(scope, file)
                else {
                    let exception = dom_exception_value(
                        scope,
                        "FileList contains an invalid File during IndexedDB structured clone.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                };
                indices.push(self.store_blob_external_object(scope, file, payload)?);
            }
            serializer.write_uint32(HOST_OBJECT_TAG_FILE_LIST);
            serializer.write_uint32(length);
            for index in indices {
                serializer.write_uint32(index);
            }
            return Some(true);
        }
        if let Some(payload) = blob_clone_payload_from_object(scope, object) {
            let index = self.store_blob_external_object(scope, object, payload)?;
            serializer.write_uint32(HOST_OBJECT_TAG_BLOB);
            serializer.write_uint32(index);
            return Some(true);
        }
        if file_system_handle_clone_payload_from_object(scope, object).is_some() {
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
        let exception = dom_exception_value(
            scope,
            "Unsupported host object during IndexedDB structured clone.",
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
    external_object_cache: RefCell<HashMap<u32, v8::Global<v8::Object>>>,
}

impl IndexedDbStructuredCloneDeserializer {
    fn blob_object_for_external_index<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        index: u32,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(object) = self.external_object_cache.borrow().get(&index) {
            return Some(v8::Local::new(scope, object));
        }
        let payload = self
            .external_objects
            .get(index as usize)
            .and_then(blob_payload_from_indexed_db_external_object)?;
        let object = build_blob_object_from_clone_payload(scope, &payload)?;
        self.external_object_cache
            .borrow_mut()
            .insert(index, v8::Global::new(scope, object));
        Some(object)
    }
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
            HOST_OBJECT_TAG_GEOMETRY => read_geometry_clone_payload(deserializer)
                .map(|payload| build_geometry_object_from_clone_payload(scope, payload))
                .or_else(|| {
                    let exception = dom_exception_value(
                        scope,
                        "Failed to deserialize IndexedDB Geometry object.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    None
                }),
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
                let Some(object) = self.blob_object_for_external_index(scope, index) else {
                    let exception = dom_exception_value(
                        scope,
                        "Missing external Blob during IndexedDB structured clone.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                };
                Some(object)
            }
            HOST_OBJECT_TAG_FILE_LIST => {
                let mut length = 0;
                if !deserializer.read_uint32(&mut length) {
                    let exception = dom_exception_value(
                        scope,
                        "Failed to deserialize IndexedDB FileList length.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                }
                let mut files = Vec::new();
                if files.try_reserve_exact(length as usize).is_err() {
                    let exception = dom_exception_value(
                        scope,
                        "IndexedDB FileList is too large to deserialize.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    return None;
                }
                for _ in 0..length {
                    let mut index = 0;
                    if !deserializer.read_uint32(&mut index)
                        || !matches!(
                            self.external_objects.get(index as usize),
                            Some(IndexedDbExternalObject::File { .. })
                        )
                    {
                        let exception = dom_exception_value(
                            scope,
                            "Missing external File during IndexedDB FileList clone.",
                            "DataCloneError",
                        );
                        scope.throw_exception(exception);
                        return None;
                    }
                    let Some(file) = self.blob_object_for_external_index(scope, index) else {
                        let exception = dom_exception_value(
                            scope,
                            "Failed to deserialize File in IndexedDB FileList.",
                            "DataCloneError",
                        );
                        scope.throw_exception(exception);
                        return None;
                    };
                    files.push(file);
                }
                build_file_list_object(scope, &files).or_else(|| {
                    let exception = dom_exception_value(
                        scope,
                        "Failed to deserialize IndexedDB FileList.",
                        "DataCloneError",
                    );
                    scope.throw_exception(exception);
                    None
                })
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
    let mut should_throw_data_clone_error = false;
    let external_objects = Rc::new(RefCell::new(Vec::new()));
    let external_object_sources = Rc::new(RefCell::new(Vec::new()));
    let serialized = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        let context = scope.get_current_context();
        let serializer = v8::ValueSerializer::new(
            &scope,
            Box::new(IndexedDbStructuredCloneSerializer {
                external_objects: Rc::clone(&external_objects),
                external_object_sources: Rc::clone(&external_object_sources),
            }),
        );
        serializer.write_header();
        match serializer.write_value(context, value) {
            Some(true) => Some(serializer.release()),
            _ => {
                should_throw_data_clone_error =
                    scope.has_caught() && scope.can_continue() && !scope.has_terminated();
                None
            }
        }
    };
    if let Some(wire_bytes) = serialized {
        let external_objects = external_objects.borrow().clone();
        return Some(IndexedDbValue::new(wire_bytes, external_objects));
    }
    if should_throw_data_clone_error {
        let exception = dom_exception_value(
            scope,
            "The value could not be cloned for IndexedDB storage.",
            "DataCloneError",
        );
        scope.throw_exception(exception);
    }
    None
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
            external_object_cache: RefCell::new(HashMap::new()),
        }),
        value.wire_bytes(),
    );
    let Some(true) = deserializer.read_header(context) else {
        return None;
    };
    deserializer.read_value(context)
}

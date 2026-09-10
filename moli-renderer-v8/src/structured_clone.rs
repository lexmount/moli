use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt,
    rc::Rc,
    sync::Arc,
};

use crate::{
    context_bootstrap::{
        CryptoKeyAlgorithmClonePayload, CryptoKeyClonePayload, FileSystemFileSnapshotClonePayload,
        FileSystemHandleClonePayload, ImageDataClonePayload, ReadableStreamClonePayload,
        TransformStreamClonePayload, WritableStreamClonePayload,
        attach_file_system_file_snapshot_clone_payload, build_file_object,
        build_file_system_handle_from_clone_payload, build_image_data_object_from_clone_payload,
        build_readable_stream_clone_shell, build_transform_stream_clone_shell,
        build_writable_stream_clone_shell, crypto_key_clone_payload_from_object,
        crypto_key_object_from_clone_payload, detach_message_port_owner_for_transfer,
        detach_transferred_message_port, dom_exception_clone_fields,
        ensure_message_port_wrapper_for_id, file_system_file_snapshot_clone_payload_from_object,
        file_system_handle_clone_payload_from_object, image_data_clone_payload_from_object,
        initialize_readable_stream_clone_shell, initialize_transform_stream_clone_shell,
        initialize_writable_stream_clone_shell, message_port_id_from_object,
        new_dom_exception_value, new_quota_exceeded_error_value, prepare_readable_stream_transfer,
        prepare_transform_stream_transfer, prepare_writable_stream_transfer,
        quota_exceeded_error_clone_fields, require_internal_stream_value,
        selected_file_from_object,
    },
    dom::native::SelectedFile,
    types::MessagePortId,
};
pub(crate) use moli_structured_clone::{
    StructuredCloneBytes as StructuredCloneWireBytes, TransferredArrayBuffer,
};
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

const HOST_OBJECT_TAG_MESSAGE_PORT: u32 = 1;
const HOST_OBJECT_TAG_IMAGE_DATA: u32 = 2;
pub(crate) const HOST_OBJECT_TAG_CRYPTO_KEY: u32 = 3;
const HOST_OBJECT_TAG_READABLE_STREAM: u32 = 4;
pub(crate) const HOST_OBJECT_TAG_BLOB: u32 = 5;
const HOST_OBJECT_TAG_DOM_EXCEPTION: u32 = 6;
pub(crate) const HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE: u32 = 7;
const HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR: u32 = 8;
const HOST_OBJECT_TAG_WRITABLE_STREAM: u32 = 9;
const HOST_OBJECT_TAG_TRANSFORM_STREAM: u32 = 10;

#[derive(Clone, Debug, Default)]
pub(crate) struct V8StructuredClonePayload {
    pub(crate) base: StructuredCloneWireBytes<MessagePortId>,
    wasm_modules: Vec<ClonedWasmModule>,
    readable_streams: Vec<ClonedReadableStream>,
    writable_streams: Vec<ClonedWritableStream>,
    transform_streams: Vec<ClonedTransformStream>,
    blobs: Vec<ClonedBlob>,
    file_system_handles: Vec<ClonedFileSystemHandle>,
    pub(crate) metadata: StructuredCloneMetadata,
}

impl V8StructuredClonePayload {
    pub(crate) fn transferred_message_ports(&self) -> &[MessagePortId] {
        &self.base.transferred_message_ports
    }
}

#[derive(Clone)]
struct ClonedWasmModule {
    clone_id: u32,
    compiled_module: Arc<v8::CompiledWasmModule>,
    instantiation_exceeds_v8_limit: bool,
}

#[derive(Clone, Debug)]
struct ClonedReadableStream {
    clone_id: u32,
    payload: ReadableStreamClonePayload,
}

#[derive(Clone, Debug)]
struct ClonedWritableStream {
    clone_id: u32,
    payload: WritableStreamClonePayload,
}

#[derive(Clone, Debug)]
struct ClonedTransformStream {
    clone_id: u32,
    payload: TransformStreamClonePayload,
}

enum DeferredStreamMaterialization {
    Readable {
        shell: v8::Global<v8::Object>,
        payload: ReadableStreamClonePayload,
    },
    Writable {
        shell: v8::Global<v8::Object>,
        payload: WritableStreamClonePayload,
    },
    Transform {
        shell: v8::Global<v8::Object>,
        payload: TransformStreamClonePayload,
    },
}

impl DeferredStreamMaterialization {
    fn initialize<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<()> {
        match self {
            Self::Readable { shell, payload } => {
                let shell = v8::Local::new(scope, shell);
                initialize_readable_stream_clone_shell(scope, shell, payload)
            }
            Self::Writable { shell, payload } => {
                let shell = v8::Local::new(scope, shell);
                initialize_writable_stream_clone_shell(scope, shell, payload)
            }
            Self::Transform { shell, payload } => {
                let shell = v8::Local::new(scope, shell);
                initialize_transform_stream_clone_shell(scope, shell, payload)
            }
        }
    }

    fn record_port_ids(&self, port_ids: &mut HashSet<MessagePortId>) {
        match self {
            Self::Readable { payload, .. } => {
                port_ids.insert(payload.port_id());
            }
            Self::Writable { payload, .. } => {
                port_ids.insert(payload.port_id());
            }
            Self::Transform { payload, .. } => {
                port_ids.extend(payload.port_ids());
            }
        }
    }
}

fn discard_all_transferred_stream_channels(
    scope: &mut v8::PinScope<'_, '_>,
    payload: &V8StructuredClonePayload,
) {
    for stream in &payload.readable_streams {
        stream.payload.discard_port(scope);
    }
    for stream in &payload.writable_streams {
        stream.payload.discard_port(scope);
    }
    for stream in &payload.transform_streams {
        stream.payload.discard_ports(scope);
    }
}

fn discard_unclaimed_transferred_stream_channels(
    scope: &mut v8::PinScope<'_, '_>,
    payload: &V8StructuredClonePayload,
    materializations: &[DeferredStreamMaterialization],
) {
    let mut claimed_port_ids = HashSet::new();
    for materialization in materializations {
        materialization.record_port_ids(&mut claimed_port_ids);
    }
    for stream in &payload.readable_streams {
        if !claimed_port_ids.contains(&stream.payload.port_id()) {
            stream.payload.discard_port(scope);
        }
    }
    for stream in &payload.writable_streams {
        if !claimed_port_ids.contains(&stream.payload.port_id()) {
            stream.payload.discard_port(scope);
        }
    }
    for stream in &payload.transform_streams {
        let port_ids = stream.payload.port_ids();
        if port_ids
            .iter()
            .all(|port_id| !claimed_port_ids.contains(port_id))
        {
            stream.payload.discard_ports(scope);
        }
    }
}

#[derive(Clone, Debug)]
struct ClonedBlob {
    clone_id: u32,
    payload: BlobClonePayload,
}

#[derive(Clone, Debug)]
struct ClonedFileSystemHandle {
    clone_id: u32,
    payload: FileSystemHandleClonePayload,
}

#[derive(Clone, Debug)]
pub(crate) enum BlobClonePayload {
    Blob {
        bytes: Vec<u8>,
        mime_type: String,
    },
    File {
        file: SelectedFile,
        opfs_snapshot: Option<FileSystemFileSnapshotClonePayload>,
    },
}

impl fmt::Debug for ClonedWasmModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClonedWasmModule")
            .field("clone_id", &self.clone_id)
            .field(
                "instantiation_exceeds_v8_limit",
                &self.instantiation_exceeds_v8_limit,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
struct ClonedWasmModuleStore {
    next_id: u32,
    modules: Vec<ClonedWasmModule>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StructuredClonePolicy {
    Runtime,
    RuntimeMessage,
    Storage,
}

impl StructuredClonePolicy {
    fn allows_wasm_module(self) -> bool {
        matches!(self, Self::Runtime | Self::RuntimeMessage)
    }

    fn metadata_for_wasm_modules(self, contains_wasm_module: bool) -> StructuredCloneMetadata {
        StructuredCloneMetadata {
            contains_wasm_module,
            origin_check_required: contains_wasm_module && self == Self::RuntimeMessage,
            locked_to_sender_agent_cluster: contains_wasm_module && self == Self::RuntimeMessage,
            sender_agent_cluster: None,
            sender_origin: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeMessageAgentCluster {
    WindowOrDedicatedWorker,
    SharedWorker,
    ServiceWorker,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StructuredCloneMetadata {
    pub(crate) contains_wasm_module: bool,
    pub(crate) origin_check_required: bool,
    pub(crate) locked_to_sender_agent_cluster: bool,
    pub(crate) sender_agent_cluster: Option<RuntimeMessageAgentCluster>,
    pub(crate) sender_origin: Option<String>,
}

struct WireSerializer {
    allowed_message_port_ids: HashSet<MessagePortId>,
    allowed_readable_streams: Vec<v8::Global<v8::Object>>,
    allowed_writable_streams: Vec<v8::Global<v8::Object>>,
    allowed_transform_streams: Vec<v8::Global<v8::Object>>,
    policy: StructuredClonePolicy,
    wasm_modules: Rc<RefCell<ClonedWasmModuleStore>>,
    blobs: Rc<RefCell<ClonedBlobStore>>,
    file_system_handles: Rc<RefCell<ClonedFileSystemHandleStore>>,
}

#[derive(Clone, Debug, Default)]
struct ClonedBlobStore {
    next_id: u32,
    blobs: Vec<ClonedBlob>,
}

#[derive(Clone, Debug, Default)]
struct ClonedFileSystemHandleStore {
    next_id: u32,
    handles: Vec<ClonedFileSystemHandle>,
}

impl v8::ValueSerializerImpl for WireSerializer {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let message = message.to_rust_string_lossy(scope);
        let exception = new_dom_exception_value(scope, &message, "DataCloneError");
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
            Some("MessagePort") => {
                if let Some(port_id) = message_port_id_from_object(scope, object) {
                    if !self.allowed_message_port_ids.contains(&port_id) {
                        throw_data_clone_exception(
                            scope,
                            "MessagePort must be listed in the postMessage transfer list.",
                        );
                        return None;
                    }
                    serializer.write_uint32(HOST_OBJECT_TAG_MESSAGE_PORT);
                    serializer.write_uint64(port_id);
                    return Some(true);
                }
            }
            Some("ImageData") => {
                if let Some(payload) = image_data_clone_payload_from_object(scope, object) {
                    write_image_data_payload(serializer, payload);
                    return Some(true);
                }
            }
            Some("CryptoKey") => {
                if write_crypto_key_payload(scope, object, serializer).is_some() {
                    return Some(true);
                }
            }
            Some("ReadableStream") => {
                let Some(index) = self.allowed_readable_streams.iter().position(|allowed| {
                    v8::Local::new(scope, allowed).strict_equals(object.into())
                }) else {
                    throw_data_clone_exception(
                        scope,
                        "ReadableStream must be listed in the postMessage transfer list.",
                    );
                    return None;
                };
                let Ok(clone_id) = u32::try_from(index) else {
                    throw_data_clone_exception(
                        scope,
                        "Too many ReadableStreams in structured clone.",
                    );
                    return None;
                };
                serializer.write_uint32(HOST_OBJECT_TAG_READABLE_STREAM);
                serializer.write_uint32(clone_id);
                return Some(true);
            }
            Some("WritableStream") => {
                let Some(index) = self.allowed_writable_streams.iter().position(|allowed| {
                    v8::Local::new(scope, allowed).strict_equals(object.into())
                }) else {
                    throw_data_clone_exception(
                        scope,
                        "WritableStream must be listed in the postMessage transfer list.",
                    );
                    return None;
                };
                let Ok(clone_id) = u32::try_from(index) else {
                    throw_data_clone_exception(
                        scope,
                        "Too many WritableStreams in structured clone.",
                    );
                    return None;
                };
                serializer.write_uint32(HOST_OBJECT_TAG_WRITABLE_STREAM);
                serializer.write_uint32(clone_id);
                return Some(true);
            }
            Some("TransformStream") => {
                let Some(index) = self.allowed_transform_streams.iter().position(|allowed| {
                    v8::Local::new(scope, allowed).strict_equals(object.into())
                }) else {
                    throw_data_clone_exception(
                        scope,
                        "TransformStream must be listed in the postMessage transfer list.",
                    );
                    return None;
                };
                let Ok(clone_id) = u32::try_from(index) else {
                    throw_data_clone_exception(
                        scope,
                        "Too many TransformStreams in structured clone.",
                    );
                    return None;
                };
                serializer.write_uint32(HOST_OBJECT_TAG_TRANSFORM_STREAM);
                serializer.write_uint32(clone_id);
                return Some(true);
            }
            Some("Blob" | "File") => {
                if let Some(payload) = blob_clone_payload_from_object(scope, object) {
                    let mut store = self.blobs.borrow_mut();
                    let clone_id = store.next_id;
                    let Some(next_id) = store.next_id.checked_add(1) else {
                        drop(store);
                        throw_data_clone_exception(scope, "Too many Blobs in structured clone.");
                        return None;
                    };
                    store.next_id = next_id;
                    store.blobs.push(ClonedBlob { clone_id, payload });
                    serializer.write_uint32(HOST_OBJECT_TAG_BLOB);
                    serializer.write_uint32(clone_id);
                    return Some(true);
                }
            }
            Some("FileSystemFileHandle" | "FileSystemDirectoryHandle") => {
                if let Some(payload) = file_system_handle_clone_payload_from_object(scope, object) {
                    let mut store = self.file_system_handles.borrow_mut();
                    let clone_id = store.next_id;
                    let Some(next_id) = store.next_id.checked_add(1) else {
                        drop(store);
                        throw_data_clone_exception(
                            scope,
                            "Too many FileSystemHandles in structured clone.",
                        );
                        return None;
                    };
                    store.next_id = next_id;
                    store
                        .handles
                        .push(ClonedFileSystemHandle { clone_id, payload });
                    serializer.write_uint32(HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE);
                    serializer.write_uint32(clone_id);
                    return Some(true);
                }
            }
            Some("QuotaExceededError") => {
                if let Some((message, quota, requested)) =
                    quota_exceeded_error_clone_fields(scope, object)
                {
                    serializer.write_uint32(HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR);
                    write_string(serializer, &message);
                    write_optional_double(serializer, quota);
                    write_optional_double(serializer, requested);
                    return Some(true);
                }
            }
            Some("DOMException") => {
                if let Some((message, name)) = dom_exception_clone_fields(scope, object) {
                    serializer.write_uint32(HOST_OBJECT_TAG_DOM_EXCEPTION);
                    write_string(serializer, &message);
                    write_string(serializer, &name);
                    return Some(true);
                }
            }
            _ => {}
        }
        throw_data_clone_exception(scope, "Unsupported host object during structured clone.");
        None
    }

    fn get_shared_array_buffer_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        _buffer: v8::Local<'s, v8::SharedArrayBuffer>,
    ) -> Option<u32> {
        throw_data_clone_exception(scope, "SharedArrayBuffer could not be cloned.");
        None
    }

    fn get_wasm_module_transfer_id(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        module: v8::Local<v8::WasmModuleObject>,
    ) -> Option<u32> {
        if !self.policy.allows_wasm_module() {
            throw_data_clone_exception(
                scope,
                "A WebAssembly.Module can not be serialized for storage.",
            );
            return None;
        }

        let mut store = self.wasm_modules.borrow_mut();
        let clone_id = store.next_id;
        let Some(next_id) = store.next_id.checked_add(1) else {
            drop(store);
            throw_data_clone_exception(scope, "Too many WebAssembly modules in structured clone.");
            return None;
        };
        store.next_id = next_id;
        store.modules.push(ClonedWasmModule {
            clone_id,
            compiled_module: Arc::new(module.get_compiled_module()),
            instantiation_exceeds_v8_limit:
                crate::context_bootstrap::module_instantiation_exceeds_v8_limit(
                    scope,
                    module.into(),
                ),
        });
        Some(clone_id)
    }
}

struct WireDeserializer {
    wasm_modules: HashMap<u32, ClonedWasmModule>,
    readable_streams: HashMap<u32, ReadableStreamClonePayload>,
    writable_streams: HashMap<u32, WritableStreamClonePayload>,
    transform_streams: HashMap<u32, TransformStreamClonePayload>,
    deferred_streams: Rc<RefCell<Vec<DeferredStreamMaterialization>>>,
    blobs: HashMap<u32, BlobClonePayload>,
    file_system_handles: HashMap<u32, FileSystemHandleClonePayload>,
}

impl v8::ValueDeserializerImpl for WireDeserializer {
    fn read_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        deserializer: &dyn v8::ValueDeserializerHelper,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let mut tag = 0;
        if !deserializer.read_uint32(&mut tag) {
            throw_data_clone_exception(
                scope,
                "Failed to deserialize structured clone host object.",
            );
            return None;
        }
        match tag {
            HOST_OBJECT_TAG_MESSAGE_PORT => {
                let mut port_id = 0;
                if !deserializer.read_uint64(&mut port_id) {
                    throw_data_clone_exception(
                        scope,
                        "Failed to deserialize transferred MessagePort.",
                    );
                    return None;
                }
                ensure_message_port_wrapper_for_id(scope, port_id)
            }
            HOST_OBJECT_TAG_IMAGE_DATA => read_image_data_payload(scope, deserializer),
            HOST_OBJECT_TAG_CRYPTO_KEY => {
                read_crypto_key_payload(scope, deserializer).or_else(|| {
                    throw_data_clone_exception(
                        scope,
                        "Failed to deserialize structured clone CryptoKey.",
                    );
                    None
                })
            }
            HOST_OBJECT_TAG_READABLE_STREAM => {
                let clone_id = read_u32(deserializer)?;
                let Some(payload) = self.readable_streams.get(&clone_id).cloned() else {
                    throw_data_clone_exception(
                        scope,
                        "Missing ReadableStream payload during structured clone.",
                    );
                    return None;
                };
                // V8 invokes host-object decoding inside a
                // DisallowJavascriptExecutionScope. Return the final stream
                // identity now, but defer controller/Promise/MessagePort
                // initialization until ReadValue has left that scope.
                let shell = build_readable_stream_clone_shell(scope);
                self.deferred_streams
                    .borrow_mut()
                    .push(DeferredStreamMaterialization::Readable {
                        shell: v8::Global::new(scope, shell),
                        payload,
                    });
                Some(shell)
            }
            HOST_OBJECT_TAG_WRITABLE_STREAM => {
                let clone_id = read_u32(deserializer)?;
                let Some(payload) = self.writable_streams.get(&clone_id).cloned() else {
                    throw_data_clone_exception(
                        scope,
                        "Missing WritableStream payload during structured clone.",
                    );
                    return None;
                };
                let shell = build_writable_stream_clone_shell(scope);
                self.deferred_streams
                    .borrow_mut()
                    .push(DeferredStreamMaterialization::Writable {
                        shell: v8::Global::new(scope, shell),
                        payload,
                    });
                Some(shell)
            }
            HOST_OBJECT_TAG_TRANSFORM_STREAM => {
                let clone_id = read_u32(deserializer)?;
                let Some(payload) = self.transform_streams.get(&clone_id).cloned() else {
                    throw_data_clone_exception(
                        scope,
                        "Missing TransformStream payload during structured clone.",
                    );
                    return None;
                };
                let shell = build_transform_stream_clone_shell(scope);
                self.deferred_streams
                    .borrow_mut()
                    .push(DeferredStreamMaterialization::Transform {
                        shell: v8::Global::new(scope, shell),
                        payload,
                    });
                Some(shell)
            }
            HOST_OBJECT_TAG_BLOB => {
                let clone_id = read_u32(deserializer)?;
                let Some(payload) = self.blobs.get(&clone_id) else {
                    throw_data_clone_exception(
                        scope,
                        "Missing Blob payload during structured clone.",
                    );
                    return None;
                };
                build_blob_object_from_clone_payload(scope, payload)
            }
            HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE => {
                let clone_id = read_u32(deserializer)?;
                let Some(payload) = self.file_system_handles.get(&clone_id) else {
                    throw_data_clone_exception(
                        scope,
                        "Missing FileSystemHandle payload during structured clone.",
                    );
                    return None;
                };
                build_file_system_handle_from_clone_payload(scope, payload).or_else(|| {
                    throw_data_clone_exception(
                        scope,
                        "FileSystemHandle is not authorized for this storage context.",
                    );
                    None
                })
            }
            HOST_OBJECT_TAG_DOM_EXCEPTION => {
                let message = read_string(deserializer)?;
                let name = read_string(deserializer)?;
                v8::Local::<v8::Object>::try_from(new_dom_exception_value(scope, &message, &name))
                    .ok()
            }
            HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR => {
                let message = read_string(deserializer)?;
                let quota = read_optional_double(deserializer)?;
                let requested = read_optional_double(deserializer)?;
                v8::Local::<v8::Object>::try_from(new_quota_exceeded_error_value(
                    scope, &message, quota, requested,
                ))
                .ok()
            }
            _ => {
                throw_data_clone_exception(scope, "Unsupported structured clone host object.");
                None
            }
        }
    }

    fn get_wasm_module_from_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        clone_id: u32,
    ) -> Option<v8::Local<'s, v8::WasmModuleObject>> {
        let Some(cloned_module) = self.wasm_modules.get(&clone_id) else {
            throw_data_clone_exception(
                scope,
                "Missing WebAssembly.Module payload during structured clone.",
            );
            return None;
        };
        let module = v8::WasmModuleObject::from_compiled_module(
            scope,
            cloned_module.compiled_module.as_ref(),
        )?;
        if cloned_module.instantiation_exceeds_v8_limit {
            crate::context_bootstrap::mark_module_instantiation_exceeds_v8_limit(
                scope,
                module.into(),
            );
        }
        rehome_deserialized_wasm_module_for_active_child_window(scope, module);
        Some(module)
    }
}

fn rehome_deserialized_wasm_module_for_active_child_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::WasmModuleObject>,
) {
    let Some(_handle) = crate::native_bridge::active_child_window_handle(scope) else {
        return;
    };
    let context = scope.get_current_context();
    let Some(prototype) = crate::context_bootstrap::webassembly_default_prototype_for_context(
        scope, context, "Module",
    ) else {
        return;
    };

    // Keep this scoped to WebAssembly.Module structured clone. Message
    // delivery enters the target realm before deserialization, so use that
    // context's stable intrinsic prototype rather than a mutable global.
    let object = v8::Local::<v8::Object>::from(module);
    let _ = object.set_prototype(scope, prototype.into());
}

pub(crate) fn blob_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobClonePayload> {
    if let Some(file) = selected_file_from_object(scope, object) {
        return Some(BlobClonePayload::File {
            file,
            opfs_snapshot: file_system_file_snapshot_clone_payload_from_object(scope, object),
        });
    }
    let bytes = crate::blob::blob_bytes_from_object(scope, object)?;
    let mime_type = crate::blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    Some(BlobClonePayload::Blob { bytes, mime_type })
}

pub(crate) fn build_blob_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &BlobClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    match payload {
        BlobClonePayload::Blob { bytes, mime_type } => {
            crate::blob::build_blob_object(scope, bytes.clone(), mime_type.clone())
        }
        BlobClonePayload::File {
            file,
            opfs_snapshot,
        } => {
            let object = build_file_object(scope, file)?;
            if let Some(snapshot) = opfs_snapshot {
                attach_file_system_file_snapshot_clone_payload(scope, object, snapshot)?;
            }
            Some(object)
        }
    }
}

fn write_image_data_payload(
    serializer: &dyn v8::ValueSerializerHelper,
    payload: ImageDataClonePayload,
) {
    serializer.write_uint32(HOST_OBJECT_TAG_IMAGE_DATA);
    serializer.write_uint32(payload.width);
    serializer.write_uint32(payload.height);
    write_raw_vec(serializer, payload.color_space.as_bytes());
    write_raw_vec(serializer, &payload.bytes);
}

fn read_image_data_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut width = 0;
    let mut height = 0;
    if !deserializer.read_uint32(&mut width) || !deserializer.read_uint32(&mut height) {
        throw_data_clone_exception(scope, "Failed to deserialize ImageData dimensions.");
        return None;
    }
    let color_space = read_raw_vec(deserializer)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| "srgb".to_owned());
    let Some(bytes) = read_raw_vec(deserializer) else {
        throw_data_clone_exception(scope, "Failed to deserialize ImageData pixel data.");
        return None;
    };
    build_image_data_object_from_clone_payload(
        scope,
        ImageDataClonePayload {
            width,
            height,
            color_space,
            bytes,
        },
    )
}

pub(crate) fn write_crypto_key_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    serializer: &dyn v8::ValueSerializerHelper,
) -> Option<()> {
    let payload = crypto_key_clone_payload_from_object(scope, object)?;
    serializer.write_uint32(HOST_OBJECT_TAG_CRYPTO_KEY);
    write_string(serializer, &payload.key_type);
    write_string(serializer, &payload.algorithm.name);
    write_optional_string(serializer, payload.algorithm.hash_name.as_deref());
    write_optional_usize(serializer, payload.algorithm.length_bits)?;
    write_optional_string(serializer, payload.algorithm.named_curve.as_deref());
    write_optional_usize(serializer, payload.algorithm.modulus_length_bits)?;
    write_optional_raw_vec(serializer, payload.algorithm.public_exponent.as_deref())?;
    serializer.write_uint32(u32::from(payload.extractable));
    serializer.write_uint32(payload.usages.len().try_into().ok()?);
    for usage in &payload.usages {
        write_string(serializer, usage);
    }
    write_raw_vec(serializer, &payload.key_bytes);
    Some(())
}

pub(crate) fn read_crypto_key_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let key_type = read_string(deserializer)?;
    let algorithm_name = read_string(deserializer)?;
    let hash_name = read_optional_string(deserializer)?;
    let length_bits = read_optional_usize(deserializer)?;
    let named_curve = read_optional_string(deserializer)?;
    let modulus_length_bits = read_optional_usize(deserializer)?;
    let public_exponent = read_optional_raw_vec(deserializer)?;
    let extractable = read_u32(deserializer)? != 0;
    let usage_count = read_u32(deserializer)?;
    if usage_count > 8 {
        return None;
    }
    let mut usages = Vec::with_capacity(usage_count as usize);
    for _ in 0..usage_count {
        usages.push(read_string(deserializer)?);
    }
    let key_bytes = read_raw_vec(deserializer)?;
    crypto_key_object_from_clone_payload(
        scope,
        CryptoKeyClonePayload {
            key_type,
            algorithm: CryptoKeyAlgorithmClonePayload {
                name: algorithm_name,
                hash_name,
                length_bits,
                named_curve,
                modulus_length_bits,
                public_exponent,
            },
            extractable,
            usages,
            key_bytes,
        },
    )
}

fn write_string(serializer: &dyn v8::ValueSerializerHelper, value: &str) {
    write_raw_vec(serializer, value.as_bytes());
}

fn write_optional_string(serializer: &dyn v8::ValueSerializerHelper, value: Option<&str>) {
    match value {
        Some(value) => {
            serializer.write_uint32(1);
            write_string(serializer, value);
        }
        None => serializer.write_uint32(0),
    }
}

fn write_optional_double(serializer: &dyn v8::ValueSerializerHelper, value: Option<f64>) {
    match value {
        Some(value) => {
            serializer.write_uint32(1);
            serializer.write_double(value);
        }
        None => serializer.write_uint32(0),
    }
}

fn write_optional_usize(
    serializer: &dyn v8::ValueSerializerHelper,
    value: Option<usize>,
) -> Option<()> {
    match value {
        Some(value) => {
            serializer.write_uint32(1);
            serializer.write_uint32(value.try_into().ok()?);
        }
        None => serializer.write_uint32(0),
    }
    Some(())
}

fn write_optional_raw_vec(
    serializer: &dyn v8::ValueSerializerHelper,
    value: Option<&[u8]>,
) -> Option<()> {
    match value {
        Some(value) => {
            serializer.write_uint32(1);
            write_raw_vec(serializer, value);
        }
        None => serializer.write_uint32(0),
    }
    Some(())
}

fn read_u32(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<u32> {
    let mut value = 0;
    deserializer.read_uint32(&mut value).then_some(value)
}

fn read_string(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<String> {
    String::from_utf8(read_raw_vec(deserializer)?).ok()
}

fn read_optional_string(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<Option<String>> {
    match read_u32(deserializer)? {
        0 => Some(None),
        1 => Some(Some(read_string(deserializer)?)),
        _ => None,
    }
}

fn read_optional_double(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<Option<f64>> {
    match read_u32(deserializer)? {
        0 => Some(None),
        1 => {
            let mut value = 0.0;
            deserializer.read_double(&mut value).then_some(Some(value))
        }
        _ => None,
    }
}

fn read_optional_usize(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<Option<usize>> {
    match read_u32(deserializer)? {
        0 => Some(None),
        1 => Some(Some(read_u32(deserializer)? as usize)),
        _ => None,
    }
}

fn read_optional_raw_vec(
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<Option<Vec<u8>>> {
    match read_u32(deserializer)? {
        0 => Some(None),
        1 => Some(Some(read_raw_vec(deserializer)?)),
        _ => None,
    }
}

fn write_raw_vec(serializer: &dyn v8::ValueSerializerHelper, bytes: &[u8]) {
    serializer.write_uint32(bytes.len() as u32);
    serializer.write_raw_bytes(bytes);
}

fn read_raw_vec(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<Vec<u8>> {
    let mut len = 0;
    if !deserializer.read_uint32(&mut len) {
        return None;
    }
    deserializer.read_raw_bytes(len as usize).map(Vec::from)
}

fn throw_data_clone_exception<'s>(scope: &mut v8::PinScope<'s, '_>, message: &str) {
    let exception = new_dom_exception_value(scope, message, "DataCloneError");
    scope.throw_exception(exception);
}

fn copy_array_buffer_bytes(buffer: v8::Local<'_, v8::ArrayBuffer>) -> Vec<u8> {
    let backing_store = buffer.get_backing_store();
    let length = backing_store.byte_length();
    if length == 0 {
        return Vec::new();
    }
    let Some(data) = backing_store.data() else {
        return Vec::new();
    };
    // SAFETY: `BackingStore::data()` is valid for the lifetime of the backing
    // store, which we keep alive for the duration of this copy.
    unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), length) }.to_vec()
}

pub(crate) fn serialize_for_wire_for_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        &[],
        &[],
        &[],
        &[],
        &[],
        StructuredClonePolicy::Runtime,
    )
}

pub(crate) fn serialize_for_wire_for_runtime_with_transfers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    array_buffer_transfers: &[v8::Local<'s, v8::ArrayBuffer>],
    message_port_transfers: &[v8::Local<'s, v8::Object>],
    readable_stream_transfers: &[v8::Local<'s, v8::Object>],
    writable_stream_transfers: &[v8::Local<'s, v8::Object>],
    transform_stream_transfers: &[v8::Local<'s, v8::Object>],
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        array_buffer_transfers,
        message_port_transfers,
        readable_stream_transfers,
        writable_stream_transfers,
        transform_stream_transfers,
        StructuredClonePolicy::Runtime,
    )
}

pub(crate) fn serialize_for_wire_for_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        &[],
        &[],
        &[],
        &[],
        &[],
        StructuredClonePolicy::Storage,
    )
}

pub(crate) fn serialize_for_wire_for_runtime_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    array_buffer_transfers: &[v8::Local<'s, v8::ArrayBuffer>],
    message_port_transfers: &[v8::Local<'s, v8::Object>],
    readable_stream_transfers: &[v8::Local<'s, v8::Object>],
    writable_stream_transfers: &[v8::Local<'s, v8::Object>],
    transform_stream_transfers: &[v8::Local<'s, v8::Object>],
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        array_buffer_transfers,
        message_port_transfers,
        readable_stream_transfers,
        writable_stream_transfers,
        transform_stream_transfers,
        StructuredClonePolicy::RuntimeMessage,
    )
}

fn serialize_for_wire_with_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    array_buffer_transfers: &[v8::Local<'s, v8::ArrayBuffer>],
    message_port_transfers: &[v8::Local<'s, v8::Object>],
    readable_stream_transfers: &[v8::Local<'s, v8::Object>],
    writable_stream_transfers: &[v8::Local<'s, v8::Object>],
    transform_stream_transfers: &[v8::Local<'s, v8::Object>],
    policy: StructuredClonePolicy,
) -> Option<V8StructuredClonePayload> {
    let context = scope.get_current_context();
    let transferred_message_ports: Vec<MessagePortId> = message_port_transfers
        .iter()
        .filter_map(|port| message_port_id_from_object(scope, *port))
        .collect();
    let wasm_modules = Rc::new(RefCell::new(ClonedWasmModuleStore::default()));
    let blobs = Rc::new(RefCell::new(ClonedBlobStore::default()));
    let file_system_handles = Rc::new(RefCell::new(ClonedFileSystemHandleStore::default()));
    let serializer = v8::ValueSerializer::new(
        scope,
        Box::new(WireSerializer {
            allowed_message_port_ids: transferred_message_ports.iter().copied().collect(),
            allowed_readable_streams: readable_stream_transfers
                .iter()
                .map(|stream| v8::Global::new(scope, *stream))
                .collect(),
            allowed_writable_streams: writable_stream_transfers
                .iter()
                .map(|stream| v8::Global::new(scope, *stream))
                .collect(),
            allowed_transform_streams: transform_stream_transfers
                .iter()
                .map(|stream| v8::Global::new(scope, *stream))
                .collect(),
            policy,
            wasm_modules: Rc::clone(&wasm_modules),
            blobs: Rc::clone(&blobs),
            file_system_handles: Rc::clone(&file_system_handles),
        }),
    );
    serializer.write_header();
    let mut transferred_array_buffers = Vec::with_capacity(array_buffer_transfers.len());
    for (index, buffer) in array_buffer_transfers.iter().enumerate() {
        if buffer.get_backing_store().is_resizable_by_user_javascript() {
            // V8's inline wire representation preserves a resizable backing
            // store's maximum length and length-tracking views. The transfer
            // list still detaches the sender-side buffer after serialization.
            continue;
        }
        let transfer_id = index as u32 + 1;
        serializer.transfer_array_buffer(transfer_id, *buffer);
        transferred_array_buffers.push(TransferredArrayBuffer {
            transfer_id,
            bytes: copy_array_buffer_bytes(*buffer),
        });
    }
    let Some(true) = serializer.write_value(context, value) else {
        return None;
    };
    for buffer in array_buffer_transfers {
        if buffer.detach(None) != Some(true) {
            throw_data_clone_exception(scope, "Failed to transfer ArrayBuffer.");
            return None;
        }
    }
    for port in message_port_transfers {
        let Some(port_id) = message_port_id_from_object(scope, *port) else {
            continue;
        };
        detach_message_port_owner_for_transfer(scope, port_id);
        detach_transferred_message_port(scope, *port);
    }
    let mut readable_streams = Vec::with_capacity(readable_stream_transfers.len());
    for (clone_id, stream) in readable_stream_transfers.iter().enumerate() {
        let prepared = prepare_readable_stream_transfer(scope, *stream)?;
        let clone_id =
            u32::try_from(clone_id).expect("a JavaScript transfer sequence length fits in u32");
        readable_streams.push(ClonedReadableStream {
            clone_id,
            payload: prepared.commit(scope),
        });
    }
    let mut writable_streams = Vec::with_capacity(writable_stream_transfers.len());
    for (clone_id, stream) in writable_stream_transfers.iter().enumerate() {
        let prepared = prepare_writable_stream_transfer(scope, *stream)?;
        let clone_id =
            u32::try_from(clone_id).expect("a JavaScript transfer sequence length fits in u32");
        writable_streams.push(ClonedWritableStream {
            clone_id,
            payload: prepared.commit(scope),
        });
    }
    let mut transform_streams = Vec::with_capacity(transform_stream_transfers.len());
    for (clone_id, stream) in transform_stream_transfers.iter().enumerate() {
        let prepared = prepare_transform_stream_transfer(scope, *stream)?;
        let clone_id =
            u32::try_from(clone_id).expect("a JavaScript transfer sequence length fits in u32");
        transform_streams.push(ClonedTransformStream {
            clone_id,
            payload: prepared.commit(scope),
        });
    }
    let wasm_modules = wasm_modules.borrow().modules.clone();
    let blobs = blobs.borrow().blobs.clone();
    let file_system_handles = file_system_handles.borrow().handles.clone();
    let metadata = policy.metadata_for_wasm_modules(!wasm_modules.is_empty());
    Some(V8StructuredClonePayload {
        base: StructuredCloneWireBytes {
            wire_bytes: serializer.release(),
            transferred_array_buffers,
            transferred_message_ports,
        },
        wasm_modules,
        readable_streams,
        writable_streams,
        transform_streams,
        blobs,
        file_system_handles,
        metadata,
    })
}

pub(crate) fn deserialize_from_wire<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<v8::Local<'s, v8::Value>> {
    deserialize_from_wire_impl(scope, payload)
}

pub(crate) fn deserialize_message_event_from_wire<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<(v8::Local<'s, v8::Value>, v8::Local<'s, v8::Array>)> {
    if validate_message_event_metadata(scope, payload).is_none() {
        discard_all_transferred_stream_channels(scope, payload);
        return None;
    }
    let value = deserialize_from_wire_impl(scope, payload)?;
    let ports = v8::Array::new(scope, payload.transferred_message_ports().len() as i32);
    for (index, port_id) in payload.transferred_message_ports().iter().enumerate() {
        if let Some(port) = ensure_message_port_wrapper_for_id(scope, *port_id) {
            let _ = ports.set_index(scope, index as u32, v8::Local::<v8::Value>::from(port));
        }
    }
    let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    Some((value, ports))
}

fn validate_message_event_metadata(
    scope: &mut v8::PinScope<'_, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<()> {
    let metadata = &payload.metadata;
    if metadata.contains_wasm_module
        && (!metadata.origin_check_required
            || !metadata.locked_to_sender_agent_cluster
            || metadata.sender_agent_cluster.is_none())
    {
        throw_data_clone_exception(scope, "Invalid WebAssembly.Module message clone metadata.");
        return None;
    }
    Some(())
}

fn deserialize_from_wire_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<v8::Local<'s, v8::Value>> {
    let context = scope.get_current_context();
    let wasm_modules = payload
        .wasm_modules
        .iter()
        .map(|module| (module.clone_id, module.clone()))
        .collect();
    let readable_streams = payload
        .readable_streams
        .iter()
        .map(|stream| (stream.clone_id, stream.payload.clone()))
        .collect();
    let writable_streams = payload
        .writable_streams
        .iter()
        .map(|stream| (stream.clone_id, stream.payload.clone()))
        .collect();
    let transform_streams = payload
        .transform_streams
        .iter()
        .map(|stream| (stream.clone_id, stream.payload.clone()))
        .collect();
    let blobs = payload
        .blobs
        .iter()
        .map(|blob| (blob.clone_id, blob.payload.clone()))
        .collect();
    let file_system_handles = payload
        .file_system_handles
        .iter()
        .map(|handle| (handle.clone_id, handle.payload.clone()))
        .collect();
    let deferred_streams = Rc::new(RefCell::new(Vec::new()));
    let deserializer = v8::ValueDeserializer::new(
        scope,
        Box::new(WireDeserializer {
            wasm_modules,
            readable_streams,
            writable_streams,
            transform_streams,
            deferred_streams: Rc::clone(&deferred_streams),
            blobs,
            file_system_handles,
        }),
        &payload.base.wire_bytes,
    );
    let Some(true) = deserializer.read_header(context) else {
        discard_all_transferred_stream_channels(scope, payload);
        return None;
    };
    for transfer in &payload.base.transferred_array_buffers {
        let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(transfer.bytes.clone());
        let backing_store = backing_store.make_shared();
        let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
        deserializer.transfer_array_buffer(transfer.transfer_id, buffer);
    }
    let Some(value) = deserializer.read_value(context) else {
        drop(deserializer);
        discard_all_transferred_stream_channels(scope, payload);
        return None;
    };
    drop(deserializer);

    let deferred = std::mem::take(&mut *deferred_streams.borrow_mut());
    discard_unclaimed_transferred_stream_channels(scope, payload, &deferred);
    for materialization in &deferred {
        if materialization.initialize(scope).is_none() {
            discard_all_transferred_stream_channels(scope, payload);
            if crate::worker::worker_termination_requested(scope) {
                return None;
            }
            require_internal_stream_value::<()>(
                None,
                "clone-shell materialization",
                "deferred transferred Streams batch",
            );
        }
    }
    Some(value)
}

use crate::context_bootstrap::{
    WebTransportErrorClonePayload, WebTransportErrorSource,
    build_web_transport_error_from_clone_payload, web_transport_error_clone_payload_from_object,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt,
    rc::Rc,
    sync::Arc,
};

use crate::{
    context_bootstrap::{
        CryptoKeyAlgorithmClonePayload, CryptoKeyClonePayload, EncodedVideoChunkClonePayload,
        EncodedVideoChunkType, FileSystemFileSnapshotClonePayload, FileSystemHandleClonePayload,
        GeometryClonePayload, ImageDataClonePayload, ReadableStreamClonePayload,
        TransferredMessagePort, TransformStreamClonePayload, WritableStreamClonePayload,
        attach_file_system_file_snapshot_clone_payload, build_dom_exception_object,
        build_encoded_video_chunk_from_clone_payload, build_file_system_handle_from_clone_payload,
        build_geometry_object_from_clone_payload, build_geometry_quad_from_cloned_points,
        build_image_data_object_from_clone_payload, build_quota_exceeded_error_object,
        build_readable_stream_clone_shell, build_transform_stream_clone_shell,
        build_writable_stream_clone_shell, crypto_key_clone_payload_from_object,
        crypto_key_object_from_clone_payload, detach_message_port_owner_for_transfer,
        detach_transferred_message_port, discard_message_port_channel, dom_exception_clone_fields,
        encoded_video_chunk_clone_payload_from_object, ensure_message_port_wrapper_for_id,
        file_system_file_snapshot_clone_payload_from_object,
        file_system_handle_clone_payload_from_object, geometry_clone_payload_from_object,
        geometry_quad_clone_points, image_data_clone_payload_from_object,
        initialize_readable_stream_clone_shell, initialize_transform_stream_clone_shell,
        initialize_writable_stream_clone_shell, message_port_transfer_from_object,
        new_disconnected_message_port_object, new_dom_exception_value,
        prepare_readable_stream_transfer, prepare_transform_stream_transfer,
        prepare_writable_stream_transfer, quota_exceeded_error_clone_fields,
        require_internal_stream_value,
    },
    types::MessagePortId,
};
pub(crate) use moli_structured_clone::{
    StructuredCloneBytes as StructuredCloneWireBytes, TransferredArrayBuffer,
};
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

const HOST_OBJECT_TAG_MESSAGE_PORT: u32 = 1;
pub(crate) const HOST_OBJECT_TAG_IMAGE_DATA: u32 = 2;
pub(crate) const HOST_OBJECT_TAG_CRYPTO_KEY: u32 = 3;
const HOST_OBJECT_TAG_READABLE_STREAM: u32 = 4;
pub(crate) const HOST_OBJECT_TAG_BLOB: u32 = 5;
pub(crate) const HOST_OBJECT_TAG_DOM_EXCEPTION: u32 = 6;
pub(crate) const HOST_OBJECT_TAG_FILE_SYSTEM_HANDLE: u32 = 7;
pub(crate) const HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR: u32 = 8;
const HOST_OBJECT_TAG_WRITABLE_STREAM: u32 = 9;
const HOST_OBJECT_TAG_TRANSFORM_STREAM: u32 = 10;
const HOST_OBJECT_TAG_ENCODED_VIDEO_CHUNK: u32 = 13;
pub(crate) const HOST_OBJECT_TAG_WEB_TRANSPORT_ERROR: u32 = 14;
pub(crate) const HOST_OBJECT_TAG_GEOMETRY: u32 = 11;
const HOST_OBJECT_TAG_TRANSFERRED_MESSAGE_PORT: u32 = 22;
const HOST_OBJECT_TAG_RTC_ENCODED_AUDIO_FRAME: u32 = 16;
const HOST_OBJECT_TAG_RTC_ENCODED_VIDEO_FRAME: u32 = 17;

const GEOMETRY_KIND_DOM_POINT_READONLY: u32 = 0;
const GEOMETRY_KIND_DOM_POINT: u32 = 1;
const GEOMETRY_KIND_DOM_RECT_READONLY: u32 = 2;
const GEOMETRY_KIND_DOM_RECT: u32 = 3;
const GEOMETRY_KIND_DOM_QUAD: u32 = 4;
const GEOMETRY_KIND_DOM_MATRIX_READONLY: u32 = 5;
const GEOMETRY_KIND_DOM_MATRIX: u32 = 6;
// Kind 4 remains readable for existing numeric Quad payloads.
const GEOMETRY_KIND_DOM_QUAD_REFERENCES: u32 = 7;

#[derive(Clone, Debug, Default)]
pub(crate) struct V8StructuredClonePayload {
    pub(crate) base: StructuredCloneWireBytes<TransferredMessagePort>,
    wasm_modules: Vec<ClonedWasmModule>,
    readable_streams: Vec<ClonedReadableStream>,
    writable_streams: Vec<ClonedWritableStream>,
    transform_streams: Vec<ClonedTransformStream>,
    blobs: Vec<ClonedBlob>,
    file_system_handles: Vec<ClonedFileSystemHandle>,
    pub(crate) metadata: StructuredCloneMetadata,
}

impl V8StructuredClonePayload {
    pub(crate) fn transferred_message_port_ids(&self) -> impl Iterator<Item = MessagePortId> + '_ {
        self.base
            .transferred_message_ports
            .iter()
            .filter_map(|port| match port {
                TransferredMessagePort::Endpoint(id) => Some(*id),
                TransferredMessagePort::Closed => None,
            })
    }

    pub(crate) fn transferred_stream_port_ids(&self) -> impl Iterator<Item = MessagePortId> + '_ {
        self.readable_streams
            .iter()
            .map(|stream| stream.payload.port_id())
            .chain(
                self.writable_streams
                    .iter()
                    .map(|stream| stream.payload.port_id()),
            )
            .chain(
                self.transform_streams
                    .iter()
                    .flat_map(|stream| stream.payload.port_ids()),
            )
    }
}

/// Only closed-port attachments need a per-deserialization identity cache.
/// Ordinary endpoints continue to use their existing native wrapper registry.
#[derive(Clone, Default)]
struct ClosedMessagePortObjects(Rc<RefCell<HashMap<u32, ClosedPortObject>>>);

struct ClosedPortObject {
    id: MessagePortId,
    object: v8::Global<v8::Object>,
}

impl ClosedMessagePortObjects {
    fn discard(&self, scope: &mut v8::PinScope<'_, '_>) {
        for (_, port) in self.0.borrow_mut().drain() {
            discard_message_port_channel(scope, port.id);
        }
    }

    fn for_payload(payload: &V8StructuredClonePayload) -> Option<Self> {
        payload
            .base
            .transferred_message_ports
            .iter()
            .any(|port| matches!(port, TransferredMessagePort::Closed))
            .then(Self::default)
    }

    fn get<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        index: u32,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(port) = self.0.borrow().get(&index) {
            return Some(v8::Local::new(scope, &port.object));
        }
        let (id, object) = new_disconnected_message_port_object(scope)?;
        self.0.borrow_mut().insert(
            index,
            ClosedPortObject {
                id,
                object: v8::Global::new(scope, object),
            },
        );
        Some(object)
    }
}

fn transferred_message_port_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: TransferredMessagePort,
    index: u32,
    closed: Option<&ClosedMessagePortObjects>,
) -> Option<v8::Local<'s, v8::Object>> {
    match port {
        TransferredMessagePort::Endpoint(id) => ensure_message_port_wrapper_for_id(scope, id),
        TransferredMessagePort::Closed => closed?.get(scope, index),
    }
}

#[derive(Clone)]
struct StoredCloneAttachments {
    blobs: Vec<ClonedBlob>,
    file_system_handles: Vec<ClonedFileSystemHandle>,
}

/// History retains serialized bytes and native capabilities, never a live JS
/// graph. Storage serialization rejects transferred objects and Wasm modules.
pub(crate) fn serialize_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<moli_history::SerializedScriptValue> {
    let payload = serialize_for_wire_for_storage(scope, value)?;
    Some(moli_history::SerializedScriptValue::new(
        payload.base.wire_bytes,
        StoredCloneAttachments {
            blobs: payload.blobs,
            file_system_handles: payload.file_system_handles,
        },
    ))
}

pub(crate) fn deserialize_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &moli_history::SerializedScriptValue,
) -> Option<v8::Local<'s, v8::Value>> {
    let attachments = value.attachments::<StoredCloneAttachments>()?;
    let payload = V8StructuredClonePayload {
        base: StructuredCloneWireBytes {
            wire_bytes: value.bytes().to_vec(),
            ..Default::default()
        },
        blobs: attachments.blobs.clone(),
        file_system_handles: attachments.file_system_handles.clone(),
        ..Default::default()
    };
    deserialize_from_wire(scope, &payload)
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
        bytes: moli_file_api::BlobData,
        mime_type: String,
    },
    File {
        bytes: moli_file_api::BlobData,
        mime_type: String,
        name: String,
        last_modified: f64,
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
            sender_opaque_origin: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeMessageAgentCluster {
    WindowOrDedicatedWorker,
    SharedWorker,
    ServiceWorker,
}

/// Native opaque security-origin identity, independent of storage partitions
/// and their nonces. Window ids are owner-local, so include the host identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeMessageOpaqueOrigin {
    owner: RuntimeOpaqueOriginOwner,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuntimeOpaqueOriginOwner {
    Window {
        host: crate::resource_owner::ResourceOwnerId,
        window: crate::native_bridge::WindowExecutionContextOwner,
    },
    Worker(crate::resource_owner::ResourceOwnerId),
}

impl RuntimeMessageOpaqueOrigin {
    pub(crate) fn window(
        host: crate::resource_owner::ResourceOwnerId,
        window: crate::native_bridge::WindowExecutionContextOwner,
    ) -> Arc<Self> {
        Arc::new(Self {
            owner: RuntimeOpaqueOriginOwner::Window { host, window },
        })
    }

    pub(crate) fn worker(owner: crate::resource_owner::ResourceOwnerId) -> Arc<Self> {
        Arc::new(Self {
            owner: RuntimeOpaqueOriginOwner::Worker(owner),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StructuredCloneMetadata {
    pub(crate) contains_wasm_module: bool,
    pub(crate) origin_check_required: bool,
    pub(crate) locked_to_sender_agent_cluster: bool,
    pub(crate) sender_agent_cluster: Option<RuntimeMessageAgentCluster>,
    pub(crate) sender_origin: Option<String>,
    pub(crate) sender_opaque_origin: Option<Arc<RuntimeMessageOpaqueOrigin>>,
}

struct WireSerializer {
    transferred_message_ports: Vec<v8::Global<v8::Object>>,
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
            Some("WebTransportError") => {
                if let Some(payload) = web_transport_error_clone_payload_from_object(scope, object)
                {
                    write_web_transport_error_payload(serializer, &payload);
                    return Some(true);
                }
            }
            Some("MessagePort") => {
                let target = moli_webapi_declare::web_api_object_target(scope, object)?;
                let Some(index) = self
                    .transferred_message_ports
                    .iter()
                    .position(|port| v8::Local::new(scope, port).strict_equals(target.into()))
                else {
                    throw_data_clone_exception(
                        scope,
                        "MessagePort must be listed in the postMessage transfer list.",
                    );
                    return None;
                };
                serializer.write_uint32(HOST_OBJECT_TAG_TRANSFERRED_MESSAGE_PORT);
                serializer.write_uint32(index as u32);
                return Some(true);
            }
            Some(
                "DOMPoint" | "DOMPointReadOnly" | "DOMRect" | "DOMRectReadOnly" | "DOMQuad"
                | "DOMMatrix" | "DOMMatrixReadOnly",
            ) => {
                return write_geometry_clone_object(scope, object, serializer);
            }
            Some("ImageData") => {
                if let Some(payload) = image_data_clone_payload_from_object(scope, object) {
                    write_image_data_payload(serializer, payload);
                    return Some(true);
                }
            }
            Some("EncodedVideoChunk") => {
                if self.policy == StructuredClonePolicy::Storage {
                    throw_data_clone_exception(scope, "EncodedVideoChunk cannot be stored.");
                    return None;
                }
                if let Some(payload) = encoded_video_chunk_clone_payload_from_object(scope, object)
                {
                    write_encoded_video_chunk_payload(serializer, payload);
                    return Some(true);
                }
            }
            Some(name @ ("RTCEncodedAudioFrame" | "RTCEncodedVideoFrame")) => {
                if self.policy == StructuredClonePolicy::Storage {
                    throw_data_clone_exception(scope, "RTC encoded frames cannot be stored.");
                    return None;
                }
                use crate::context_bootstrap::encoded_frames::{FrameKind, write_clone};
                let (tag, kind) = if name == "RTCEncodedAudioFrame" {
                    (HOST_OBJECT_TAG_RTC_ENCODED_AUDIO_FRAME, FrameKind::Audio)
                } else {
                    (HOST_OBJECT_TAG_RTC_ENCODED_VIDEO_FRAME, FrameKind::Video)
                };
                serializer.write_uint32(tag);
                return write_clone(scope, object, serializer, kind);
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
            Some("QuotaExceededError")
                if write_quota_exceeded_error_payload(scope, object, serializer).is_some() =>
            {
                return Some(true);
            }
            Some("DOMException")
                if write_dom_exception_payload(scope, object, serializer).is_some() =>
            {
                return Some(true);
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
    transferred_message_ports: Vec<TransferredMessagePort>,
    closed_message_ports: Option<ClosedMessagePortObjects>,
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
            HOST_OBJECT_TAG_WEB_TRANSPORT_ERROR => {
                let payload = read_web_transport_error_payload(deserializer)?;
                build_web_transport_error_from_clone_payload(scope, &payload)
            }
            HOST_OBJECT_TAG_TRANSFERRED_MESSAGE_PORT => {
                let index = read_u32(deserializer);
                index
                    .and_then(|index| {
                        let port = *self.transferred_message_ports.get(index as usize)?;
                        transferred_message_port_object(
                            scope,
                            port,
                            index,
                            self.closed_message_ports.as_ref(),
                        )
                    })
                    .or_else(|| {
                        throw_data_clone_exception(
                            scope,
                            "Failed to deserialize transferred MessagePort.",
                        );
                        None
                    })
            }
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
            HOST_OBJECT_TAG_ENCODED_VIDEO_CHUNK => read_encoded_video_chunk_payload(deserializer)
                .and_then(|payload| build_encoded_video_chunk_from_clone_payload(scope, payload))
                .or_else(|| {
                    throw_data_clone_exception(scope, "Failed to deserialize EncodedVideoChunk.");
                    None
                }),
            HOST_OBJECT_TAG_RTC_ENCODED_AUDIO_FRAME | HOST_OBJECT_TAG_RTC_ENCODED_VIDEO_FRAME => {
                use crate::context_bootstrap::encoded_frames::{FrameKind, read_clone};
                let kind = if tag == HOST_OBJECT_TAG_RTC_ENCODED_AUDIO_FRAME {
                    FrameKind::Audio
                } else {
                    FrameKind::Video
                };
                read_clone(scope, deserializer, kind).or_else(|| {
                    throw_data_clone_exception(scope, "Failed to deserialize RTC encoded frame.");
                    None
                })
            }
            HOST_OBJECT_TAG_CRYPTO_KEY => {
                read_crypto_key_payload(scope, deserializer).or_else(|| {
                    throw_data_clone_exception(
                        scope,
                        "Failed to deserialize structured clone CryptoKey.",
                    );
                    None
                })
            }
            HOST_OBJECT_TAG_GEOMETRY => {
                read_geometry_clone_object(scope, deserializer).or_else(|| {
                    throw_data_clone_exception(
                        scope,
                        "Failed to deserialize structured clone Geometry object.",
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
            HOST_OBJECT_TAG_DOM_EXCEPTION => read_dom_exception_payload(scope, deserializer),
            HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR => {
                read_quota_exceeded_error_payload(scope, deserializer)
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
    let bytes = crate::blob::blob_data_from_object(scope, object)?;
    let mime_type = crate::blob::blob_mime_type_from_object(scope, object).unwrap_or_default();
    if let Some((name, last_modified)) =
        crate::context_bootstrap::file_clone_metadata(scope, object)
    {
        return Some(BlobClonePayload::File {
            bytes,
            mime_type,
            name,
            last_modified,
            opfs_snapshot: file_system_file_snapshot_clone_payload_from_object(scope, object),
        });
    }
    Some(BlobClonePayload::Blob { bytes, mime_type })
}

pub(crate) fn build_blob_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &BlobClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    match payload {
        BlobClonePayload::Blob { bytes, mime_type } => {
            crate::blob::build_blob_object_from_data(scope, bytes.clone(), mime_type.clone())
        }
        BlobClonePayload::File {
            bytes,
            mime_type,
            name,
            last_modified,
            opfs_snapshot,
        } => {
            let (object, _) = crate::context_bootstrap::build_file_object_unpublished(
                scope,
                bytes.clone(),
                mime_type.clone(),
                name,
                *last_modified,
            )?;
            if let Some(snapshot) = opfs_snapshot {
                attach_file_system_file_snapshot_clone_payload(scope, object, snapshot)?;
            }
            Some(object)
        }
    }
}

pub(crate) fn write_image_data_payload(
    serializer: &dyn v8::ValueSerializerHelper,
    payload: ImageDataClonePayload,
) {
    serializer.write_uint32(HOST_OBJECT_TAG_IMAGE_DATA);
    serializer.write_uint32(payload.width);
    serializer.write_uint32(payload.height);
    write_raw_vec(serializer, payload.color_space.as_bytes());
    write_raw_vec(serializer, &payload.bytes);
}

pub(crate) fn read_image_data_payload<'s>(
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

fn write_geometry_clone_payload(
    serializer: &dyn v8::ValueSerializerHelper,
    payload: GeometryClonePayload,
) {
    serializer.write_uint32(HOST_OBJECT_TAG_GEOMETRY);
    match payload {
        GeometryClonePayload::Point { mutable, values } => {
            serializer.write_uint32(if mutable {
                GEOMETRY_KIND_DOM_POINT
            } else {
                GEOMETRY_KIND_DOM_POINT_READONLY
            });
            write_doubles(serializer, &values);
        }
        GeometryClonePayload::Rect { mutable, values } => {
            serializer.write_uint32(if mutable {
                GEOMETRY_KIND_DOM_RECT
            } else {
                GEOMETRY_KIND_DOM_RECT_READONLY
            });
            write_doubles(serializer, &values);
        }
        GeometryClonePayload::Quad { points } => {
            serializer.write_uint32(GEOMETRY_KIND_DOM_QUAD);
            for point in points {
                write_doubles(serializer, &point);
            }
        }
        GeometryClonePayload::Matrix {
            mutable,
            is_2d,
            values,
        } => {
            serializer.write_uint32(if mutable {
                GEOMETRY_KIND_DOM_MATRIX
            } else {
                GEOMETRY_KIND_DOM_MATRIX_READONLY
            });
            serializer.write_uint32(u32::from(is_2d));
            if is_2d {
                write_doubles(
                    serializer,
                    &[
                        values[0], values[1], values[4], values[5], values[12], values[13],
                    ],
                );
            } else {
                write_doubles(serializer, &values);
            }
        }
    }
}

fn read_geometry_clone_payload(
    kind: u32,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<GeometryClonePayload> {
    match kind {
        GEOMETRY_KIND_DOM_POINT_READONLY | GEOMETRY_KIND_DOM_POINT => {
            Some(GeometryClonePayload::Point {
                mutable: kind == GEOMETRY_KIND_DOM_POINT,
                values: read_doubles(deserializer)?,
            })
        }
        GEOMETRY_KIND_DOM_RECT_READONLY | GEOMETRY_KIND_DOM_RECT => {
            Some(GeometryClonePayload::Rect {
                mutable: kind == GEOMETRY_KIND_DOM_RECT,
                values: read_doubles(deserializer)?,
            })
        }
        GEOMETRY_KIND_DOM_QUAD => {
            let mut points = [[0.0; 4]; 4];
            for point in &mut points {
                *point = read_doubles(deserializer)?;
            }
            Some(GeometryClonePayload::Quad { points })
        }
        GEOMETRY_KIND_DOM_MATRIX_READONLY | GEOMETRY_KIND_DOM_MATRIX => {
            let is_2d = match read_u32(deserializer)? {
                0 => false,
                1 => true,
                _ => return None,
            };
            let values = if is_2d {
                let [m11, m12, m21, m22, m41, m42] = read_doubles(deserializer)?;
                [
                    m11, m12, 0.0, 0.0, m21, m22, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, m41, m42, 0.0, 1.0,
                ]
            } else {
                read_doubles(deserializer)?
            };
            Some(GeometryClonePayload::Matrix {
                mutable: kind == GEOMETRY_KIND_DOM_MATRIX,
                is_2d,
                values,
            })
        }
        _ => None,
    }
}

fn write_doubles(serializer: &dyn v8::ValueSerializerHelper, values: &[f64]) {
    for value in values {
        serializer.write_double(*value);
    }
}

fn read_doubles<const N: usize>(
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<[f64; N]> {
    let mut values = [0.0; N];
    for value in &mut values {
        if !deserializer.read_double(value) {
            return None;
        }
    }
    Some(values)
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

pub(crate) fn write_dom_exception_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    serializer: &dyn v8::ValueSerializerHelper,
) -> Option<()> {
    let (message, name) = dom_exception_clone_fields(scope, object)?;
    serializer.write_uint32(HOST_OBJECT_TAG_DOM_EXCEPTION);
    write_string(serializer, &message);
    write_string(serializer, &name);
    Some(())
}

pub(crate) fn read_dom_exception_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let message = read_string(deserializer)?;
    let name = read_string(deserializer)?;
    Some(build_dom_exception_object(scope, &message, &name))
}

pub(crate) fn write_quota_exceeded_error_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    serializer: &dyn v8::ValueSerializerHelper,
) -> Option<()> {
    let (message, quota, requested) = quota_exceeded_error_clone_fields(scope, object)?;
    serializer.write_uint32(HOST_OBJECT_TAG_QUOTA_EXCEEDED_ERROR);
    write_string(serializer, &message);
    write_optional_double(serializer, quota);
    write_optional_double(serializer, requested);
    Some(())
}

pub(crate) fn read_quota_exceeded_error_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let message = read_string(deserializer)?;
    let quota = read_optional_double(deserializer)?;
    let requested = read_optional_double(deserializer)?;
    Some(build_quota_exceeded_error_object(
        scope, &message, quota, requested,
    ))
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

fn write_encoded_video_chunk_payload(
    serializer: &dyn v8::ValueSerializerHelper,
    payload: EncodedVideoChunkClonePayload,
) {
    serializer.write_uint32(HOST_OBJECT_TAG_ENCODED_VIDEO_CHUNK);
    serializer.write_uint32(match payload.kind {
        EncodedVideoChunkType::Key => 0,
        EncodedVideoChunkType::Delta => 1,
    });
    serializer.write_uint64(payload.timestamp as u64);
    serializer.write_uint32(u32::from(payload.duration.is_some()));
    if let Some(duration) = payload.duration {
        serializer.write_uint64(duration);
    }
    write_raw_vec(serializer, &payload.bytes);
}

fn read_encoded_video_chunk_payload(
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<EncodedVideoChunkClonePayload> {
    let kind = match read_u32(deserializer)? {
        0 => EncodedVideoChunkType::Key,
        1 => EncodedVideoChunkType::Delta,
        _ => return None,
    };
    let mut timestamp = 0;
    if !deserializer.read_uint64(&mut timestamp) {
        return None;
    }
    let duration = match read_u32(deserializer)? {
        0 => None,
        1 => {
            let mut duration = 0;
            if !deserializer.read_uint64(&mut duration) {
                return None;
            }
            Some(duration)
        }
        _ => return None,
    };
    Some(EncodedVideoChunkClonePayload {
        kind,
        timestamp: timestamp as i64,
        duration,
        bytes: read_raw_vec(deserializer)?,
    })
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

/// Transfer steps retain author order within each transfer phase. MessagePorts
/// move together after all other transfers and any Window target checks.
#[derive(Clone, Copy)]
pub(crate) enum WireTransfer<'s> {
    ArrayBuffer(v8::Local<'s, v8::ArrayBuffer>),
    MessagePort(v8::Local<'s, v8::Object>),
    ReadableStream(v8::Local<'s, v8::Object>),
    WritableStream(v8::Local<'s, v8::Object>),
    TransformStream(v8::Local<'s, v8::Object>),
}

#[derive(Default)]
pub(crate) struct WireTransfers<'a, 's> {
    pub entries: &'a [WireTransfer<'s>],
}

/// Serialized data whose MessagePorts have not yet been disentangled.
/// Rejecting a prepared message must release its private stream channels.
#[must_use]
pub(crate) struct PreparedStructuredClone<'s> {
    payload: V8StructuredClonePayload,
    message_ports: Vec<v8::Local<'s, v8::Object>>,
}

impl<'s> PreparedStructuredClone<'s> {
    pub(crate) fn metadata_mut(&mut self) -> &mut StructuredCloneMetadata {
        &mut self.payload.metadata
    }

    pub(crate) fn commit_message_ports(
        mut self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<V8StructuredClonePayload> {
        // Validate the entire list before moving any endpoint. A later invalid
        // port must not detach an earlier valid one.
        for port in self.message_ports.iter().copied() {
            let Some(transfer) = message_port_transfer_from_object(scope, port) else {
                self.discard(scope);
                throw_data_clone_exception(scope, "A transferred MessagePort is detached.");
                return None;
            };
            self.payload.base.transferred_message_ports.push(transfer);
        }
        for (port, transfer) in self
            .message_ports
            .iter()
            .copied()
            .zip(self.payload.base.transferred_message_ports.iter().copied())
        {
            if let TransferredMessagePort::Endpoint(port_id) = transfer {
                detach_message_port_owner_for_transfer(scope, port_id);
            }
            detach_transferred_message_port(scope, port);
        }
        Some(self.payload)
    }

    pub(crate) fn discard(self, scope: &mut v8::PinScope<'_, '_>) {
        // Port ids accumulated during validation still belong to the author.
        // Only private stream-transfer channels have been committed here.
        discard_all_transferred_stream_channels(scope, &self.payload);
    }

    fn apply_transfer_steps(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        transfers: &WireTransfers<'_, 's>,
    ) -> Option<()> {
        let mut array_buffer_index = 0;
        for transfer in transfers.entries {
            match *transfer {
                WireTransfer::ArrayBuffer(buffer) => {
                    array_buffer_index += 1;
                    if buffer.was_detached() {
                        throw_data_clone_exception(scope, "A transferred ArrayBuffer is detached.");
                        return None;
                    }
                    // Payload getters may have changed the backing bytes.
                    if !buffer.get_backing_store().is_resizable_by_user_javascript() {
                        self.payload
                            .base
                            .transferred_array_buffers
                            .push(TransferredArrayBuffer {
                                transfer_id: array_buffer_index,
                                bytes: copy_array_buffer_bytes(buffer),
                            });
                    }
                    if buffer.detach(None) != Some(true) {
                        throw_data_clone_exception(scope, "Failed to transfer ArrayBuffer.");
                        return None;
                    }
                }
                WireTransfer::MessagePort(_) => {}
                WireTransfer::ReadableStream(stream) => {
                    let prepared = prepare_readable_stream_transfer(scope, stream)?;
                    self.payload.readable_streams.push(ClonedReadableStream {
                        clone_id: self.payload.readable_streams.len() as u32,
                        payload: prepared.commit(scope),
                    });
                }
                WireTransfer::WritableStream(stream) => {
                    let prepared = prepare_writable_stream_transfer(scope, stream)?;
                    self.payload.writable_streams.push(ClonedWritableStream {
                        clone_id: self.payload.writable_streams.len() as u32,
                        payload: prepared.commit(scope),
                    });
                }
                WireTransfer::TransformStream(stream) => {
                    let prepared = prepare_transform_stream_transfer(scope, stream)?;
                    self.payload.transform_streams.push(ClonedTransformStream {
                        clone_id: self.payload.transform_streams.len() as u32,
                        payload: prepared.commit(scope),
                    });
                }
            }
        }
        Some(())
    }
}

pub(crate) fn serialize_for_wire_for_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        &WireTransfers::default(),
        StructuredClonePolicy::Runtime,
    )
}

pub(crate) fn serialize_for_wire_for_runtime_with_transfers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfers: &WireTransfers<'_, 's>,
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(scope, value, transfers, StructuredClonePolicy::Runtime)
}

pub(crate) fn serialize_for_wire_for_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        &WireTransfers::default(),
        StructuredClonePolicy::Storage,
    )
}

pub(crate) fn serialize_for_wire_for_runtime_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfers: &WireTransfers<'_, 's>,
) -> Option<V8StructuredClonePayload> {
    serialize_for_wire_with_policy(
        scope,
        value,
        transfers,
        StructuredClonePolicy::RuntimeMessage,
    )
}

fn serialize_for_wire_with_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfers: &WireTransfers<'_, 's>,
    policy: StructuredClonePolicy,
) -> Option<V8StructuredClonePayload> {
    prepare_for_wire_with_policy(scope, value, transfers, policy)?.commit_message_ports(scope)
}

pub(crate) fn prepare_for_window_post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfers: &WireTransfers<'_, 's>,
) -> Option<PreparedStructuredClone<'s>> {
    prepare_for_wire_with_policy(
        scope,
        value,
        transfers,
        StructuredClonePolicy::RuntimeMessage,
    )
}

fn prepare_for_wire_with_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfers: &WireTransfers<'_, 's>,
    policy: StructuredClonePolicy,
) -> Option<PreparedStructuredClone<'s>> {
    let mut array_buffer_transfers = Vec::new();
    let mut message_port_transfers = Vec::new();
    let mut readable_stream_transfers = Vec::new();
    let mut writable_stream_transfers = Vec::new();
    let mut transform_stream_transfers = Vec::new();
    for transfer in transfers.entries {
        match *transfer {
            WireTransfer::ArrayBuffer(buffer) => array_buffer_transfers.push(buffer),
            WireTransfer::MessagePort(port) => message_port_transfers.push(port),
            WireTransfer::ReadableStream(stream) => readable_stream_transfers.push(stream),
            WireTransfer::WritableStream(stream) => writable_stream_transfers.push(stream),
            WireTransfer::TransformStream(stream) => transform_stream_transfers.push(stream),
        }
    }
    let context = scope.get_current_context();
    let wasm_modules = Rc::new(RefCell::new(ClonedWasmModuleStore::default()));
    let blobs = Rc::new(RefCell::new(ClonedBlobStore::default()));
    let file_system_handles = Rc::new(RefCell::new(ClonedFileSystemHandleStore::default()));
    let serializer = v8::ValueSerializer::new(
        scope,
        Box::new(WireSerializer {
            transferred_message_ports: message_port_transfers
                .iter()
                .map(|port| v8::Global::new(scope, *port))
                .collect(),
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
    for (index, buffer) in array_buffer_transfers.iter().enumerate() {
        if buffer.get_backing_store().is_resizable_by_user_javascript() {
            // V8's inline wire representation preserves a resizable backing
            // store's maximum length and length-tracking views. The transfer
            // list still detaches the sender-side buffer after serialization.
            continue;
        }
        let transfer_id = index as u32 + 1;
        serializer.transfer_array_buffer(transfer_id, *buffer);
    }
    let Some(true) = serializer.write_value(context, value) else {
        return None;
    };
    let wasm_modules = wasm_modules.borrow().modules.clone();
    let blobs = blobs.borrow().blobs.clone();
    let file_system_handles = file_system_handles.borrow().handles.clone();
    let metadata = policy.metadata_for_wasm_modules(!wasm_modules.is_empty());
    let mut prepared = PreparedStructuredClone {
        payload: V8StructuredClonePayload {
            base: StructuredCloneWireBytes {
                wire_bytes: serializer.release(),
                transferred_array_buffers: Vec::with_capacity(array_buffer_transfers.len()),
                transferred_message_ports: Vec::with_capacity(message_port_transfers.len()),
            },
            wasm_modules,
            readable_streams: Vec::with_capacity(readable_stream_transfers.len()),
            writable_streams: Vec::with_capacity(writable_stream_transfers.len()),
            transform_streams: Vec::with_capacity(transform_stream_transfers.len()),
            blobs,
            file_system_handles,
            metadata,
        },
        message_ports: message_port_transfers,
    };
    if prepared.apply_transfer_steps(scope, transfers).is_none() {
        prepared.discard(scope);
        return None;
    }
    Some(prepared)
}

pub(crate) fn deserialize_from_wire<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<v8::Local<'s, v8::Value>> {
    let closed = ClosedMessagePortObjects::for_payload(payload);
    let value = deserialize_from_wire_impl(scope, payload, closed.clone());
    if value.is_none()
        && let Some(closed) = closed
    {
        closed.discard(scope);
    }
    value
}

pub(crate) fn deserialize_message_event_from_wire<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<(v8::Local<'s, v8::Value>, v8::Local<'s, v8::Array>)> {
    if validate_message_event_metadata(scope, payload).is_none() {
        discard_all_transferred_stream_channels(scope, payload);
        return None;
    }
    let closed = ClosedMessagePortObjects::for_payload(payload);
    let Some(value) = deserialize_from_wire_impl(scope, payload, closed.clone()) else {
        if let Some(closed) = closed {
            closed.discard(scope);
        }
        return None;
    };
    let ports = v8::Array::new(scope, payload.base.transferred_message_ports.len() as i32);
    for (index, transfer) in payload.base.transferred_message_ports.iter().enumerate() {
        let Some(port) =
            transferred_message_port_object(scope, *transfer, index as u32, closed.as_ref())
        else {
            if let Some(closed) = closed {
                closed.discard(scope);
            }
            discard_all_transferred_stream_channels(scope, payload);
            return None;
        };
        if ports.set_index(scope, index as u32, v8::Local::<v8::Value>::from(port)) != Some(true) {
            if let Some(closed) = closed {
                closed.discard(scope);
            }
            discard_all_transferred_stream_channels(scope, payload);
            return None;
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
    closed_message_ports: Option<ClosedMessagePortObjects>,
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
            transferred_message_ports: payload.base.transferred_message_ports.clone(),
            closed_message_ports,
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

pub(crate) fn write_geometry_clone_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    serializer: &dyn v8::ValueSerializerHelper,
) -> Option<bool> {
    if let Some(points) = geometry_quad_clone_points(scope, object) {
        serializer.write_uint32(HOST_OBJECT_TAG_GEOMETRY);
        serializer.write_uint32(GEOMETRY_KIND_DOM_QUAD_REFERENCES);
        let context = scope.get_current_context();
        for point in points {
            if !serializer.write_value(context, point.into())? {
                return Some(false);
            }
        }
        return Some(true);
    }
    let payload = geometry_clone_payload_from_object(scope, object)?;
    write_geometry_clone_payload(serializer, payload);
    Some(true)
}

pub(crate) fn read_geometry_clone_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let kind = read_u32(deserializer)?;
    if kind == GEOMETRY_KIND_DOM_QUAD_REFERENCES {
        let points = [
            read_geometry_quad_point(scope, deserializer)?,
            read_geometry_quad_point(scope, deserializer)?,
            read_geometry_quad_point(scope, deserializer)?,
            read_geometry_quad_point(scope, deserializer)?,
        ];
        return Some(build_geometry_quad_from_cloned_points(scope, points));
    }
    let payload = read_geometry_clone_payload(kind, deserializer)?;
    Some(build_geometry_object_from_clone_payload(scope, payload))
}

fn read_geometry_quad_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let value = deserializer.read_value(scope.get_current_context())?;
    let point = v8::Local::<v8::Object>::try_from(value).ok()?;
    crate::web_api_interfaces::DOMPoint::is_instance(scope, point).then_some(point)
}

pub(crate) fn write_web_transport_error_payload(
    serializer: &dyn v8::ValueSerializerHelper,
    payload: &WebTransportErrorClonePayload,
) {
    serializer.write_uint32(HOST_OBJECT_TAG_WEB_TRANSPORT_ERROR);
    // A new wire tag keeps lossless DOMString code units separate from the
    // existing UTF-8 DOMException encoding retained by older stored values.
    let bytes: Vec<_> = payload
        .message
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    write_raw_vec(serializer, &bytes);
    serializer.write_uint32(match payload.source {
        WebTransportErrorSource::Stream => 0,
        WebTransportErrorSource::Session => 1,
    });
    serializer.write_uint32(u32::from(payload.stream_error_code.is_some()));
    if let Some(code) = payload.stream_error_code {
        serializer.write_uint32(code);
    }
}

pub(crate) fn read_web_transport_error_payload(
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<WebTransportErrorClonePayload> {
    let bytes = read_raw_vec(deserializer)?;
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let message = bytes
        .chunks_exact(2)
        .map(|units| u16::from_le_bytes([units[0], units[1]]))
        .collect();
    let source = match read_u32(deserializer)? {
        0 => WebTransportErrorSource::Stream,
        1 => WebTransportErrorSource::Session,
        _ => return None,
    };
    let stream_error_code = match read_u32(deserializer)? {
        0 => None,
        1 => Some(read_u32(deserializer)?),
        _ => return None,
    };
    Some(WebTransportErrorClonePayload {
        message,
        source,
        stream_error_code,
    })
}

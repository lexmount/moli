use super::*;
use crate::context_bootstrap::{
    close_stream, enqueue_byte_chunk, error_stream, readable_stream_has_pipe_owner,
};
use crate::protocol_types::SubresourceResponseBody;
use crate::types::NetworkBodySourceId;
use crate::util::{get_private_value, set_private_value};
use crate::worker::get_worker_state;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use moli_web_mime::{is_form_urlencoded_mime, multipart_form_data_boundary};
use moli_webapi_declare::WebApiObject;
use parking_lot::Mutex;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

const BODY_SOURCE_KIND_BYTES: &str = "bytes";
const BODY_SOURCE_KIND_REGISTRY_BYTES: &str = "registry-bytes";
const BODY_SOURCE_KIND_SUBRESOURCE_BODY: &str = "subresource-body";
const BODY_SOURCE_KIND_PENDING_STREAM: &str = "pending-stream";
const FILTERED_RESPONSE_INTERNAL_BODY_SOURCE_SLOT: &str = "__lmFilteredResponseInternalBodySource";
const FILTERED_RESPONSE_INTERNAL_BODY_STREAM_SLOT: &str = "__lmFilteredResponseInternalBodyStream";
const NETWORK_BODY_SOURCE_ID_SLOT: &str = "__lmNetworkBodySourceId";
const NETWORK_BODY_SOURCE_OWNER_SLOT: &str = "__lmNetworkBodySourceOwner";
const NETWORK_BODY_SOURCE_CHUNK_SIZE: usize = 64 * 1024;
const NETWORK_BODY_SOURCE_MEMORY_LIMIT: usize = 1024 * 1024;
pub(in crate::network_host) const BODY_FORM_DATA_UNSUPPORTED_CONTENT_TYPE_ERROR_TEXT: &str =
    "Body.formData only supports application/x-www-form-urlencoded or multipart/form-data bodies";

static NEXT_NETWORK_BODY_SOURCE_ID: AtomicU64 = AtomicU64::new(1);
static NETWORK_BODY_SOURCES: OnceLock<Mutex<HashMap<NetworkBodySourceId, NetworkBodySourceState>>> =
    OnceLock::new();

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct OwnedNetworkBodySourceDeclaration<'scope> {
    #[webapi(slot = NETWORK_BODY_SOURCE_KIND_SLOT)]
    kind: &'static str,
    #[webapi(slot = NETWORK_BODY_BYTES_SLOT)]
    bytes: v8::Local<'scope, v8::ArrayBuffer>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct RegisteredNetworkBodySourceDeclaration<'scope> {
    #[webapi(slot = NETWORK_BODY_SOURCE_KIND_SLOT)]
    kind: &'static str,
    #[webapi(slot = NETWORK_BODY_SOURCE_ID_SLOT)]
    id: v8::Local<'scope, v8::BigInt>,
    #[webapi(slot = NETWORK_BODY_SOURCE_OWNER_SLOT)]
    owner: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(method, callback = registry_body_source_pull_callback, length = 1)]
    pull: (),
    #[webapi(method, callback = registry_body_source_cancel_callback, length = 1)]
    cancel: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct PendingNetworkBodySourceDeclaration<'scope> {
    #[webapi(slot = NETWORK_BODY_SOURCE_KIND_SLOT)]
    kind: &'static str,
    #[webapi(slot = NETWORK_BODY_SOURCE_ID_SLOT)]
    id: v8::Local<'scope, v8::BigInt>,
    #[webapi(slot = NETWORK_BODY_SOURCE_OWNER_SLOT)]
    owner: v8::Local<'scope, v8::Object>,
    #[webapi(method, callback = pending_body_source_pull_callback, length = 1)]
    pull: (),
    #[webapi(method, callback = pending_body_source_cancel_callback, length = 1)]
    cancel: (),
}

#[derive(Debug)]
enum NetworkBodySourceState {
    Memory {
        bytes: Vec<u8>,
        offset: usize,
    },
    File {
        path: PathBuf,
        file: Option<File>,
        offset: usize,
        len: usize,
    },
    // Reuses the protocol/CDP subresource carrier so worker fetch can hand large
    // bodies to Web consumers without first rebuilding a single materialized Vec.
    Subresource {
        body: SubresourceResponseBody,
        offset: usize,
    },
}

impl NetworkBodySourceState {
    fn new(id: NetworkBodySourceId, bytes: Vec<u8>) -> Self {
        if bytes.len() > NETWORK_BODY_SOURCE_MEMORY_LIMIT
            && let Ok(spooled) = Self::new_spooled(id, &bytes)
        {
            return spooled;
        }
        Self::Memory { bytes, offset: 0 }
    }

    fn new_spooled(id: NetworkBodySourceId, bytes: &[u8]) -> std::io::Result<Self> {
        let path = unique_network_body_spool_path(id)?;
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        configure_secure_network_body_spool_file_options(&mut options);
        let mut file = options.open(&path)?;
        if let Err(error) = file
            .write_all(bytes)
            .and_then(|_| file.flush())
            .and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()))
        {
            // The temp file exists after open(). If initialization fails before
            // the path enters NetworkBodySourceState, remove it explicitly.
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self::File {
            path,
            file: Some(file),
            offset: 0,
            len: bytes.len(),
        })
    }

    fn clone_remaining(&self) -> io::Result<Vec<u8>> {
        match self {
            Self::Memory { bytes, offset } => {
                Ok(bytes.get(*offset..).map(<[u8]>::to_vec).unwrap_or_default())
            }
            Self::File { path, offset, .. } => read_network_body_spool_remaining(path, *offset),
            Self::Subresource { body, offset } => body.materialize_bytes_from(*offset),
        }
    }

    fn into_remaining(mut self) -> io::Result<Vec<u8>> {
        match &mut self {
            Self::Memory { bytes, offset } => {
                if *offset == 0 {
                    Ok(std::mem::take(bytes))
                } else {
                    Ok(bytes.get(*offset..).map(<[u8]>::to_vec).unwrap_or_default())
                }
            }
            Self::File { file, offset, .. } => {
                let mut bytes = Vec::new();
                let file = file.as_mut().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "response body spool file is closed",
                    )
                })?;
                file.seek(SeekFrom::Start(*offset as u64))?;
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            }
            Self::Subresource { body, offset } => body.materialize_bytes_from(*offset),
        }
    }

    fn take_next_chunk(&mut self, chunk_size: usize) -> io::Result<Option<Vec<u8>>> {
        match self {
            Self::Memory { bytes, offset } => {
                if *offset >= bytes.len() {
                    return Ok(None);
                }
                let next_offset = offset.saturating_add(chunk_size).min(bytes.len());
                let chunk = bytes[*offset..next_offset].to_vec();
                *offset = next_offset;
                Ok(Some(chunk))
            }
            Self::File {
                file, offset, len, ..
            } => {
                if *offset >= *len {
                    return Ok(None);
                }
                let file = file.as_mut().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "response body spool file is closed",
                    )
                })?;
                file.seek(SeekFrom::Start(*offset as u64))?;
                let mut chunk = vec![0; chunk_size.min(len.saturating_sub(*offset))];
                let read = file.read(&mut chunk)?;
                if read == 0 {
                    *offset = *len;
                    return Ok(None);
                }
                chunk.truncate(read);
                *offset = offset.saturating_add(read);
                Ok(Some(chunk))
            }
            Self::Subresource { body, offset } => {
                let chunk = body.read_chunk(*offset, chunk_size)?;
                if chunk.is_empty() {
                    *offset = body.len();
                    return Ok(None);
                }
                *offset = offset.saturating_add(chunk.len());
                Ok(Some(chunk))
            }
        }
    }

    fn is_done(&self) -> bool {
        match self {
            Self::Memory { bytes, offset } => *offset >= bytes.len(),
            Self::File { offset, len, .. } => *offset >= *len,
            Self::Subresource { body, offset } => *offset >= body.len(),
        }
    }
}

impl Drop for NetworkBodySourceState {
    fn drop(&mut self) {
        if let Self::File { path, file, .. } = self {
            let _ = file.take();
            let _ = fs::remove_file(path);
        }
    }
}

enum PendingBodyMaterializationKind {
    Text,
    Json,
    ArrayBuffer,
    Bytes,
    Blob { mime_type: String },
    FormData { content_type: String },
}

pub(crate) struct PendingBodyMaterialization {
    resolver: v8::Global<v8::PromiseResolver>,
    kind: PendingBodyMaterializationKind,
}

struct PendingBodyMaterializationBatch {
    id: NetworkBodySourceId,
    materializations: Vec<PendingBodyMaterialization>,
}

pub(crate) struct PendingNetworkBodySourceState {
    stream: v8::Weak<v8::Object>,
    bytes: Vec<u8>,
    stream_offset: usize,
    streaming: bool,
    pull_requested: bool,
    closed: bool,
    error: Option<String>,
    error_reason: Option<v8::Global<v8::Value>>,
    materializations: Vec<PendingBodyMaterialization>,
}

enum PendingBodyRejection {
    Reason(v8::Global<v8::Value>),
    Message(String),
}

pub(in crate::network_host) enum NetworkBodyConsumption<'s> {
    Ready(v8::Local<'s, v8::Value>),
    Rejected(v8::Local<'s, v8::Value>),
    Pending(v8::Local<'s, v8::Promise>),
    Failed,
}

pub(in crate::network_host) enum NetworkBodyConsumptionKind {
    Text,
    Json,
    ArrayBuffer,
    Bytes,
    Blob { mime_type: String },
    FormData { content_type: String },
}

impl From<NetworkBodyConsumptionKind> for PendingBodyMaterializationKind {
    fn from(kind: NetworkBodyConsumptionKind) -> Self {
        match kind {
            NetworkBodyConsumptionKind::Text => Self::Text,
            NetworkBodyConsumptionKind::Json => Self::Json,
            NetworkBodyConsumptionKind::ArrayBuffer => Self::ArrayBuffer,
            NetworkBodyConsumptionKind::Bytes => Self::Bytes,
            NetworkBodyConsumptionKind::Blob { mime_type } => Self::Blob { mime_type },
            NetworkBodyConsumptionKind::FormData { content_type } => {
                Self::FormData { content_type }
            }
        }
    }
}

impl PendingBodyMaterializationKind {
    fn clone_for_ready(&self) -> Self {
        match self {
            Self::Text => Self::Text,
            Self::Json => Self::Json,
            Self::ArrayBuffer => Self::ArrayBuffer,
            Self::Bytes => Self::Bytes,
            Self::Blob { mime_type } => Self::Blob {
                mime_type: mime_type.clone(),
            },
            Self::FormData { content_type } => Self::FormData {
                content_type: content_type.clone(),
            },
        }
    }
}

pub(crate) fn new_network_body_source_id() -> NetworkBodySourceId {
    NEXT_NETWORK_BODY_SOURCE_ID
        .fetch_add(1, Ordering::Relaxed)
        .max(1)
}

fn context_host_mut<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<&'s mut crate::native_bridge::JsContextHost> {
    context_host_ptr_from_global_bridge(scope).map(|ptr| unsafe { &mut *ptr })
}

pub(in crate::network_host) fn set_network_body_owned_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::ArrayBuffer>> {
    let buffer = blob::array_buffer_from_bytes(scope, bytes)?;
    // Keep body storage behind a dedicated source object. Today this source is
    // still ArrayBuffer-backed; stream/spool-backed variants should extend this
    // module instead of teaching Fetch/XHR call sites about more hidden slots.
    let source = OwnedNetworkBodySourceDeclaration::new(BODY_SOURCE_KIND_BYTES, buffer)
        .bind(scope)
        .expect("owned network body source declaration should bind");
    set_network_body_source_object(scope, object, source);
    Some(buffer)
}

fn set_network_body_response_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    body: moli_fetch::ResponseBody,
) -> Option<v8::Local<'s, v8::Object>> {
    let bytes = body
        .try_into_materialized_bytes()
        .expect("fetch Response body should remain materialized at the V8 boundary");
    set_network_body_registry_bytes(scope, object, bytes);
    let source = network_body_source_from_object(scope, object)?;
    Some(new_readable_stream_from_source(scope, source))
}

pub(in crate::network_host) fn network_body_source_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, NETWORK_BODY_SOURCE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_network_body_source_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, object, NETWORK_BODY_SOURCE_SLOT, source.into());
}

fn filtered_response_internal_body_source_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, FILTERED_RESPONSE_INTERNAL_BODY_SOURCE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn filtered_response_internal_body_stream_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, FILTERED_RESPONSE_INTERNAL_BODY_STREAM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_filtered_response_internal_body_source_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        object,
        FILTERED_RESPONSE_INTERNAL_BODY_SOURCE_SLOT,
        source.into(),
    );
}

fn set_filtered_response_internal_body_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        object,
        FILTERED_RESPONSE_INTERNAL_BODY_STREAM_SLOT,
        stream.into(),
    );
}

fn set_filtered_response_internal_body_source_and_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
) {
    set_filtered_response_internal_body_source_object(scope, object, source);
    set_filtered_response_internal_body_stream_object(scope, object, stream);
}

pub(in crate::network_host) fn set_filtered_response_internal_body_from_response_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    body: moli_fetch::ResponseBody,
) {
    let bytes = body
        .try_into_materialized_bytes()
        .expect("fetch Response body should remain materialized at the V8 boundary");
    let source = network_body_source_object_from_bytes(scope, Some(object), bytes);
    set_filtered_response_internal_body_source_object(scope, object, source);
}

pub(in crate::network_host) fn set_filtered_response_internal_body_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
) {
    let source = network_body_source_object_from_bytes(scope, Some(object), bytes);
    set_filtered_response_internal_body_source_object(scope, object, source);
}

pub(in crate::network_host) fn set_filtered_response_internal_body_from_subresource_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    body: SubresourceResponseBody,
) {
    let source = network_body_source_object_from_subresource_body(scope, Some(object), body);
    set_filtered_response_internal_body_source_object(scope, object, source);
}

fn set_network_body_registry_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
) {
    let source = network_body_source_object_from_bytes(scope, Some(object), bytes);
    set_network_body_source_object(scope, object, source);
}

pub(in crate::network_host) fn network_body_source_object_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: Option<v8::Local<'s, v8::Object>>,
    bytes: Vec<u8>,
) -> v8::Local<'s, v8::Object> {
    let id = register_network_body_bytes(bytes);
    let source = RegisteredNetworkBodySourceDeclaration::new(
        BODY_SOURCE_KIND_REGISTRY_BYTES,
        v8::BigInt::new_from_u64(scope, id),
        owner,
    )
    .bind(scope)
    .expect("registry network body source declaration should bind");
    track_registry_body_source_lifetime(scope, source, id);
    source
}

pub(in crate::network_host) fn network_body_source_object_from_subresource_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: Option<v8::Local<'s, v8::Object>>,
    body: SubresourceResponseBody,
) -> v8::Local<'s, v8::Object> {
    let id = register_network_body_subresource_body(body);
    let source = RegisteredNetworkBodySourceDeclaration::new(
        BODY_SOURCE_KIND_SUBRESOURCE_BODY,
        v8::BigInt::new_from_u64(scope, id),
        owner,
    )
    .bind(scope)
    .expect("subresource network body source declaration should bind");
    track_registry_body_source_lifetime(scope, source, id);
    source
}

pub(in crate::network_host) fn network_body_stream_from_response_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    body: moli_fetch::ResponseBody,
) -> Option<v8::Local<'s, v8::Object>> {
    set_network_body_response_body(scope, object, body)
}

pub(in crate::network_host) fn network_body_stream_from_subresource_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    body: SubresourceResponseBody,
) -> v8::Local<'s, v8::Object> {
    let source = network_body_source_object_from_subresource_body(scope, Some(object), body);
    set_network_body_source_object(scope, object, source);
    new_readable_stream_from_source(scope, source)
}

pub(crate) fn pending_network_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    id: NetworkBodySourceId,
) -> v8::Local<'s, v8::Object> {
    let (source, stream) = pending_network_body_source_and_stream(scope, object, id);
    set_network_body_source_object(scope, object, source);
    stream
}

pub(in crate::network_host) fn set_filtered_response_internal_body_from_pending_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    id: NetworkBodySourceId,
) {
    let (source, stream) = pending_network_body_source_and_stream(scope, object, id);
    set_filtered_response_internal_body_source_and_stream_object(scope, object, source, stream);
}

fn pending_network_body_source_and_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    id: NetworkBodySourceId,
) -> (v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>) {
    let source = PendingNetworkBodySourceDeclaration::new(
        BODY_SOURCE_KIND_PENDING_STREAM,
        v8::BigInt::new_from_u64(scope, id),
        object,
    )
    .bind(scope)
    .expect("pending network body source declaration should bind");

    let stream = new_readable_stream_from_source(scope, source);
    if let Some(host) = context_host_mut(scope) {
        host.pending_network_body_sources.insert(
            id,
            PendingNetworkBodySourceState {
                stream: v8::Weak::new(scope, stream),
                bytes: Vec::new(),
                stream_offset: 0,
                streaming: false,
                pull_requested: false,
                closed: false,
                error: None,
                error_reason: None,
                materializations: Vec::new(),
            },
        );
    } else if let Some(worker_state) = get_worker_state(scope) {
        worker_state
            .borrow_mut()
            .pending_network_body_sources
            .insert(
                id,
                PendingNetworkBodySourceState {
                    stream: v8::Weak::new(scope, stream),
                    bytes: Vec::new(),
                    stream_offset: 0,
                    streaming: false,
                    pull_requested: false,
                    closed: false,
                    error: None,
                    error_reason: None,
                    materializations: Vec::new(),
                },
            );
    }
    (source, stream)
}

pub(crate) fn enqueue_pending_network_body_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: NetworkBodySourceId,
    bytes: Vec<u8>,
) {
    if let Some(host) = context_host_mut(scope) {
        enqueue_pending_network_body_chunk_in_maps(
            scope,
            &mut host.pending_network_body_sources,
            &host.pending_network_body_clones,
            id,
            bytes,
        );
    } else if let Some(worker_state) = get_worker_state(scope) {
        let mut worker_state = worker_state.borrow_mut();
        let clone_ids = worker_state.pending_network_body_clones.clone();
        enqueue_pending_network_body_chunk_in_maps(
            scope,
            &mut worker_state.pending_network_body_sources,
            &clone_ids,
            id,
            bytes,
        );
    }
}

fn enqueue_pending_network_body_chunk_in_maps(
    scope: &mut v8::PinScope<'_, '_>,
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    clones: &HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
    bytes: Vec<u8>,
) {
    let clone_ids = pending_body_clone_descendants(clones, id);
    if let Some(state) = sources.get_mut(&id)
        && !state.closed
        && state.error.is_none()
    {
        append_pending_body_state_bytes(scope, state, &bytes);
    }
    for clone_id in clone_ids {
        if let Some(clone) = sources.get_mut(&clone_id) {
            append_pending_body_state_bytes(scope, clone, &bytes);
        }
    }
}

pub(crate) fn close_pending_network_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: NetworkBodySourceId,
) {
    let materialization_batches = if let Some(host) = context_host_mut(scope) {
        close_pending_network_body_stream_in_maps(
            scope,
            &mut host.pending_network_body_sources,
            &mut host.pending_network_body_clones,
            id,
        )
    } else if let Some(worker_state) = get_worker_state(scope) {
        let mut worker_state = worker_state.borrow_mut();
        let clone_ids =
            take_pending_body_clone_descendants(&mut worker_state.pending_network_body_clones, id);
        close_pending_network_body_stream_with_clone_ids(
            scope,
            &mut worker_state.pending_network_body_sources,
            id,
            clone_ids,
        )
    } else {
        Vec::new()
    };
    for batch in materialization_batches {
        resolve_pending_body_materializations(scope, batch.id, batch.materializations);
    }
}

fn close_pending_network_body_stream_in_maps(
    scope: &mut v8::PinScope<'_, '_>,
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    clones: &mut HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
) -> Vec<PendingBodyMaterializationBatch> {
    let clone_ids = take_pending_body_clone_descendants(clones, id);
    close_pending_network_body_stream_with_clone_ids(scope, sources, id, clone_ids)
}

fn close_pending_network_body_stream_with_clone_ids(
    scope: &mut v8::PinScope<'_, '_>,
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    id: NetworkBodySourceId,
    clone_ids: Vec<NetworkBodySourceId>,
) -> Vec<PendingBodyMaterializationBatch> {
    let mut batches = Vec::new();
    if let Some(state) = sources.get_mut(&id) {
        state.closed = true;
        let close_now = state.streaming || !state.materializations.is_empty();
        if state.streaming {
            emit_pending_body_state_chunk(scope, state);
        }
        if close_now
            && pending_body_state_has_no_buffered_bytes(state)
            && let Some(stream) = state.stream.to_local(scope)
        {
            close_stream(scope, stream);
        }
        let materializations = std::mem::take(&mut state.materializations);
        if !materializations.is_empty() {
            batches.push(PendingBodyMaterializationBatch {
                id,
                materializations,
            });
        }
    }
    for clone_id in clone_ids {
        if let Some(clone) = sources.get_mut(&clone_id) {
            clone.closed = true;
            let close_now = clone.streaming || !clone.materializations.is_empty();
            if clone.streaming {
                emit_pending_body_state_chunk(scope, clone);
            }
            if close_now
                && pending_body_state_has_no_buffered_bytes(clone)
                && let Some(stream) = clone.stream.to_local(scope)
            {
                close_stream(scope, stream);
            }
            let materializations = std::mem::take(&mut clone.materializations);
            if !materializations.is_empty() {
                batches.push(PendingBodyMaterializationBatch {
                    id: clone_id,
                    materializations,
                });
            }
        }
    }
    batches
}

pub(crate) fn error_pending_network_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: NetworkBodySourceId,
    error_text: String,
) {
    let reason = v8_string(scope, &error_text)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    error_pending_network_body_stream_with_reason(scope, id, error_text, reason);
}

pub(crate) fn error_pending_network_body_stream_with_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: NetworkBodySourceId,
    error_text: String,
    reason: v8::Local<'s, v8::Value>,
) {
    let materializations = if let Some(host) = context_host_mut(scope) {
        error_pending_network_body_stream_in_maps(
            scope,
            &mut host.pending_network_body_sources,
            &mut host.pending_network_body_clones,
            id,
            &error_text,
            reason,
        )
    } else if let Some(worker_state) = get_worker_state(scope) {
        let mut worker_state = worker_state.borrow_mut();
        let clone_ids =
            take_pending_body_clone_descendants(&mut worker_state.pending_network_body_clones, id);
        error_pending_network_body_stream_with_clone_ids(
            scope,
            &mut worker_state.pending_network_body_sources,
            id,
            clone_ids,
            &error_text,
            reason,
        )
    } else {
        Vec::new()
    };
    reject_pending_body_materializations_with_reason(scope, materializations, reason);
}

fn error_pending_network_body_stream_in_maps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    clones: &mut HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
    error_text: &str,
    reason: v8::Local<'s, v8::Value>,
) -> Vec<PendingBodyMaterialization> {
    let clone_ids = take_pending_body_clone_descendants(clones, id);
    error_pending_network_body_stream_with_clone_ids(
        scope, sources, id, clone_ids, error_text, reason,
    )
}

fn pending_body_clone_descendants(
    clones: &HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
) -> Vec<NetworkBodySourceId> {
    let mut descendants = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = clones.get(&id).cloned().unwrap_or_default();
    while let Some(clone_id) = stack.pop() {
        if !seen.insert(clone_id) {
            continue;
        }
        descendants.push(clone_id);
        if let Some(children) = clones.get(&clone_id) {
            stack.extend(children.iter().copied());
        }
    }
    descendants
}

fn take_pending_body_clone_descendants(
    clones: &mut HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
) -> Vec<NetworkBodySourceId> {
    let mut descendants = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = clones.remove(&id).unwrap_or_default();
    while let Some(clone_id) = stack.pop() {
        if !seen.insert(clone_id) {
            continue;
        }
        descendants.push(clone_id);
        if let Some(children) = clones.remove(&clone_id) {
            stack.extend(children);
        }
    }
    descendants
}

fn pending_body_clone_root(
    clones: &HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
) -> NetworkBodySourceId {
    let mut root = id;
    let mut seen = HashSet::new();
    while seen.insert(root) {
        let Some(parent) = clones
            .iter()
            .find_map(|(parent, children)| children.contains(&root).then_some(*parent))
        else {
            break;
        };
        root = parent;
    }
    root
}

fn remove_pending_body_clone_link(
    clones: &mut HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
) {
    let mut empty_parents = Vec::new();
    for (parent, children) in clones.iter_mut() {
        children.retain(|child| *child != id);
        if children.is_empty() {
            empty_parents.push(*parent);
        }
    }
    for parent in empty_parents {
        clones.remove(&parent);
    }
}

fn pending_body_active_branch_count(
    sources: &HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    clones: &HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    root_id: NetworkBodySourceId,
) -> usize {
    usize::from(sources.contains_key(&root_id))
        + pending_body_clone_descendants(clones, root_id)
            .into_iter()
            .filter(|id| sources.contains_key(id))
            .count()
}

fn cancel_pending_network_body_source_in_maps(
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    clones: &mut HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    id: NetworkBodySourceId,
) -> Option<NetworkBodySourceId> {
    let root_id = pending_body_clone_root(clones, id);
    let has_descendants = !pending_body_clone_descendants(clones, id).is_empty();
    sources.remove(&id);
    if !has_descendants {
        clones.remove(&id);
        remove_pending_body_clone_link(clones, id);
    }
    if pending_body_active_branch_count(sources, clones, root_id) == 0 {
        let _ = take_pending_body_clone_descendants(clones, root_id);
        clones.remove(&root_id);
        Some(root_id)
    } else {
        None
    }
}

fn error_pending_network_body_stream_with_clone_ids<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    id: NetworkBodySourceId,
    clone_ids: Vec<NetworkBodySourceId>,
    error_text: &str,
    reason: v8::Local<'s, v8::Value>,
) -> Vec<PendingBodyMaterialization> {
    let mut materializations = if let Some(state) = sources.get_mut(&id) {
        if state.closed {
            debug_assert!(
                state.error.is_some(),
                "pending network body source must not be errored after a clean close"
            );
            return Vec::new();
        }
        state.closed = true;
        state.error = Some(error_text.to_owned());
        state.error_reason = Some(v8::Global::new(scope, reason));
        if let Some(stream) = state.stream.to_local(scope) {
            error_stream(scope, stream, reason);
        }
        std::mem::take(&mut state.materializations)
    } else {
        Vec::new()
    };
    for clone_id in clone_ids {
        if let Some(clone) = sources.get_mut(&clone_id) {
            if clone.closed {
                debug_assert!(
                    clone.error.is_some(),
                    "pending network body clone must not be errored after a clean close"
                );
                continue;
            }
            clone.closed = true;
            clone.error = Some(error_text.to_owned());
            clone.error_reason = Some(v8::Global::new(scope, reason));
            if let Some(stream) = clone.stream.to_local(scope) {
                error_stream(scope, stream, reason);
            }
            materializations.extend(std::mem::take(&mut clone.materializations));
        }
    }
    materializations
}

fn network_body_source_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, slot)
}

fn network_body_source_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    network_body_source_slot_value(scope, object, NETWORK_BODY_SOURCE_KIND_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn network_body_source_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    network_body_source_slot_value(scope, object, NETWORK_BODY_BYTES_SLOT)
        .and_then(|value| blob::buffer_source_bytes_from_value(scope, value))
}

fn try_network_body_bytes_from_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    consume_registry_body: bool,
) -> Result<Option<Vec<u8>>, String> {
    let source_kind = network_body_source_kind(scope, object);
    match source_kind.as_deref() {
        Some(BODY_SOURCE_KIND_REGISTRY_BYTES) => {
            let Some(id) = registry_body_source_id(scope, object) else {
                return Ok(None);
            };
            return if consume_registry_body {
                try_take_remaining_network_body_bytes(id)
            } else {
                try_clone_remaining_network_body_bytes(id)
            };
        }
        Some(BODY_SOURCE_KIND_SUBRESOURCE_BODY) => {
            let Some(id) = registry_body_source_id(scope, object) else {
                return Ok(None);
            };
            return if consume_registry_body {
                try_take_remaining_network_body_bytes(id)
            } else {
                try_clone_remaining_network_body_bytes(id)
            };
        }
        Some(kind) if kind != BODY_SOURCE_KIND_BYTES => return Ok(None),
        _ => {}
    }
    if let Some(bytes) = network_body_source_bytes(scope, object) {
        return Ok(Some(bytes));
    }
    Ok(object
        .get(scope, v8str(scope, NETWORK_BODY_SLOT).into())
        .and_then(|value| {
            (!value.is_null_or_undefined()).then(|| {
                value
                    .to_string(scope)
                    .map(|value| value.to_rust_string_lossy(scope).into_bytes())
                    .unwrap_or_default()
            })
        }))
}

pub(crate) fn try_network_body_bytes_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<Vec<u8>>, String> {
    if let Some(source) = network_body_source_from_object(scope, object) {
        return try_network_body_bytes_from_storage(scope, source, false);
    }
    try_network_body_bytes_from_storage(scope, object, false)
}

pub(in crate::network_host) fn take_network_body_bytes_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    try_take_network_body_bytes_from_object(scope, object)
        .ok()
        .flatten()
}

pub(in crate::network_host) fn try_take_network_body_bytes_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<Vec<u8>>, String> {
    if let Some(source) = network_body_source_from_object(scope, object) {
        return try_network_body_bytes_from_storage(scope, source, true);
    }
    try_network_body_bytes_from_storage(scope, object, true)
}

pub(in crate::network_host) fn try_network_body_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<v8::Local<'s, v8::Value>>, String> {
    let source = network_body_source_from_object(scope, object).unwrap_or(object);
    if network_body_source_kind(scope, source)
        .as_deref()
        .is_some_and(|kind| {
            matches!(
                kind,
                BODY_SOURCE_KIND_REGISTRY_BYTES | BODY_SOURCE_KIND_SUBRESOURCE_BODY
            )
        })
    {
        return try_network_body_bytes_from_storage(scope, source, false).map(|bytes| {
            bytes
                .and_then(|bytes| blob::array_buffer_from_bytes(scope, bytes))
                .map(Into::into)
        });
    }
    Ok(
        network_body_source_slot_value(scope, source, NETWORK_BODY_BYTES_SLOT)
            .or_else(|| source.get(scope, v8str(scope, NETWORK_BODY_SLOT).into()))
            .filter(|value| !value.is_null_or_undefined()),
    )
}

pub(in crate::network_host) fn clone_pending_network_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    clone_pending_network_body_source_and_stream(scope, source, owner).map(|(source, stream)| {
        set_network_body_source_object(scope, owner, source);
        stream
    })
}

fn clone_pending_network_body_source_and_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    if network_body_source_kind(scope, source).as_deref() != Some(BODY_SOURCE_KIND_PENDING_STREAM) {
        return None;
    }
    let original_id = registry_body_source_id(scope, source)?;
    let clone_id = new_network_body_source_id();
    let (clone_source, stream) = pending_network_body_source_and_stream(scope, owner, clone_id);
    let mut close_now = false;
    let mut error_now = None;
    let mut error_reason_now = None;
    let mut found_original = false;
    if let Some(host) = context_host_mut(scope) {
        let outcome = clone_pending_network_body_source_in_maps(
            scope,
            &mut host.pending_network_body_sources,
            original_id,
            clone_id,
        );
        found_original = outcome.found_original;
        close_now = outcome.close_now;
        error_now = outcome.error_now;
        error_reason_now = outcome.error_reason_now;
    } else if let Some(worker_state) = get_worker_state(scope) {
        let outcome = clone_pending_network_body_source_in_maps(
            scope,
            &mut worker_state.borrow_mut().pending_network_body_sources,
            original_id,
            clone_id,
        );
        found_original = outcome.found_original;
        close_now = outcome.close_now;
        error_now = outcome.error_now;
        error_reason_now = outcome.error_reason_now;
    }
    if !found_original {
        if let Some(host) = context_host_mut(scope) {
            host.pending_network_body_sources.remove(&clone_id);
        } else if let Some(worker_state) = get_worker_state(scope) {
            worker_state
                .borrow_mut()
                .pending_network_body_sources
                .remove(&clone_id);
        }
        return None;
    }
    if let Some(error_text) = error_now {
        if let Some(reason) = error_reason_now {
            let reason = v8::Local::new(scope, &reason);
            error_pending_network_body_stream_with_reason(scope, clone_id, error_text, reason);
        } else {
            error_pending_network_body_stream(scope, clone_id, error_text);
        }
    } else if close_now {
        close_pending_network_body_stream(scope, clone_id);
    } else if let Some(host) = context_host_mut(scope) {
        host.pending_network_body_clones
            .entry(original_id)
            .or_default()
            .push(clone_id);
    } else if let Some(worker_state) = get_worker_state(scope) {
        worker_state
            .borrow_mut()
            .pending_network_body_clones
            .entry(original_id)
            .or_default()
            .push(clone_id);
    }
    Some((clone_source, stream))
}

pub(in crate::network_host) fn clone_filtered_response_internal_body_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: v8::Local<'s, v8::Object>,
    to: v8::Local<'s, v8::Object>,
) {
    let Some(source) = filtered_response_internal_body_source_from_object(scope, from) else {
        return;
    };
    let source_kind = network_body_source_kind(scope, source);
    if source_kind.as_deref() == Some(BODY_SOURCE_KIND_PENDING_STREAM) {
        if let Some((clone_source, stream)) =
            clone_pending_network_body_source_and_stream(scope, source, to)
        {
            set_filtered_response_internal_body_source_and_stream_object(
                scope,
                to,
                clone_source,
                stream,
            );
        }
        return;
    }
    if let Ok(Some(bytes)) = try_network_body_bytes_from_storage(scope, source, false) {
        let clone_source = network_body_source_object_from_bytes(scope, Some(to), bytes);
        set_filtered_response_internal_body_source_object(scope, to, clone_source);
    }
}

struct ClonePendingBodySourceOutcome {
    found_original: bool,
    close_now: bool,
    error_now: Option<String>,
    error_reason_now: Option<v8::Global<v8::Value>>,
}

fn clone_pending_network_body_source_in_maps(
    scope: &mut v8::PinScope<'_, '_>,
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    original_id: NetworkBodySourceId,
    clone_id: NetworkBodySourceId,
) -> ClonePendingBodySourceOutcome {
    let mut outcome = ClonePendingBodySourceOutcome {
        found_original: false,
        close_now: false,
        error_now: None,
        error_reason_now: None,
    };
    if let Some(original) = sources.get(&original_id) {
        outcome.found_original = true;
        let snapshot = original.bytes.clone();
        outcome.close_now = original.closed;
        outcome.error_now = original.error.clone();
        outcome.error_reason_now = original
            .error_reason
            .as_ref()
            .map(|reason| v8::Global::new(scope, v8::Local::new(scope, reason)));
        if let Some(clone) = sources.get_mut(&clone_id) {
            append_pending_body_state_bytes(scope, clone, &snapshot);
        }
    }
    outcome
}

pub(in crate::network_host) fn consume_network_body_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
) -> NetworkBodyConsumption<'s> {
    consume_network_body_value_from_object_inner(scope, object, kind, None).0
}

pub(in crate::network_host) fn consume_network_body_value_from_object_with_chunk_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
    chunk_callback: v8::Local<'s, v8::Function>,
) -> (NetworkBodyConsumption<'s>, Option<v8::Global<v8::Object>>) {
    consume_network_body_value_from_object_inner(scope, object, kind, Some(chunk_callback))
}

pub(in crate::network_host) fn network_body_value_is_pending_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    network_body_source_from_object(scope, object)
        .and_then(|source| network_body_source_kind(scope, source))
        .as_deref()
        == Some(BODY_SOURCE_KIND_PENDING_STREAM)
}

pub(in crate::network_host) fn consume_filtered_response_internal_body_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
) -> Option<NetworkBodyConsumption<'s>> {
    let source = filtered_response_internal_body_source_from_object(scope, object)?;
    Some(consume_network_body_value_from_source_inner(scope, object, Some(source), kind, None).0)
}

pub(in crate::network_host) fn consume_filtered_response_internal_body_value_from_object_with_chunk_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
    chunk_callback: v8::Local<'s, v8::Function>,
) -> Option<(NetworkBodyConsumption<'s>, Option<v8::Global<v8::Object>>)> {
    let source = filtered_response_internal_body_source_from_object(scope, object)?;
    if network_body_source_kind(scope, source).as_deref() == Some(BODY_SOURCE_KIND_PENDING_STREAM)
        && let Some(stream) = filtered_response_internal_body_stream_from_object(scope, object)
    {
        return Some(consume_readable_body_stream(
            scope,
            stream,
            kind,
            Some(chunk_callback),
        ));
    }
    Some(consume_network_body_value_from_source_inner(
        scope,
        object,
        Some(source),
        kind,
        Some(chunk_callback),
    ))
}

fn consume_network_body_value_from_object_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
    chunk_callback: Option<v8::Local<'s, v8::Function>>,
) -> (NetworkBodyConsumption<'s>, Option<v8::Global<v8::Object>>) {
    let explicit_source = network_body_source_from_object(scope, object);
    consume_network_body_value_from_source_inner(
        scope,
        object,
        explicit_source,
        kind,
        chunk_callback,
    )
}

fn consume_network_body_value_from_source_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    explicit_source: Option<v8::Local<'s, v8::Object>>,
    kind: NetworkBodyConsumptionKind,
    chunk_callback: Option<v8::Local<'s, v8::Function>>,
) -> (NetworkBodyConsumption<'s>, Option<v8::Global<v8::Object>>) {
    let source = explicit_source.unwrap_or(object);
    if network_body_source_kind(scope, source).as_deref() == Some(BODY_SOURCE_KIND_PENDING_STREAM) {
        if let Some(chunk_callback) = chunk_callback
            && let Some(stream) = readable_body_stream_from_object(scope, object)
        {
            return consume_readable_body_stream(scope, stream, kind, Some(chunk_callback));
        }
        let Some(id) = registry_body_source_id(scope, source) else {
            return (NetworkBodyConsumption::Failed, None);
        };
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            return (NetworkBodyConsumption::Failed, None);
        };
        let promise = resolver.get_promise(scope);
        let materialization_kind = PendingBodyMaterializationKind::from(kind);
        let mut ready = None;
        let mut rejected = None;
        if let Some(host) = context_host_mut(scope) {
            inspect_pending_body_source_for_materialization(
                &mut host.pending_network_body_sources,
                id,
                scope,
                resolver,
                materialization_kind.clone_for_ready(),
                &mut ready,
                &mut rejected,
            );
        } else if let Some(worker_state) = get_worker_state(scope) {
            inspect_pending_body_source_for_materialization(
                &mut worker_state.borrow_mut().pending_network_body_sources,
                id,
                scope,
                resolver,
                materialization_kind.clone_for_ready(),
                &mut ready,
                &mut rejected,
            );
        } else {
            rejected = Some(PendingBodyRejection::Message(
                "Failed to materialize response body".to_owned(),
            ));
        }
        if let Some(rejection) = rejected {
            match rejection {
                PendingBodyRejection::Reason(reason) => {
                    let reason = v8::Local::new(scope, &reason);
                    let _ = resolver.reject(scope, reason);
                }
                PendingBodyRejection::Message(error_text) => {
                    reject_body_materialization(scope, resolver, &error_text);
                }
            }
            return (NetworkBodyConsumption::Pending(promise), None);
        }
        if let Some(bytes) = ready {
            resolve_body_materialization(scope, resolver, bytes, materialization_kind);
            return (NetworkBodyConsumption::Pending(promise), None);
        }
        return (NetworkBodyConsumption::Pending(promise), None);
    }

    if explicit_source.is_none()
        && let Some(stream) = readable_body_stream_from_object(scope, object)
    {
        return consume_readable_body_stream(scope, stream, kind, chunk_callback);
    }

    let bytes = match try_network_body_bytes_from_storage(scope, source, true) {
        Ok(Some(bytes)) => bytes,
        Ok(None) if explicit_source.is_none() && object_has_null_body_slot(scope, object) => {
            Vec::new()
        }
        Ok(None) => return (NetworkBodyConsumption::Failed, None),
        Err(_) => return (NetworkBodyConsumption::Failed, None),
    };
    match body_materialization_value(scope, &bytes, PendingBodyMaterializationKind::from(kind)) {
        Ok(value) => (NetworkBodyConsumption::Ready(value), None),
        Err(error) => (NetworkBodyConsumption::Rejected(error), None),
    }
}

fn readable_body_stream_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_branded_response_object(scope, object) {
        let value = response_slot_value(scope, object, RESPONSE_BODY_SLOT)?;
        return readable_body_stream_from_value(scope, value);
    }
    if is_branded_request_object(scope, object) {
        let value = request_slot_value(scope, object, REQUEST_BODY_SLOT)?;
        return readable_body_stream_from_value(scope, value);
    }
    let value = object.get(scope, v8str(scope, REQUEST_BODY_SLOT).into())?;
    readable_body_stream_from_value(scope, value)
}

fn readable_body_stream_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if value.is_null_or_undefined() {
        return None;
    }
    let Ok(stream) = v8::Local::<v8::Object>::try_from(value) else {
        return None;
    };
    if crate::context_bootstrap::object_prototype_matches(scope, stream, "ReadableStream") {
        return Some(stream);
    }
    None
}

fn consume_readable_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    kind: NetworkBodyConsumptionKind,
    chunk_callback: Option<v8::Local<'s, v8::Function>>,
) -> (NetworkBodyConsumption<'s>, Option<v8::Global<v8::Object>>) {
    let global = scope.get_current_context().global(scope);
    let Some(consumer) = global
        .get(scope, v8str(scope, BODY_STREAM_CONSUMER_SLOT).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return (NetworkBodyConsumption::Failed, None);
    };
    let (kind_name, mime_type) = body_stream_consumer_args(&kind);
    let Some(kind_value) = v8_string(scope, kind_name) else {
        return (NetworkBodyConsumption::Failed, None);
    };
    let Some(mime_value) = v8_string(scope, mime_type.unwrap_or_default()) else {
        return (NetworkBodyConsumption::Failed, None);
    };
    let this = v8::undefined(scope).into();
    let (value, cancel_handle) = if let Some(chunk_callback) = chunk_callback {
        let cancel_handle = v8::Global::new(scope, stream);
        (
            consumer.call(
                scope,
                this,
                &[
                    stream.into(),
                    kind_value.into(),
                    mime_value.into(),
                    chunk_callback.into(),
                ],
            ),
            Some(cancel_handle),
        )
    } else {
        (
            consumer.call(
                scope,
                this,
                &[stream.into(), kind_value.into(), mime_value.into()],
            ),
            None,
        )
    };
    let Some(value) = value else {
        return (NetworkBodyConsumption::Failed, None);
    };
    let consumption = if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        NetworkBodyConsumption::Pending(promise)
    } else {
        NetworkBodyConsumption::Ready(value)
    };
    (consumption, cancel_handle)
}

fn body_stream_consumer_args(kind: &NetworkBodyConsumptionKind) -> (&'static str, Option<&str>) {
    match kind {
        NetworkBodyConsumptionKind::Text => ("text", None),
        NetworkBodyConsumptionKind::Json => ("json", None),
        NetworkBodyConsumptionKind::ArrayBuffer => ("arrayBuffer", None),
        NetworkBodyConsumptionKind::Bytes => ("bytes", None),
        NetworkBodyConsumptionKind::Blob { mime_type } => ("blob", Some(mime_type.as_str())),
        NetworkBodyConsumptionKind::FormData { content_type } => {
            ("formData", Some(content_type.as_str()))
        }
    }
}

fn object_has_null_body_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if is_branded_response_object(scope, object) {
        return response_slot_value(scope, object, RESPONSE_BODY_SLOT)
            .is_some_and(|value| value.is_null_or_undefined());
    }
    if is_branded_request_object(scope, object) {
        return request_slot_value(scope, object, REQUEST_BODY_SLOT)
            .is_some_and(|value| value.is_null_or_undefined());
    }
    object
        .get(scope, v8str(scope, REQUEST_BODY_SLOT).into())
        .is_some_and(|value| value.is_null_or_undefined())
}

fn inspect_pending_body_source_for_materialization<'s>(
    sources: &mut HashMap<NetworkBodySourceId, PendingNetworkBodySourceState>,
    id: NetworkBodySourceId,
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    kind: PendingBodyMaterializationKind,
    ready: &mut Option<Vec<u8>>,
    rejected: &mut Option<PendingBodyRejection>,
) {
    if let Some(state) = sources.get_mut(&id) {
        if let Some(error_text) = state.error.clone() {
            *rejected = Some(
                state
                    .error_reason
                    .as_ref()
                    .map(|reason| {
                        PendingBodyRejection::Reason(v8::Global::new(
                            scope,
                            v8::Local::new(scope, reason),
                        ))
                    })
                    .unwrap_or(PendingBodyRejection::Message(error_text)),
            );
            sources.remove(&id);
        } else if state.closed {
            *ready = Some(state.bytes.clone());
            sources.remove(&id);
        } else {
            state.materializations.push(PendingBodyMaterialization {
                resolver: v8::Global::new(scope, resolver),
                kind,
            });
        }
    } else {
        *rejected = Some(PendingBodyRejection::Message(
            "Failed to materialize response body".to_owned(),
        ));
    }
}

fn register_network_body_bytes(bytes: Vec<u8>) -> NetworkBodySourceId {
    let id = new_network_body_source_id();
    NETWORK_BODY_SOURCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .insert(id, NetworkBodySourceState::new(id, bytes));
    id
}

fn register_network_body_subresource_body(body: SubresourceResponseBody) -> NetworkBodySourceId {
    let id = new_network_body_source_id();
    NETWORK_BODY_SOURCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .insert(id, NetworkBodySourceState::Subresource { body, offset: 0 });
    id
}

fn read_network_body_spool_remaining(path: &Path, offset: usize) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn unique_network_body_spool_path(id: NetworkBodySourceId) -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir().join("moli-renderer-body-source");
    create_secure_network_body_spool_root(&root)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(root.join(format!("body-{}-{id}-{nanos}.bin", std::process::id())))
}

#[cfg(unix)]
fn create_secure_network_body_spool_root(root: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_secure_network_body_spool_root(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)
}

#[cfg(unix)]
fn configure_secure_network_body_spool_file_options(options: &mut OpenOptions) {
    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_secure_network_body_spool_file_options(_options: &mut OpenOptions) {}

fn try_clone_remaining_network_body_bytes(
    id: NetworkBodySourceId,
) -> Result<Option<Vec<u8>>, String> {
    NETWORK_BODY_SOURCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .get(&id)
        .map(NetworkBodySourceState::clone_remaining)
        .transpose()
        .map_err(|error| error.to_string())
}

fn take_remaining_network_body_bytes(id: NetworkBodySourceId) -> Option<Vec<u8>> {
    try_take_remaining_network_body_bytes(id).ok().flatten()
}

fn try_take_remaining_network_body_bytes(
    id: NetworkBodySourceId,
) -> Result<Option<Vec<u8>>, String> {
    NETWORK_BODY_SOURCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .remove(&id)
        .map(NetworkBodySourceState::into_remaining)
        .transpose()
        .map_err(|error| error.to_string())
}

fn try_take_next_network_body_chunk(id: NetworkBodySourceId) -> Result<Option<Vec<u8>>, String> {
    let mut sources = NETWORK_BODY_SOURCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock();
    let Some(source) = sources.get_mut(&id) else {
        return Ok(None);
    };
    match source.take_next_chunk(NETWORK_BODY_SOURCE_CHUNK_SIZE) {
        Ok(chunk) => {
            if chunk.is_none() || source.is_done() {
                sources.remove(&id);
            }
            Ok(chunk)
        }
        Err(error) => {
            sources.remove(&id);
            Err(error.to_string())
        }
    }
}

fn append_pending_body_state_bytes(
    scope: &mut v8::PinScope<'_, '_>,
    state: &mut PendingNetworkBodySourceState,
    bytes: &[u8],
) {
    if state.closed || state.error.is_some() {
        return;
    }
    state.bytes.extend_from_slice(bytes);
    if !state.pull_requested
        && state.streaming
        && let Some(stream) = state.stream.to_local(scope)
        && readable_stream_has_pipe_owner(scope, stream)
    {
        state.pull_requested = true;
    }
    if state.pull_requested {
        emit_pending_body_state_chunk(scope, state);
    }
}

fn emit_pending_body_state_chunk(
    scope: &mut v8::PinScope<'_, '_>,
    state: &mut PendingNetworkBodySourceState,
) {
    let Some(stream) = state.stream.to_local(scope) else {
        return;
    };
    let remaining = state.bytes.len().saturating_sub(state.stream_offset);
    if remaining == 0 {
        if state.closed {
            close_stream(scope, stream);
        }
        return;
    }
    if !state.pull_requested {
        return;
    }
    let chunk_len = remaining.min(NETWORK_BODY_SOURCE_CHUNK_SIZE);
    let start = state.stream_offset;
    let end = start + chunk_len;
    let chunk = state.bytes[start..end].to_vec();
    state.stream_offset = end;
    state.pull_requested = false;
    if let Some(buffer) = blob::array_buffer_from_bytes(scope, chunk) {
        let byte_length = buffer.byte_length();
        if let Some(array) = v8::Uint8Array::new(scope, buffer, 0, byte_length) {
            let _ = enqueue_byte_chunk(scope, stream, array.into());
        }
    }
    compact_pending_body_state_after_stream_emit(state);
    if state.closed && pending_body_state_has_no_buffered_bytes(state) {
        close_stream(scope, stream);
    }
}

fn compact_pending_body_state_after_stream_emit(state: &mut PendingNetworkBodySourceState) {
    if !state.streaming || !state.materializations.is_empty() || state.stream_offset == 0 {
        return;
    }
    // Once a Response body is consumed as a stream, Web body methods and clone()
    // are no longer allowed. Drop bytes that have already crossed into the JS
    // stream queue so the Rust pending source does not retain the full body.
    state.bytes.drain(..state.stream_offset);
    state.stream_offset = 0;
}

fn pending_body_state_has_no_buffered_bytes(state: &PendingNetworkBodySourceState) -> bool {
    state.stream_offset >= state.bytes.len()
}

#[cfg(test)]
pub(crate) fn pending_network_body_source_buffered_len_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    id: NetworkBodySourceId,
) -> Option<usize> {
    if let Some(host) = context_host_mut(scope) {
        return host
            .pending_network_body_sources
            .get(&id)
            .map(|state| state.bytes.len().saturating_sub(state.stream_offset));
    }
    if let Some(worker_state) = get_worker_state(scope) {
        return worker_state
            .borrow()
            .pending_network_body_sources
            .get(&id)
            .map(|state| state.bytes.len().saturating_sub(state.stream_offset));
    }
    None
}

fn network_body_source_has_remaining(id: NetworkBodySourceId) -> bool {
    NETWORK_BODY_SOURCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .get(&id)
        .map(|source| !source.is_done())
        .unwrap_or(false)
}

fn track_registry_body_source_lifetime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    id: NetworkBodySourceId,
) {
    // The registry keeps bytes out of a V8 ArrayBuffer until a Web consumer
    // asks for them. Tie that registry entry to the JS source object as well
    // as to explicit pull/cancel/body-method consumption so unconsumed
    // responses do not leak Rust memory after V8 collects the source.
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, source, move || {
        let _ = take_remaining_network_body_bytes(id);
    });
}

fn resolve_pending_body_materializations(
    scope: &mut v8::PinScope<'_, '_>,
    id: NetworkBodySourceId,
    materializations: Vec<PendingBodyMaterialization>,
) {
    if materializations.is_empty() {
        return;
    }
    let mut bytes: Vec<u8> = take_pending_network_body_source_bytes(scope, id).unwrap_or_default();
    let materialization_count = materializations.len();
    for (index, materialization) in materializations.into_iter().enumerate() {
        let resolver = v8::Local::new(scope, &materialization.resolver);
        // Most body sources have exactly one pending materialization. Move the
        // completed buffer into that consumer and clone only when fan-out is
        // actually needed.
        let bytes = if index + 1 == materialization_count {
            std::mem::take(&mut bytes)
        } else {
            bytes.clone()
        };
        resolve_body_materialization(scope, resolver, bytes, materialization.kind);
    }
}

fn take_pending_network_body_source_bytes(
    scope: &mut v8::PinScope<'_, '_>,
    id: NetworkBodySourceId,
) -> Option<Vec<u8>> {
    if let Some(host) = context_host_mut(scope) {
        return host
            .pending_network_body_sources
            .remove(&id)
            .map(|state| state.bytes);
    }
    get_worker_state(scope).and_then(|worker_state| {
        worker_state
            .borrow_mut()
            .pending_network_body_sources
            .remove(&id)
            .map(|state| state.bytes)
    })
}

fn reject_pending_body_materializations_with_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    materializations: Vec<PendingBodyMaterialization>,
    reason: v8::Local<'s, v8::Value>,
) {
    for materialization in materializations {
        let resolver = v8::Local::new(scope, &materialization.resolver);
        let _ = resolver.reject(scope, reason);
    }
}

fn reject_body_materialization<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error_text: &str,
) {
    let error = v8_string(scope, error_text)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn resolve_body_materialization<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    bytes: Vec<u8>,
    kind: PendingBodyMaterializationKind,
) {
    match body_materialization_value(scope, &bytes, kind) {
        Ok(value) => {
            let _ = resolver.resolve(scope, value);
        }
        Err(error) => {
            let _ = resolver.reject(scope, error);
        }
    }
}

fn body_materialization_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
    kind: PendingBodyMaterializationKind,
) -> Result<v8::Local<'s, v8::Value>, v8::Local<'s, v8::Value>> {
    match kind {
        PendingBodyMaterializationKind::Text => v8_string(scope, &String::from_utf8_lossy(bytes))
            .map(Into::into)
            .ok_or_else(|| v8::undefined(scope).into()),
        PendingBodyMaterializationKind::Json => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            v8_json_parse(scope, &text).ok_or_else(|| {
                v8_string(scope, "SyntaxError: JSON parse error")
                    .map(|message| v8::Exception::syntax_error(scope, message))
                    .unwrap_or_else(|| v8::undefined(scope).into())
            })
        }
        PendingBodyMaterializationKind::ArrayBuffer => {
            blob::array_buffer_from_bytes(scope, bytes.to_vec())
                .map(Into::into)
                .ok_or_else(|| v8::undefined(scope).into())
        }
        PendingBodyMaterializationKind::Bytes => {
            let byte_len = bytes.len();
            blob::array_buffer_from_bytes(scope, bytes.to_vec())
                .and_then(|buffer| v8::Uint8Array::new(scope, buffer, 0, byte_len))
                .map(Into::into)
                .ok_or_else(|| v8::undefined(scope).into())
        }
        PendingBodyMaterializationKind::Blob { mime_type } => {
            blob::build_blob_object(scope, bytes.to_vec(), mime_type)
                .map(Into::into)
                .ok_or_else(|| v8::undefined(scope).into())
        }
        PendingBodyMaterializationKind::FormData { content_type } => {
            let multipart_boundary = multipart_form_data_boundary(&content_type);
            if let Some(boundary) = multipart_boundary.as_deref() {
                crate::context_bootstrap::form_data_object_from_multipart_bytes(
                    scope, bytes, boundary,
                )
                .ok_or_else(|| {
                    v8_string(scope, "Failed to materialize response body")
                        .map(|message| v8::Exception::type_error(scope, message))
                        .unwrap_or_else(|| v8::undefined(scope).into())
                })
            } else if is_form_urlencoded_mime(&content_type) {
                crate::context_bootstrap::form_data_object_from_urlencoded_bytes(scope, bytes)
                    .ok_or_else(|| {
                        v8_string(scope, "Failed to materialize response body")
                            .map(|message| v8::Exception::type_error(scope, message))
                            .unwrap_or_else(|| v8::undefined(scope).into())
                    })
            } else {
                Err(
                    v8_string(scope, BODY_FORM_DATA_UNSUPPORTED_CONTENT_TYPE_ERROR_TEXT)
                        .map(|message| v8::Exception::type_error(scope, message))
                        .unwrap_or_else(|| v8::undefined(scope).into()),
                )
            }
        }
    }
}

fn registry_body_source_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<NetworkBodySourceId> {
    let value = network_body_source_slot_value(scope, source, NETWORK_BODY_SOURCE_ID_SLOT)?;
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (n, _lossless) = big.u64_value();
        return Some(n);
    }
    value.uint32_value(scope).map(NetworkBodySourceId::from)
}

fn registry_body_source_pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    mark_registry_body_source_used(scope, source);
    let controller = args.get(0);
    let chunk_result = registry_body_source_id(scope, source)
        .map(try_take_next_network_body_chunk)
        .transpose()
        .map(Option::flatten);
    if let Ok(controller) = v8::Local::<v8::Object>::try_from(controller) {
        let bytes = match chunk_result {
            Ok(bytes) => bytes.unwrap_or_default(),
            Err(error_text) => {
                let reason = v8_string(scope, &error_text)
                    .map(|message| v8::Exception::type_error(scope, message))
                    .unwrap_or_else(|| v8::undefined(scope).into());
                let _ = call_controller_method(scope, controller, "error", &[reason]);
                rv.set_undefined();
                return;
            }
        };
        if !bytes.is_empty()
            && let Some(buffer) = blob::array_buffer_from_bytes(scope, bytes)
        {
            let byte_length = buffer.byte_length();
            if let Some(chunk) = v8::Uint8Array::new(scope, buffer, 0, byte_length) {
                let _ = call_controller_method(scope, controller, "enqueue", &[chunk.into()]);
            }
        }
        if registry_body_source_id(scope, source)
            .is_none_or(|id| !network_body_source_has_remaining(id))
        {
            let _ = call_controller_method(scope, controller, "close", &[]);
        }
    }
    rv.set_undefined();
}

fn registry_body_source_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(id) = registry_body_source_id(scope, args.this()) {
        let _ = take_remaining_network_body_bytes(id);
    }
    mark_registry_body_source_used(scope, args.this());
    rv.set_undefined();
}

fn pending_body_source_pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    mark_registry_body_source_used(scope, source);
    if let Some(id) = registry_body_source_id(scope, source) {
        pull_pending_network_body_source(scope, id);
    }
    rv.set_undefined();
}

fn pull_pending_network_body_source(scope: &mut v8::PinScope<'_, '_>, id: NetworkBodySourceId) {
    if let Some(host) = context_host_mut(scope) {
        if let Some(state) = host.pending_network_body_sources.get_mut(&id) {
            state.streaming = true;
            state.pull_requested = true;
            emit_pending_body_state_chunk(scope, state);
        }
    } else if let Some(worker_state) = get_worker_state(scope) {
        let mut worker_state = worker_state.borrow_mut();
        if let Some(state) = worker_state.pending_network_body_sources.get_mut(&id) {
            state.streaming = true;
            state.pull_requested = true;
            emit_pending_body_state_chunk(scope, state);
        }
    }
}

fn pending_body_source_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(id) = registry_body_source_id(scope, args.this()) {
        if let Some(host) = context_host_mut(scope) {
            if let Some(root_id) = cancel_pending_network_body_source_in_maps(
                &mut host.pending_network_body_sources,
                &mut host.pending_network_body_clones,
                id,
            ) {
                let _ = host.cancel_streaming_subresource_body_source(root_id);
            }
        } else if let Some(worker_state) = get_worker_state(scope) {
            let mut worker_state = worker_state.borrow_mut();
            let worker_state = &mut *worker_state;
            let _ = cancel_pending_network_body_source_in_maps(
                &mut worker_state.pending_network_body_sources,
                &mut worker_state.pending_network_body_clones,
                id,
            );
        }
    }
    mark_registry_body_source_used(scope, args.this());
    rv.set_undefined();
}

fn mark_registry_body_source_used<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) {
    if let Some(owner) = get_private_value(scope, source, NETWORK_BODY_SOURCE_OWNER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        if is_branded_response_object(scope, owner) {
            set_response_slot_bool(scope, owner, RESPONSE_BODY_USED_SLOT, true);
        } else if is_branded_request_object(scope, owner) {
            set_request_slot_bool(scope, owner, REQUEST_BODY_USED_SLOT, true);
        }
    }
}

fn call_controller_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    controller: v8::Local<'s, v8::Object>,
    name: &'static str,
    argv: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let method = controller.get(scope, v8str(scope, name).into())?;
    let method = v8::Local::<v8::Function>::try_from(method).ok()?;
    method.call(scope, controller.into(), argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_body_source_subresource_reads_through_fallible_source() {
        let body = SubresourceResponseBody::from_bytes(b"hello world".to_vec());
        let id = register_network_body_subresource_body(body);

        assert_eq!(
            try_clone_remaining_network_body_bytes(id).unwrap().unwrap(),
            b"hello world"
        );
        assert_eq!(
            try_take_next_network_body_chunk(id).unwrap().unwrap(),
            b"hello world"
        );
        assert!(!network_body_source_has_remaining(id));
    }

    #[test]
    fn network_body_source_file_read_errors_are_not_silent_empty_bodies() {
        let id = new_network_body_source_id();
        let missing_path = std::env::temp_dir().join(format!(
            "moli-missing-network-body-source-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_file(&missing_path);
        NETWORK_BODY_SOURCES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .insert(
                id,
                NetworkBodySourceState::File {
                    path: missing_path,
                    file: None,
                    offset: 0,
                    len: 5,
                },
            );

        assert!(try_clone_remaining_network_body_bytes(id).is_err());
        assert!(try_take_next_network_body_chunk(id).is_err());
        assert!(!network_body_source_has_remaining(id));

        let id = new_network_body_source_id();
        let missing_path = std::env::temp_dir().join(format!(
            "moli-missing-network-body-source-take-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_file(&missing_path);
        NETWORK_BODY_SOURCES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .insert(
                id,
                NetworkBodySourceState::File {
                    path: missing_path,
                    file: None,
                    offset: 0,
                    len: 5,
                },
            );

        assert!(try_take_remaining_network_body_bytes(id).is_err());
        assert!(!network_body_source_has_remaining(id));
    }
}

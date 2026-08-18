use moli_crypto::fill_secure_random;
use moli_storage_service::{
    DirectoryEntry, EntryKind, FileSnapshotIdentity, Opfs, OpfsBucketKey, OpfsError, OpfsPath,
    OpfsResult, OpfsSyncAccessHandleId, OpfsWritableId, SharedStorageService, StorageBucketId,
    StorageBucketLocator, StorageOpfsMutationLease, StorageOpfsSyncAccessLease,
    StorageOpfsWritableLease, StorageService, StorageServiceTaskError, SyncAccessMode,
    WritableCommand, WritableMode,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};
use serde::{Deserialize, Serialize};
use uuid::Builder as UuidBuilder;

use super::storage_buckets::{
    StorageBucketQuotaOwner, current_storage_bucket_storage_key,
    storage_bucket_quota_owner_for_locator, with_storage_bucket_store_entry,
};
use super::stream_adapter::{
    initialize_writable_stream_object, rejected_promise_value, set_resolved_promise,
    writable_stream_locked, writable_stream_write_internal,
};
use crate::{
    blob,
    context_bootstrap::file_api::build_file_object,
    dom::native::SelectedFile,
    opfs_owner_tasks::{
        OpfsDirectoryIteratorDescriptor, OpfsDirectoryIteratorNextAction,
        OpfsDirectoryIteratorRegistry, OpfsDirectoryIteratorSettlement, OpfsHandleAccessContext,
        OpfsHandlePathState, OpfsHandleRegistry,
    },
    opfs_task_result::{OpfsGetFileTaskResult, OpfsReadFileResult, OpfsTaskResult},
    util::{
        context_host_ptr_from_global_bridge, get_private_value,
        materialize_hidden_function_template_prototype, set_private_value, throw_type_error,
        v8_string, v8str,
    },
    webidl,
};

const FILE_SYSTEM_HANDLE_BRAND_SLOT: &str = "__moliFileSystemHandleBrand";
const FILE_SYSTEM_HANDLE_STATE_SLOT: &str = "__moliFileSystemHandleState";
const FILE_SYSTEM_HANDLE_ID_SLOT: &str = "__moliFileSystemHandleId";
const FILE_SYSTEM_ITERATOR_BRAND_SLOT: &str = "__moliFileSystemIteratorBrand";
const FILE_SYSTEM_ITERATOR_ID_SLOT: &str = "__moliFileSystemIteratorId";
const FILE_SYSTEM_ITERATOR_PROTOTYPE_SLOT: &str = "__moliFileSystemIteratorPrototype";
const FILE_SYSTEM_WRITABLE_BRAND_SLOT: &str = "__moliFileSystemWritableBrand";
const FILE_SYSTEM_WRITABLE_MODE_SLOT: &str = "__moliFileSystemWritableMode";
const FILE_SYSTEM_WRITABLE_SINK_STATE_SLOT: &str = "__moliFileSystemWritableSinkState";
const FILE_SYSTEM_WRITABLE_SINK_BRAND_SLOT: &str = "__moliFileSystemWritableSinkBrand";
const FILE_SYSTEM_FILE_SNAPSHOT_STATE_SLOT: &str = "__moliFileSystemFileSnapshotState";
const FILE_SYSTEM_SYNC_ACCESS_BRAND_SLOT: &str = "__moliFileSystemSyncAccessBrand";
const FILE_SYSTEM_SYNC_ACCESS_STATE_SLOT: &str = "__moliFileSystemSyncAccessState";
const FILE_SYSTEM_SYNC_ACCESS_CLOSED_SLOT: &str = "__moliFileSystemSyncAccessClosed";
const MAX_SYNC_ACCESS_FILE_OFFSET: u64 = i64::MAX as u64;
const MAX_SYNC_ACCESS_WRITE_SIZE: usize = i32::MAX as usize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSystemHandleState {
    locator: StorageBucketLocator,
    path: Vec<String>,
    kind: EntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSystemFileSnapshotState {
    locator: StorageBucketLocator,
    path: Vec<String>,
    entry_id: u64,
    version_id: u64,
}

/// Trusted renderer-side attachment for structured-cloning an OPFS handle.
///
/// This value never enters the JavaScript wire bytes. Every runtime clone uses
/// the same origin-bound capability model: the receiver must be same-origin
/// and attached to the same storage service/profile, while the original
/// locator is preserved even when its full storage key differs from the
/// receiver's ambient key.
#[derive(Clone, Debug)]
pub(crate) struct FileSystemHandleClonePayload {
    source_origin: String,
    storage_service: std::sync::Weak<StorageService>,
    locator: StorageBucketLocator,
    path: OpfsPath,
    kind: EntryKind,
}

/// Trusted runtime-clone attachment for an OPFS-backed `File` snapshot.
///
/// Bytes and ordinary File metadata remain in the Blob clone payload. This
/// attachment carries only the backing namespace identity and a weak identity
/// for the partition service which authorized it, so a receiving realm in the
/// same partition can retain invalidation semantics without exposing host
/// paths to script or extending the partition lifetime.
#[derive(Clone, Debug)]
pub(crate) struct FileSystemFileSnapshotClonePayload {
    state: FileSystemFileSnapshotState,
    storage_service: std::sync::Weak<StorageService>,
}

/// Storage-key-relative metadata persisted as an IndexedDB external object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileSystemHandleDurablePayload {
    pub(crate) bucket_id: Option<u64>,
    pub(crate) path: Vec<String>,
    pub(crate) kind: EntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSystemWritableSinkState {
    locator: StorageBucketLocator,
    writer_id: u64,
    #[serde(default)]
    access_id: Option<u32>,
}

impl FileSystemWritableSinkState {
    fn writer_id(&self) -> Option<OpfsWritableId> {
        OpfsWritableId::from_raw(self.writer_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSystemSyncAccessHandleState {
    locator: StorageBucketLocator,
    handle_id: u64,
    mode: String,
    #[serde(default)]
    access_id: Option<u32>,
}

impl FileSystemSyncAccessHandleState {
    fn handle_id(&self) -> Option<OpfsSyncAccessHandleId> {
        OpfsSyncAccessHandleId::from_raw(self.handle_id)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "FileSystemWritableFileStreamMode", rename_all = "lowercase")]
enum FileSystemWritableMode {
    Exclusive,
    #[default]
    Siloed,
}

impl FileSystemWritableMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Exclusive => "exclusive",
            Self::Siloed => "siloed",
        }
    }
}

impl From<FileSystemWritableMode> for WritableMode {
    fn from(value: FileSystemWritableMode) -> Self {
        match value {
            FileSystemWritableMode::Exclusive => Self::Exclusive,
            FileSystemWritableMode::Siloed => Self::Siloed,
        }
    }
}

impl From<WritableMode> for FileSystemWritableMode {
    fn from(value: WritableMode) -> Self {
        match value {
            WritableMode::Exclusive => Self::Exclusive,
            WritableMode::Siloed => Self::Siloed,
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemCreateWritableOptions")]
struct FileSystemCreateWritableOptions {
    #[webidl(name = "keepExistingData", default = false)]
    keep_existing_data: bool,
    #[webidl(
        converter = "enum",
        default = FileSystemWritableMode::Siloed
    )]
    mode: FileSystemWritableMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "FileSystemSyncAccessHandleMode", rename_all = "kebab-case")]
enum FileSystemSyncAccessHandleMode {
    InPlace,
    ReadOnly,
    #[default]
    Readwrite,
    ReadwriteUnsafe,
}

impl FileSystemSyncAccessHandleMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InPlace => "in-place",
            Self::ReadOnly => "read-only",
            Self::Readwrite => "readwrite",
            Self::ReadwriteUnsafe => "readwrite-unsafe",
        }
    }
}

impl From<FileSystemSyncAccessHandleMode> for SyncAccessMode {
    fn from(value: FileSystemSyncAccessHandleMode) -> Self {
        match value {
            FileSystemSyncAccessHandleMode::InPlace | FileSystemSyncAccessHandleMode::Readwrite => {
                Self::Readwrite
            }
            FileSystemSyncAccessHandleMode::ReadOnly => Self::ReadOnly,
            FileSystemSyncAccessHandleMode::ReadwriteUnsafe => Self::ReadwriteUnsafe,
        }
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemCreateSyncAccessHandleOptions")]
struct FileSystemCreateSyncAccessHandleOptions {
    #[webidl(
        converter = "enum",
        default = FileSystemSyncAccessHandleMode::Readwrite
    )]
    mode: FileSystemSyncAccessHandleMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "FileSystemPermissionMode", rename_all = "lowercase")]
enum FileSystemPermissionMode {
    #[default]
    Read,
    Readwrite,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemHandlePermissionDescriptor")]
struct FileSystemHandlePermissionDescriptor {
    #[webidl(converter = "enum", default = FileSystemPermissionMode::Read)]
    mode: FileSystemPermissionMode,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemReadWriteOptions")]
struct FileSystemReadWriteOptions {
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    at: Option<u64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileSystemSyncAccessHandle.truncate")]
struct FileSystemSyncAccessHandleTruncateArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long_long")]
    size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "WriteCommandType", rename_all = "lowercase")]
enum FileSystemWriteCommandType {
    Write,
    Seek,
    Truncate,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WriteParams")]
struct FileSystemWriteParams<'s> {
    #[webidl(name = "type", required, converter = "enum")]
    command_type: FileSystemWriteCommandType,
    #[webidl(nullable)]
    size: Option<u64>,
    #[webidl(nullable)]
    position: Option<u64>,
    #[webidl(converter = "raw")]
    data: Option<v8::Local<'s, v8::Value>>,
}

impl FileSystemHandleState {
    fn new(locator: StorageBucketLocator, path: &OpfsPath, kind: EntryKind) -> Self {
        Self {
            locator,
            path: path.components().to_vec(),
            kind,
        }
    }

    fn opfs_path(&self) -> Result<OpfsPath, OpfsError> {
        OpfsPath::from_components(self.path.clone())
    }
}

impl FileSystemFileSnapshotState {
    fn new(locator: StorageBucketLocator, path: &OpfsPath, identity: FileSnapshotIdentity) -> Self {
        Self {
            locator,
            path: path.components().to_vec(),
            entry_id: identity.entry_id(),
            version_id: identity.version_id(),
        }
    }

    fn opfs_path(&self) -> Result<OpfsPath, OpfsError> {
        OpfsPath::from_components(self.path.clone())
    }

    fn identity(&self) -> Option<FileSnapshotIdentity> {
        FileSnapshotIdentity::from_raw(self.entry_id, self.version_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IteratorMode {
    Entries,
    Keys,
    Values,
}

impl IteratorMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Entries => "entries",
            Self::Keys => "keys",
            Self::Values => "values",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "entries" => Some(Self::Entries),
            "keys" => Some(Self::Keys),
            "values" => Some(Self::Values),
            _ => None,
        }
    }
}

enum HandleAccessError {
    Security,
    InvalidState,
    Stale,
    Backend(OpfsError),
}

type OpfsHandleAccess = Option<OpfsHandleAccessContext>;

enum OpfsCompletionSink {
    Page(crate::page_task_queue::RendererPageOpfsTaskProducer),
    Worker {
        task_id: u64,
        sender: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerOpfsCompletion>,
    },
}

impl OpfsCompletionSink {
    /// Consume the exact terminal capability.
    ///
    /// A Page capability already contains its task identity. Worker identity
    /// stays local to the Worker transport and is valid for that worker state's
    /// lifetime.
    fn send(self, result: OpfsTaskResult) {
        match self {
            Self::Page(sender) => {
                let _ = sender.send(result);
            }
            Self::Worker { task_id, sender } => {
                let _ = sender.send(crate::worker::WorkerOpfsCompletion { task_id, result });
            }
        }
    }
}

/// Synchronous rollback authority retained outside the asynchronous terminal
/// closure. It says which owner accepted the pending entry; it cannot publish a
/// completion and carries no scheduler route.
enum RegisteredOpfsTaskCancellation {
    Page {
        task_id: crate::page_task_queue::RendererPageOpfsTaskId,
    },
    Worker {
        task_id: u64,
    },
}

struct RegisteredOpfsTask {
    cancellation: RegisteredOpfsTaskCancellation,
    completion: OpfsCompletionSink,
}

impl RegisteredOpfsTask {
    fn page(
        task_id: crate::page_task_queue::RendererPageOpfsTaskId,
        completion: crate::page_task_queue::RendererPageOpfsTaskProducer,
    ) -> Self {
        Self {
            cancellation: RegisteredOpfsTaskCancellation::Page { task_id },
            completion: OpfsCompletionSink::Page(completion),
        }
    }

    fn worker(
        task_id: u64,
        sender: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerOpfsCompletion>,
    ) -> Self {
        Self {
            cancellation: RegisteredOpfsTaskCancellation::Worker { task_id },
            completion: OpfsCompletionSink::Worker { task_id, sender },
        }
    }

    fn into_parts(self) -> (RegisteredOpfsTaskCancellation, OpfsCompletionSink) {
        (self.cancellation, self.completion)
    }
}

fn register_opfs_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: StorageBucketLocator,
    handle_access: OpfsHandleAccess,
) -> Option<RegisteredOpfsTask> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let (task_id, sender) =
            // SAFETY: registration runs synchronously in the current live V8
            // context and stores only a Global resolver plus owner tokens.
            unsafe { &mut *host_ptr }.register_pending_opfs_task(
                scope,
                resolver,
                locator,
                handle_access,
            )?;
        return Some(RegisteredOpfsTask::page(task_id, sender));
    }
    let (task_id, sender) =
        crate::worker::register_worker_opfs_task(scope, resolver, locator, handle_access)?;
    Some(RegisteredOpfsTask::worker(task_id, sender))
}

fn register_opfs_iterator_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    locator: StorageBucketLocator,
    registry: OpfsDirectoryIteratorRegistry,
    iterator_id: u32,
    handle_access: OpfsHandleAccess,
) -> Option<RegisteredOpfsTask> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let (task_id, sender) =
            // SAFETY: registration runs synchronously in the current live V8
            // context. Iterator resolvers stay in the context-owned registry.
            unsafe { &mut *host_ptr }.register_pending_opfs_iterator_task(
                scope,
                locator,
                registry,
                iterator_id,
                v8::Global::new(scope, iterator),
                handle_access,
            )?;
        return Some(RegisteredOpfsTask::page(task_id, sender));
    }
    let (task_id, sender) = crate::worker::register_worker_opfs_iterator_task(
        scope,
        locator,
        registry,
        iterator_id,
        v8::Global::new(scope, iterator),
        handle_access,
    )?;
    Some(RegisteredOpfsTask::worker(task_id, sender))
}

fn register_opfs_move_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    handle: v8::Local<'s, v8::Object>,
    mutation: crate::opfs_owner_tasks::OpfsHandleMutationGuard,
    locator: StorageBucketLocator,
    handle_access: OpfsHandleAccess,
) -> Option<RegisteredOpfsTask> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let (task_id, sender) =
            // SAFETY: registration runs synchronously in the current live V8
            // context and stores only context-owned Global handles and tokens.
            unsafe { &mut *host_ptr }
                .register_pending_opfs_move_task(
                    scope,
                    resolver,
                    handle,
                    mutation,
                    locator,
                    handle_access,
                )?;
        return Some(RegisteredOpfsTask::page(task_id, sender));
    }
    let (task_id, sender) = crate::worker::register_worker_opfs_move_task(
        scope,
        resolver,
        handle,
        mutation,
        locator,
        handle_access,
    )?;
    Some(RegisteredOpfsTask::worker(task_id, sender))
}

fn current_opfs_handle_registry(scope: &mut v8::PinScope<'_, '_>) -> Option<OpfsHandleRegistry> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the current global bridge owns this live context host.
        return unsafe { &*host_ptr }.opfs_handle_registry();
    }
    crate::worker::worker_opfs_handle_registry(scope)
}

fn ensure_current_opfs_handle_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<OpfsHandleRegistry> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the current global bridge owns this live context host and the
        // callback runs on its renderer sequence.
        return Some(unsafe { &mut *host_ptr }.ensure_opfs_handle_registry());
    }
    crate::worker::ensure_worker_opfs_handle_registry(scope)
}

fn current_opfs_directory_iterator_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<OpfsDirectoryIteratorRegistry> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the current global bridge owns this live context host.
        return unsafe { &*host_ptr }.opfs_directory_iterator_registry();
    }
    crate::worker::worker_opfs_directory_iterator_registry(scope)
}

fn ensure_current_opfs_directory_iterator_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<OpfsDirectoryIteratorRegistry> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the current global bridge owns this live context host and the
        // callback runs on its renderer sequence.
        return Some(unsafe { &mut *host_ptr }.ensure_opfs_directory_iterator_registry());
    }
    crate::worker::ensure_worker_opfs_directory_iterator_registry(scope)
}

fn cancel_registered_opfs_task(
    scope: &mut v8::PinScope<'_, '_>,
    cancellation: RegisteredOpfsTaskCancellation,
) {
    match cancellation {
        RegisteredOpfsTaskCancellation::Page { task_id } => {
            if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
                // SAFETY: cancellation occurs synchronously in the same callback
                // that registered the task, before control returns to V8.
                let cancelled = unsafe { &mut *host_ptr }.cancel_pending_opfs_task(task_id);
                assert!(
                    cancelled,
                    "a synchronous OPFS dispatch failure must cancel its exact pending task"
                );
            }
        }
        RegisteredOpfsTaskCancellation::Worker { task_id } => {
            crate::worker::cancel_worker_opfs_task(scope, task_id);
        }
    }
}

fn dispatch_opfs_task<'s, T, Operation, Wrap>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: StorageBucketLocator,
    service: SharedStorageService,
    handle_access: OpfsHandleAccess,
    operation: Operation,
    wrap: Wrap,
) where
    T: Send + 'static,
    Operation: FnOnce(&Opfs) -> OpfsResult<T> + Send + 'static,
    Wrap: FnOnce(Result<OpfsResult<T>, StorageServiceTaskError>) -> OpfsTaskResult + Send + 'static,
{
    let Some(registered) =
        register_opfs_task(scope, resolver, locator.clone(), handle_access.clone())
    else {
        let result = Ok(service.with_opfs(operation));
        settle_opfs_task_result(
            scope,
            resolver,
            &locator,
            handle_access.as_ref(),
            wrap(result),
        );
        return;
    };

    let (cancellation, completion_sink) = registered.into_parts();
    let dispatch = service.dispatch_opfs(operation, move |result| {
        completion_sink.send(wrap(result));
    });
    if dispatch.is_err() {
        cancel_registered_opfs_task(scope, cancellation);
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to dispatch the OPFS operation to the storage owner.",
        );
    }
}

fn opfs_quota_owner(
    scope: &mut v8::PinScope<'_, '_>,
    locator: &StorageBucketLocator,
) -> Result<Option<StorageBucketQuotaOwner>, HandleAccessError> {
    storage_bucket_quota_owner_for_locator(scope, locator)
        .map(Some)
        .ok_or(HandleAccessError::InvalidState)
}

fn opfs_max_usage(owner: &StorageBucketQuotaOwner) -> OpfsResult<u64> {
    owner.max_opfs_usage().map_err(|_| OpfsError::InvalidState)
}

fn with_opfs_quota_mutation<T>(
    service: &SharedStorageService,
    locator: &StorageBucketLocator,
    fallback_quota: u64,
    quota_owner: Option<StorageBucketQuotaOwner>,
    operation: impl FnOnce(&Opfs, u64) -> OpfsResult<T>,
) -> OpfsResult<T> {
    if let Some(owner) = quota_owner {
        service.with_opfs_quota_commit(locator, move |opfs| {
            let max_usage = opfs_max_usage(&owner)?;
            operation(opfs, max_usage)
        })
    } else {
        service.with_opfs(|opfs| operation(opfs, fallback_quota))
    }
}

fn dispatch_opfs_quota_mutation_task<'s, T, Operation, Wrap>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: StorageBucketLocator,
    service: SharedStorageService,
    fallback_quota: u64,
    quota_owner: Option<StorageBucketQuotaOwner>,
    handle_access: OpfsHandleAccess,
    operation: Operation,
    wrap: Wrap,
) where
    T: Send + 'static,
    Operation: FnOnce(&Opfs, u64) -> OpfsResult<T> + Send + 'static,
    Wrap: FnOnce(Result<OpfsResult<T>, StorageServiceTaskError>) -> OpfsTaskResult + Send + 'static,
{
    let Some(registered) =
        register_opfs_task(scope, resolver, locator.clone(), handle_access.clone())
    else {
        let result = Ok(with_opfs_quota_mutation(
            &service,
            &locator,
            fallback_quota,
            quota_owner,
            operation,
        ));
        settle_opfs_task_result(
            scope,
            resolver,
            &locator,
            handle_access.as_ref(),
            wrap(result),
        );
        return;
    };

    let (cancellation, completion_sink) = registered.into_parts();
    let dispatch = if let Some(owner) = quota_owner {
        service.dispatch_opfs_quota_commit(
            locator,
            move |opfs| {
                let max_usage = opfs_max_usage(&owner)?;
                operation(opfs, max_usage)
            },
            move |result| {
                completion_sink.send(wrap(result));
            },
        )
    } else {
        service.dispatch_opfs(
            move |opfs| operation(opfs, fallback_quota),
            move |result| {
                completion_sink.send(wrap(result));
            },
        )
    };
    if dispatch.is_err() {
        cancel_registered_opfs_task(scope, cancellation);
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to dispatch the OPFS mutation to the storage owner.",
        );
    }
}

#[derive(WebApiObject)]
#[webapi(interface = "FileSystemDirectoryHandle", require_prototype)]
struct FileSystemDirectoryHandleObjectDeclaration {
    #[webapi(slot, name = FILE_SYSTEM_HANDLE_BRAND_SLOT, constructor_default = true)]
    brand: bool,
    #[webapi(slot = FILE_SYSTEM_HANDLE_STATE_SLOT)]
    state_json: String,
    #[webapi(slot = FILE_SYSTEM_HANDLE_ID_SLOT)]
    handle_id: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "FileSystemFileHandle", require_prototype)]
struct FileSystemFileHandleObjectDeclaration {
    #[webapi(slot, name = FILE_SYSTEM_HANDLE_BRAND_SLOT, constructor_default = true)]
    brand: bool,
    #[webapi(slot = FILE_SYSTEM_HANDLE_STATE_SLOT)]
    state_json: String,
    #[webapi(slot = FILE_SYSTEM_HANDLE_ID_SLOT)]
    handle_id: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemHandle", enumerable)]
struct FileSystemHandlePrototypeDeclaration {
    #[webapi(accessor_property, getter = file_system_handle_kind_getter_callback)]
    kind: (),
    #[webapi(accessor_property, getter = file_system_handle_name_getter_callback)]
    name: (),
    #[webapi(method, length = 0, callback = file_system_handle_query_permission_callback)]
    query_permission: (),
    #[webapi(method, length = 0, callback = file_system_handle_request_permission_callback)]
    request_permission: (),
    #[webapi(method, length = 1, callback = file_system_handle_is_same_entry_callback)]
    is_same_entry: (),
    #[webapi(method, length = 0, callback = file_system_handle_get_unique_id_callback)]
    get_unique_id: (),
    #[webapi(method, length = 0, callback = file_system_handle_remove_callback)]
    remove: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemDirectoryHandle", enumerable)]
struct FileSystemDirectoryHandlePrototypeDeclaration {
    #[webapi(method, length = 1, callback = file_system_directory_get_file_handle_callback)]
    get_file_handle: (),
    #[webapi(method, length = 1, callback = file_system_directory_get_directory_handle_callback)]
    get_directory_handle: (),
    #[webapi(method, length = 1, callback = file_system_directory_remove_entry_callback)]
    remove_entry: (),
    #[webapi(method, length = 1, callback = file_system_directory_resolve_callback)]
    resolve: (),
    #[webapi(method, length = 1, callback = file_system_directory_move_callback)]
    move_: (),
    #[webapi(method, length = 0, callback = file_system_directory_entries_callback)]
    entries: (),
    #[webapi(method, length = 0, callback = file_system_directory_keys_callback)]
    keys: (),
    #[webapi(method, length = 0, callback = file_system_directory_values_callback)]
    values: (),
    #[webapi(alias = "entries", symbol = "asyncIterator")]
    async_iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemFileHandle", enumerable)]
struct FileSystemFileHandlePrototypeDeclaration {
    #[webapi(method, length = 0, callback = file_system_file_create_writable_callback)]
    create_writable: (),
    #[webapi(method, length = 0, callback = file_system_file_get_file_callback)]
    get_file: (),
    #[webapi(method, length = 1, callback = file_system_file_move_callback)]
    move_: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemFileHandle", enumerable)]
struct FileSystemFileHandleSyncPrototypeDeclaration {
    #[webapi(
        method,
        length = 0,
        callback = file_system_file_create_sync_access_handle_callback
    )]
    create_sync_access_handle: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "FileSystemWritableFileStream", require_prototype)]
struct FileSystemWritableFileStreamObjectDeclaration {
    #[webapi(slot, name = FILE_SYSTEM_WRITABLE_BRAND_SLOT, constructor_default = true)]
    brand: bool,
    #[webapi(slot = FILE_SYSTEM_WRITABLE_MODE_SLOT)]
    mode: String,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemWritableFileStream", enumerable)]
struct FileSystemWritableFileStreamPrototypeDeclaration {
    #[webapi(accessor_property, getter = file_system_writable_mode_getter_callback)]
    mode: (),
    #[webapi(method, length = 1, callback = file_system_writable_write_callback)]
    write: (),
    #[webapi(method, length = 1, callback = file_system_writable_seek_callback)]
    seek: (),
    #[webapi(method, length = 1, callback = file_system_writable_truncate_callback)]
    truncate: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct FileSystemWritableSinkObjectDeclaration {
    #[webapi(slot, name = FILE_SYSTEM_WRITABLE_SINK_BRAND_SLOT, constructor_default = true)]
    brand: bool,
    #[webapi(slot = FILE_SYSTEM_WRITABLE_SINK_STATE_SLOT)]
    state_json: String,
    #[webapi(method, length = 1, callback = file_system_writable_sink_write_callback)]
    write: (),
    #[webapi(method, length = 0, callback = file_system_writable_sink_close_callback)]
    close: (),
    #[webapi(method, length = 1, callback = file_system_writable_sink_abort_callback)]
    abort: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "FileSystemSyncAccessHandle", require_prototype)]
struct FileSystemSyncAccessHandleObjectDeclaration {
    #[webapi(slot, name = FILE_SYSTEM_SYNC_ACCESS_BRAND_SLOT, constructor_default = true)]
    brand: bool,
    #[webapi(slot = FILE_SYSTEM_SYNC_ACCESS_STATE_SLOT)]
    state_json: String,
    #[webapi(slot, name = FILE_SYSTEM_SYNC_ACCESS_CLOSED_SLOT, constructor_default = false)]
    closed: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemSyncAccessHandle", enumerable)]
struct FileSystemSyncAccessHandlePrototypeDeclaration {
    #[webapi(accessor_property, getter = file_system_sync_access_mode_getter_callback)]
    mode: (),
    #[webapi(method, length = 0, callback = file_system_sync_access_close_callback)]
    close: (),
    #[webapi(method, length = 0, callback = file_system_sync_access_flush_callback)]
    flush: (),
    #[webapi(method, length = 0, callback = file_system_sync_access_get_size_callback)]
    get_size: (),
    #[webapi(method, length = 1, callback = file_system_sync_access_truncate_callback)]
    truncate: (),
    #[webapi(method, length = 1, callback = file_system_sync_access_read_callback)]
    read: (),
    #[webapi(method, length = 1, callback = file_system_sync_access_write_callback)]
    write: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct FileSystemDirectoryIteratorObjectDeclaration {
    #[webapi(slot = FILE_SYSTEM_ITERATOR_BRAND_SLOT, init = true)]
    brand: (),
    #[webapi(slot = FILE_SYSTEM_ITERATOR_ID_SLOT)]
    iterator_id: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "FileSystemDirectoryHandle AsyncIterator",
    intrinsic_prototype_parent = v8::Intrinsic::AsyncIteratorPrototype,
    prototype_to_string_tag = "FileSystemDirectoryHandle AsyncIterator",
    readonly_prototype,
    enumerable
)]
struct FileSystemDirectoryIteratorPrototypeDeclaration {
    #[webapi(
        method,
        length = 0,
        callback = file_system_directory_iterator_next_callback
    )]
    next: (),
}

pub(in crate::context_bootstrap) fn install_opfs_constructor_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "FileSystemHandle" => {
            FileSystemHandlePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "FileSystemDirectoryHandle" => {
            FileSystemDirectoryHandlePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "FileSystemFileHandle" => {
            FileSystemFileHandlePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "FileSystemWritableFileStream" => {
            FileSystemWritableFileStreamPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "FileSystemSyncAccessHandle" => {
            FileSystemSyncAccessHandlePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn install_file_system_file_handle_sync_template_binding<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FileSystemFileHandleSyncPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn resolve_opfs_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: StorageBucketLocator,
) {
    let (service, key, _) = match service_for_locator(scope, &locator) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let handle_access = current_handle_access_context(scope, locator.storage_key());
    dispatch_opfs_task(
        scope,
        resolver,
        locator,
        service,
        handle_access,
        move |opfs| opfs.ensure_root(&key),
        OpfsTaskResult::GetRoot,
    );
}

pub(in crate::context_bootstrap) fn resolve_opfs_root_with_handle_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: StorageBucketLocator,
    handle_access: OpfsHandleAccessContext,
) {
    let (service, key, _) =
        match service_for_locator_with_handle_access(scope, &locator, &handle_access) {
            Ok(values) => values,
            Err(error) => {
                reject_handle_access_error(scope, resolver, error);
                return;
            }
        };
    dispatch_opfs_task(
        scope,
        resolver,
        locator,
        service,
        Some(handle_access),
        move |opfs| opfs.ensure_root(&key),
        OpfsTaskResult::GetRoot,
    );
}

fn build_handle_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    locator: &StorageBucketLocator,
    path: &OpfsPath,
    kind: EntryKind,
    handle_access: OpfsHandleAccess,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = FileSystemHandleState::new(locator.clone(), path, kind);
    let state_json = serde_json::to_string(&state).ok()?;
    let registry = ensure_current_opfs_handle_registry(scope)?;
    let handle_id = registry.insert(path.clone(), handle_access);
    let handle = match kind {
        EntryKind::Directory => {
            FileSystemDirectoryHandleObjectDeclaration::new(state_json, handle_id as f64)
                .bind(scope)
                .ok()
        }
        EntryKind::File => FileSystemFileHandleObjectDeclaration::new(state_json, handle_id as f64)
            .bind(scope)
            .ok(),
    };
    let Some(handle) = handle else {
        registry.remove(handle_id);
        return None;
    };
    let cleanup_registry = registry.clone();
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, handle, move || {
        cleanup_registry.remove(handle_id);
    });
    Some(handle)
}

pub(crate) fn file_system_handle_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<FileSystemHandleClonePayload> {
    let state = handle_state(scope, object)?;
    let path = handle_path_state(scope, object)?.current();
    let handle_access = handle_access(scope, object);
    let storage_service = match handle_access.as_ref() {
        Some(access) => {
            service_for_locator_with_handle_access(scope, &state.locator, access)
                .ok()?
                .0
        }
        None => service_for_locator(scope, &state.locator).ok()?.0,
    };
    let source_origin = handle_source_origin(scope, handle_access.as_ref())?;
    Some(FileSystemHandleClonePayload {
        source_origin,
        storage_service: std::sync::Arc::downgrade(&storage_service),
        locator: state.locator,
        path,
        kind: state.kind,
    })
}

pub(crate) fn build_file_system_handle_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &FileSystemHandleClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiving_storage_key = current_storage_bucket_storage_key(scope)?;
    if serialized_storage_key_origin(&receiving_storage_key).as_deref()
        != Some(payload.source_origin.as_str())
    {
        return None;
    }
    let receiving_service =
        with_storage_bucket_store_entry(scope, |store| store.storage_service())?;
    let sending_service = payload.storage_service.upgrade()?;
    if !std::sync::Arc::ptr_eq(&receiving_service, &sending_service) {
        return None;
    }
    service_for_locator_with_storage_key(scope, &payload.locator, payload.locator.storage_key())
        .ok()?;
    let handle_access = Some(current_clone_access_context(
        scope,
        payload.locator.storage_key(),
    )?);
    build_handle_object(
        scope,
        &payload.locator,
        &payload.path,
        payload.kind,
        handle_access,
    )
}

pub(crate) fn file_system_file_snapshot_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<FileSystemFileSnapshotClonePayload> {
    let state = file_system_file_snapshot_state(scope, object)?;
    let storage_service = with_storage_bucket_store_entry(scope, |store| store.storage_service())?;
    Some(FileSystemFileSnapshotClonePayload {
        state,
        storage_service: std::sync::Arc::downgrade(&storage_service),
    })
}

pub(crate) fn attach_file_system_file_snapshot_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    payload: &FileSystemFileSnapshotClonePayload,
) -> Option<()> {
    let receiving_service =
        with_storage_bucket_store_entry(scope, |store| store.storage_service())?;
    let Some(sending_service) = payload.storage_service.upgrade() else {
        return Some(());
    };
    if !std::sync::Arc::ptr_eq(&receiving_service, &sending_service) {
        // The File bytes are still a valid ordinary structured-clone copy.
        // Do not attach a locator which the receiving partition cannot
        // validate against the original storage owner.
        return Some(());
    }
    attach_file_system_file_snapshot_state(scope, object, &payload.state)
}

pub(crate) fn file_system_handle_durable_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<FileSystemHandleDurablePayload> {
    let state = handle_state(scope, object)?;
    let path = handle_path_state(scope, object)?.current();
    let authorized_storage_key = match handle_access(scope, object) {
        Some(access) => {
            service_for_locator_with_handle_access(scope, &state.locator, &access).ok()?;
            access.storage_key().to_owned()
        }
        None => {
            service_for_locator(scope, &state.locator).ok()?;
            current_storage_bucket_storage_key(scope)?
        }
    };
    if authorized_storage_key != state.locator.storage_key() {
        return None;
    }
    Some(FileSystemHandleDurablePayload {
        bucket_id: state.locator.bucket_id().map(StorageBucketId::get),
        path: path.components().to_vec(),
        kind: state.kind,
    })
}

pub(crate) fn build_file_system_handle_from_durable_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &FileSystemHandleDurablePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    let storage_key = current_storage_bucket_storage_key(scope)?;
    let locator = match payload.bucket_id {
        Some(bucket_id) => {
            StorageBucketLocator::named(storage_key, StorageBucketId::new(bucket_id)?)
        }
        None => StorageBucketLocator::default_bucket(storage_key),
    };
    let path = OpfsPath::from_components(payload.path.clone()).ok()?;
    // Chromium can materialize an IndexedDB handle wrapper before an async
    // bucket lookup reports that its persistent bucket was deleted. Keep the
    // old locator on the wrapper; every observable operation rechecks bucket
    // liveness and therefore cannot bind it to a same-name replacement.
    let handle_access = current_handle_access_context(scope, locator.storage_key());
    build_handle_object(scope, &locator, &path, payload.kind, handle_access)
}

fn handle_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let value =
        get_private_value(scope, object, FILE_SYSTEM_HANDLE_ID_SLOT)?.number_value(scope)?;
    (value.is_finite() && value.fract() == 0.0 && value >= 1.0 && value <= u32::MAX as f64)
        .then_some(value as u32)
}

fn handle_path_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<OpfsHandlePathState> {
    let handle_id = handle_id(scope, object)?;
    current_opfs_handle_registry(scope)?.path_state(handle_id)
}

fn handle_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> OpfsHandleAccess {
    let handle_id = handle_id(scope, object)?;
    current_opfs_handle_registry(scope)?.handle_access(handle_id)
}

fn derived_handle_access(
    scope: &mut v8::PinScope<'_, '_>,
    access_id: Option<u32>,
) -> Result<OpfsHandleAccess, HandleAccessError> {
    let Some(access_id) = access_id else {
        return Ok(None);
    };
    current_opfs_handle_registry(scope)
        .and_then(|registry| registry.derived_access(access_id))
        .map(Some)
        .ok_or(HandleAccessError::InvalidState)
}

fn service_for_derived_handle_access(
    scope: &mut v8::PinScope<'_, '_>,
    locator: &StorageBucketLocator,
    access_id: Option<u32>,
) -> Result<(SharedStorageService, OpfsBucketKey, u64, OpfsHandleAccess), HandleAccessError> {
    let handle_access = derived_handle_access(scope, access_id)?;
    let (service, key, quota) = match handle_access.as_ref() {
        Some(access) => service_for_locator_with_handle_access(scope, locator, access)?,
        None => service_for_locator(scope, locator)?,
    };
    Ok((service, key, quota, handle_access))
}

fn service_for_handle_with_quota<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    state: &FileSystemHandleState,
) -> Result<
    (
        SharedStorageService,
        OpfsBucketKey,
        OpfsHandlePathState,
        u64,
        OpfsHandleAccess,
    ),
    HandleAccessError,
> {
    let handle_access = handle_access(scope, object);
    let (service, key, quota) = match handle_access.as_ref() {
        Some(access) => service_for_locator_with_handle_access(scope, &state.locator, access)?,
        None => service_for_locator(scope, &state.locator)?,
    };
    let path = handle_path_state(scope, object).ok_or(HandleAccessError::InvalidState)?;
    Ok((service, key, path, quota, handle_access))
}

fn service_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    state: &FileSystemHandleState,
) -> Result<
    (
        SharedStorageService,
        OpfsBucketKey,
        OpfsHandlePathState,
        OpfsHandleAccess,
    ),
    HandleAccessError,
> {
    let (service, key, path, _, handle_access) =
        service_for_handle_with_quota(scope, object, state)?;
    Ok((service, key, path, handle_access))
}

fn handle_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<FileSystemHandleState> {
    let branded = get_private_value(scope, object, FILE_SYSTEM_HANDLE_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    if !branded {
        return None;
    }
    let json = get_private_value(scope, object, FILE_SYSTEM_HANDLE_STATE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&json).ok()
}

fn file_system_file_snapshot_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<FileSystemFileSnapshotState> {
    let json = get_private_value(scope, object, FILE_SYSTEM_FILE_SNAPSHOT_STATE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&json).ok()
}

fn attach_file_system_file_snapshot_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    state: &FileSystemFileSnapshotState,
) -> Option<()> {
    let state_json = serde_json::to_string(state).ok()?;
    let state_json = v8_string(scope, &state_json)?;
    set_private_value(
        scope,
        object,
        FILE_SYSTEM_FILE_SNAPSHOT_STATE_SLOT,
        state_json.into(),
    );
    Some(())
}

fn service_for_state_with_handle_access(
    scope: &mut v8::PinScope<'_, '_>,
    state: &FileSystemHandleState,
    handle_access: Option<&OpfsHandleAccessContext>,
) -> Result<(SharedStorageService, OpfsBucketKey, OpfsPath), HandleAccessError> {
    let (service, key, _) = match handle_access {
        Some(access) => service_for_locator_with_handle_access(scope, &state.locator, access)?,
        None => service_for_locator(scope, &state.locator)?,
    };
    let path = state.opfs_path().map_err(HandleAccessError::Backend)?;
    Ok((service, key, path))
}

fn service_for_file_snapshot_state(
    scope: &mut v8::PinScope<'_, '_>,
    state: &FileSystemFileSnapshotState,
) -> Result<(SharedStorageService, OpfsBucketKey, OpfsPath), HandleAccessError> {
    // Snapshot state is a private, trusted attachment. Unlike a handle, a
    // structured-cloned File is data and may cross origins. Validation only
    // checks its original backing identity; it does not expose namespace data
    // or create a handle in the receiving realm.
    let (service, live) = with_storage_bucket_store_entry(scope, |store| {
        (
            store.storage_service(),
            store.bucket_locator_is_live(&state.locator),
        )
    })
    .ok_or(HandleAccessError::InvalidState)?;
    if !live {
        return Err(HandleAccessError::Stale);
    }
    let key =
        StorageService::opfs_bucket_key(&state.locator).map_err(HandleAccessError::Backend)?;
    let path = state.opfs_path().map_err(HandleAccessError::Backend)?;
    Ok((service, key, path))
}

fn service_for_locator(
    scope: &mut v8::PinScope<'_, '_>,
    locator: &StorageBucketLocator,
) -> Result<(SharedStorageService, OpfsBucketKey, u64), HandleAccessError> {
    let current_storage_key =
        current_storage_bucket_storage_key(scope).ok_or(HandleAccessError::InvalidState)?;
    service_for_locator_with_storage_key(scope, locator, &current_storage_key)
}

fn current_handle_access_context(
    scope: &mut v8::PinScope<'_, '_>,
    storage_key: &str,
) -> Option<OpfsHandleAccessContext> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    // SAFETY: handle construction runs synchronously in the current live V8
    // context and stores only a copyable execution-context identity.
    let identity =
        unsafe { &*host_ptr }.current_runtime_window_execution_context_identity(scope)?;
    Some(OpfsHandleAccessContext::window(
        identity,
        storage_key.to_owned(),
    ))
}

fn current_clone_access_context(
    scope: &mut v8::PinScope<'_, '_>,
    storage_key: &str,
) -> Option<OpfsHandleAccessContext> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: clone deserialization runs synchronously in the receiving
        // live V8 context and stores only its execution-context identity.
        let identity =
            unsafe { &*host_ptr }.current_runtime_window_execution_context_identity(scope)?;
        return Some(OpfsHandleAccessContext::window(
            identity,
            storage_key.to_owned(),
        ));
    }
    crate::worker::worker_storage_key(scope)?;
    Some(OpfsHandleAccessContext::worker(storage_key.to_owned()))
}

fn serialized_storage_key_origin(storage_key: &str) -> Option<String> {
    moli_storage_key::deserialize_serialized_storage_key(storage_key)
        .map(|key| key.origin().to_owned())
}

fn handle_source_origin(
    scope: &mut v8::PinScope<'_, '_>,
    handle_access: Option<&OpfsHandleAccessContext>,
) -> Option<String> {
    if let Some(identity) = handle_access.and_then(OpfsHandleAccessContext::window_identity) {
        let host_ptr = context_host_ptr_from_global_bridge(scope)?;
        // SAFETY: the context-owned handle registry keeps the exact live
        // wrapper realm identity. Do not use the ambient active child here:
        // a prior cross-realm message may leave a different child selected
        // while the sender serializes its next handle.
        let host = unsafe { &mut *host_ptr };
        let source_storage_key = host
            .storage_context_for_window_execution_context_identity(identity)?
            .storage_key()
            .serialized_storage_key();
        return serialized_storage_key_origin(&source_storage_key);
    }
    if let Some(access) = handle_access {
        return serialized_storage_key_origin(access.storage_key());
    }
    let source_storage_key = current_storage_bucket_storage_key(scope)?;
    serialized_storage_key_origin(&source_storage_key)
}

fn service_for_locator_with_handle_access(
    scope: &mut v8::PinScope<'_, '_>,
    locator: &StorageBucketLocator,
    access: &OpfsHandleAccessContext,
) -> Result<(SharedStorageService, OpfsBucketKey, u64), HandleAccessError> {
    ensure_handle_access_is_current(scope, access)?;
    service_for_locator_with_storage_key(scope, locator, access.storage_key())
}

fn ensure_handle_access_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    access: &OpfsHandleAccessContext,
) -> Result<(), HandleAccessError> {
    match (
        context_host_ptr_from_global_bridge(scope),
        access.window_identity(),
    ) {
        (Some(host_ptr), Some(identity)) => {
            let host = unsafe { &mut *host_ptr };
            if !host.window_execution_context_identity_is_current(identity) {
                return Err(HandleAccessError::InvalidState);
            }
        }
        (None, None) if crate::worker::worker_storage_key(scope).is_some() => {}
        _ => return Err(HandleAccessError::InvalidState),
    }
    Ok(())
}

fn service_for_locator_with_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
    locator: &StorageBucketLocator,
    current_storage_key: &str,
) -> Result<(SharedStorageService, OpfsBucketKey, u64), HandleAccessError> {
    if current_storage_key != locator.storage_key() {
        return Err(HandleAccessError::Security);
    }
    let (service, live, quota) = with_storage_bucket_store_entry(scope, |store| {
        (
            store.storage_service(),
            store.bucket_locator_is_live(locator),
            store.opfs_quota_for_locator(locator),
        )
    })
    .ok_or(HandleAccessError::InvalidState)?;
    if !live {
        return Err(HandleAccessError::Stale);
    }
    let quota = quota.ok_or(HandleAccessError::Stale)?;
    let key = StorageService::opfs_bucket_key(locator).map_err(HandleAccessError::Backend)?;
    Ok((service, key, quota))
}

fn file_system_handle_kind_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = handle_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let kind = state.kind.to_string();
    if let Some(kind) = v8_string(scope, &kind) {
        rv.set(kind.into());
    }
}

fn file_system_handle_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = handle_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let name = state.path.last().map(String::as_str).unwrap_or("");
    if let Some(name) = v8_string(scope, name) {
        rv.set(name.into());
    }
}

fn file_system_handle_query_permission_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    file_system_handle_permission_callback(scope, args, rv, "queryPermission");
}

fn file_system_handle_request_permission_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    file_system_handle_permission_callback(scope, args, rv, "requestPermission");
}

fn file_system_handle_permission_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    if handle_state(scope, args.this()).is_none() {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    }
    let descriptor_object = match webidl::dictionary_arg(
        &args,
        0,
        webidl::Context::argument(
            match method {
                "queryPermission" => "FileSystemHandle.queryPermission",
                _ => "FileSystemHandle.requestPermission",
            },
            1,
        ),
    ) {
        Ok(value) => value,
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    let descriptor = match descriptor_object {
        Some(object) => {
            match webidl::parse_dictionary_object::<FileSystemHandlePermissionDescriptor>(
                scope, object,
            ) {
                Ok(descriptor) => descriptor,
                Err(error) if error.is_pending_exception() => return,
                Err(error) => {
                    reject_type_error(scope, resolver, &error.to_string());
                    return;
                }
            }
        }
        None => FileSystemHandlePermissionDescriptor::default(),
    };
    let _mode = descriptor.mode;
    // Chromium gives sandboxed/OPFS handles fixed granted read and write
    // permission objects. Permission queries operate on those grants rather
    // than probing entry or bucket liveness, so a handle whose entry was
    // removed still reports "granted".
    let granted = v8str(scope, "granted");
    let _ = resolver.resolve(scope, granted.into());
}

fn file_system_handle_is_same_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(left) = handle_state(scope, args.this()) else {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    };
    if args.length() < 1 {
        reject_type_error(
            scope,
            resolver,
            "FileSystemHandle.isSameEntry requires a handle.",
        );
        return;
    }
    let Ok(other) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        reject_type_error(scope, resolver, "The argument is not a FileSystemHandle.");
        return;
    };
    let Some(right) = handle_state(scope, other) else {
        reject_type_error(scope, resolver, "The argument is not a FileSystemHandle.");
        return;
    };
    if left.locator != right.locator {
        let _ = resolver.resolve(scope, v8::Boolean::new(scope, false).into());
        return;
    }
    let (service, _, left_path, handle_access) = match service_for_handle(scope, args.this(), &left)
    {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let Some(right_path) = handle_path_state(scope, other) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The compared file system handle is no longer active.",
        );
        return;
    };
    let same_kind = left.kind == right.kind;
    dispatch_opfs_task(
        scope,
        resolver,
        left.locator,
        service,
        handle_access,
        move |_opfs| Ok(same_kind && left_path.current() == right_path.current()),
        OpfsTaskResult::IsSameEntry,
    );
}

fn file_system_handle_get_unique_id_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) = handle_state(scope, args.this()) else {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    };
    let (service, key, path, handle_access) = match service_for_handle(scope, args.this(), &state) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let mut random_bytes = [0_u8; 16];
    if let Err(error) = fill_secure_random(&mut random_bytes) {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            &format!("Failed to generate an OPFS unique ID: {error}"),
        );
        return;
    }
    let candidate = UuidBuilder::from_random_bytes(random_bytes)
        .into_uuid()
        .to_string();
    let unique_id_service = service.clone();
    let locator = state.locator;
    let kind = state.kind;
    dispatch_opfs_task(
        scope,
        resolver,
        locator,
        service,
        handle_access,
        move |_opfs| {
            Ok(unique_id_service.opfs_unique_id_or_insert(key, path.current(), kind, candidate))
        },
        OpfsTaskResult::GetUniqueId,
    );
}

fn file_system_handle_remove_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) = handle_state(scope, args.this()) else {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    };
    let Some(recursive) = boolean_option(scope, &args, 0, "recursive", resolver) else {
        return;
    };
    let (service, key, path, handle_access) = match service_for_handle(scope, args.this(), &state) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let mutation_lease_service = service.clone();
    dispatch_opfs_task(
        scope,
        resolver,
        state.locator,
        service,
        handle_access,
        move |opfs| {
            let path = path.current();
            if path.is_root() {
                opfs.clear_bucket(&key)?;
                Ok(None)
            } else {
                opfs.remove_entry_with_mutation_lease(&key, &path, recursive)
                    .map(|lease| Some(StorageOpfsMutationLease::new(mutation_lease_service, lease)))
            }
        },
        OpfsTaskResult::Remove,
    );
}

fn file_system_file_move_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    file_system_handle_move_callback(scope, args, rv, EntryKind::File);
}

fn file_system_directory_move_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    file_system_handle_move_callback(scope, args, rv, EntryKind::Directory);
}

enum FileSystemHandleMoveTarget {
    Rename(String),
    Reparent {
        destination_parent: OpfsPath,
        new_name: Option<String>,
    },
}

fn file_system_handle_move_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    expected_kind: EntryKind,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let state = match handle_state(scope, args.this()) {
        Some(state) if state.kind == expected_kind => state,
        _ => {
            reject_type_error(scope, resolver, "Illegal invocation");
            return;
        }
    };
    if args.length() < 1 {
        reject_type_error(
            scope,
            resolver,
            "FileSystemHandle.move requires a name or destination directory.",
        );
        return;
    }

    let destination_state = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|object| handle_state(scope, object))
        .filter(|state| state.kind == EntryKind::Directory);
    let (service, key, source, quota, handle_access) =
        match service_for_handle_with_quota(scope, args.this(), &state) {
            Ok(values) => values,
            Err(error) => {
                reject_handle_access_error(scope, resolver, error);
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(owner) => owner,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let target = if args.length() >= 2 {
        let Some(destination_state) = destination_state else {
            reject_type_error(
                scope,
                resolver,
                "FileSystemHandle.move destination must be a directory handle.",
            );
            return;
        };
        if destination_state.locator != state.locator {
            reject_dom_exception(
                scope,
                resolver,
                "InvalidModificationError",
                "An entry cannot be moved between different file systems.",
            );
            return;
        }
        let Some(new_name) = name_argument(scope, &args, 1, resolver) else {
            return;
        };
        let destination_parent = match destination_state.opfs_path() {
            Ok(path) => path,
            Err(error) => {
                reject_opfs_error(scope, resolver, error);
                return;
            }
        };
        FileSystemHandleMoveTarget::Reparent {
            destination_parent,
            new_name: Some(new_name),
        }
    } else if let Some(destination_state) = destination_state {
        if destination_state.locator != state.locator {
            reject_dom_exception(
                scope,
                resolver,
                "InvalidModificationError",
                "An entry cannot be moved between different file systems.",
            );
            return;
        }
        let destination_parent = match destination_state.opfs_path() {
            Ok(path) => path,
            Err(error) => {
                reject_opfs_error(scope, resolver, error);
                return;
            }
        };
        FileSystemHandleMoveTarget::Reparent {
            destination_parent,
            new_name: None,
        }
    } else {
        let Some(new_name) = name_argument(scope, &args, 0, resolver) else {
            return;
        };
        FileSystemHandleMoveTarget::Rename(new_name)
    };

    let locator = state.locator;
    let Some(mutation) = source.try_begin_mutation() else {
        reject_dom_exception(
            scope,
            resolver,
            "NoModificationAllowedError",
            "The file system handle already has a pending mutation.",
        );
        return;
    };
    let operation_source = source.clone();
    let mutation_lease_service = service.clone();
    let operation = move |opfs: &Opfs, max_usage: u64| {
        let source_path = operation_source.current();
        let (destination_parent, new_name) = match target {
            FileSystemHandleMoveTarget::Rename(new_name) => {
                let destination_parent = source_path.parent().ok_or_else(|| {
                    OpfsError::InvalidModification("the OPFS root cannot be moved".to_owned())
                })?;
                (destination_parent, new_name)
            }
            FileSystemHandleMoveTarget::Reparent {
                destination_parent,
                new_name,
            } => {
                let new_name = new_name.unwrap_or_else(|| source_path.name().to_owned());
                if new_name.is_empty() {
                    return Err(OpfsError::InvalidModification(
                        "the OPFS root cannot be moved".to_owned(),
                    ));
                }
                (destination_parent, new_name)
            }
        };
        let (destination, mutation_lease) = opfs.move_entry_with_mutation_lease(
            &key,
            &source_path,
            expected_kind,
            &destination_parent,
            &new_name,
            Some(max_usage),
        )?;
        operation_source.replace(destination.clone());
        Ok((
            destination,
            StorageOpfsMutationLease::new(mutation_lease_service, mutation_lease),
        ))
    };
    let Some(registered) = register_opfs_move_task(
        scope,
        resolver,
        args.this(),
        mutation,
        locator.clone(),
        handle_access.clone(),
    ) else {
        let result = Ok(with_opfs_quota_mutation(
            &service,
            &locator,
            quota,
            quota_owner,
            operation,
        ));
        settle_opfs_move_task_result(
            scope,
            resolver,
            args.this(),
            &locator,
            handle_access.as_ref(),
            OpfsTaskResult::Move(result),
        );
        return;
    };
    let (cancellation, completion_sink) = registered.into_parts();
    let dispatch = if let Some(owner) = quota_owner {
        service.dispatch_opfs_quota_commit(
            locator,
            move |opfs| {
                let max_usage = opfs_max_usage(&owner)?;
                operation(opfs, max_usage)
            },
            move |result| {
                completion_sink.send(OpfsTaskResult::Move(result));
            },
        )
    } else {
        service.dispatch_opfs(
            move |opfs| operation(opfs, quota),
            move |result| {
                completion_sink.send(OpfsTaskResult::Move(result));
            },
        )
    };
    if dispatch.is_err() {
        cancel_registered_opfs_task(scope, cancellation);
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to dispatch the OPFS move to the storage owner.",
        );
    }
}

fn file_system_directory_get_file_handle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    get_child_handle(scope, args, rv, EntryKind::File);
}

fn file_system_directory_get_directory_handle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    get_child_handle(scope, args, rv, EntryKind::Directory);
}

fn get_child_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: EntryKind,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) = directory_state_or_reject(scope, args.this(), resolver) else {
        return;
    };
    let Some(name) = required_name(scope, &args, resolver) else {
        return;
    };
    let Some(create) = boolean_option(scope, &args, 1, "create", resolver) else {
        return;
    };
    let (service, key, parent, quota, handle_access) =
        match service_for_handle_with_quota(scope, args.this(), &state) {
            Ok(values) => values,
            Err(error) => {
                reject_handle_access_error(scope, resolver, error);
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    dispatch_opfs_quota_mutation_task(
        scope,
        resolver,
        state.locator,
        service,
        quota,
        quota_owner,
        handle_access,
        move |opfs, max_usage| match kind {
            EntryKind::Directory => {
                opfs.get_directory_with_quota(&key, &parent.current(), &name, create, max_usage)
            }
            EntryKind::File => {
                opfs.get_file_with_quota(&key, &parent.current(), &name, create, max_usage)
            }
        },
        move |result| OpfsTaskResult::GetChild { kind, result },
    );
}

fn file_system_directory_remove_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) = directory_state_or_reject(scope, args.this(), resolver) else {
        return;
    };
    let Some(name) = required_name(scope, &args, resolver) else {
        return;
    };
    let Some(recursive) = boolean_option(scope, &args, 1, "recursive", resolver) else {
        return;
    };
    let (service, key, parent, handle_access) = match service_for_handle(scope, args.this(), &state)
    {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let mutation_lease_service = service.clone();
    dispatch_opfs_task(
        scope,
        resolver,
        state.locator,
        service,
        handle_access,
        move |opfs| {
            let child = parent.current().child(&name)?;
            opfs.remove_entry_with_mutation_lease(&key, &child, recursive)
                .map(|lease| Some(StorageOpfsMutationLease::new(mutation_lease_service, lease)))
        },
        OpfsTaskResult::Remove,
    );
}

fn file_system_directory_resolve_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(base_state) = directory_state_or_reject(scope, args.this(), resolver) else {
        return;
    };
    if args.length() < 1 {
        reject_type_error(
            scope,
            resolver,
            "FileSystemDirectoryHandle.resolve requires a handle.",
        );
        return;
    }
    let Ok(target_object) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        reject_type_error(scope, resolver, "The argument is not a FileSystemHandle.");
        return;
    };
    let Some(target_state) = handle_state(scope, target_object) else {
        reject_type_error(scope, resolver, "The argument is not a FileSystemHandle.");
        return;
    };
    if base_state.locator != target_state.locator {
        let _ = resolver.resolve(scope, v8::null(scope).into());
        return;
    }
    let (service, key, base, handle_access) =
        match service_for_handle(scope, args.this(), &base_state) {
            Ok(values) => values,
            Err(error) => {
                reject_handle_access_error(scope, resolver, error);
                return;
            }
        };
    let Some(target) = handle_path_state(scope, target_object) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The target file system handle is no longer active.",
        );
        return;
    };
    dispatch_opfs_task(
        scope,
        resolver,
        base_state.locator,
        service,
        handle_access,
        move |opfs| opfs.resolve(&key, &base.current(), &target.current()),
        OpfsTaskResult::Resolve,
    );
}

fn file_system_directory_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    build_directory_iterator(scope, args.this(), IteratorMode::Entries, &mut rv);
}

fn file_system_directory_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    build_directory_iterator(scope, args.this(), IteratorMode::Keys, &mut rv);
}

fn file_system_directory_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    build_directory_iterator(scope, args.this(), IteratorMode::Values, &mut rv);
}

fn build_directory_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    mode: IteratorMode,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) =
        handle_state(scope, receiver).filter(|state| state.kind == EntryKind::Directory)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(state_json) = serde_json::to_string(&state).ok() else {
        throw_type_error(scope, "Failed to serialize directory iterator state");
        return;
    };
    let handle_access = handle_id(scope, receiver)
        .and_then(|handle_id| current_opfs_handle_registry(scope)?.handle_access(handle_id));
    let Some(registry) = ensure_current_opfs_directory_iterator_registry(scope) else {
        throw_type_error(scope, "Directory iterator owner is unavailable");
        return;
    };
    let iterator_id = registry.insert(OpfsDirectoryIteratorDescriptor {
        state_json,
        mode: mode.as_str().to_owned(),
        handle_access,
    });
    match FileSystemDirectoryIteratorObjectDeclaration::new(iterator_id as f64).bind(scope) {
        Ok(iterator) => {
            let Some(prototype) = file_system_directory_iterator_prototype(scope) else {
                registry.remove(iterator_id);
                throw_type_error(scope, "Failed to create directory iterator prototype");
                return;
            };
            if iterator.set_prototype(scope, prototype.into()) != Some(true) {
                registry.remove(iterator_id);
                throw_type_error(scope, "Failed to bind directory iterator prototype");
                return;
            }
            let cleanup_registry = registry.clone();
            crate::v8_finalizer::track_context_owned_v8_finalizer(scope, iterator, move || {
                cleanup_registry.remove(iterator_id);
            });
            rv.set(iterator.into());
        }
        Err(_) => {
            registry.remove(iterator_id);
            throw_type_error(scope, "Failed to create directory iterator");
        }
    }
}

fn file_system_directory_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) = get_private_value(scope, global, FILE_SYSTEM_ITERATOR_PROTOTYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }
    let template = FileSystemDirectoryIteratorPrototypeDeclaration::build(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(
        scope,
        global,
        FILE_SYSTEM_ITERATOR_PROTOTYPE_SLOT,
        prototype.into(),
    );
    Some(prototype)
}

fn file_system_directory_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    if !get_private_value(scope, args.this(), FILE_SYSTEM_ITERATOR_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    }
    let Some(iterator_id) = directory_iterator_id(scope, args.this()) else {
        reject_type_error(scope, resolver, "Directory iterator state is unavailable.");
        return;
    };
    let Some(registry) = current_opfs_directory_iterator_registry(scope) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Directory iterator owner is unavailable.",
        );
        return;
    };
    let Some(descriptor) = registry.descriptor(iterator_id) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Directory iterator is no longer active.",
        );
        return;
    };
    let Ok(state) = serde_json::from_str::<FileSystemHandleState>(&descriptor.state_json) else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Directory iterator state is invalid.",
        );
        return;
    };
    let (service, key, directory) = match service_for_state_with_handle_access(
        scope,
        &state,
        descriptor.handle_access.as_ref(),
    ) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let action = registry.enqueue_next(iterator_id, v8::Global::new(scope, resolver));
    match action {
        Some(OpfsDirectoryIteratorNextAction::Cached { descriptor, entry }) => {
            settle_directory_iterator_next(scope, resolver, descriptor, entry);
            return;
        }
        Some(OpfsDirectoryIteratorNextAction::Queued) => return,
        Some(OpfsDirectoryIteratorNextAction::StartLoad) => {}
        None => {
            reject_dom_exception(
                scope,
                resolver,
                "InvalidStateError",
                "Directory iterator is no longer active.",
            );
            return;
        }
    }

    let locator = state.locator;
    let Some(registered) = register_opfs_iterator_task(
        scope,
        args.this(),
        locator.clone(),
        registry.clone(),
        iterator_id,
        descriptor.handle_access.clone(),
    ) else {
        let result = service.with_opfs(|opfs| opfs.read_directory(&key, &directory));
        settle_opfs_directory_iterator_task_result(
            scope,
            &registry,
            iterator_id,
            &locator,
            descriptor.handle_access.as_ref(),
            OpfsTaskResult::ReadDirectory(Ok(result)),
        );
        return;
    };

    let (cancellation, completion_sink) = registered.into_parts();
    let dispatch = service.dispatch_opfs(
        move |opfs| opfs.read_directory(&key, &directory),
        move |result| {
            completion_sink.send(OpfsTaskResult::ReadDirectory(result));
        },
    );
    if dispatch.is_err() {
        cancel_registered_opfs_task(scope, cancellation);
        let exception = crate::context_bootstrap::new_dom_exception_value(
            scope,
            "Failed to dispatch the directory read to the storage owner.",
            "UnknownError",
        );
        reject_directory_iterator_load(scope, &registry, iterator_id, exception);
    }
}

fn settle_directory_iterator_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    descriptor: OpfsDirectoryIteratorDescriptor,
    entry: Option<DirectoryEntry>,
) {
    let Some(entry) = entry else {
        let result = ObjectLiteralDeclaration::bind(scope);
        result.set_string_property(scope, "value", v8::undefined(scope).into());
        result.set_string_property(scope, "done", v8::Boolean::new(scope, true).into());
        let _ = resolver.resolve(scope, result.into_value());
        return;
    };
    let Ok(state) = serde_json::from_str::<FileSystemHandleState>(&descriptor.state_json) else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Directory iterator state is invalid.",
        );
        return;
    };
    let Some(mode) = IteratorMode::parse(&descriptor.mode) else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Directory iterator mode is invalid.",
        );
        return;
    };
    let directory = match state.opfs_path() {
        Ok(path) => path,
        Err(error) => {
            reject_opfs_error(scope, resolver, error);
            return;
        }
    };
    let child_path = match directory.child(&entry.name) {
        Ok(path) => path,
        Err(error) => {
            reject_opfs_error(scope, resolver, error);
            return;
        }
    };
    let Some(handle) = build_handle_object(
        scope,
        &state.locator,
        &child_path,
        entry.kind,
        descriptor.handle_access,
    ) else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to create an iterated file system handle.",
        );
        return;
    };
    let value: v8::Local<'s, v8::Value> = match mode {
        IteratorMode::Entries => {
            let pair = v8::Array::new(scope, 2);
            if let Some(name) = v8_string(scope, &entry.name) {
                let _ = pair.set_index(scope, 0, name.into());
            }
            let _ = pair.set_index(scope, 1, handle.into());
            pair.into()
        }
        IteratorMode::Keys => v8_string(scope, &entry.name)
            .map(Into::into)
            .unwrap_or_else(|| v8::undefined(scope).into()),
        IteratorMode::Values => handle.into(),
    };
    let result = ObjectLiteralDeclaration::bind(scope);
    result.set_string_property(scope, "value", value);
    result.set_string_property(scope, "done", v8::Boolean::new(scope, false).into());
    let _ = resolver.resolve(scope, result.into_value());
}

fn directory_iterator_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let value =
        get_private_value(scope, iterator, FILE_SYSTEM_ITERATOR_ID_SLOT)?.number_value(scope)?;
    (value.is_finite() && value.fract() == 0.0 && value >= 1.0 && value <= u32::MAX as f64)
        .then_some(value as u32)
}

fn file_system_file_create_writable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) = file_state_or_reject(scope, args.this(), resolver) else {
        return;
    };
    let options_object = match webidl::dictionary_arg(
        &args,
        0,
        webidl::Context::argument("FileSystemFileHandle.createWritable", 1),
    ) {
        Ok(value) => value,
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    let options = match options_object {
        Some(options) => {
            match webidl::parse_dictionary_object::<FileSystemCreateWritableOptions>(scope, options)
            {
                Ok(options) => options,
                Err(error) if error.is_pending_exception() => return,
                Err(error) => {
                    reject_type_error(scope, resolver, &error.to_string());
                    return;
                }
            }
        }
        None => FileSystemCreateWritableOptions {
            keep_existing_data: false,
            mode: FileSystemWritableMode::Siloed,
        },
    };
    let (service, key, path, handle_access) = match service_for_handle(scope, args.this(), &state) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let mode = WritableMode::from(options.mode);
    let lease_service = service.clone();
    dispatch_opfs_task(
        scope,
        resolver,
        state.locator,
        service,
        handle_access,
        move |opfs| {
            let writer_id =
                opfs.create_writable(&key, &path.current(), options.keep_existing_data, mode)?;
            Ok(StorageOpfsWritableLease::new(lease_service, writer_id))
        },
        move |result| OpfsTaskResult::CreateWritable { mode, result },
    );
}

fn file_system_file_create_sync_access_handle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) = file_state_or_reject(scope, args.this(), resolver) else {
        return;
    };
    let options_object = match webidl::dictionary_arg(
        &args,
        0,
        webidl::Context::argument("FileSystemFileHandle.createSyncAccessHandle", 1),
    ) {
        Ok(value) => value,
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    let options = match options_object {
        Some(options) => match webidl::parse_dictionary_object::<
            FileSystemCreateSyncAccessHandleOptions,
        >(scope, options)
        {
            Ok(options) => options,
            Err(error) if error.is_pending_exception() => return,
            Err(error) => {
                reject_type_error(scope, resolver, &error.to_string());
                return;
            }
        },
        None => FileSystemCreateSyncAccessHandleOptions::default(),
    };
    let (service, key, path, handle_access) = match service_for_handle(scope, args.this(), &state) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    let mode = SyncAccessMode::from(options.mode);
    let web_mode = options.mode.as_str().to_owned();
    let lease_service = service.clone();
    dispatch_opfs_task(
        scope,
        resolver,
        state.locator,
        service,
        handle_access,
        move |opfs| {
            let handle_id = opfs.create_sync_access_handle(&key, &path.current(), mode)?;
            Ok(StorageOpfsSyncAccessLease::new(lease_service, handle_id))
        },
        move |result| OpfsTaskResult::CreateSyncAccessHandle {
            mode: web_mode,
            result,
        },
    );
}

fn build_sync_access_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    locator: StorageBucketLocator,
    lease: StorageOpfsSyncAccessLease,
    mode: String,
    handle_access: OpfsHandleAccess,
) -> Option<v8::Local<'s, v8::Object>> {
    let handle_id = lease.handle_id();
    let registry = ensure_current_opfs_handle_registry(scope)?;
    let access_id = handle_access.map(|access| registry.insert_derived_access(access));
    let state_json = match serde_json::to_string(&FileSystemSyncAccessHandleState {
        locator,
        handle_id: handle_id.get(),
        mode,
        access_id,
    }) {
        Ok(state_json) => state_json,
        Err(_) => {
            if let Some(access_id) = access_id {
                registry.remove_derived_access(access_id);
            }
            return None;
        }
    };
    let handle = match FileSystemSyncAccessHandleObjectDeclaration::new(state_json).bind(scope) {
        Ok(handle) => handle,
        Err(_) => {
            if let Some(access_id) = access_id {
                registry.remove_derived_access(access_id);
            }
            return None;
        }
    };
    let cleanup_registry = registry.clone();
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, handle, move || {
        if let Some(access_id) = access_id {
            cleanup_registry.remove_derived_access(access_id);
        }
        drop(lease);
    });
    Some(handle)
}

fn sync_access_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<FileSystemSyncAccessHandleState> {
    if !get_private_value(scope, object, FILE_SYSTEM_SYNC_ACCESS_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return None;
    }
    let json = get_private_value(scope, object, FILE_SYSTEM_SYNC_ACCESS_STATE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&json).ok()
}

fn sync_access_is_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, object, FILE_SYSTEM_SYNC_ACCESS_CLOSED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn open_sync_access_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(FileSystemSyncAccessHandleState, OpfsSyncAccessHandleId)> {
    let Some(state) = sync_access_state(scope, object) else {
        throw_type_error(scope, "Illegal invocation");
        return None;
    };
    if sync_access_is_closed(scope, object) {
        throw_sync_access_closed(scope);
        return None;
    }
    let Some(handle_id) = state.handle_id() else {
        throw_sync_access_closed(scope);
        return None;
    };
    Some((state, handle_id))
}

fn file_system_sync_access_mode_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = sync_access_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if let Some(mode) = v8_string(scope, &state.mode) {
        rv.set(mode.into());
    }
}

fn file_system_sync_access_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = sync_access_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if sync_access_is_closed(scope, args.this()) {
        rv.set_undefined();
        return;
    }
    set_private_value(
        scope,
        args.this(),
        FILE_SYSTEM_SYNC_ACCESS_CLOSED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if let Some(handle_id) = state.handle_id() {
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok((service, _, quota, _)) => {
                let quota_owner = match opfs_quota_owner(scope, &state.locator) {
                    Ok(owner) => owner,
                    Err(error) => {
                        throw_handle_access_error(scope, error);
                        return;
                    }
                };
                if let Err(error) = with_opfs_quota_mutation(
                    &service,
                    &state.locator,
                    quota,
                    quota_owner,
                    move |opfs, max_usage| opfs.close_sync(handle_id, Some(max_usage)),
                ) {
                    throw_opfs_error(scope, error);
                    return;
                }
            }
            Err(_) => {
                if let Some(service) =
                    with_storage_bucket_store_entry(scope, |store| store.storage_service())
                {
                    let _ = service.with_opfs(|opfs| opfs.close_sync(handle_id, None));
                }
            }
        }
    }
    rv.set_undefined();
}

fn file_system_sync_access_flush_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, handle_id)) = open_sync_access_state(scope, args.this()) else {
        return;
    };
    let (service, _, quota, _) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                throw_handle_access_error(scope, error);
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(owner) => owner,
        Err(error) => {
            throw_handle_access_error(scope, error);
            return;
        }
    };
    match with_opfs_quota_mutation(
        &service,
        &state.locator,
        quota,
        quota_owner,
        move |opfs, max_usage| opfs.flush_sync(handle_id, Some(max_usage)),
    ) {
        Ok(()) => rv.set_undefined(),
        Err(error) => throw_opfs_error(scope, error),
    }
}

fn file_system_sync_access_get_size_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, handle_id)) = open_sync_access_state(scope, args.this()) else {
        return;
    };
    let (service, _, _, _) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                throw_handle_access_error(scope, error);
                return;
            }
        };
    match service.with_opfs(|opfs| opfs.sync_size(handle_id)) {
        Ok(size) => rv.set(v8::Number::new(scope, size as f64).into()),
        Err(error) => throw_opfs_error(scope, error),
    }
}

fn file_system_sync_access_truncate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, handle_id)) = open_sync_access_state(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FileSystemSyncAccessHandleTruncateArgs>(scope, &args)
    else {
        return;
    };
    if parsed.size > MAX_SYNC_ACCESS_FILE_OFFSET {
        throw_type_error(scope, "Cannot truncate file to given length");
        return;
    }
    let (service, _, quota, _) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                throw_handle_access_error(scope, error);
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(owner) => owner,
        Err(error) => {
            throw_handle_access_error(scope, error);
            return;
        }
    };
    match with_opfs_quota_mutation(
        &service,
        &state.locator,
        quota,
        quota_owner,
        move |opfs, max_usage| opfs.sync_truncate(handle_id, parsed.size, Some(max_usage)),
    ) {
        Ok(()) => rv.set_undefined(),
        Err(error) => throw_opfs_error(scope, error),
    }
}

fn file_system_sync_access_read_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, handle_id)) = open_sync_access_state(scope, args.this()) else {
        return;
    };
    let buffer = args.get(0);
    let Some(length) = sync_buffer_source_length(scope, &args, buffer) else {
        return;
    };
    let Some(options) = sync_read_write_options(scope, &args) else {
        return;
    };
    if options
        .at
        .is_some_and(|offset| offset > MAX_SYNC_ACCESS_FILE_OFFSET)
    {
        throw_type_error(scope, "Cannot read at given offset");
        return;
    }
    let (service, _, _, _) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                throw_handle_access_error(scope, error);
                return;
            }
        };
    let bytes = match service.with_opfs(|opfs| opfs.sync_read(handle_id, length, options.at)) {
        Ok(bytes) => bytes,
        Err(error) => {
            throw_opfs_error(scope, error);
            return;
        }
    };
    if !copy_sync_bytes_to_buffer_source(scope, buffer, &bytes) {
        return;
    }
    rv.set(v8::Number::new(scope, bytes.len() as f64).into());
}

fn file_system_sync_access_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, handle_id)) = open_sync_access_state(scope, args.this()) else {
        return;
    };
    if args.length() < 1 {
        throw_type_error(
            scope,
            "FileSystemSyncAccessHandle.write requires a BufferSource.",
        );
        return;
    }
    let buffer = args.get(0);
    let Some(bytes) = blob::buffer_source_bytes_from_value(scope, buffer) else {
        throw_type_error(
            scope,
            "FileSystemSyncAccessHandle.write requires a BufferSource.",
        );
        return;
    };
    if blob::buffer_source_has_shared_or_resizable_backing_store(buffer)
        && buffer_source_is_resizable(buffer)
    {
        throw_type_error(
            scope,
            "FileSystemSyncAccessHandle.write does not accept a resizable BufferSource.",
        );
        return;
    }
    if bytes.len() > MAX_SYNC_ACCESS_WRITE_SIZE {
        throw_type_error(scope, "Cannot write more than 2GB");
        return;
    }
    let Some(options) = sync_read_write_options(scope, &args) else {
        return;
    };
    if options
        .at
        .is_some_and(|offset| offset > MAX_SYNC_ACCESS_FILE_OFFSET)
    {
        throw_type_error(scope, "Cannot write at given offset");
        return;
    }
    if options.at.is_some_and(|offset| {
        offset
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .is_none_or(|end| end > MAX_SYNC_ACCESS_FILE_OFFSET)
    }) {
        let error = crate::context_bootstrap::new_quota_exceeded_error_value(
            scope,
            "No capacity available for this operation",
            None,
            None,
        );
        scope.throw_exception(error);
        return;
    }
    let (service, _, quota, _) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                throw_handle_access_error(scope, error);
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(owner) => owner,
        Err(error) => {
            throw_handle_access_error(scope, error);
            return;
        }
    };
    match with_opfs_quota_mutation(
        &service,
        &state.locator,
        quota,
        quota_owner,
        move |opfs, max_usage| opfs.sync_write(handle_id, &bytes, options.at, Some(max_usage)),
    ) {
        Ok(written) => rv.set(v8::Number::new(scope, written as f64).into()),
        Err(error) => throw_opfs_error(scope, error),
    }
}

fn sync_buffer_source_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Option<usize> {
    if args.length() < 1 {
        throw_type_error(
            scope,
            "FileSystemSyncAccessHandle.read requires a BufferSource.",
        );
        return None;
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        if buffer.was_detached() || buffer.get_backing_store().is_resizable_by_user_javascript() {
            throw_type_error(
                scope,
                "FileSystemSyncAccessHandle.read requires a fixed, attached BufferSource.",
            );
            return None;
        }
        return Some(buffer.byte_length());
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let Some(backing_store) = view.get_backing_store() else {
            throw_type_error(
                scope,
                "FileSystemSyncAccessHandle.read received a detached view.",
            );
            return None;
        };
        if backing_store.is_resizable_by_user_javascript() {
            throw_type_error(
                scope,
                "FileSystemSyncAccessHandle.read does not accept a resizable BufferSource.",
            );
            return None;
        }
        return Some(view.byte_length());
    }
    throw_type_error(
        scope,
        "FileSystemSyncAccessHandle.read requires a BufferSource.",
    );
    None
}

fn buffer_source_is_resizable(value: v8::Local<'_, v8::Value>) -> bool {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        return buffer.get_backing_store().is_resizable_by_user_javascript();
    }
    v8::Local::<v8::ArrayBufferView>::try_from(value)
        .ok()
        .and_then(|view| view.get_backing_store())
        .is_some_and(|backing_store| backing_store.is_resizable_by_user_javascript())
}

fn copy_sync_bytes_to_buffer_source(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    bytes: &[u8],
) -> bool {
    if bytes.is_empty() {
        return true;
    }
    let target = if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        buffer.get_backing_store().data().map(|data| data.as_ptr())
    } else if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let data = view.data();
        (!data.is_null()).then_some(data)
    } else {
        None
    };
    let Some(target) = target else {
        throw_type_error(
            scope,
            "FileSystemSyncAccessHandle.read received a detached BufferSource.",
        );
        return false;
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
    }
    true
}

fn sync_read_write_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<FileSystemReadWriteOptions> {
    let object = match webidl::dictionary_arg(
        args,
        1,
        webidl::Context::argument("FileSystemSyncAccessHandle.read/write", 2),
    ) {
        Ok(value) => value,
        Err(error) => {
            throw_type_error(scope, &error.to_string());
            return None;
        }
    };
    match object {
        Some(object) => {
            match webidl::parse_dictionary_object::<FileSystemReadWriteOptions>(scope, object) {
                Ok(options) => Some(options),
                Err(error) if error.is_pending_exception() => None,
                Err(error) => {
                    throw_type_error(scope, &error.to_string());
                    None
                }
            }
        }
        None => Some(FileSystemReadWriteOptions::default()),
    }
}

fn throw_sync_access_closed(scope: &mut v8::PinScope<'_, '_>) {
    let error = crate::context_bootstrap::new_dom_exception_value(
        scope,
        "The access handle was already closed.",
        "InvalidStateError",
    );
    scope.throw_exception(error);
}

fn throw_handle_access_error(scope: &mut v8::PinScope<'_, '_>, error: HandleAccessError) {
    let error = handle_access_error_value(scope, error);
    scope.throw_exception(error);
}

fn throw_opfs_error(scope: &mut v8::PinScope<'_, '_>, error: OpfsError) {
    let error = opfs_error_value(scope, error);
    scope.throw_exception(error);
}

fn build_writable_file_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    locator: StorageBucketLocator,
    lease: StorageOpfsWritableLease,
    mode: FileSystemWritableMode,
    handle_access: OpfsHandleAccess,
) -> Option<v8::Local<'s, v8::Object>> {
    let writer_id = lease.writer_id();
    let registry = ensure_current_opfs_handle_registry(scope)?;
    let access_id = handle_access.map(|access| registry.insert_derived_access(access));
    let state_json = match serde_json::to_string(&FileSystemWritableSinkState {
        locator,
        writer_id: writer_id.get(),
        access_id,
    }) {
        Ok(state_json) => state_json,
        Err(_) => {
            if let Some(access_id) = access_id {
                registry.remove_derived_access(access_id);
            }
            return None;
        }
    };
    let sink = match FileSystemWritableSinkObjectDeclaration::new(state_json).bind(scope) {
        Ok(sink) => sink,
        Err(_) => {
            if let Some(access_id) = access_id {
                registry.remove_derived_access(access_id);
            }
            return None;
        }
    };
    let stream = match FileSystemWritableFileStreamObjectDeclaration::new(mode.as_str().to_owned())
        .bind(scope)
    {
        Ok(stream) => stream,
        Err(_) => {
            if let Some(access_id) = access_id {
                registry.remove_derived_access(access_id);
            }
            return None;
        }
    };
    initialize_writable_stream_object(scope, stream, Some(sink), 1.0, None);
    let cleanup_registry = registry.clone();
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, stream, move || {
        if let Some(access_id) = access_id {
            cleanup_registry.remove_derived_access(access_id);
        }
        drop(lease);
    });
    Some(stream)
}

fn writable_stream_is_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, stream, FILE_SYSTEM_WRITABLE_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn file_system_writable_mode_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_is_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if let Some(mode) = get_private_value(scope, args.this(), FILE_SYSTEM_WRITABLE_MODE_SLOT) {
        rv.set(mode);
    }
}

fn file_system_writable_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    enqueue_file_system_writable_chunk(scope, args.this(), args.get(0), &mut rv);
}

fn file_system_writable_seek_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let command = v8::Object::new(scope);
    let _ = command.set(
        scope,
        v8str(scope, "type").into(),
        v8str(scope, "seek").into(),
    );
    let _ = command.set(scope, v8str(scope, "position").into(), args.get(0));
    enqueue_file_system_writable_chunk(scope, args.this(), command.into(), &mut rv);
}

fn file_system_writable_truncate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let command = v8::Object::new(scope);
    let _ = command.set(
        scope,
        v8str(scope, "type").into(),
        v8str(scope, "truncate").into(),
    );
    let _ = command.set(scope, v8str(scope, "size").into(), args.get(0));
    enqueue_file_system_writable_chunk(scope, args.this(), command.into(), &mut rv);
}

fn enqueue_file_system_writable_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_is_branded(scope, stream) {
        set_rejected_type_error(scope, rv, "Illegal invocation");
        return;
    }
    if writable_stream_locked(scope, stream) {
        set_rejected_type_error(scope, rv, "WritableStream is locked");
        return;
    }
    if let Some(result) = writable_stream_write_internal(scope, stream, chunk) {
        rv.set(result);
    } else {
        set_resolved_promise(scope, rv, v8::undefined(scope).into());
    }
}

fn set_rejected_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    message: &str,
) {
    let exception = v8::String::new(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    if let Some(promise) = rejected_promise_value(scope, exception) {
        rv.set(promise);
    } else {
        rv.set_undefined();
    }
}

fn writable_sink_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
) -> Option<FileSystemWritableSinkState> {
    if !get_private_value(scope, sink, FILE_SYSTEM_WRITABLE_SINK_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return None;
    }
    let json = get_private_value(scope, sink, FILE_SYSTEM_WRITABLE_SINK_STATE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&json).ok()
}

enum WritableSinkError {
    Type(String),
    Dom { name: &'static str, message: String },
    WebIdl(webidl::WebIdlError),
    Access(HandleAccessError),
    Backend(OpfsError),
}

struct WritableSinkCommand {
    command: WritableCommand,
    snapshot_validation: Option<WritableSnapshotValidation>,
}

struct WritableSnapshotValidation {
    service: SharedStorageService,
    key: OpfsBucketKey,
    path: OpfsPath,
    identity: FileSnapshotIdentity,
}

struct WritableData {
    bytes: Vec<u8>,
    snapshot_validation: Option<WritableSnapshotValidation>,
}

fn file_system_writable_sink_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = writable_sink_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(writer_id) = state.writer_id() else {
        throw_type_error(scope, "The writable stream is in an invalid state.");
        return;
    };
    let command = match writable_command_from_chunk(scope, args.get(0)) {
        Ok(command) => command,
        Err(error) => {
            abort_and_throw_writable_error(scope, writer_id, error);
            return;
        }
    };
    let (service, _, quota, handle_access) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                abort_and_throw_writable_error(scope, writer_id, WritableSinkError::Access(error));
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(owner) => owner,
        Err(error) => {
            abort_and_throw_writable_error(scope, writer_id, WritableSinkError::Access(error));
            return;
        }
    };
    if command
        .snapshot_validation
        .as_ref()
        .is_some_and(|validation| !std::sync::Arc::ptr_eq(&service, &validation.service))
    {
        abort_and_throw_writable_error(
            scope,
            writer_id,
            WritableSinkError::Access(HandleAccessError::Security),
        );
        return;
    }
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        abort_writable_from_context(scope, writer_id);
        return;
    };
    let WritableSinkCommand {
        command,
        snapshot_validation,
    } = command;
    let snapshot_validation = snapshot_validation
        .map(|validation| (validation.key, validation.path, validation.identity));
    let cleanup = StorageOpfsWritableLease::new(service.clone(), writer_id);
    dispatch_opfs_quota_mutation_task(
        scope,
        resolver,
        state.locator,
        service,
        quota,
        quota_owner,
        handle_access,
        move |opfs, max_usage| {
            let result = (|| {
                if let Some((key, path, identity)) = snapshot_validation {
                    opfs.validate_file_snapshot(&key, &path, identity)?;
                }
                opfs.writable_command(writer_id, command, Some(max_usage))
            })();
            if result.is_err() {
                let _ = opfs.abort_writable(writer_id);
            }
            result
        },
        move |result| OpfsTaskResult::WritableCommand { result, cleanup },
    );
}

fn file_system_writable_sink_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = writable_sink_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(writer_id) = state.writer_id() else {
        throw_type_error(scope, "The writable stream is in an invalid state.");
        return;
    };
    let (service, _, quota, handle_access) =
        match service_for_derived_handle_access(scope, &state.locator, state.access_id) {
            Ok(values) => values,
            Err(error) => {
                abort_and_throw_writable_error(scope, writer_id, WritableSinkError::Access(error));
                return;
            }
        };
    let quota_owner = match opfs_quota_owner(scope, &state.locator) {
        Ok(owner) => owner,
        Err(error) => {
            abort_and_throw_writable_error(scope, writer_id, WritableSinkError::Access(error));
            return;
        }
    };
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        abort_writable_from_context(scope, writer_id);
        return;
    };
    let cleanup = StorageOpfsWritableLease::new(service.clone(), writer_id);
    dispatch_opfs_quota_mutation_task(
        scope,
        resolver,
        state.locator,
        service,
        quota,
        quota_owner,
        handle_access,
        move |opfs, max_usage| {
            let result = opfs.close_writable(writer_id, Some(max_usage));
            if result.is_err() {
                let _ = opfs.abort_writable(writer_id);
            }
            result
        },
        move |result| OpfsTaskResult::WritableCommand { result, cleanup },
    );
}

fn file_system_writable_sink_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = writable_sink_state(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(writer_id) = state.writer_id() else {
        throw_type_error(scope, "The writable stream is in an invalid state.");
        return;
    };
    abort_writable_from_context(scope, writer_id);
    rv.set_undefined();
}

fn writable_command_from_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<WritableSinkCommand, WritableSinkError> {
    if chunk.is_null() {
        return Err(WritableSinkError::Type(
            "FileSystemWritableFileStream.write requires non-null data.".to_owned(),
        ));
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(chunk) {
        if blob::is_blob_object(scope, object) {
            let data = writable_data_bytes(scope, chunk)?;
            return Ok(WritableSinkCommand {
                command: WritableCommand::Write {
                    data: data.bytes,
                    position: None,
                },
                snapshot_validation: data.snapshot_validation,
            });
        }
        if let Some(bytes) = blob::buffer_source_bytes_from_value(scope, chunk) {
            return Ok(WritableSinkCommand {
                command: WritableCommand::Write {
                    data: bytes,
                    position: None,
                },
                snapshot_validation: None,
            });
        }
        let params = webidl::parse_dictionary_object::<FileSystemWriteParams<'_>>(scope, object)
            .map_err(WritableSinkError::WebIdl)?;
        return writable_command_from_params(scope, params);
    }
    let data = writable_data_bytes(scope, chunk)?;
    Ok(WritableSinkCommand {
        command: WritableCommand::Write {
            data: data.bytes,
            position: None,
        },
        snapshot_validation: data.snapshot_validation,
    })
}

fn writable_command_from_params<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    params: FileSystemWriteParams<'s>,
) -> Result<WritableSinkCommand, WritableSinkError> {
    match params.command_type {
        FileSystemWriteCommandType::Truncate => params
            .size
            .map(|size| WritableSinkCommand {
                command: WritableCommand::Truncate(size),
                snapshot_validation: None,
            })
            .ok_or_else(|| WritableSinkError::Dom {
                name: "SyntaxError",
                message: "Invalid params passed. truncate requires a size argument.".to_owned(),
            }),
        FileSystemWriteCommandType::Seek => params
            .position
            .map(|position| WritableSinkCommand {
                command: WritableCommand::Seek(position),
                snapshot_validation: None,
            })
            .ok_or_else(|| WritableSinkError::Dom {
                name: "SyntaxError",
                message: "Invalid params passed. seek requires a position argument.".to_owned(),
            }),
        FileSystemWriteCommandType::Write => {
            let Some(data) = params.data else {
                return Err(WritableSinkError::Dom {
                    name: "SyntaxError",
                    message: "Invalid params passed. write requires a data argument.".to_owned(),
                });
            };
            if data.is_null() {
                return Err(WritableSinkError::Type(
                    "Invalid params passed. write requires non-null data.".to_owned(),
                ));
            }
            let data = writable_data_bytes(scope, data)?;
            Ok(WritableSinkCommand {
                command: WritableCommand::Write {
                    data: data.bytes,
                    position: params.position,
                },
                snapshot_validation: data.snapshot_validation,
            })
        }
    }
}

fn writable_data_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<WritableData, WritableSinkError> {
    if value.is_null() {
        return Err(WritableSinkError::Type(
            "Writable data must not be null.".to_owned(),
        ));
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && blob::is_blob_object(scope, object)
    {
        let snapshot_validation = opfs_file_snapshot_validation(scope, object)?;
        let bytes =
            blob::blob_bytes_from_object(scope, object).ok_or_else(|| WritableSinkError::Dom {
                name: "NotFoundError",
                message: "The Blob backing data is no longer available.".to_owned(),
            })?;
        return Ok(WritableData {
            bytes,
            snapshot_validation,
        });
    }
    if let Some(bytes) = blob::buffer_source_bytes_from_value(scope, value) {
        return Ok(WritableData {
            bytes,
            snapshot_validation: None,
        });
    }
    webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::member("WriteParams", "data"),
    )
    .map(|value| WritableData {
        bytes: value.0.into_bytes(),
        snapshot_validation: None,
    })
    .map_err(WritableSinkError::WebIdl)
}

fn opfs_file_snapshot_validation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<WritableSnapshotValidation>, WritableSinkError> {
    let Some(value) = get_private_value(scope, object, FILE_SYSTEM_FILE_SNAPSHOT_STATE_SLOT) else {
        return Ok(None);
    };
    let json = value
        .to_string(scope)
        .ok_or_else(|| WritableSinkError::Type("Invalid OPFS File snapshot state.".to_owned()))?
        .to_rust_string_lossy(scope);
    let state = serde_json::from_str::<FileSystemFileSnapshotState>(&json)
        .map_err(|_| WritableSinkError::Type("Invalid OPFS File snapshot state.".to_owned()))?;
    let identity = state
        .identity()
        .ok_or_else(|| WritableSinkError::Type("Invalid OPFS File snapshot state.".to_owned()))?;
    let (service, key, path) =
        service_for_file_snapshot_state(scope, &state).map_err(WritableSinkError::Access)?;
    Ok(Some(WritableSnapshotValidation {
        service,
        key,
        path,
        identity,
    }))
}

fn abort_and_throw_writable_error(
    scope: &mut v8::PinScope<'_, '_>,
    writer_id: OpfsWritableId,
    error: WritableSinkError,
) {
    abort_writable_from_context(scope, writer_id);
    throw_writable_error(scope, error);
}

fn abort_writable_from_context(scope: &mut v8::PinScope<'_, '_>, writer_id: OpfsWritableId) {
    let service = with_storage_bucket_store_entry(scope, |store| store.storage_service());
    if let Some(service) = service {
        drop(StorageOpfsWritableLease::new(service, writer_id));
    }
}

fn throw_writable_error(scope: &mut v8::PinScope<'_, '_>, error: WritableSinkError) {
    if let Some(exception) = writable_error_value(scope, error) {
        scope.throw_exception(exception);
    }
}

fn writable_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: WritableSinkError,
) -> Option<v8::Local<'s, v8::Value>> {
    match error {
        WritableSinkError::Type(message) => v8::String::new(scope, &message)
            .map(|message| v8::Exception::type_error(scope, message)),
        WritableSinkError::Dom { name, message } => Some(
            crate::context_bootstrap::new_dom_exception_value(scope, &message, name),
        ),
        WritableSinkError::WebIdl(error) if error.is_pending_exception() => None,
        WritableSinkError::WebIdl(error) => v8::String::new(scope, &error.to_string())
            .map(|message| v8::Exception::type_error(scope, message)),
        WritableSinkError::Access(error) => Some(handle_access_error_value(scope, error)),
        WritableSinkError::Backend(OpfsError::InvalidState) => {
            v8::String::new(scope, "The writable stream is closed or errored.")
                .map(|message| v8::Exception::type_error(scope, message))
        }
        WritableSinkError::Backend(error) => Some(opfs_error_value(scope, error)),
    }
}

fn file_system_file_get_file_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = handle_promise_resolver(scope, &args, &mut rv) else {
        return;
    };
    let Some(state) =
        handle_state(scope, args.this()).filter(|state| state.kind == EntryKind::File)
    else {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    };
    let (service, key, path, handle_access) = match service_for_handle(scope, args.this(), &state) {
        Ok(values) => values,
        Err(error) => {
            reject_handle_access_error(scope, resolver, error);
            return;
        }
    };
    dispatch_opfs_task(
        scope,
        resolver,
        state.locator,
        service,
        handle_access,
        move |opfs| {
            let path = path.current();
            let snapshot = opfs.read_file(&key, &path)?;
            Ok(OpfsReadFileResult { path, snapshot })
        },
        move |result| OpfsTaskResult::GetFile(OpfsGetFileTaskResult { result }),
    );
}

pub(crate) fn settle_opfs_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: &StorageBucketLocator,
    handle_access: Option<&OpfsHandleAccessContext>,
    result: OpfsTaskResult,
) {
    let access_check = match handle_access {
        Some(access) => service_for_locator_with_handle_access(scope, locator, access),
        None => service_for_locator(scope, locator),
    };
    if let Err(error) = access_check {
        reject_handle_access_error(scope, resolver, error);
        return;
    }
    match result {
        OpfsTaskResult::CreateSyncAccessHandle { mode, result } => {
            let Some(lease) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while creating the OPFS sync access session.",
            ) else {
                return;
            };
            let Some(handle) = build_sync_access_handle(
                scope,
                locator.clone(),
                lease,
                mode,
                handle_access.cloned(),
            ) else {
                reject_dom_exception(
                    scope,
                    resolver,
                    "UnknownError",
                    "Failed to create a FileSystemSyncAccessHandle wrapper.",
                );
                return;
            };
            let _ = resolver.resolve(scope, handle.into());
        }
        OpfsTaskResult::CreateWritable { mode, result } => {
            let Some(lease) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while creating the OPFS writer.",
            ) else {
                return;
            };
            let Some(stream) = build_writable_file_stream(
                scope,
                locator.clone(),
                lease,
                FileSystemWritableMode::from(mode),
                handle_access.cloned(),
            ) else {
                reject_dom_exception(
                    scope,
                    resolver,
                    "UnknownError",
                    "Failed to create a FileSystemWritableFileStream wrapper.",
                );
                return;
            };
            let _ = resolver.resolve(scope, stream.into());
        }
        OpfsTaskResult::GetRoot(result) => {
            let Some(path) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while opening the OPFS root.",
            ) else {
                return;
            };
            settle_handle_task_result(
                scope,
                resolver,
                locator,
                &path,
                EntryKind::Directory,
                handle_access.cloned(),
            );
        }
        OpfsTaskResult::GetChild { kind, result } => {
            let Some(path) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while opening the OPFS child entry.",
            ) else {
                return;
            };
            settle_handle_task_result(
                scope,
                resolver,
                locator,
                &path,
                kind,
                handle_access.cloned(),
            );
        }
        OpfsTaskResult::GetFile(result) => {
            settle_get_file_task_result(scope, resolver, locator, result)
        }
        OpfsTaskResult::IsSameEntry(result) => {
            let Some(is_same) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while comparing OPFS handles.",
            ) else {
                return;
            };
            let _ = resolver.resolve(scope, v8::Boolean::new(scope, is_same).into());
        }
        OpfsTaskResult::GetUniqueId(result) => {
            let Some(unique_id) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while getting the OPFS unique ID.",
            ) else {
                return;
            };
            let Some(unique_id) = v8_string(scope, &unique_id) else {
                reject_dom_exception(
                    scope,
                    resolver,
                    "UnknownError",
                    "Failed to create the OPFS unique ID string.",
                );
                return;
            };
            let _ = resolver.resolve(scope, unique_id.into());
        }
        OpfsTaskResult::Move(_) => reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "The storage owner returned a move result without its handle.",
        ),
        OpfsTaskResult::ReadDirectory(_) => reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "The storage owner returned a directory iterator result for a Promise task.",
        ),
        OpfsTaskResult::Remove(result) => {
            let Some(mutation_lease) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while removing the OPFS entry.",
            ) else {
                return;
            };
            drop(mutation_lease);
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        OpfsTaskResult::Resolve(result) => {
            let Some(components) = take_opfs_owner_result(
                scope,
                resolver,
                result,
                "The storage owner failed while resolving the OPFS entry.",
            ) else {
                return;
            };
            let Some(components) = components else {
                let _ = resolver.resolve(scope, v8::null(scope).into());
                return;
            };
            let array = v8::Array::new(scope, components.len() as i32);
            for (index, component) in components.iter().enumerate() {
                if let Some(component) = v8_string(scope, component) {
                    let _ = array.set_index(scope, index as u32, component.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
        OpfsTaskResult::WritableCommand {
            result,
            mut cleanup,
        } => match result {
            Ok(Ok(())) => {
                cleanup.disarm();
                let _ = resolver.resolve(scope, v8::undefined(scope).into());
            }
            Ok(Err(error)) => {
                if let Some(exception) =
                    writable_error_value(scope, WritableSinkError::Backend(error))
                {
                    let _ = resolver.reject(scope, exception);
                }
            }
            Err(_) => reject_dom_exception(
                scope,
                resolver,
                "UnknownError",
                "The storage owner failed while updating the OPFS writer.",
            ),
        },
    }
}

pub(crate) fn settle_opfs_move_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    handle: v8::Local<'s, v8::Object>,
    locator: &StorageBucketLocator,
    handle_access: Option<&OpfsHandleAccessContext>,
    result: OpfsTaskResult,
) {
    let access_check = match handle_access {
        Some(access) => service_for_locator_with_handle_access(scope, locator, access),
        None => service_for_locator(scope, locator),
    };
    if let Err(error) = access_check {
        reject_handle_access_error(scope, resolver, error);
        return;
    }
    let OpfsTaskResult::Move(result) = result else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "The storage owner returned an invalid move result.",
        );
        return;
    };
    let Some((destination, mutation_lease)) = take_opfs_owner_result(
        scope,
        resolver,
        result,
        "The storage owner failed while moving the OPFS entry.",
    ) else {
        return;
    };
    let Some(mut state) = handle_state(scope, handle).filter(|state| &state.locator == locator)
    else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The moved handle is no longer active.",
        );
        return;
    };
    if handle_path_state(scope, handle).is_none() {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The moved handle state is no longer active.",
        );
        return;
    }
    state.path = destination.components().to_vec();
    let Ok(state_json) = serde_json::to_string(&state) else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to update the moved handle.",
        );
        return;
    };
    let Some(state_json) = v8_string(scope, &state_json) else {
        reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to update the moved handle.",
        );
        return;
    };
    set_private_value(
        scope,
        handle,
        FILE_SYSTEM_HANDLE_STATE_SLOT,
        state_json.into(),
    );
    drop(mutation_lease);
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

pub(crate) fn settle_opfs_directory_iterator_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &OpfsDirectoryIteratorRegistry,
    iterator_id: u32,
    locator: &StorageBucketLocator,
    handle_access: Option<&OpfsHandleAccessContext>,
    result: OpfsTaskResult,
) {
    let access_check = match handle_access {
        Some(access) => service_for_locator_with_handle_access(scope, locator, access),
        None => service_for_locator(scope, locator),
    };
    if let Err(error) = access_check {
        let exception = handle_access_error_value(scope, error);
        reject_directory_iterator_load(scope, registry, iterator_id, exception);
        return;
    }
    let OpfsTaskResult::ReadDirectory(result) = result else {
        let exception = crate::context_bootstrap::new_dom_exception_value(
            scope,
            "The storage owner returned an invalid directory iterator result.",
            "UnknownError",
        );
        reject_directory_iterator_load(scope, registry, iterator_id, exception);
        return;
    };
    let entries = match result {
        Ok(Ok(entries)) => entries,
        Ok(Err(error)) => {
            let exception = opfs_error_value(scope, error);
            reject_directory_iterator_load(scope, registry, iterator_id, exception);
            return;
        }
        Err(_) => {
            let exception = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "The storage owner failed while reading the OPFS directory.",
                "UnknownError",
            );
            reject_directory_iterator_load(scope, registry, iterator_id, exception);
            return;
        }
    };
    for settlement in registry.complete_load(iterator_id, entries) {
        settle_opfs_directory_iterator_settlement(scope, settlement);
    }
}

fn settle_opfs_directory_iterator_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    settlement: OpfsDirectoryIteratorSettlement,
) {
    let resolver = v8::Local::new(scope, &settlement.resolver);
    settle_directory_iterator_next(scope, resolver, settlement.descriptor, settlement.entry);
}

fn reject_directory_iterator_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &OpfsDirectoryIteratorRegistry,
    iterator_id: u32,
    exception: v8::Local<'s, v8::Value>,
) {
    for resolver in registry.fail_load(iterator_id) {
        let resolver = v8::Local::new(scope, &resolver);
        let _ = resolver.reject(scope, exception);
    }
}

fn take_opfs_owner_result<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    result: Result<OpfsResult<T>, StorageServiceTaskError>,
    task_failure_message: &'static str,
) -> Option<T> {
    match result {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            reject_opfs_error(scope, resolver, error);
            None
        }
        Err(_) => {
            reject_dom_exception(scope, resolver, "UnknownError", task_failure_message);
            None
        }
    }
}

fn settle_handle_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: &StorageBucketLocator,
    path: &OpfsPath,
    kind: EntryKind,
    handle_access: OpfsHandleAccess,
) {
    match build_handle_object(scope, locator, path, kind, handle_access) {
        Some(handle) => {
            let _ = resolver.resolve(scope, handle.into());
        }
        None => reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to create a file system handle wrapper.",
        ),
    }
}

fn settle_get_file_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    locator: &StorageBucketLocator,
    result: OpfsGetFileTaskResult,
) {
    let Some(result) = take_opfs_owner_result(
        scope,
        resolver,
        result.result,
        "The storage owner failed while reading the OPFS file.",
    ) else {
        return;
    };
    let snapshot = result.snapshot;
    let snapshot_state =
        FileSystemFileSnapshotState::new(locator.clone(), &result.path, snapshot.identity);
    let file = SelectedFile {
        bytes: snapshot.bytes,
        mime_type: String::new(),
        name: snapshot.name,
        last_modified: snapshot.modified_ms as f64,
    };
    match build_file_object(scope, &file) {
        Some(file) => {
            if attach_file_system_file_snapshot_state(scope, file, &snapshot_state).is_none() {
                reject_dom_exception(
                    scope,
                    resolver,
                    "UnknownError",
                    "Failed to attach the File snapshot backing state.",
                );
                return;
            }
            let _ = resolver.resolve(scope, file.into());
        }
        None => reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to create a File snapshot.",
        ),
    }
}

fn directory_state_or_reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<FileSystemHandleState> {
    match handle_state(scope, object) {
        Some(state) if state.kind == EntryKind::Directory => Some(state),
        _ => {
            reject_type_error(scope, resolver, "Illegal invocation");
            None
        }
    }
}

fn file_state_or_reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<FileSystemHandleState> {
    match handle_state(scope, object) {
        Some(state) if state.kind == EntryKind::File => Some(state),
        _ => {
            reject_type_error(scope, resolver, "Illegal invocation");
            None
        }
    }
}

fn handle_promise_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    rv.set(resolver.get_promise(scope).into());
    Some(resolver)
}

fn required_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<String> {
    name_argument(scope, args, 0, resolver)
}

fn name_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<String> {
    if args.length() <= index || args.get(index).is_symbol() {
        reject_type_error(scope, resolver, "A file system entry name is required.");
        return None;
    }
    let Some(value) = args.get(index).to_string(scope) else {
        reject_type_error(scope, resolver, "The file system entry name is invalid.");
        return None;
    };
    let name = value.to_rust_string_lossy(scope);
    if let Err(error) = OpfsPath::root().child(&name) {
        reject_opfs_error(scope, resolver, error);
        return None;
    }
    Some(name)
}

fn boolean_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    property: &'static str,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<bool> {
    if args.length() <= index || args.get(index).is_null_or_undefined() {
        return Some(false);
    }
    let Some(options) = args.get(index).to_object(scope) else {
        reject_type_error(
            scope,
            resolver,
            "File system handle options must be an object.",
        );
        return None;
    };
    let Some(value) = options.get(scope, v8str(scope, property).into()) else {
        reject_type_error(
            scope,
            resolver,
            "Failed to read file system handle options.",
        );
        return None;
    };
    Some(value.boolean_value(scope))
}

fn reject_handle_access_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: HandleAccessError,
) {
    let exception = handle_access_error_value(scope, error);
    let _ = resolver.reject(scope, exception);
}

fn handle_access_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: HandleAccessError,
) -> v8::Local<'s, v8::Value> {
    match error {
        HandleAccessError::Security => crate::context_bootstrap::new_dom_exception_value(
            scope,
            "The handle is not authorized for this storage context.",
            "SecurityError",
        ),
        HandleAccessError::InvalidState => crate::context_bootstrap::new_dom_exception_value(
            scope,
            "The file system handle execution context is unavailable.",
            "InvalidStateError",
        ),
        HandleAccessError::Stale => crate::context_bootstrap::new_dom_exception_value(
            scope,
            "The storage bucket that owns this handle no longer exists.",
            "NotFoundError",
        ),
        HandleAccessError::Backend(error) => opfs_error_value(scope, error),
    }
}

fn reject_opfs_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: OpfsError,
) {
    let exception = opfs_error_value(scope, error);
    let _ = resolver.reject(scope, exception);
}

fn opfs_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: OpfsError,
) -> v8::Local<'s, v8::Value> {
    let message = error.to_string();
    match error {
        OpfsError::InvalidName(_) => v8::String::new(scope, &message)
            .map(|message| v8::Exception::type_error(scope, message))
            .unwrap_or_else(|| v8::undefined(scope).into()),
        OpfsError::NotFound(_) => {
            crate::context_bootstrap::new_dom_exception_value(scope, &message, "NotFoundError")
        }
        OpfsError::TypeMismatch { .. } => {
            crate::context_bootstrap::new_dom_exception_value(scope, &message, "TypeMismatchError")
        }
        OpfsError::DirectoryNotEmpty(_) | OpfsError::InvalidModification(_) => {
            crate::context_bootstrap::new_dom_exception_value(
                scope,
                &message,
                "InvalidModificationError",
            )
        }
        OpfsError::NoModificationAllowed(_) => crate::context_bootstrap::new_dom_exception_value(
            scope,
            &message,
            "NoModificationAllowedError",
        ),
        OpfsError::InvalidState => {
            crate::context_bootstrap::new_dom_exception_value(scope, &message, "InvalidStateError")
        }
        OpfsError::QuotaExceeded { .. } => {
            crate::context_bootstrap::new_quota_exceeded_error_value(
                scope,
                "The operation failed because it would cause the application to exceed its storage quota.",
                None,
                None,
            )
        }
        OpfsError::CorruptCatalog(_) | OpfsError::CatalogJson(_) | OpfsError::Io { .. } => {
            crate::context_bootstrap::new_dom_exception_value(scope, &message, "UnknownError")
        }
    }
}

fn reject_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let exception = v8::String::new(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, exception);
}

fn reject_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    name: &str,
    message: &str,
) {
    let exception = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, exception);
}

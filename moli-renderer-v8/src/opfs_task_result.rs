//! OPFS storage-owner outcomes shared by Window and Worker settlements.
//!
//! These values describe completed storage work or immutable Blob input reads. They carry no
//! Page task identity, scheduler route, or Worker transport identity; those
//! capabilities remain in their respective execution domains.

use moli_storage_service::{
    DirectoryEntry, EntryKind, FileSnapshot, FileSnapshotIdentity, OpfsBucketKey, OpfsPath,
    OpfsResult, SharedStorageService, StorageOpfsMutationLease, StorageOpfsSyncAccessLease,
    StorageOpfsWritableLease, StorageServiceTaskError, WritableMode,
};

#[derive(Debug)]
pub(crate) enum OpfsTaskResult {
    StorageBucket(moli_storage_service::SqlBucketOperationResult),
    CreateSyncAccessHandle {
        mode: String,
        result: OpfsOwnerResult<StorageOpfsSyncAccessLease>,
    },
    CreateWritable {
        mode: WritableMode,
        result: OpfsOwnerResult<StorageOpfsWritableLease>,
    },
    GetRoot(OpfsOwnerResult<OpfsPath>),
    GetChild {
        kind: EntryKind,
        result: OpfsOwnerResult<OpfsPath>,
    },
    GetFile(OpfsGetFileTaskResult),
    IsSameEntry(OpfsOwnerResult<bool>),
    GetUniqueId(OpfsOwnerResult<String>),
    Move(OpfsOwnerResult<(OpfsPath, StorageOpfsMutationLease)>),
    ReadDirectory(OpfsOwnerResult<Vec<DirectoryEntry>>),
    Remove(OpfsOwnerResult<Option<StorageOpfsMutationLease>>),
    Resolve(OpfsOwnerResult<Option<Vec<String>>>),
    WritableInput {
        bytes: std::io::Result<Vec<u8>>,
        position: Option<u64>,
        snapshot_validation: Option<(OpfsBucketKey, OpfsPath, FileSnapshotIdentity)>,
        service: SharedStorageService,
        cleanup: StorageOpfsWritableLease,
    },
    WritableCommand {
        result: OpfsOwnerResult<()>,
        cleanup: StorageOpfsWritableLease,
    },
}

pub(crate) type OpfsOwnerResult<T> = Result<OpfsResult<T>, StorageServiceTaskError>;

#[derive(Debug)]
pub(crate) struct OpfsGetFileTaskResult {
    pub(crate) result: OpfsOwnerResult<OpfsReadFileResult>,
}

#[derive(Debug)]
pub(crate) struct OpfsReadFileResult {
    pub(crate) path: OpfsPath,
    pub(crate) snapshot: FileSnapshot,
}

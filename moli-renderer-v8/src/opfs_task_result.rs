//! OPFS storage-owner outcomes shared by Window and Worker settlements.
//!
//! These values describe only the completed storage operation. They carry no
//! Page task identity, scheduler route, or Worker transport identity; those
//! capabilities remain in their respective execution domains.

use moli_storage_service::{
    DirectoryEntry, EntryKind, FileSnapshot, OpfsPath, OpfsResult, StorageOpfsMutationLease,
    StorageOpfsSyncAccessLease, StorageOpfsWritableLease, StorageServiceTaskError, WritableMode,
};

#[derive(Debug)]
pub(crate) enum OpfsTaskResult {
    DeleteStorageBucket(moli_storage_service::SqlBucketDeletionResult),
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

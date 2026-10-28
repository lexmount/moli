use super::stores::collection::CollectionRequest;
use super::{CursorDirection, IdbKeyRangeQuery, IndexInfo, PreparedObjectStoreWrite};
use crate::native_bridge::OwnerDispatchScope;

pub(super) enum IndexedDbCursorSource {
    ObjectStore,
    Index(IndexInfo),
}

pub(super) struct IndexedDbCursorOpenOperation {
    pub(super) source: IndexedDbCursorSource,
    pub(super) query: Option<IdbKeyRangeQuery>,
    pub(super) direction: CursorDirection,
    pub(super) key_only: bool,
}

impl IndexedDbCursorOpenOperation {
    pub(super) fn object_store(
        query: Option<IdbKeyRangeQuery>,
        direction: CursorDirection,
        key_only: bool,
    ) -> Self {
        Self {
            source: IndexedDbCursorSource::ObjectStore,
            query,
            direction,
            key_only,
        }
    }

    pub(super) fn index(
        index_info: IndexInfo,
        query: Option<IdbKeyRangeQuery>,
        direction: CursorDirection,
        key_only: bool,
    ) -> Self {
        Self {
            source: IndexedDbCursorSource::Index(index_info),
            query,
            direction,
            key_only,
        }
    }
}

type SqlOperationStart = Box<
    dyn for<'s, 'i> FnOnce(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        moli_indexeddb::sqlite::service::TaskPermit,
    ) -> bool,
>;

pub(super) enum IndexedDbCursorIteration {
    Continue(Option<super::Key>),
    Advance(u32),
    PrimaryKey {
        key: super::Key,
        primary_key: super::Key,
    },
}

pub(super) enum IndexedDbTransactionOperation {
    /// Accepted schema work that could not acquire native storage capacity.
    /// It keeps its position in this queue and fails on a later owner task.
    InternalError(super::IndexedDbError),
    /// Prepared by the common dispatcher, after selecting the SQL backend.
    /// `true` keeps this logical operation in flight until its completion.
    Sql {
        permit: moli_indexeddb::sqlite::service::TaskPermit,
        start: SqlOperationStart,
    },
    ObjectStoreGet {
        query: IdbKeyRangeQuery,
    },
    ObjectStoreGetAll(CollectionRequest),
    ObjectStoreGetKey {
        query: IdbKeyRangeQuery,
    },
    ObjectStoreCount {
        query: Option<IdbKeyRangeQuery>,
    },
    OpenCursor(IndexedDbCursorOpenOperation),
    CursorIterate {
        cursor: v8::Global<v8::Object>,
        movement: IndexedDbCursorIteration,
    },
    CursorDelete {
        key: super::Key,
    },
    ObjectStoreWrite {
        prepared: PreparedObjectStoreWrite,
        add_only: bool,
    },
    ObjectStoreDelete {
        query: IdbKeyRangeQuery,
    },
    ObjectStoreClear,
    IndexGet {
        query: IdbKeyRangeQuery,
    },
    IndexGetKey {
        query: IdbKeyRangeQuery,
    },
    IndexGetAll(CollectionRequest),
    IndexCount {
        query: Option<IdbKeyRangeQuery>,
    },
}

impl IndexedDbTransactionOperation {
    pub(super) fn sql<F>(
        mut permit: moli_indexeddb::sqlite::service::TaskPermit,
        start: F,
    ) -> Result<Self, super::IndexedDbError>
    where
        F: for<'s, 'i> FnOnce(
                &mut v8::PinScope<'s, 'i>,
                v8::Local<'s, v8::Object>,
                moli_indexeddb::sqlite::service::TaskPermit,
            ) -> bool
            + 'static,
    {
        let bytes = permit
            .reserved_bytes()
            .saturating_add(std::mem::size_of::<IndexedDbPendingTransactionOperation>())
            .saturating_add(std::mem::size_of::<F>());
        permit
            .try_resize(bytes)
            .map_err(|error| super::IndexedDbError::Resource(error.to_string()))?;
        Ok(Self::Sql {
            permit,
            start: Box::new(start),
        })
    }
}

// Query arguments are converted at the API boundary and retained as native snapshots.
// Write values and keys are also prepared synchronously, before request admission.
pub(super) struct IndexedDbPendingTransactionOperation {
    owner: OwnerDispatchScope,
    source: v8::Global<v8::Object>,
    request: v8::Global<v8::Object>,
    store_name: String,
    kind: IndexedDbTransactionOperation,
}

pub(super) struct IndexedDbTransactionOperationLocals<'s> {
    pub(super) source: v8::Local<'s, v8::Object>,
    pub(super) request: v8::Local<'s, v8::Object>,
    pub(super) store_name: String,
    pub(super) kind: IndexedDbTransactionOperation,
}

impl IndexedDbPendingTransactionOperation {
    pub(super) fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        source: v8::Local<'s, v8::Object>,
        request: v8::Local<'s, v8::Object>,
        store_name: impl Into<String>,
        kind: IndexedDbTransactionOperation,
    ) -> Self {
        Self {
            owner,
            source: v8::Global::new(scope, source),
            request: v8::Global::new(scope, request),
            store_name: store_name.into(),
            kind,
        }
    }

    pub(super) fn owner(&self) -> OwnerDispatchScope {
        self.owner
    }

    pub(super) fn request<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let target = v8::Local::new(scope, &self.request);
        // Internal schema work uses its transaction as the completion target;
        // it has no Web request or request error event to dispatch on abort.
        super::indexed_db_typed_wrapper_is(scope, target, super::IndexedDbWrapperKind::Request)
            .then_some(target)
    }

    pub(super) fn into_locals<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> IndexedDbTransactionOperationLocals<'s> {
        IndexedDbTransactionOperationLocals {
            source: v8::Local::new(scope, &self.source),
            request: v8::Local::new(scope, &self.request),
            store_name: self.store_name,
            kind: self.kind,
        }
    }
}

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

pub(super) enum IndexedDbTransactionOperation {
    ObjectStoreGet {
        query: IdbKeyRangeQuery,
    },
    ObjectStoreGetAll {
        query: Option<IdbKeyRangeQuery>,
        count: Option<usize>,
        direction: CursorDirection,
    },
    ObjectStoreGetKey {
        query: IdbKeyRangeQuery,
    },
    ObjectStoreGetAllKeys {
        query: Option<IdbKeyRangeQuery>,
        count: Option<usize>,
        direction: CursorDirection,
    },
    ObjectStoreCount {
        query: Option<IdbKeyRangeQuery>,
    },
    OpenCursor(IndexedDbCursorOpenOperation),
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
    IndexGetAll {
        query: Option<IdbKeyRangeQuery>,
        count: Option<usize>,
        direction: CursorDirection,
    },
    IndexGetAllKeys {
        query: Option<IdbKeyRangeQuery>,
        count: Option<usize>,
        direction: CursorDirection,
    },
    IndexCount {
        query: Option<IdbKeyRangeQuery>,
    },
}

// All operation arguments have already been converted at the API boundary.
// Retain JS handles only for request/source identity, never for deferred keys.
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
    ) -> v8::Local<'s, v8::Object> {
        v8::Local::new(scope, &self.request)
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

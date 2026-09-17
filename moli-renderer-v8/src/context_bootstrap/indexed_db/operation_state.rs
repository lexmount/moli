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

pub(super) enum IndexedDbTransactionOperationInput<'s> {
    ObjectStoreGet {
        query: v8::Local<'s, v8::Value>,
    },
    ObjectStoreGetAll {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    ObjectStoreGetKey {
        query: v8::Local<'s, v8::Value>,
    },
    ObjectStoreGetAllKeys {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    ObjectStoreCount {
        query: v8::Local<'s, v8::Value>,
    },
    OpenCursor(IndexedDbCursorOpenOperation),
    ObjectStoreWrite {
        prepared: PreparedObjectStoreWrite,
        add_only: bool,
    },
    ObjectStoreDelete {
        key: v8::Local<'s, v8::Value>,
    },
    ObjectStoreClear,
    IndexGet {
        query: v8::Local<'s, v8::Value>,
    },
    IndexGetKey {
        query: v8::Local<'s, v8::Value>,
    },
    IndexGetAll {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    IndexGetAllKeys {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    IndexCount {
        query: v8::Local<'s, v8::Value>,
    },
}

pub(super) struct IndexedDbPendingTransactionOperation {
    owner: OwnerDispatchScope,
    source: v8::Global<v8::Object>,
    request: v8::Global<v8::Object>,
    store_name: String,
    kind: IndexedDbPendingTransactionOperationKind,
}

enum IndexedDbPendingTransactionOperationKind {
    ObjectStoreGet {
        query: v8::Global<v8::Value>,
    },
    ObjectStoreGetAll {
        query: v8::Global<v8::Value>,
        count: v8::Global<v8::Value>,
        direction: v8::Global<v8::Value>,
    },
    ObjectStoreGetKey {
        query: v8::Global<v8::Value>,
    },
    ObjectStoreGetAllKeys {
        query: v8::Global<v8::Value>,
        count: v8::Global<v8::Value>,
        direction: v8::Global<v8::Value>,
    },
    ObjectStoreCount {
        query: v8::Global<v8::Value>,
    },
    OpenCursor(IndexedDbCursorOpenOperation),
    ObjectStoreWrite {
        prepared: PreparedObjectStoreWrite,
        add_only: bool,
    },
    ObjectStoreDelete {
        key: v8::Global<v8::Value>,
    },
    ObjectStoreClear,
    IndexGet {
        query: v8::Global<v8::Value>,
    },
    IndexGetKey {
        query: v8::Global<v8::Value>,
    },
    IndexGetAll {
        query: v8::Global<v8::Value>,
        count: v8::Global<v8::Value>,
        direction: v8::Global<v8::Value>,
    },
    IndexGetAllKeys {
        query: v8::Global<v8::Value>,
        count: v8::Global<v8::Value>,
        direction: v8::Global<v8::Value>,
    },
    IndexCount {
        query: v8::Global<v8::Value>,
    },
}

pub(super) struct IndexedDbTransactionOperationLocals<'s> {
    pub(super) source: v8::Local<'s, v8::Object>,
    pub(super) request: v8::Local<'s, v8::Object>,
    pub(super) store_name: String,
    pub(super) kind: IndexedDbTransactionOperationKindLocals<'s>,
}

pub(super) enum IndexedDbTransactionOperationKindLocals<'s> {
    ObjectStoreGet {
        query: v8::Local<'s, v8::Value>,
    },
    ObjectStoreGetAll {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    ObjectStoreGetKey {
        query: v8::Local<'s, v8::Value>,
    },
    ObjectStoreGetAllKeys {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    ObjectStoreCount {
        query: v8::Local<'s, v8::Value>,
    },
    OpenCursor(IndexedDbCursorOpenOperation),
    ObjectStoreWrite {
        prepared: PreparedObjectStoreWrite,
        add_only: bool,
    },
    ObjectStoreDelete {
        key: v8::Local<'s, v8::Value>,
    },
    ObjectStoreClear,
    IndexGet {
        query: v8::Local<'s, v8::Value>,
    },
    IndexGetKey {
        query: v8::Local<'s, v8::Value>,
    },
    IndexGetAll {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    IndexGetAllKeys {
        query: v8::Local<'s, v8::Value>,
        count: v8::Local<'s, v8::Value>,
        direction: v8::Local<'s, v8::Value>,
    },
    IndexCount {
        query: v8::Local<'s, v8::Value>,
    },
}

impl IndexedDbPendingTransactionOperation {
    pub(super) fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        source: v8::Local<'s, v8::Object>,
        request: v8::Local<'s, v8::Object>,
        store_name: impl Into<String>,
        input: IndexedDbTransactionOperationInput<'s>,
    ) -> Self {
        use IndexedDbPendingTransactionOperationKind as Pending;
        use IndexedDbTransactionOperationInput as Input;

        let kind = match input {
            Input::ObjectStoreGet { query } => Pending::ObjectStoreGet {
                query: v8::Global::new(scope, query),
            },
            Input::ObjectStoreGetAll {
                query,
                count,
                direction,
            } => Pending::ObjectStoreGetAll {
                query: v8::Global::new(scope, query),
                count: v8::Global::new(scope, count),
                direction: v8::Global::new(scope, direction),
            },
            Input::ObjectStoreGetKey { query } => Pending::ObjectStoreGetKey {
                query: v8::Global::new(scope, query),
            },
            Input::ObjectStoreGetAllKeys {
                query,
                count,
                direction,
            } => Pending::ObjectStoreGetAllKeys {
                query: v8::Global::new(scope, query),
                count: v8::Global::new(scope, count),
                direction: v8::Global::new(scope, direction),
            },
            Input::ObjectStoreCount { query } => Pending::ObjectStoreCount {
                query: v8::Global::new(scope, query),
            },
            Input::OpenCursor(operation) => Pending::OpenCursor(operation),
            Input::ObjectStoreWrite { prepared, add_only } => {
                Pending::ObjectStoreWrite { prepared, add_only }
            }
            Input::ObjectStoreDelete { key } => Pending::ObjectStoreDelete {
                key: v8::Global::new(scope, key),
            },
            Input::ObjectStoreClear => Pending::ObjectStoreClear,
            Input::IndexGet { query } => Pending::IndexGet {
                query: v8::Global::new(scope, query),
            },
            Input::IndexGetKey { query } => Pending::IndexGetKey {
                query: v8::Global::new(scope, query),
            },
            Input::IndexGetAll {
                query,
                count,
                direction,
            } => Pending::IndexGetAll {
                query: v8::Global::new(scope, query),
                count: v8::Global::new(scope, count),
                direction: v8::Global::new(scope, direction),
            },
            Input::IndexGetAllKeys {
                query,
                count,
                direction,
            } => Pending::IndexGetAllKeys {
                query: v8::Global::new(scope, query),
                count: v8::Global::new(scope, count),
                direction: v8::Global::new(scope, direction),
            },
            Input::IndexCount { query } => Pending::IndexCount {
                query: v8::Global::new(scope, query),
            },
        };
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
        use IndexedDbPendingTransactionOperationKind as Pending;
        use IndexedDbTransactionOperationKindLocals as Locals;

        let kind = match self.kind {
            Pending::ObjectStoreGet { query } => Locals::ObjectStoreGet {
                query: v8::Local::new(scope, &query),
            },
            Pending::ObjectStoreGetAll {
                query,
                count,
                direction,
            } => Locals::ObjectStoreGetAll {
                query: v8::Local::new(scope, &query),
                count: v8::Local::new(scope, &count),
                direction: v8::Local::new(scope, &direction),
            },
            Pending::ObjectStoreGetKey { query } => Locals::ObjectStoreGetKey {
                query: v8::Local::new(scope, &query),
            },
            Pending::ObjectStoreGetAllKeys {
                query,
                count,
                direction,
            } => Locals::ObjectStoreGetAllKeys {
                query: v8::Local::new(scope, &query),
                count: v8::Local::new(scope, &count),
                direction: v8::Local::new(scope, &direction),
            },
            Pending::ObjectStoreCount { query } => Locals::ObjectStoreCount {
                query: v8::Local::new(scope, &query),
            },
            Pending::OpenCursor(operation) => Locals::OpenCursor(operation),
            Pending::ObjectStoreWrite { prepared, add_only } => {
                Locals::ObjectStoreWrite { prepared, add_only }
            }
            Pending::ObjectStoreDelete { key } => Locals::ObjectStoreDelete {
                key: v8::Local::new(scope, &key),
            },
            Pending::ObjectStoreClear => Locals::ObjectStoreClear,
            Pending::IndexGet { query } => Locals::IndexGet {
                query: v8::Local::new(scope, &query),
            },
            Pending::IndexGetKey { query } => Locals::IndexGetKey {
                query: v8::Local::new(scope, &query),
            },
            Pending::IndexGetAll {
                query,
                count,
                direction,
            } => Locals::IndexGetAll {
                query: v8::Local::new(scope, &query),
                count: v8::Local::new(scope, &count),
                direction: v8::Local::new(scope, &direction),
            },
            Pending::IndexGetAllKeys {
                query,
                count,
                direction,
            } => Locals::IndexGetAllKeys {
                query: v8::Local::new(scope, &query),
                count: v8::Local::new(scope, &count),
                direction: v8::Local::new(scope, &direction),
            },
            Pending::IndexCount { query } => Locals::IndexCount {
                query: v8::Local::new(scope, &query),
            },
        };
        IndexedDbTransactionOperationLocals {
            source: v8::Local::new(scope, &self.source),
            request: v8::Local::new(scope, &self.request),
            store_name: self.store_name,
            kind,
        }
    }
}

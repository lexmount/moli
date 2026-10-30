use super::*;
use crate::context_bootstrap::indexed_db::{
    ValueDecodeSession, typed_state::sql::sql_transaction_cancellation,
};
use moli_indexeddb::sqlite::{Seek, service::Completion};

pub(super) fn prepare(
    client: &DatabaseClient,
    transaction: TransactionHandle,
    store: moli_indexeddb::IndexedDbName,
    query: Query,
    kind: CollectionKind,
) -> IndexedDbTransactionOperation {
    let client = client.clone();
    IndexedDbTransactionOperation::sql(move |scope, request| {
        if let Err(error) = ensure_sql_transaction_rollback(scope, request) {
            fail_request(scope, request, &error);
            return false;
        }
        CollectionRead {
            client,
            transaction,
            store,
            query,
            kind,
            array: None,
            length: 0,
            session: ValueDecodeSession::new(),
        }
        .submit(scope, request)
    })
}

struct CollectionRead {
    client: DatabaseClient,
    transaction: TransactionHandle,
    store: moli_indexeddb::IndexedDbName,
    query: Query,
    kind: CollectionKind,
    array: Option<v8::Global<v8::Array>>,
    length: u32,
    session: ValueDecodeSession,
}

impl CollectionRead {
    fn submit<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        request: v8::Local<'s, v8::Object>,
    ) -> bool {
        let client = self.client.clone();
        let command = Command::QueryPage {
            transaction: self.transaction,
            store: self.store.clone(),
            query: self.query.clone(),
        };
        let submitted = submit_indexed_db_sql_task(
            scope,
            request,
            &client,
            command,
            move |scope, request, completion| self.receive(scope, request, completion),
        );
        if let Err(error) = submitted {
            fail_request(scope, request, &error);
            false
        } else {
            true
        }
    }

    fn receive<'s>(
        mut self,
        scope: &mut v8::PinScope<'s, '_>,
        request: v8::Local<'s, v8::Object>,
        completion: Completion,
    ) {
        let page = match completion.result {
            Ok(CommandResult::RecordPage(page)) => page,
            Err(error) => {
                let error = request_error_object(scope, &error);
                self.reject(scope, request, error);
                return;
            }
            _ => {
                let error = request_error_object(
                    scope,
                    &IndexedDbError::InvalidState("Invalid SQL collection result".into()),
                );
                self.reject(scope, request, error);
                return;
            }
        };
        let cancel = sql_transaction_cancellation(scope, request);
        v8::tc_scope!(let scope, scope);
        let count = u32::try_from(page.records.len()).ok();
        let Some(count) =
            count.filter(|count| *count <= self.query.limit && (*count != 0 || !page.has_more))
        else {
            let error = request_error_object(
                scope,
                &IndexedDbError::Corruption("Invalid SQL collection progress".into()),
            );
            self.reject(scope, request, error);
            return;
        };
        let array = match self.array.as_ref() {
            Some(array) => v8::Local::new(scope, array),
            None => v8::Array::new(scope, 0),
        };
        let appended = self.append(scope, array, &page.records);
        if scope.is_execution_terminating() || !scope.can_continue() {
            if scope.has_caught() {
                scope.rethrow();
            }
            drop(self);
            cancel();
            return;
        }
        match appended {
            Ok(Some(())) => (),
            Err(error) => {
                scope.reset();
                let error = request_error_object(scope, &error);
                self.reject(scope, request, error);
                return;
            }
            Ok(None) => {
                let error = scope.exception().unwrap_or_else(|| {
                    request_error_object(
                        scope,
                        &IndexedDbError::Serialization(
                            "Unable to decode IndexedDB collection".into(),
                        ),
                    )
                });
                self.reject(scope, request, error);
                return;
            }
        }
        self.length += count;
        self.query.limit -= count;
        if page.has_more && self.query.limit != 0 {
            let last = page
                .records
                .last()
                .expect("continuation batches are nonempty");
            self.query.seek = Some(Seek {
                key: last.key.clone(),
                primary_key: Some(last.primary_key.clone()),
                inclusive: false,
            });
            self.query.skip = 0;
            if self.array.is_none() {
                self.array = Some(v8::Global::new(scope, array));
            }
            // Release native wire/body buffers before submitting another batch.
            drop(page);
            if !self.submit(scope, request) {
                operation::finish_transaction_operation(scope, request);
            }
        } else {
            drop(page);
            self.session.publish();
            store_request_success(scope, request, array.into());
            operation::finish_transaction_operation(scope, request);
        }
    }

    fn append<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        array: v8::Local<'s, v8::Array>,
        records: &[CursorSnapshotEntry],
    ) -> Result<Option<()>, IndexedDbError> {
        for (index, record) in records.iter().enumerate() {
            let Some(value) = record_result(scope, self.kind, record, &self.session)? else {
                return Ok(None);
            };
            let index = u32::try_from(index)
                .ok()
                .and_then(|index| self.length.checked_add(index))
                .ok_or_else(|| {
                    IndexedDbError::Resource("IndexedDB collection exceeds array length".into())
                })?;
            if moli_webapi_declare::define_array_data_property(scope, array, index, value).is_none()
            {
                return Ok(None);
            }
        }
        Ok(Some(()))
    }

    fn reject<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        request: v8::Local<'s, v8::Object>,
        error: v8::Local<'s, v8::Value>,
    ) {
        store_request_error(scope, request, error);
        drop(self);
        operation::finish_transaction_operation(scope, request);
    }
}

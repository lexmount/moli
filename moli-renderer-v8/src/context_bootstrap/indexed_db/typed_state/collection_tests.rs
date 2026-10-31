use super::*;

#[test]
fn transaction_collection_defers_registry_mutation_and_keeps_retained_state() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let context;
    let table;
    let mut transactions = Vec::new();
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Context::new(scope, Default::default());
        context = v8::Global::new(scope, local);
        let scope = &mut v8::ContextScope::new(scope, local);
        table = ensure_indexed_db_runtime_state_table(scope);
        let database = v8::Object::new(scope);
        register_indexed_db_wrapper(scope, database, IndexedDbWrapperKind::Database, None);
        for retained in [false, true] {
            let transaction = v8::Object::new(scope);
            register_indexed_db_wrapper(
                scope,
                transaction,
                IndexedDbWrapperKind::Transaction,
                None,
            );
            register_indexed_db_transaction_lifecycle(
                scope,
                transaction,
                database,
                None,
                TransactionMode::ReadOnly,
                false,
            );
            let id = indexed_db_typed_state_id(scope, transaction).unwrap();
            table
                .borrow_mut()
                .transactions
                .get_mut(&id)
                .unwrap()
                .finished = true;
            release_indexed_db_transaction_dispatch_refs(scope, transaction);
            transactions.push((id, v8::Weak::new(scope, transaction)));
            if retained {
                assert_eq!(
                    local.global(scope).create_data_property(
                        scope,
                        v8str(scope, "heldTransaction").into(),
                        transaction.into(),
                    ),
                    Some(true),
                );
            }
        }
    }
    // Finalizers can run while a native lookup has the registry borrowed.
    let guard = table.borrow();
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    assert!(transactions[0].1.is_empty());
    assert!(!transactions[1].1.is_empty());
    drop(guard);
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, local);
        ensure_indexed_db_runtime_state_table(scope);
        assert!(!table.borrow().transactions.contains_key(&transactions[0].0));
        assert!(!table.borrow().wrappers.contains_key(&transactions[0].0));
        assert_eq!(table.borrow().transactions.len(), 1);
        let transaction = transactions[1].1.to_local(scope).unwrap();
        assert_eq!(
            indexed_db_transaction_mode(scope, transaction),
            Some(TransactionMode::ReadOnly)
        );
        assert_eq!(
            local
                .global(scope)
                .delete(scope, v8str(scope, "heldTransaction").into()),
            Some(true)
        );
    }
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    assert!(transactions[1].1.is_empty());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let local = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, local);
    ensure_indexed_db_runtime_state_table(scope);
    assert!(table.borrow().transactions.is_empty());
    assert!(!table.borrow().wrappers.contains_key(&transactions[1].0));
}

#[test]
fn cursor_collection_releases_snapshots_without_borrowing_the_runtime_table() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let context;
    let table;
    let discarded;
    let discarded_entries;
    let retained_entries;
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Context::new(scope, Default::default());
        context = v8::Global::new(scope, local);
        let scope = &mut v8::ContextScope::new(scope, local);
        table = ensure_indexed_db_runtime_state_table(scope);
        let mut snapshots = Vec::new();
        let mut cursors = Vec::new();
        for retained in [false, true] {
            let request = v8::Object::new(scope);
            register_indexed_db_wrapper(scope, request, IndexedDbWrapperKind::Request, None);
            let null = v8::null(scope);
            register_indexed_db_request_lifecycle(scope, request, null.into(), null.into(), false);
            let cursor = v8::Object::new(scope);
            register_indexed_db_wrapper(scope, cursor, IndexedDbWrapperKind::Cursor, None);
            register_indexed_db_cursor_lifecycle(
                scope,
                cursor,
                vec![CursorSnapshotEntry {
                    key: Key::from("key"),
                    primary_key: Key::from("primary"),
                    value: None,
                }],
                CursorDirection::default_next(),
                true,
            );
            set_private_value(
                scope,
                cursor,
                "moli.IndexedDb.gcTestRequest",
                request.into(),
            );
            assert!(set_indexed_db_typed_slot_value(
                scope,
                request,
                INDEXED_DB_REQUEST_RESULT_SLOT,
                cursor.into()
            ));
            let state = indexed_db_cursor_state(scope, cursor).unwrap();
            snapshots.push(Rc::downgrade(&state.entries));
            cursors.push(v8::Weak::new(scope, cursor));
            if retained {
                let global = local.global(scope);
                assert_eq!(
                    global.create_data_property(
                        scope,
                        v8str(scope, "heldCursor").into(),
                        cursor.into()
                    ),
                    Some(true)
                );
            }
        }
        discarded_entries = snapshots.remove(0);
        retained_entries = snapshots.remove(0);
        discarded = cursors.remove(0);
    }
    // Collection can happen during any V8 allocation made by a state getter.
    // Holding the table guard here catches finalizers that mutate that table.
    let guard = table.borrow();
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    assert!(discarded.is_empty());
    assert!(discarded_entries.upgrade().is_none());
    assert!(retained_entries.upgrade().is_some());
    drop(guard);
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, local);
        ensure_indexed_db_runtime_state_table(scope);
        assert_eq!(table.borrow().cursors.len(), 1);
        assert_eq!(table.borrow().requests.len(), 1);
        let global = local.global(scope);
        let cursor = global
            .get(scope, v8str(scope, "heldCursor").into())
            .unwrap()
            .to_object(scope)
            .unwrap();
        let state = indexed_db_cursor_state(scope, cursor).unwrap();
        assert_eq!(state.entries[0].key, Key::from("key"));
        assert_eq!(state.position, Some(0));
        assert_eq!(
            global.delete(scope, v8str(scope, "heldCursor").into()),
            Some(true)
        );
    }
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    assert!(retained_entries.upgrade().is_none());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let local = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, local);
    ensure_indexed_db_runtime_state_table(scope);
    assert!(table.borrow().cursors.is_empty());
    assert!(table.borrow().requests.is_empty());
    assert!(table.borrow().collectible_wrappers.is_empty());
}

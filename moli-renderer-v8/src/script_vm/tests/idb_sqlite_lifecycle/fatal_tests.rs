use super::*;
use moli_indexeddb::sqlite::service::Completion;

#[derive(Clone)]
struct NativeAbort {
    client: DatabaseClient,
    sender: mpsc::Sender<Completion>,
}

fn abort_native_transaction(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let control = scope.get_slot::<NativeAbort>().unwrap().clone();
    let transaction = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
    let handle =
        crate::context_bootstrap::sql_transaction_handle_for_test(scope, transaction).unwrap();
    control
        .client
        .submit(
            Command::Abort {
                transaction: handle,
            },
            move |reply| {
                let _ = control.sender.send(reply);
            },
        )
        .unwrap();
}

#[tokio::test]
async fn idb_sqlite_native_transaction_loss_aborts_even_when_request_error_is_canceled() {
    let (mut vm, loader, backend) = fixture();
    let (sender, receiver) = mpsc::channel();
    vm.with_default_context_scope(|scope, _| {
        scope.set_slot(NativeAbort {
            client: backend.client.clone(),
            sender,
        });
        let callback = v8::Function::new(scope, abort_native_transaction).unwrap();
        let global = scope.get_current_context().global(scope);
        global
            .set(
                scope,
                crate::util::v8str(scope, "abortNative").into(),
                callback.into(),
            )
            .unwrap();
        Ok(())
    })
    .unwrap();
    vm.eval(r#"(() => {
        globalThis.events = [];
        globalThis.nextKey = -1;
        globalThis.getterReads = 0;
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'rolled-back'}).onsuccess = () => {
            events.push('first');
            // Real native ROLLBACK is queued before the following SQL count.
            // No test callback waits for SQL or enters the connection on V8.
            abortNative(tx);
            store.count().onerror = event => {
                events.push('current:' + event.target.error.name);
                event.preventDefault();
                try { store.put({get group() { getterReads++; return 'must-not-run'; }}); }
                catch(error) { events.push(error.name); }
            };
            store.put({group:'queued'}).onerror = event => {
                events.push('queued:' + event.target.error.name);
                event.preventDefault();
            };
        };
        tx.onabort = () => events.push('abort:' + tx.error.name);
        tx.oncomplete = () => events.push('bad-complete');
        const next = db.transaction('records','readwrite');
        next.objectStore('records').put({group:'next'}).onsuccess = event => nextKey = event.target.result;
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    let native = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(native.result.is_ok());
    assert_eq!(native.active_transaction, None);
    drop(native);
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["first","current:AbortError","TransactionInactiveError","queued:AbortError","abort:UnknownError"]"#
    );
    assert_eq!(vm.eval("[nextKey,getterReads].join(',')").unwrap(), "1,0");
    vm.eval("globalThis.saved = -1; db.transaction('records').objectStore('records').count().onsuccess = event => saved = event.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(saved)").unwrap(), "1");
}

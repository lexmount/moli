use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_finishes_window_and_worker_admissions_and_closes_idle_connections() {
    for worker in [false, true] {
        let mut ctx = TestContext::new();
        let mut browser_context = BrowserContext::new("BID-idb-cancel".into());
        browser_context.set_active_target_id("TID-idb-cancel");
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));
        let origin = "https://idb-cancel.example";
        ctx.install_buffered_navigation_fixture_for_session_owner(
            Url::parse(&format!("{origin}/app")).unwrap(),
            "<!doctype html><html><body>cancel</body></html>".into(),
            None,
        )
        .await;
        install_storage_test_completion_binding(&mut ctx).await;
        let manager = ctx.with_fixture(|conn| {
            conn.browser_context
                .as_ref()
                .unwrap()
                .indexed_db_manager_for_test()
                .clone()
        });
        let key = first_party_storage_key_for_origin(origin);
        let client = seed_indexed_db_usage(&manager, &key);
        // Hold a real native transaction, so both the Window and Worker
        // requests remain admitted but unstarted until the CDP clear.
        let (sender, receiver) = tokio::sync::oneshot::channel();
        client
            .open_connection(Arc::new(|| {}), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        let moli_indexeddb::sqlite::service::CommandResult::Connection(connection) =
            receiver.await.unwrap().result.unwrap()
        else {
            panic!("expected a logical connection receipt");
        };
        let blocker = connection
            .queue_transaction_start(
                &["items".into()],
                moli_indexeddb::TransactionMode::ReadWrite,
                Arc::new(|| {}),
            )
            .unwrap();
        let backend = client.allocate_transaction_handle().unwrap();
        execute_indexed_db_command(
            &client,
            moli_indexeddb::sqlite::service::Command::Begin {
                transaction: backend,
                mode: moli_indexeddb::TransactionMode::ReadWrite,
                stores: std::collections::BTreeSet::from(["items".into()]),
                version: None,
                admission: Some(blocker.handle().clone()),
            },
        )
        .unwrap();
        let probe = r#"(send => {
  const opening = indexedDB.open("site-data", 1);
  opening.onerror = () => send("open-error:" + opening.error.name);
  opening.onsuccess = () => {
    const db = opening.result;
    const idleOpening = indexedDB.open("site-data", 1);
    idleOpening.onerror = () => send("idle-open-error:" + idleOpening.error.name);
    idleOpening.onsuccess = () => {
      const idle = idleOpening.result;
      globalThis.connections = [db, idle];
      const trace = [];
      let clearCompleted = false, activeClosed = false, idleCloses = 0;
      const tx = db.transaction("items");
      const request = tx.objectStore("items").get("key");
      request.onsuccess = () => trace.push("unexpected-success");
      request.onerror = event => { event.preventDefault(); trace.push("request:" + request.error.name); };
      tx.oncomplete = () => trace.push("unexpected-complete");
      tx.onabort = () => trace.push("abort:" + tx.error.name);
      const reopen = () => {
        if (!clearCompleted || !activeClosed || idleCloses !== 1) return;
        let oldVersion;
        const fresh = indexedDB.open("site-data", 1);
        fresh.onupgradeneeded = event => {
          oldVersion = event.oldVersion;
          fresh.result.createObjectStore("fresh");
        };
        fresh.onerror = () => send("reopen-error:" + fresh.error.name);
        fresh.onsuccess = () => {
          let closedReceiverError;
          try { db.transaction("items"); } catch (error) { closedReceiverError = error.name; }
          const stores = Array.from(fresh.result.objectStoreNames);
          fresh.result.close();
          send(JSON.stringify({stage:"done", trace, idleCloses,
            readyState:request.readyState, closedReceiverError, oldVersion, stores}));
        };
      };
      globalThis.completeIndexedDbClear = () => { clearCompleted = true; reopen(); };
      db.onclose = () => { trace.push("close"); activeClosed = true; reopen(); };
      idle.onclose = () => { idleCloses++; reopen(); };
      send(JSON.stringify({stage:"queued", readyState:request.readyState}));
    };
  };
})"#;
        let expression = if worker {
            let source = serde_json::to_string(&format!(
                r#"
({probe})(payload => postMessage(payload));
self.onmessage = event => {{
  if (event.data === "clear-completed") completeIndexedDbClear();
}};
"#
            ))
            .unwrap();
            format!(
                r#"
globalThis.cancelWorker = new Worker(URL.createObjectURL(new Blob([{source}], {{type:"text/javascript"}})));
cancelWorker.onmessage = event => {{
  globalThis.__moliStorageTestComplete(event.data);
  if (JSON.parse(event.data).stage === "done") cancelWorker.terminate();
}};
cancelWorker.onerror = event => globalThis.__moliStorageTestComplete("worker-error:" + event.message);
"scheduled";
"#
            )
        } else {
            format!("({probe})(globalThis.__moliStorageTestComplete); 'scheduled';")
        };
        let value = ctx
            .evaluate_fixture_expression(&expression, false)
            .await
            .unwrap();
        assert_eq!(value["value"], "scheduled");
        assert_eq!(
            serde_json::from_str::<Value>(&wait_for_storage_test_completion(&mut ctx).await)
                .unwrap(),
            json!({"stage":"queued", "readyState":"pending"}),
            "worker={worker}"
        );
        ctx.process_async(json!({
            "id": 92_010, "method":"Storage.clearDataForOrigin",
            "params":{"origin":origin, "storageTypes":"indexeddb"}
        }))
        .await;
        ctx.expect_result(92_010, json!({}), None);
        // A connection can be forced closed before all databases in this
        // owner's clear have finished. Reopen only after the CDP completion
        // releases that owner fence, using the real Window/Worker message path.
        let completed = if worker {
            "cancelWorker.postMessage('clear-completed'); 'notified';"
        } else {
            "completeIndexedDbClear(); 'notified';"
        };
        let value = ctx
            .evaluate_fixture_expression(completed, false)
            .await
            .unwrap();
        assert_eq!(value["value"], "notified");
        let result = wait_for_storage_test_completion(&mut ctx).await;
        assert_eq!(
            serde_json::from_str::<Value>(&result)
                .unwrap_or_else(|error| panic!("worker={worker}: {result}: {error}")),
            json!({
                "stage":"done", "trace":["request:AbortError", "abort:AbortError", "close"],
                "idleCloses":1, "readyState":"done", "closedReceiverError":"InvalidStateError",
                "oldVersion":0, "stores":["fresh"]
            }),
            "worker={worker}: {result}"
        );
        assert!(
            execute_indexed_db_command(
                &client,
                moli_indexeddb::sqlite::service::Command::Commit {
                    transaction: backend,
                    quota: None
                }
            )
            .is_err()
        );
        assert!(connection.is_forced_closed());
        assert!(!blocker.handle().is_ready());
    }
}

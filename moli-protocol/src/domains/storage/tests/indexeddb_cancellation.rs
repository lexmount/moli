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
        seed_indexed_db_usage(&manager, &key);
        // Hold the predecessor outside the JS event loop so the tested request
        // cannot become runnable before the CDP clear command is processed.
        let (blocker, backend) = {
            let mut manager = manager.lock();
            let db = manager
                .open(moli_core::storage::IndexedDbOpenOptions {
                    origin: key,
                    name: "site-data".into(),
                    version: Some(1),
                })
                .unwrap()
                .database;
            let blocker = manager
                .queue_transaction_start(
                    db,
                    &["items".into()],
                    moli_core::storage::IndexedDbTransactionMode::ReadWrite,
                    Arc::new(|| {}),
                )
                .unwrap();
            let backend = manager.start_queued_transaction(blocker.handle()).unwrap();
            (blocker, backend)
        };
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
      let activeClosed = false, idleCloses = 0;
      const tx = db.transaction("items");
      const request = tx.objectStore("items").get("key");
      request.onsuccess = () => trace.push("unexpected-success");
      request.onerror = event => { event.preventDefault(); trace.push("request:" + request.error.name); };
      tx.oncomplete = () => trace.push("unexpected-complete");
      tx.onabort = () => trace.push("abort:" + tx.error.name);
      const reopen = () => {
        if (!activeClosed || idleCloses !== 1) return;
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
      db.onclose = () => { trace.push("close"); activeClosed = true; reopen(); };
      idle.onclose = () => { idleCloses++; reopen(); };
      send(JSON.stringify({stage:"queued", readyState:request.readyState}));
    };
  };
})"#;
        let expression = if worker {
            let source =
                serde_json::to_string(&format!("({probe})(payload => postMessage(payload));"))
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
        let result = wait_for_storage_test_completion(&mut ctx).await;
        assert_eq!(
            serde_json::from_str::<Value>(&result).unwrap(),
            json!({
                "stage":"done", "trace":["request:AbortError", "abort:AbortError", "close"],
                "idleCloses":1, "readyState":"done", "closedReceiverError":"InvalidStateError",
                "oldVersion":0, "stores":["fresh"]
            }),
            "worker={worker}: {result}"
        );
        assert!(manager.lock().commit_transaction(backend).is_err());
        assert!(!blocker.handle().is_ready());
    }
}

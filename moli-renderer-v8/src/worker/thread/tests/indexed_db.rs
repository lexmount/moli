use super::super::WorkerGlobalKind;
use super::*;

#[tokio::test]
async fn indexed_db_lazy_blob_read_does_not_settle_after_worker_close() {
    ensure_v8();
    let manager = crate::new_indexed_db_manager(None).expect("worker IndexedDB manager");
    let source = r#"
(async () => {
  const request = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const completed = transaction => new Promise((resolve, reject) => {
    transaction.oncomplete = resolve;
    transaction.onabort = () => reject(transaction.error);
  });
  const open = indexedDB.open('close-blob', 1);
  open.onupgradeneeded = () => open.result.createObjectStore('values');
  const db = await request(open);
  const write = db.transaction('values', 'readwrite');
  const written = completed(write);
  write.objectStore('values').put(new Blob(['cached body']), 1);
  await written;

  const read = db.transaction('values');
  const readDone = completed(read);
  const blob = await request(read.objectStore('values').get(1));
  await readDone;
  const fileReaderText = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsText(blob);
  });
  if (fileReaderText !== 'cached body') throw new Error('wrong FileReader contents');
  const remove = db.transaction('values', 'readwrite');
  const removed = completed(remove);
  remove.objectStore('values').delete(1);
  await removed;
  db.close();

  // Deleting preserves the handed-out Blob. Its source can now complete the
  // read inline, while promise settlement still belongs to a future task.
  const readOrder = [];
  await Promise.all(Array.from({ length: 16 }, (_, index) => blob.text().then(text => {
    if (text !== 'cached body') throw new Error('wrong Blob contents');
    readOrder.push(index);
  })));
  postMessage(readOrder.join(','));

  if (closeWithFileReader) {
    const reader = new FileReader();
    reader.onloadstart = () => { postMessage('closing'); close(); };
    for (const type of ['progress', 'load', 'loadend', 'error', 'abort']) {
      reader.addEventListener(type, () => postMessage('late'));
    }
    reader.readAsText(blob);
    return;
  }
  let settled = false;
  const onRead = () => {
    if (settled) {
      postMessage('late');
    } else {
      settled = true;
      postMessage('closing');
      close();
    }
  };
  blob.text().then(onRead);
  blob.text().then(onRead);
})().catch(error => postMessage('error:' + error.name + ':' + error.message));
"#;
    for close_with_file_reader in [false, true] {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                format!("const closeWithFileReader = {close_with_file_reader};\n{source}"),
                "https://indexeddb-close-blob.test/worker.js".to_owned(),
            )
            .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
        );
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("worker Blob setup timeout")
            .expect("worker Blob setup closed");
        assert_eq!(
            expect_post_json(message),
            "\"0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15\""
        );
        let closing = timeout(TIMEOUT, handle.recv())
            .await
            .expect("worker Blob close timeout")
            .expect("worker Blob close notification");
        assert_eq!(expect_post_json(closing), "\"closing\"");
        let after_close = timeout(TIMEOUT, handle.recv())
            .await
            .expect("closed worker should release its parent channel");
        handle.terminate_and_join();
        assert!(
            after_close.is_none(),
            "callback ran after close: {after_close:?}"
        );
    }
}

#[tokio::test]
async fn indexed_db_global_attribute_preserves_receivers_and_identity_across_worker_kinds() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://indexeddb-attribute.test".to_owned(),
        "https://indexeddb-attribute.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let cases = [
        (
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            "https://indexeddb-attribute.test/worker.js",
        ),
        (
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            "data:text/javascript,",
        ),
        (
            WorkerGlobalKind::Shared {
                name: "indexedDB".to_owned(),
                storage_key,
            },
            "https://indexeddb-attribute.test/shared.js",
        ),
        (
            WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url::Url::parse("https://indexeddb-attribute.test/").unwrap(),
            },
            "https://indexeddb-attribute.test/sw.js",
        ),
    ];
    let fixture = include_str!("../../../../tests/fixtures/worker-indexeddb-attribute.js");
    for (kind, script_url) in cases {
        let (bootstrap_tx, mut bootstrap_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = format!(
            "{fixture}\nconst result = workerIndexedDBAttributeProbe(); if (result.state !== 'pass' || result.checks.length !== 53) throw new Error(JSON.stringify(result));"
        );
        let handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, script_url.to_owned())
                .with_global_kind(kind)
                .with_bootstrap_completion_sender(bootstrap_tx),
        );
        let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
            .await
            .expect("IndexedDB attribute probe should finish")
            .expect("IndexedDB attribute probe should report completion");
        handle.terminate_and_join();
        bootstrap.result.unwrap_or_else(|error| {
            panic!("IndexedDB attribute probe failed for {script_url}: {error:?}")
        });
    }
}

#[tokio::test]
async fn indexed_db_preserves_native_exception_data_across_worker_kinds() {
    ensure_v8();
    let manager = crate::new_indexed_db_manager(None).unwrap();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://indexeddb-exception-data.test".to_owned(),
        "https://indexeddb-exception-data.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for (kind, script_kind) in [
        (
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerScriptKind::Classic,
        ),
        (
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerScriptKind::Module,
        ),
        (
            WorkerGlobalKind::Shared {
                name: "exceptions".to_owned(),
                storage_key,
            },
            WorkerScriptKind::Classic,
        ),
        (
            WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url::Url::parse("https://indexeddb-exception-data.test/").unwrap(),
            },
            WorkerScriptKind::Classic,
        ),
    ] {
        let source = format!(
            "{}\nindexedDBExceptionPayloadProbe(globalThis).then(result => console.log(JSON.stringify(result)), error => console.log(JSON.stringify({{error: error.name + ':' + error.message}})));",
            include_str!("../../../../tests/fixtures/indexeddb-exception-payloads.js"),
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source,
                "https://indexeddb-exception-data.test/worker.js".to_owned(),
            )
            .with_global_kind(kind)
            .with_script_kind(script_kind)
            .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        handle.terminate_and_join();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected the completed exception data probe, got {message:?}");
        };
        let result: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        assert_eq!(result["rows"].as_array().unwrap().len(), 225, "{result}");
    }
}

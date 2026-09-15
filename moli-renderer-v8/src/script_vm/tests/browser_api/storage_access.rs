use super::*;

use super::service_worker_drain::drain_service_worker_test_until_eval_equals;

async fn spawn_storage_access_opfs_frame_server() -> (u16, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Storage Access OPFS frame server");
    let port = listener
        .local_addr()
        .expect("Storage Access OPFS frame server address")
        .port();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..3 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept Storage Access OPFS frame request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut chunk)
                    .await
                    .expect("read Storage Access OPFS frame request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let request_line = request.lines().next().unwrap_or_default().to_owned();
            let body = if request_line.starts_with("GET /outer.html ") {
                format!(
                    r#"<!doctype html><meta charset="utf-8"><body><script>
async function runStorageAccessCapabilityProbe() {{
  const access = await document.requestStorageAccess({{ all: true }});
  const root = await access.getDirectory();
  const crossSiteRootIsEmpty =
    await root.getFileHandle("first-party.txt").then(() => false, () => true);
  if (!crossSiteRootIsEmpty) {{
    throw new Error("cross-site capability unexpectedly exposed top-origin data");
  }}
  top.postMessage({{
    kind: "storage-access-cross-origin-probe",
    root
  }}, "*");
  await root.getFileHandle("sender-still-usable.txt", {{ create: true }});
  const frame = document.createElement("iframe");
  frame.src = "http://127.0.0.1:{port}/inner.html";
  (document.body || document.documentElement || document).appendChild(frame);
}}
addEventListener("message", event => {{
  if (event.data !== "storage-access-grant-ready") {{
    return;
  }}
  void runStorageAccessCapabilityProbe().catch(error => {{
    window.top.postMessage(JSON.stringify({{
      stage: "outer",
      error: `${{error && error.name}}:${{error && error.message}}`
    }}), "*");
  }});
}});
void document.requestStorageAccess({{ getDirectory: true }}).then(
  () => window.top.postMessage({{
    kind: "storage-access-default-request",
    outcome: "resolved"
  }}, "*"),
  error => window.top.postMessage({{
    kind: "storage-access-default-request",
    outcome: error && error.name
  }}, "*")
);
</script>"#
                )
            } else if request_line.starts_with("GET /inner.html ") {
                r#"<!doctype html><meta charset="utf-8"><script>
(async () => {
  const access = await document.requestStorageAccess({ getDirectory: true });
  const root = await access.getDirectory();
  const fileHandle = await root.getFileHandle("first-party.txt");
  const file = await fileHandle.getFile();
  const handleReadFirstPartyFile =
    file.name === "first-party.txt" && file.size === 0;
  const clonedRoot = structuredClone(root);
  const clonedFileHandle = await clonedRoot.getFileHandle("first-party.txt");
  const clonedHandleReadFirstPartyFile =
    (await clonedFileHandle.getFile()).name === "first-party.txt";
  const clonedHandlePermission =
    await clonedFileHandle.queryPermission({ mode: "readwrite" });
  const windowCapabilityFile =
    await clonedRoot.getFileHandle("window-capability.txt", { create: true });
  const windowCapabilityWritable = await windowCapabilityFile.createWritable();
  await windowCapabilityWritable.write("window capability");
  await windowCapabilityWritable.close();
  const windowCapabilityWrite =
    await (await windowCapabilityFile.getFile()).text() === "window capability";
  let iteratorSawSameEntry = false;
  for await (const entry of root.values()) {
    if (entry.name === "first-party.txt") {
      iteratorSawSameEntry = await entry.isSameEntry(fileHandle);
    }
  }
  const ambientRoot = await navigator.storage.getDirectory();
  const ambientHasFirstPartyFile =
    await ambientRoot.getFileHandle("first-party.txt").then(() => true, () => false);
  const partitionedFile =
    await ambientRoot.getFileHandle("partitioned-only.txt", { create: true });
  const partitionedWritable = await partitionedFile.createWritable();
  await partitionedWritable.write("partitioned");
  await partitionedWritable.close();
  top.postMessage({ kind: "ambient-opfs-handle-probe", root: ambientRoot }, "*");
  const workerCapability = await new Promise((resolve, reject) => {
    const worker = new Worker("/opfs-capability-worker.js");
    worker.onmessage = event => resolve(event.data);
    worker.onmessageerror = () => reject(new Error("worker-messageerror"));
    worker.onerror = event => reject(new Error(event.message || "worker-error"));
    worker.postMessage(root);
  });
  window.top.postMessage(JSON.stringify({
    stage: "inner",
    handleReadFirstPartyFile,
    clonedHandleReadFirstPartyFile,
    clonedHandlePermission,
    windowCapabilityWrite,
    iteratorSawSameEntry,
    ambientHasFirstPartyFile,
    workerCapability
  }), "*");
})().catch(error => {
  window.top.postMessage(JSON.stringify({
    stage: "inner",
    error: `${error && error.name}:${error && error.message}`
  }), "*");
});
</script>"#
                    .to_owned()
            } else if request_line.starts_with("GET /opfs-capability-worker.js ") {
                r#"self.onmessage = async event => {
  let stage = "received";
  try {
    const root = event.data;
    stage = "get-first-party-handle";
    const firstPartyFile = await root.getFileHandle("first-party.txt");
    stage = "read-first-party-file";
    const capabilityReadFirstPartyFile =
      (await firstPartyFile.getFile()).name === "first-party.txt";
    stage = "permission";
    const permission = await root.requestPermission({ mode: "readwrite" });
    stage = "ambient-root";
    const ambientRoot = await navigator.storage.getDirectory();
    const ambientHasFirstPartyFile =
      await ambientRoot.getFileHandle("first-party.txt").then(() => true, () => false);
    stage = "create-worker-file";
    const workerFile =
      await root.getFileHandle("worker-capability.txt", { create: true });
    stage = "write-worker-file";
    const writable = await workerFile.createWritable();
    await writable.write("worker capability");
    await writable.close();
    const wroteThroughCapability =
      await (await workerFile.getFile()).text() === "worker capability";
    stage = "create-sync-file";
    const syncFile =
      await root.getFileHandle("worker-sync-capability.bin", { create: true });
    stage = "create-sync-access-handle";
    const sync = await syncFile.createSyncAccessHandle();
    let syncRoundTrip;
    try {
      stage = "sync-write";
      const input = new Uint8Array([7, 8, 9]);
      const written = sync.write(input, { at: 0 });
      sync.flush();
      stage = "sync-read";
      const output = new Uint8Array(3);
      const read = sync.read(output, { at: 0 });
      syncRoundTrip =
        written === 3 &&
        read === 3 &&
        sync.getSize() === 3 &&
        output.join(",") === "7,8,9";
    } finally {
      sync.close();
    }
    stage = "iterate-root";
    let iteratorSawWorkerFile = false;
    for await (const entry of root.values()) {
      if (entry.name === "worker-capability.txt") {
        iteratorSawWorkerFile = await entry.isSameEntry(workerFile);
      }
    }
    postMessage({
      capabilityReadFirstPartyFile,
      permission,
      ambientHasFirstPartyFile,
      wroteThroughCapability,
      syncRoundTrip,
      iteratorSawWorkerFile
    });
  } catch (error) {
    postMessage({ stage, error: `${error && error.name}:${error && error.message}` });
  }
};"#
                .to_owned()
            } else {
                "<!doctype html><title>not found</title>".to_owned()
            };
            let status = if request_line.contains(" /outer.html ")
                || request_line.contains(" /inner.html ")
                || request_line.contains(" /opfs-capability-worker.js ")
            {
                "200 OK"
            } else {
                "404 Not Found"
            };
            let content_type = if request_line.contains(" /opfs-capability-worker.js ") {
                "text/javascript"
            } else {
                "text/html"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write Storage Access OPFS frame response");
            requests.push(request_line);
        }
        requests
    });
    (port, server)
}

#[test]
fn storage_access_handle_get_directory_surface_and_capability_gate() {
    let mut vm = new_storage_page_task_executor_test_vm("https://storage-access-opfs.test/");

    vm.exec(
        r#"
globalThis.__storageAccessOpfsProbe = "pending";
(async () => {
  const outcome = async promise => {
    try {
      await promise;
      return "resolved";
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };
  const empty = await outcome(document.requestStorageAccess({}));
  const allFalse = await outcome(document.requestStorageAccess({ all: false }));
  const unrelated = await document.requestStorageAccess({ indexedDB: true });
  const denied = await outcome(unrelated.getDirectory());
  const handle = await document.requestStorageAccess({ getDirectory: true });
  const root = await handle.getDirectory();
  const file = await root.getFileHandle("capability.txt", { create: true });
  return {
    constructorType: typeof StorageAccessHandle,
    constructorName: StorageAccessHandle.name,
    constructorRejects: (() => {
      try {
        new StorageAccessHandle();
        return false;
      } catch (error) {
        return error instanceof TypeError;
      }
    })(),
    handleBrand: handle instanceof StorageAccessHandle,
    tag: Object.prototype.toString.call(handle),
    getDirectoryName: StorageAccessHandle.prototype.getDirectory.name,
    getDirectoryLength: StorageAccessHandle.prototype.getDirectory.length,
    rootBrand: root instanceof FileSystemDirectoryHandle,
    fileName: file.name,
    empty,
    allFalse,
    denied
  };
})().then(
  value => { globalThis.__storageAccessOpfsProbe = JSON.stringify(value); },
  error => {
    globalThis.__storageAccessOpfsProbe =
      `error:${error && error.name}:${error && error.message}`;
  }
);
"scheduled"
        "#,
        None,
    )
    .expect("StorageAccessHandle OPFS probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageAccessOpfsProbe)")
        .expect("StorageAccessHandle OPFS probe should settle");
    assert_eq!(
        result,
        r#"{"constructorType":"function","constructorName":"StorageAccessHandle","constructorRejects":true,"handleBrand":true,"tag":"[object StorageAccessHandle]","getDirectoryName":"getDirectory","getDirectoryLength":0,"rootBrand":true,"fileName":"capability.txt","empty":"SecurityError:true","allFalse":"SecurityError:true","denied":"SecurityError:true"}"#
    );
}

#[tokio::test]
async fn nested_same_origin_iframe_can_read_first_party_opfs_only_through_storage_access() {
    let (port, server) = spawn_storage_access_opfs_frame_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let top_url = format!("http://127.0.0.1:{port}/top.html");
    let outer_url = format!("http://localhost:{port}/outer.html");
    let outer_url_literal =
        serde_json::to_string(&outer_url).expect("outer frame URL should serialize");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(&top_url, &loader);
    let top_storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &Url::parse(&top_url).expect("top frame URL should parse"),
        None,
    )
    .serialized_storage_key();
    let locator = moli_storage_service::StorageBucketLocator::default_bucket(top_storage_key);
    let storage_service = vm.storage_bucket_store.lock().storage_service();
    let bucket_key = moli_storage_service::StorageService::opfs_bucket_key(&locator)
        .expect("top frame OPFS bucket key should be valid");
    storage_service
        .with_opfs(|opfs| {
            let root = opfs.ensure_root(&bucket_key)?;
            opfs.get_file(&bucket_key, &root, "first-party.txt", true)
        })
        .expect("top frame first-party OPFS fixture should be created");

    vm.eval(&format!(
        r#"
globalThis.__storageAccessOpfsFrameResult = null;
globalThis.__storageAccessCrossOriginRejected = false;
globalThis.__ordinaryOpfsCloneResult = null;
globalThis.__storageAccessDefaultRequest = null;
addEventListener("message", event => {{
  if (event.data && event.data.kind === "storage-access-default-request") {{
    globalThis.__storageAccessDefaultRequest = event.data.outcome;
    return;
  }}
  if (event.data && event.data.kind === "ambient-opfs-handle-probe") {{
    void (async () => {{
      const clonedRoot = event.data.root;
      const partitionedFile =
        await clonedRoot.getFileHandle("partitioned-only.txt");
      const readPartitionedFile =
        await (await partitionedFile.getFile()).text() === "partitioned";
      const keptBackingLocator =
        await clonedRoot.getFileHandle("first-party.txt")
          .then(() => false, error => error && error.name === "NotFoundError");
      globalThis.__ordinaryOpfsCloneResult = JSON.stringify({{
        readPartitionedFile,
        keptBackingLocator
      }});
    }})().catch(error => {{
      globalThis.__ordinaryOpfsCloneResult = JSON.stringify({{
        error: `${{error && error.name}}:${{error && error.message}}`
      }});
    }});
    return;
  }}
  if (typeof event.data === "string") {{
    globalThis.__storageAccessOpfsFrameResult = event.data;
  }}
}});
addEventListener("messageerror", () => {{
  globalThis.__storageAccessCrossOriginRejected = true;
}});
const frame = document.createElement("iframe");
frame.src = {outer_url_literal};
(document.body || document.documentElement || document).appendChild(frame);
"#
    ))
    .expect("Storage Access OPFS frame setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        "String(globalThis.__storageAccessDefaultRequest !== null)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__storageAccessDefaultRequest")
            .expect("default third-party Storage Access result should be readable"),
        "NotAllowedError",
        "a passively embedded third-party frame must not receive first-party OPFS access"
    );

    let outer_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("outer third-party frame should have a live default realm")
        .context_id;
    let activated = vm
        .evaluate_expression_payload_in_context_with_await(
            Some(outer_context_id),
            r#"document.requestStorageAccess({ getDirectory: true }).then(
  () => "resolved",
  error => error && error.name
)"#,
            true,
            true,
            None,
        )
        .expect("third-party Storage Access request with activation should complete");
    assert_eq!(activated["type"], "string");
    assert_eq!(activated["value"], "resolved");

    let embedding_origin = Url::parse(&top_url)
        .expect("top frame URL should parse")
        .origin()
        .ascii_serialization();
    let requesting_origin = Url::parse(&outer_url)
        .expect("outer frame URL should parse")
        .origin()
        .ascii_serialization();
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::json!({ "name": "storage-access" }),
        setting: "denied".to_owned(),
        origin: Some(embedding_origin.clone()),
        embedded_origin: Some(requesting_origin.clone()),
    }]);
    let denied_with_activation = vm
        .evaluate_expression_payload_in_context_with_await(
            Some(outer_context_id),
            r#"document.requestStorageAccess({ getDirectory: true }).then(
  () => "resolved",
  error => error && error.name
)"#,
            true,
            true,
            None,
        )
        .expect("denied third-party Storage Access request should complete");
    assert_eq!(denied_with_activation["type"], "string");
    assert_eq!(denied_with_activation["value"], "NotAllowedError");

    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::json!({ "name": "storage-access" }),
        setting: "granted".to_owned(),
        origin: Some(embedding_origin),
        embedded_origin: Some(requesting_origin),
    }]);
    vm.eval(
        r#"document.querySelector("iframe").contentWindow.postMessage(
  "storage-access-grant-ready",
  "*"
)"#,
    )
    .expect("top frame should notify the granted third-party frame");

    drain_service_worker_test_until_eval_equals(&mut vm,
&browser_context_runtime,
"String(globalThis.__storageAccessOpfsFrameResult !== null && globalThis.__ordinaryOpfsCloneResult !== null)",
"true")
    .await;
    assert_eq!(
        vm.eval("globalThis.__storageAccessOpfsFrameResult")
            .expect("nested Storage Access OPFS result should be readable"),
        r#"{"stage":"inner","handleReadFirstPartyFile":true,"clonedHandleReadFirstPartyFile":true,"clonedHandlePermission":"granted","windowCapabilityWrite":true,"iteratorSawSameEntry":true,"ambientHasFirstPartyFile":false,"workerCapability":{"capabilityReadFirstPartyFile":true,"permission":"granted","ambientHasFirstPartyFile":false,"wroteThroughCapability":true,"syncRoundTrip":true,"iteratorSawWorkerFile":true}}"#
    );
    assert_eq!(
        vm.eval("globalThis.__ordinaryOpfsCloneResult")
            .expect("ordinary cross-StorageKey OPFS clone result should be readable"),
        r#"{"readPartitionedFile":true,"keptBackingLocator":true}"#
    );
    assert_eq!(
        vm.eval("String(globalThis.__storageAccessCrossOriginRejected)")
            .expect("cross-origin Storage Access clone result should be readable"),
        "true"
    );

    let requests = server
        .await
        .expect("Storage Access OPFS frame server should finish");
    assert_eq!(
        requests,
        vec![
            "GET /outer.html HTTP/1.1".to_owned(),
            "GET /inner.html HTTP/1.1".to_owned(),
            "GET /opfs-capability-worker.js HTTP/1.1".to_owned()
        ]
    );
}

#[test]
fn nested_first_party_storage_key_stays_partitioned_below_cross_site_parent() {
    let top = moli_storage_key::MoliStorageKey::first_party_from_url(
        &Url::parse("https://top.example/").unwrap(),
        None,
    );
    let cross_site_parent = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &Url::parse("https://third.example/").unwrap(),
        top.top_level_site().to_owned(),
        None,
    );
    let nested_top = moli_storage_key::MoliStorageKey::first_party_from_url(
        &Url::parse("https://top.example/nested").unwrap(),
        None,
    )
    .with_cross_site_ancestor();

    assert!(cross_site_parent.has_cross_site_ancestor());
    assert_eq!(nested_top.origin(), top.origin());
    assert_eq!(nested_top.top_level_site(), top.top_level_site());
    assert_ne!(
        nested_top.serialized_storage_key(),
        top.serialized_storage_key()
    );
}

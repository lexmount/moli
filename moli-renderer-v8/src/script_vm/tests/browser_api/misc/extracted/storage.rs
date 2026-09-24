use super::*;

#[test]
fn navigator_storage_apis_are_secure_context_only() {
    let mut vm = new_storage_test_vm("http://insecure-storage-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(navigator);
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const childProto = Object.getPrototypeOf(frame.contentWindow.navigator);
              return JSON.stringify({
                secure: globalThis.isSecureContext === true,
                clipboardInNavigator: "clipboard" in navigator,
                storageInNavigator: "storage" in navigator,
                storageBucketsInNavigator: "storageBuckets" in navigator,
                userAgentDataInNavigator: "userAgentData" in navigator,
                clipboardInProto: Object.prototype.hasOwnProperty.call(proto, "clipboard"),
                storageInProto: Object.prototype.hasOwnProperty.call(proto, "storage"),
                storageBucketsInProto: Object.prototype.hasOwnProperty.call(proto, "storageBuckets"),
                userAgentDataInProto:
                  Object.prototype.hasOwnProperty.call(proto, "userAgentData"),
                clipboardValueType: typeof navigator.clipboard,
                storageValueType: typeof navigator.storage,
                storageBucketsValueType: typeof navigator.storageBuckets,
                userAgentDataValueType: typeof navigator.userAgentData,
                clipboardGlobal: "Clipboard" in globalThis,
                clipboardItemGlobal: "ClipboardItem" in globalThis,
                storageManagerGlobal: "StorageManager" in globalThis,
                storageEstimateGlobal: "StorageEstimate" in globalThis,
                storageBucketManagerGlobal: "StorageBucketManager" in globalThis,
                storageBucketGlobal: "StorageBucket" in globalThis,
                fileSystemHandleGlobal: "FileSystemHandle" in globalThis,
                fileSystemFileHandleGlobal: "FileSystemFileHandle" in globalThis,
                fileSystemDirectoryHandleGlobal: "FileSystemDirectoryHandle" in globalThis,
                fileSystemWritableFileStreamGlobal:
                  "FileSystemWritableFileStream" in globalThis,
                fileSystemSyncAccessHandleGlobal:
                  "FileSystemSyncAccessHandle" in globalThis,
                childClipboardInNavigator: "clipboard" in frame.contentWindow.navigator,
                childStorageInNavigator: "storage" in frame.contentWindow.navigator,
                childStorageBucketsInNavigator: "storageBuckets" in frame.contentWindow.navigator,
                childUserAgentDataInNavigator:
                  "userAgentData" in frame.contentWindow.navigator,
                childClipboardInProto:
                  Object.prototype.hasOwnProperty.call(childProto, "clipboard"),
                childStorageInProto: Object.prototype.hasOwnProperty.call(childProto, "storage"),
                childStorageBucketsInProto: Object.prototype.hasOwnProperty.call(childProto, "storageBuckets"),
                childUserAgentDataInProto:
                  Object.prototype.hasOwnProperty.call(childProto, "userAgentData"),
                childClipboardGlobal: "Clipboard" in frame.contentWindow,
                childClipboardItemGlobal: "ClipboardItem" in frame.contentWindow,
                childFileSystemHandleGlobal:
                  "FileSystemHandle" in frame.contentWindow,
                childFileSystemFileHandleGlobal:
                  "FileSystemFileHandle" in frame.contentWindow,
                childFileSystemDirectoryHandleGlobal:
                  "FileSystemDirectoryHandle" in frame.contentWindow,
                childFileSystemWritableFileStreamGlobal:
                  "FileSystemWritableFileStream" in frame.contentWindow,
                childFileSystemSyncAccessHandleGlobal:
                  "FileSystemSyncAccessHandle" in frame.contentWindow
              });
            })()
            "#,
        )
        .expect("insecure navigator storage surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"secure":false,"clipboardInNavigator":false,"storageInNavigator":false,"storageBucketsInNavigator":false,"userAgentDataInNavigator":false,"clipboardInProto":false,"storageInProto":false,"storageBucketsInProto":false,"userAgentDataInProto":false,"clipboardValueType":"undefined","storageValueType":"undefined","storageBucketsValueType":"undefined","userAgentDataValueType":"undefined","clipboardGlobal":false,"clipboardItemGlobal":false,"storageManagerGlobal":false,"storageEstimateGlobal":false,"storageBucketManagerGlobal":false,"storageBucketGlobal":false,"fileSystemHandleGlobal":false,"fileSystemFileHandleGlobal":false,"fileSystemDirectoryHandleGlobal":false,"fileSystemWritableFileStreamGlobal":false,"fileSystemSyncAccessHandleGlobal":false,"childClipboardInNavigator":false,"childStorageInNavigator":false,"childStorageBucketsInNavigator":false,"childUserAgentDataInNavigator":false,"childClipboardInProto":false,"childStorageInProto":false,"childStorageBucketsInProto":false,"childUserAgentDataInProto":false,"childClipboardGlobal":false,"childClipboardItemGlobal":false,"childFileSystemHandleGlobal":false,"childFileSystemFileHandleGlobal":false,"childFileSystemDirectoryHandleGlobal":false,"childFileSystemWritableFileStreamGlobal":false,"childFileSystemSyncAccessHandleGlobal":false}"#
    );
}
#[tokio::test]
async fn data_url_frame_hides_secure_storage_and_opfs_interfaces() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://data-url-storage-exposure.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__dataUrlStorageExposureProbe = "pending";
  addEventListener("message", event => {
    if (event.data && event.data.probe === "data-url-storage-exposure") {
      globalThis.__dataUrlStorageExposureProbe = JSON.stringify(event.data);
    }
  });

  const frame = document.createElement("iframe");
  const childSource = `<script>
    parent.postMessage({
      probe: "data-url-storage-exposure",
      secure: isSecureContext,
      storageInNavigator: "storage" in navigator,
      storageManagerGlobal: "StorageManager" in globalThis,
      storageEstimateGlobal: "StorageEstimate" in globalThis,
      storageBucketManagerGlobal: "StorageBucketManager" in globalThis,
      storageBucketGlobal: "StorageBucket" in globalThis,
      fileSystemHandleGlobal: "FileSystemHandle" in globalThis,
      fileSystemFileHandleGlobal: "FileSystemFileHandle" in globalThis,
      fileSystemDirectoryHandleGlobal: "FileSystemDirectoryHandle" in globalThis,
      fileSystemWritableFileStreamGlobal:
        "FileSystemWritableFileStream" in globalThis,
      fileSystemSyncAccessHandleGlobal:
        "FileSystemSyncAccessHandle" in globalThis
    }, "*");
  <\/script>`;
  frame.src = `data:text/html,${encodeURIComponent(childSource)}`;
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("data URL storage exposure setup should evaluate");
    assert_eq!(setup, "queued");

    for _ in 0..8 {
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let result = vm
            .eval("String(globalThis.__dataUrlStorageExposureProbe)")
            .expect("data URL storage exposure result should evaluate");
        if result != "pending" {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("data URL storage exposure child load should advance");
    }

    let result = vm
        .eval("String(globalThis.__dataUrlStorageExposureProbe)")
        .expect("data URL storage exposure result should settle");
    assert_eq!(
        result,
        r#"{"probe":"data-url-storage-exposure","secure":false,"storageInNavigator":false,"storageManagerGlobal":false,"storageEstimateGlobal":false,"storageBucketManagerGlobal":false,"storageBucketGlobal":false,"fileSystemHandleGlobal":false,"fileSystemFileHandleGlobal":false,"fileSystemDirectoryHandleGlobal":false,"fileSystemWritableFileStreamGlobal":false,"fileSystemSyncAccessHandleGlobal":false}"#
    );
}
#[test]
fn navigator_legacy_webkit_storage_quota_surface_queues_callbacks_asynchronously() {
    let mut vm = new_storage_test_vm("https://legacy-webkit-storage-quota.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              const temporary = navigator.webkitTemporaryStorage;
              const persistent = navigator.webkitPersistentStorage;
              temporary.queryUsageAndQuota((usage, quota) => {
                calls.push(`temporary:${usage}:${quota}`);
              });
              persistent.requestQuota(4096, granted => {
                calls.push(`persistent:${granted}`);
              });
              webkitStorageInfo.queryUsageAndQuota(TEMPORARY, (usage, quota) => {
                calls.push(`info:${usage}:${quota}`);
              });
              webkitStorageInfo.requestQuota(PERSISTENT, 8192, granted => {
                calls.push(`info-request:${granted}`);
              });
              return JSON.stringify({
                temporary: [
                  temporary instanceof Object,
                  Object.prototype.toString.call(temporary),
                  typeof temporary.queryUsageAndQuota,
                  temporary.queryUsageAndQuota.length,
                  typeof temporary.requestQuota,
                  temporary.requestQuota.length
                ].join("|"),
                persistent: [
                  persistent instanceof Object,
                  Object.prototype.toString.call(persistent),
                  typeof persistent.queryUsageAndQuota,
                  persistent.queryUsageAndQuota.length,
                  typeof persistent.requestQuota,
                  persistent.requestQuota.length
                ].join("|"),
                window: [
                  TEMPORARY,
                  PERSISTENT,
                  Object.prototype.toString.call(webkitStorageInfo),
                  typeof webkitStorageInfo.queryUsageAndQuota,
                  webkitStorageInfo.queryUsageAndQuota.length,
                  typeof webkitStorageInfo.requestQuota,
                  webkitStorageInfo.requestQuota.length,
                  webkitStorageInfo.TEMPORARY,
                  webkitStorageInfo.PERSISTENT
                ].join("|"),
                calls
              });
            })()
            "#,
        )
        .expect("legacy webkit storage quota callbacks should evaluate");

    assert_eq!(
        result,
        r#"{"temporary":"true|[object Object]|function|1|function|2","persistent":"true|[object Object]|function|1|function|2","window":"0|1|[object Object]|function|2|function|3|0|1","calls":[]}"#
    );
}
#[test]
fn navigator_storage_persist_does_not_grant_persistent_storage_by_default() {
    let mut vm = new_storage_test_vm("https://navigator-storage-persist.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__storagePersistGrantProbe = "pending";
          Promise.all([
            navigator.storage.persist(),
            navigator.storage.persisted(),
            navigator.permissions.query({ name: "persistent-storage" })
          ]).then(([persist, persisted, permission]) => {
            globalThis.__storagePersistGrantProbe = [
              persist,
              persisted,
              permission instanceof PermissionStatus,
              permission.name,
              permission.state
            ].join("|");
          }, error => {
            globalThis.__storagePersistGrantProbe = `error:${error && error.name}`;
          });
        })()
        "#,
    )
    .expect("StorageManager persist grant probe should evaluate");

    let result = vm
        .eval("String(globalThis.__storagePersistGrantProbe)")
        .expect("StorageManager persist grant promise should settle");

    assert_eq!(result, "false|false|true|persistent-storage|granted");
}
#[tokio::test]
async fn navigator_storage_methods_reject_in_opaque_origin_frame() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-manager-opaque.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__storageManagerOpaqueMessages = [];
  addEventListener("message", event => {
    __storageManagerOpaqueMessages.push(String(event.data));
  });

  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.srcdoc = `<script>
    const outcome = async (label, promise) => {
      try {
        await promise;
        parent.postMessage(label + ":resolved", "*");
      } catch (error) {
        parent.postMessage(label + ":" + (error && error.name), "*");
      }
    };
    Promise.all([
      outcome("persisted", navigator.storage.persisted()),
      outcome("persist", navigator.storage.persist()),
      outcome("estimate", navigator.storage.estimate()),
      outcome("getDirectory", navigator.storage.getDirectory())
    ]);
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("opaque StorageManager setup should evaluate");
    assert_eq!(setup, "queued");

    for _ in 0..8 {
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let message_count = vm
            .eval("String(__storageManagerOpaqueMessages.length)")
            .expect("opaque StorageManager message count should evaluate");
        if message_count == "4" {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque StorageManager child load should advance");
    }

    let result = vm
        .eval("JSON.stringify(__storageManagerOpaqueMessages.sort())")
        .expect("opaque StorageManager messages should evaluate");
    assert_eq!(
        result,
        r#"["estimate:TypeError","getDirectory:SecurityError","persist:TypeError","persisted:TypeError"]"#
    );
}
#[test]
fn navigator_storage_webidl_receiver_checks_match_promise_shape() {
    let mut vm = new_storage_test_vm("https://navigator-storage-receiver.test/");

    vm.eval(
        r#"
            (() => {
              const probe = {
                getter: "pending",
                estimateReturn: "pending",
                estimateReject: "pending",
                persistedReturn: "pending",
                persistedReject: "pending",
                persistReturn: "pending",
                persistReject: "pending",
                getDirectoryReturn: "pending",
                getDirectoryReject: "pending"
              };
              globalThis.__navigatorStorageReceiverProbe = probe;

              const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, "storage").get;
              try {
                getter.call(Navigator.prototype);
                probe.getter = "accepted";
              } catch (error) {
                probe.getter = error && error.name;
              }

              for (const method of ["estimate", "persisted", "persist", "getDirectory"]) {
                try {
                  const promise = StorageManager.prototype[method].call({});
                  probe[`${method}Return`] = Object.prototype.toString.call(promise);
                  promise.then(
                    () => { probe[`${method}Reject`] = "resolved"; },
                    (error) => { probe[`${method}Reject`] = error && error.name; }
                  );
                } catch (error) {
                  probe[`${method}Return`] = `throw:${error && error.name}`;
                  probe[`${method}Reject`] = "not-called";
                }
              }
            })()
            "#,
    )
    .expect("navigator storage receiver probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__navigatorStorageReceiverProbe)")
        .expect("navigator storage receiver promises should settle");

    assert_eq!(
        result,
        r#"{"getter":"TypeError","estimateReturn":"[object Promise]","estimateReject":"TypeError","persistedReturn":"[object Promise]","persistedReject":"TypeError","persistReturn":"[object Promise]","persistReject":"TypeError","getDirectoryReturn":"[object Promise]","getDirectoryReject":"TypeError"}"#
    );
}
#[test]
fn navigator_storage_buckets_minimal_surface_matches_idlharness() {
    let mut vm = new_storage_test_vm("https://storage-buckets-idl.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketsSurfaceProbe = "pending";
        (async () => {
          const manager = navigator.storageBuckets;
          const managerProto = StorageBucketManager.prototype;
          const bucketProto = StorageBucket.prototype;
          const promiseOutcome = async (fn, receiver, ...args) => {
            let value;
            try {
              value = fn.call(receiver, ...args);
            } catch (error) {
              return `throw:${error && error.name}`;
            }
            const isPromise = value instanceof Promise;
            try {
              await value;
              return `resolved:${isPromise}`;
            } catch (error) {
              return `rejected:${isPromise}:${error && error.name}`;
            }
          };
          const attributeOutcome = (getter, receiver) => {
            try {
              getter.call(receiver);
              return "accepted";
            } catch (error) {
              return error && error.name;
            }
          };
          await manager.open("bucket_name3");
          const bucket = await manager.open("bucket_name1");
          await manager.open("bucket_name2");
          const keysBeforeDelete = await manager.keys();
          await manager.delete("bucket_name3");
          await manager.delete("missing");
          const keysAfterDelete = await manager.keys();
          const nameGetter = Object.getOwnPropertyDescriptor(bucketProto, "name").get;
          return {
            managerSameObject: manager === navigator.storageBuckets,
            managerType: typeof manager,
            managerTag: Object.prototype.toString.call(manager),
            managerInstanceof: manager instanceof StorageBucketManager,
            managerOwnOpen: Object.hasOwn(manager, "open"),
            openName: managerProto.open.name,
            openLength: managerProto.open.length,
            keysName: managerProto.keys.name,
            keysLength: managerProto.keys.length,
            deleteName: managerProto.delete.name,
            deleteLength: managerProto.delete.length,
            bucketTag: Object.prototype.toString.call(bucket),
            bucketInstanceof: bucket instanceof StorageBucket,
            bucketName: bucket.name,
            bucketPersistType: typeof bucket.persist,
            bucketPersistLength: bucket.persist.length,
            bucketPersistedLength: bucket.persisted.length,
            bucketEstimateLength: bucket.estimate.length,
            bucketDurabilityLength: bucket.durability.length,
            bucketSetExpiresLength: bucket.setExpires.length,
            bucketExpiresLength: bucket.expires.length,
            bucketGetDirectoryLength: bucket.getDirectory.length,
            keysBeforeDelete,
            keysAfterDelete,
            managerIllegalReceiver: await promiseOutcome(managerProto.keys, {}),
            bucketIllegalReceiver: await promiseOutcome(bucketProto.persisted, {}),
            bucketNameIllegalReceiver: attributeOutcome(nameGetter, {})
          };
        })().then(
          value => { globalThis.__storageBucketsSurfaceProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketsSurfaceProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage buckets surface probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketsSurfaceProbe)")
        .expect("storage buckets surface probe should settle");

    assert_eq!(
        result,
        r#"{"managerSameObject":true,"managerType":"object","managerTag":"[object StorageBucketManager]","managerInstanceof":true,"managerOwnOpen":false,"openName":"open","openLength":1,"keysName":"keys","keysLength":0,"deleteName":"delete","deleteLength":1,"bucketTag":"[object StorageBucket]","bucketInstanceof":true,"bucketName":"bucket_name1","bucketPersistType":"function","bucketPersistLength":0,"bucketPersistedLength":0,"bucketEstimateLength":0,"bucketDurabilityLength":0,"bucketSetExpiresLength":1,"bucketExpiresLength":0,"bucketGetDirectoryLength":0,"keysBeforeDelete":["bucket_name1","bucket_name2","bucket_name3"],"keysAfterDelete":["bucket_name1","bucket_name2"],"managerIllegalReceiver":"rejected:true:TypeError","bucketIllegalReceiver":"rejected:true:TypeError","bucketNameIllegalReceiver":"TypeError"}"#
    );
}
#[test]
fn storage_bucket_manager_rejects_invalid_chromium_bucket_names() {
    let mut vm = new_storage_test_vm("https://storage-bucket-invalid-names.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketInvalidNameProbe = "pending";
        (async () => {
          const manager = navigator.storageBuckets;
          const name64 = "a".repeat(64);
          const outcome = async (label, promise) => {
            try {
              await promise;
              return `${label}:resolved`;
            } catch (error) {
              return `${label}:${error && error.name}`;
            }
          };
          const openResults = await Promise.all([
            outcome("empty", manager.open("")),
            outcome("uppercase", manager.open("Upper")),
            outcome("nonascii", manager.open("caf\u00e9")),
            outcome("leading-dash", manager.open("-bucket")),
            outcome("leading-underscore", manager.open("_bucket")),
            outcome("length64", manager.open(name64)),
            outcome("valid-dash", manager.open("bucket-a")),
            outcome("valid-underscore", manager.open("bucket_a"))
          ]);
          const deleteResults = await Promise.all([
            outcome("delete-uppercase", manager.delete("Upper")),
            outcome("delete-valid", manager.delete("bucket-a"))
          ]);
          const keys = await manager.keys();
          return { openResults, deleteResults, keys };
        })().then(
          value => { globalThis.__storageBucketInvalidNameProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketInvalidNameProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket invalid-name probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketInvalidNameProbe)")
        .expect("storage bucket invalid-name probe should settle");

    assert_eq!(
        result,
        r#"{"openResults":["empty:TypeError","uppercase:TypeError","nonascii:TypeError","leading-dash:TypeError","leading-underscore:TypeError","length64:TypeError","valid-dash:resolved","valid-underscore:resolved"],"deleteResults":["delete-uppercase:TypeError","delete-valid:resolved"],"keys":["bucket_a"]}"#
    );
}
#[test]
fn storage_bucket_indexeddb_is_bucket_scoped() {
    let mut vm = new_storage_page_task_executor_test_vm("https://storage-bucket-idb.test/");
    let db_name = format!(
        "storage-bucket-idb-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos()
    );

    vm.exec(
        &r#"
        globalThis.__storageBucketIndexedDbProbe = "pending";
        (async () => {
          const dbName = "__MOLI_BUCKET_IDB_DB_NAME__";
          const openDb = (factory, name) => new Promise((resolve, reject) => {
            const open = factory.open(name, 1);
            let upgraded = false;
            open.onerror = () => reject(`open:${open.error && open.error.name}`);
            open.onupgradeneeded = () => {
              upgraded = true;
              open.result.createObjectStore("kv");
            };
            open.onsuccess = () => resolve({ db: open.result, upgraded });
          });
          const putValue = (db, value) => new Promise((resolve, reject) => {
            const tx = db.transaction("kv", "readwrite");
            tx.objectStore("kv").put(value, "key");
            tx.onerror = () => reject(`put:${tx.error && tx.error.name}`);
            tx.oncomplete = resolve;
          });
          const getValue = (db) => new Promise((resolve, reject) => {
            const tx = db.transaction("kv", "readonly");
            const get = tx.objectStore("kv").get("key");
            get.onerror = () => reject(`get:${get.error && get.error.name}`);
            get.onsuccess = () => resolve(get.result ?? null);
          });

          const bucketA = await navigator.storageBuckets.open("a");
          const bucketB = await navigator.storageBuckets.open("b");
          const openedA = await openDb(bucketA.indexedDB, dbName);
          await putValue(openedA.db, "bucket-a");
          openedA.db.close();

          const readA = await openDb(bucketA.indexedDB, dbName);
          const valueA = await getValue(readA.db);
          readA.db.close();

          const openedB = await openDb(bucketB.indexedDB, dbName);
          openedB.db.close();
          const openedGlobal = await openDb(indexedDB, dbName);
          openedGlobal.db.close();
          const estimate = await bucketA.estimate();

          await navigator.storageBuckets.delete("a");
          const reopenedA = await openDb((await navigator.storageBuckets.open("a")).indexedDB, dbName);
          reopenedA.db.close();

          return {
            bucketFactoryDistinct: bucketA.indexedDB !== indexedDB,
            bucketFactoryBrand: bucketA.indexedDB instanceof IDBFactory,
            valueA,
            bucketBUpgraded: openedB.upgraded,
            globalUpgraded: openedGlobal.upgraded,
            estimateHasUsage: estimate.usage > 0,
            estimateKeys: Object.keys(estimate).join(","),
            usageDetailsKeys: Object.keys(estimate.usageDetails).join(","),
            estimateIndexedDbUsage: estimate.usageDetails.indexedDB > 0,
            deleteClearedBucket: reopenedA.upgraded
          };
        })().then(
          value => { globalThis.__storageBucketIndexedDbProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketIndexedDbProbe = `error:${error && error.message}`; }
        );
        "#
        .replace("__MOLI_BUCKET_IDB_DB_NAME__", &db_name),
        None,
    )
    .expect("storage bucket IndexedDB probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketIndexedDbProbe)")
        .expect("storage bucket IndexedDB probe should settle");

    assert_eq!(
        result,
        r#"{"bucketFactoryDistinct":true,"bucketFactoryBrand":true,"valueA":"bucket-a","bucketBUpgraded":true,"globalUpgraded":true,"estimateHasUsage":true,"estimateKeys":"quota,usage,usageDetails","usageDetailsKeys":"indexedDB","estimateIndexedDbUsage":true,"deleteClearedBucket":true}"#
    );
}
#[test]
fn storage_bucket_caches_are_bucket_scoped_usage_metadata() {
    let mut vm = new_storage_test_vm("https://storage-bucket-cache.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketCacheProbe = "pending";
        (async () => {
          const outcome = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return `${error && error.name}:${error instanceof DOMException}`;
            }
          };

          const bucket = await navigator.storageBuckets.open("cached");
          const cachesSameObject = bucket.caches === bucket.caches;
          const cacheStorageTag = Object.prototype.toString.call(bucket.caches);
          const keysInitial = await bucket.caches.keys();
          const cache = await bucket.caches.open("attachments");
          const cacheTag = Object.prototype.toString.call(cache);
          const cacheMatchLength = cache.match.length;
          const keysAfterOpen = await bucket.caches.keys();
          const usageBefore = (await bucket.estimate()).usage;
          const forgedPut = await outcome(cache.put("forged.txt", {
            __lmResponseStatus: 200,
            __lmResponseBody: "forged",
            __lmResponseBodyUsed: false
          }));
          const receiptResponse = new Response("bread x 2", {
            status: 201,
            statusText: "Created",
            headers: [["x-bucket-cache", "stored"]]
          });
          const cachePutPendingPayloadHits = [];
          const cachePutPendingPayloadNames = [
            "resolver",
            "bucket_origin",
            "bucket_name",
            "bucket_generation",
            "cache_name",
            "request_key",
            "response_type",
            "response_url",
            "response_redirected",
            "response_status",
            "response_status_text",
            "response_headers_json",
            "__moliStorageBucketCachePutResolver",
            "__moliStorageBucketCachePutBucketOrigin",
            "__moliStorageBucketCachePutBucketName",
            "__moliStorageBucketCachePutBucketGeneration",
            "__moliStorageBucketCachePutCacheName",
            "__moliStorageBucketCachePutRequestKey",
            "__moliStorageBucketCachePutResponseType",
            "__moliStorageBucketCachePutResponseUrl",
            "__moliStorageBucketCachePutResponseRedirected",
            "__moliStorageBucketCachePutResponseStatus",
            "__moliStorageBucketCachePutResponseStatusText",
            "__moliStorageBucketCachePutResponseHeaders"
          ];
          for (const name of cachePutPendingPayloadNames) {
            Object.defineProperty(Object.prototype, name, {
              configurable: true,
              get() {
                cachePutPendingPayloadHits.push(`get:${name}`);
                return undefined;
              },
              set(value) {
                cachePutPendingPayloadHits.push(`set:${name}`);
                Object.defineProperty(this, name, {
                  configurable: true,
                  enumerable: true,
                  writable: true,
                  value
                });
              }
            });
          }
          try {
            await cache.put("receipt1.txt", receiptResponse);
          } finally {
            for (const name of cachePutPendingPayloadNames) {
              delete Object.prototype[name];
            }
          }
          const receiptResponseBodyUsedAfterPut = receiptResponse.bodyUsed;
          const reusedResponsePut = await outcome(cache.put("receipt2.txt", receiptResponse));
          const afterPut = await bucket.estimate();
          const responseInitSetterHits = [];
          const responseInitNames = ["status", "statusText", "headers"];
          for (const name of responseInitNames) {
            Object.defineProperty(Object.prototype, name, {
              configurable: true,
              get() { return undefined; },
              set(value) {
                const receiverKind = this instanceof Response ? "response" : "plain";
                responseInitSetterHits.push(`${receiverKind}:${name}`);
                Object.defineProperty(this, name, {
                  configurable: true,
                  enumerable: true,
                  writable: true,
                  value
                });
              }
            });
          }
          let matched;
          let matchedText;
          let missingMatch;
          try {
            matched = await cache.match("receipt1.txt");
            matchedText = await matched.text();
            missingMatch = await cache.match("missing.txt");
          } finally {
            for (const name of responseInitNames) {
              delete Object.prototype[name];
            }
          }

          const sibling = await navigator.storageBuckets.open("sibling");
          const siblingKeys = await sibling.caches.keys();
          const siblingUsage = (await sibling.estimate()).usage;

          const deletedCache = await bucket.caches.delete("attachments");
          const keysAfterCacheDelete = await bucket.caches.keys();
          const usageAfterCacheDelete = (await bucket.estimate()).usage;

          await navigator.storageBuckets.delete("cached");
          const stalePut = await outcome(cache.put("stale", new Response("x")));
          const staleMatch = await outcome(cache.match("receipt1.txt"));
          const recreated = await navigator.storageBuckets.open("cached");
          const recreatedKeys = await recreated.caches.keys();
          const recreatedUsage = (await recreated.estimate()).usage;

          await navigator.storageBuckets.delete("cached");
          await navigator.storageBuckets.delete("sibling");

          return {
            cachesSameObject,
            cacheStorageTag,
            keysInitial,
            cacheTag,
            cacheMatchLength,
            keysAfterOpen,
            forgedPut,
            receiptResponseBodyUsedAfterPut,
            reusedResponsePut,
            usageIncreased: afterPut.usage > usageBefore,
            cacheUsageDetail: afterPut.usageDetails.caches > 0,
            usageDetailsKeys: Object.keys(afterPut.usageDetails).join(","),
            cachePutPendingPayloadHits,
            matchedStatus: matched.status,
            matchedStatusText: matched.statusText,
            matchedHeader: matched.headers.get("x-bucket-cache"),
            matchedText,
            responseInitSetterHits: responseInitSetterHits
              .filter(hit => hit.startsWith("plain:")),
            missingMatchType: typeof missingMatch,
            siblingKeys,
            siblingUsage,
            deletedCache,
            keysAfterCacheDelete,
            usageAfterCacheDelete,
            stalePut,
            staleMatch,
            recreatedKeys,
            recreatedUsage
          };
        })().then(
          value => { globalThis.__storageBucketCacheProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketCacheProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket CacheStorage probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketCacheProbe)")
        .expect("storage bucket CacheStorage probe should settle");

    assert_eq!(
        result,
        r#"{"cachesSameObject":true,"cacheStorageTag":"[object CacheStorage]","keysInitial":[],"cacheTag":"[object Cache]","cacheMatchLength":1,"keysAfterOpen":["attachments"],"forgedPut":"TypeError:false","receiptResponseBodyUsedAfterPut":true,"reusedResponsePut":"TypeError:false","usageIncreased":true,"cacheUsageDetail":true,"usageDetailsKeys":"caches","cachePutPendingPayloadHits":[],"matchedStatus":201,"matchedStatusText":"Created","matchedHeader":"stored","matchedText":"bread x 2","responseInitSetterHits":[],"missingMatchType":"undefined","siblingKeys":[],"siblingUsage":0,"deletedCache":true,"keysAfterCacheDelete":[],"usageAfterCacheDelete":0,"stalePut":"UnknownError:true","staleMatch":"UnknownError:true","recreatedKeys":[],"recreatedUsage":0}"#
    );
}
#[test]
fn storage_bucket_expiration_is_bucket_metadata() {
    let mut vm = new_storage_page_task_executor_test_vm("https://storage-bucket-expires.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketExpiresProbe = "pending";
        (async () => {
          const manager = navigator.storageBuckets;
          const day = 24 * 60 * 60 * 1000;
          const outcome = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return `${error && error.name}:${error instanceof DOMException}`;
            }
          };
          const typeErrorOutcome = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return `${error && error.name}:${error instanceof TypeError}`;
            }
          };
          const fromOpenExpires = Date.now() + day;
          const fromOpen = await navigator.storageBuckets.open("from-open", {
            expires: fromOpenExpires
          });
          const fromOpenInitialMatches = await fromOpen.expires() === fromOpenExpires;
          const nullOpen = await navigator.storageBuckets.open("null-open", { expires: null });
          const nullOpenExpires = await nullOpen.expires();
          const fromOpenUpdatedExpires = Date.now() + (2 * day);
          await fromOpen.setExpires(fromOpenUpdatedExpires);
          const fromOpenUpdatedMatches =
            await (await navigator.storageBuckets.open("from-open")).expires() ===
            fromOpenUpdatedExpires;
          const first = await navigator.storageBuckets.open("expiring");
          const initial = await first.expires();
          const expiringExpires = Date.now() + (3 * day);
          await first.setExpires(expiringExpires);
          const reopened = await navigator.storageBuckets.open("expiring");
          const afterReopenMatches = await reopened.expires() === expiringExpires;
          await navigator.storageBuckets.delete("expiring");
          const recreated = await navigator.storageBuckets.open("expiring");
          const afterDelete = await recreated.expires();
          const clearable = await navigator.storageBuckets.open("clearable", {
            expires: Date.now() + (4 * day)
          });
          await clearable.setExpires(null);
          const clearedExpires = await clearable.expires();
          const clearableStillLive = await outcome(clearable.durability());

          const invalidPastOpen = await typeErrorOutcome(
            manager.open("past-open", { expires: Date.now() - 1 })
          );
          const pruned = await manager.open("pruned");
          const prunedRoot = await pruned.getDirectory();
          const prunedFile = await prunedRoot.getFileHandle("expired.txt", { create: true });
          const prunedWriter = await prunedFile.createWritable();
          await prunedWriter.write("expired bytes");
          await prunedWriter.close();
          const setPastOutcome = await outcome(pruned.setExpires(Date.now() - 1));
          const keysAfterExpired = await manager.keys();
          const staleAfterExpire = await outcome(pruned.expires());
          const staleRootAfterExpire = await outcome(prunedRoot.getFileHandle("expired.txt"));
          const recreatedAfterExpire = await manager.open("pruned");
          const recreatedAfterExpireExpires = await recreatedAfterExpire.expires();
          const recreatedAfterExpireRoot = await recreatedAfterExpire.getDirectory();
          const recreatedAfterExpireFile = await outcome(
            recreatedAfterExpireRoot.getFileHandle("expired.txt")
          );
          const oldAfterRecreate = await outcome(pruned.expires());

          await manager.delete("from-open");
          await manager.delete("null-open");
          await manager.delete("expiring");
          await manager.delete("clearable");
          await manager.delete("pruned");

          return {
            fromOpenInitialMatches,
            nullOpenExpires,
            fromOpenUpdatedMatches,
            initial,
            afterReopenMatches,
            afterDelete,
            clearedExpires,
            clearableStillLive,
            invalidPastOpen,
            setPastOutcome,
            keysAfterExpiredContainsPruned: keysAfterExpired.includes("pruned"),
            staleAfterExpire,
            staleRootAfterExpire,
            recreatedAfterExpireExpires,
            recreatedAfterExpireFile,
            oldAfterRecreate
          };
        })().then(
          value => { globalThis.__storageBucketExpiresProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketExpiresProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket expiration probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketExpiresProbe)")
        .expect("storage bucket expiration probe should settle");

    assert_eq!(
        result,
        r#"{"fromOpenInitialMatches":true,"nullOpenExpires":null,"fromOpenUpdatedMatches":true,"initial":null,"afterReopenMatches":true,"afterDelete":null,"clearedExpires":null,"clearableStillLive":"resolved","invalidPastOpen":"TypeError:true","setPastOutcome":"resolved","keysAfterExpiredContainsPruned":false,"staleAfterExpire":"UnknownError:true","staleRootAfterExpire":"NotFoundError:true","recreatedAfterExpireExpires":null,"recreatedAfterExpireFile":"NotFoundError:true","oldAfterRecreate":"UnknownError:true"}"#
    );
}
#[test]
fn storage_bucket_durability_is_bucket_metadata() {
    let mut vm = new_storage_test_vm("https://storage-bucket-durability.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketDurabilityProbe = "pending";
        (async () => {
          const outcome = async (promise) => {
            try {
              return await promise;
            } catch (error) {
              return `${error && error.name}:${error instanceof DOMException}`;
            }
          };

          const strict = await navigator.storageBuckets.open("durable", {
            durability: "strict"
          });
          const strictInitial = await strict.durability();
          const strictReopened = await (await navigator.storageBuckets.open("durable")).durability();
          await navigator.storageBuckets.delete("durable");
          const staleAfterDelete = await outcome(strict.durability());

          const recreated = await navigator.storageBuckets.open("durable");
          const oldAfterRecreate = await outcome(strict.durability());
          const recreatedDurability = await recreated.durability();
          await navigator.storageBuckets.delete("durable");

          const explicitRelaxed = await navigator.storageBuckets.open("relaxed", {
            durability: "relaxed"
          });
          const explicitRelaxedDurability = await explicitRelaxed.durability();
          await navigator.storageBuckets.delete("relaxed");

          const invalidDurability = await navigator.storageBuckets.open("bad", {
            durability: "durable"
          }).then(
            () => "resolved",
            error => `${error && error.name}:${error instanceof TypeError}`
          );

          return {
            strictInitial,
            strictReopened,
            staleAfterDelete,
            oldAfterRecreate,
            recreatedDurability,
            explicitRelaxedDurability,
            invalidDurability
          };
        })().then(
          value => { globalThis.__storageBucketDurabilityProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketDurabilityProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket durability probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketDurabilityProbe)")
        .expect("storage bucket durability probe should settle");

    assert_eq!(
        result,
        r#"{"strictInitial":"strict","strictReopened":"strict","staleAfterDelete":"UnknownError:true","oldAfterRecreate":"UnknownError:true","recreatedDurability":"relaxed","explicitRelaxedDurability":"relaxed","invalidDurability":"TypeError:true"}"#
    );
}
#[test]
fn storage_bucket_quota_is_bucket_metadata() {
    let mut vm = new_storage_test_vm("https://storage-bucket-quota.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketQuotaProbe = "pending";
        (async () => {
          const defaultBucket = await navigator.storageBuckets.open("default");
          const defaultQuota = (await defaultBucket.estimate()).quota;
          await navigator.storageBuckets.delete("default");

          const quotaBucket = await navigator.storageBuckets.open("quota", {
            quota: 4096
          });
          const quotaInitial = (await quotaBucket.estimate()).quota;
          const quotaReopened =
            (await (await navigator.storageBuckets.open("quota")).estimate()).quota;
          await navigator.storageBuckets.delete("quota");

          const recreatedQuota =
            (await (await navigator.storageBuckets.open("quota")).estimate()).quota;
          await navigator.storageBuckets.delete("quota");

          const fractionalQuota =
            (await (await navigator.storageBuckets.open("fractional", {
              quota: 42.8
            })).estimate()).quota;
          await navigator.storageBuckets.delete("fractional");

          const invalid = async (quota) => navigator.storageBuckets.open("bad", {
            quota
          }).then(
            () => "resolved",
            error => `${error && error.name}:${error instanceof TypeError}`
          );

          return {
            defaultQuota,
            quotaInitial,
            quotaReopened,
            recreatedQuota,
            fractionalQuota,
            negative: await invalid(-1),
            zero: await invalid(0),
            aboveMax: await invalid(Number.MAX_SAFE_INTEGER + 1),
            infinity: await invalid(Infinity)
          };
        })().then(
          value => { globalThis.__storageBucketQuotaProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketQuotaProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket quota probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketQuotaProbe)")
        .expect("storage bucket quota probe should settle");

    assert_eq!(
        result,
        r#"{"defaultQuota":1073741824,"quotaInitial":4096,"quotaReopened":4096,"recreatedQuota":1073741824,"fractionalQuota":42,"negative":"TypeError:true","zero":"TypeError:true","aboveMax":"TypeError:true","infinity":"TypeError:true"}"#
    );
}
#[test]
fn storage_bucket_cache_put_enforces_bucket_quota() {
    let mut vm = new_storage_test_vm("https://storage-bucket-cache-quota.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketCacheQuotaProbe = "pending";
        (async () => {
          const outcome = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return [
                error && error.name,
                error instanceof QuotaExceededError,
                error instanceof DOMException,
                error && error.quota,
                error && error.requested > error.quota
              ].join(":");
            }
          };

          const bucket = await navigator.storageBuckets.open("quota-cache", {
            quota: 128
          });
          const cache = await bucket.caches.open("entries");
          await cache.put("small", new Response("ok"));
          const beforeLarge = await bucket.estimate();

          const rejected = await outcome(
            cache.put("large", new Response("x".repeat(128)))
          );
          const afterRejected = await bucket.estimate();
          const matchedSmall = await cache.match("small");
          const smallText = await matchedSmall.text();
          const largeMatch = await cache.match("large");

          await navigator.storageBuckets.delete("quota-cache");

          return {
            quota: beforeLarge.quota,
            smallUsagePositive: beforeLarge.usage > 0,
            rejected,
            usageUnchanged: afterRejected.usage === beforeLarge.usage,
            cacheUsageUnchanged:
              afterRejected.usageDetails.caches === beforeLarge.usageDetails.caches,
            smallText,
            largeMatchType: typeof largeMatch
          };
        })().then(
          value => { globalThis.__storageBucketCacheQuotaProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketCacheQuotaProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket CacheStorage quota probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketCacheQuotaProbe)")
        .expect("storage bucket CacheStorage quota probe should settle");

    assert_eq!(
        result,
        r#"{"quota":128,"smallUsagePositive":true,"rejected":"QuotaExceededError:true:true:128:true","usageUnchanged":true,"cacheUsageUnchanged":true,"smallText":"ok","largeMatchType":"undefined"}"#
    );
}
#[test]
fn storage_bucket_indexeddb_put_enforces_bucket_quota() {
    let mut vm = new_storage_page_task_executor_test_vm("https://storage-bucket-idb-quota.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketIndexedDbQuotaProbe = "pending";
        (async () => {
          const openDb = (factory) => new Promise((resolve, reject) => {
            const open = factory.open("quota-db", 1);
            open.onerror = () => reject(`open:${open.error && open.error.name}`);
            open.onupgradeneeded = () => {
              open.result.createObjectStore("kv");
            };
            open.onsuccess = () => resolve(open.result);
          });
          const putOutcome = (db, key, value) => new Promise((resolve) => {
            let settled = false;
            const finish = (value) => {
              if (!settled) {
                settled = true;
                resolve(value);
              }
            };
            const tx = db.transaction("kv", "readwrite");
            const request = tx.objectStore("kv").put(value, key);
            request.onerror = () => finish({
              kind: "request",
              name: request.error && request.error.name,
              quota: request.error && request.error.quota,
              requestedGreater: request.error && request.error.requested > request.error.quota,
              dom: request.error instanceof DOMException,
              quotaError: request.error instanceof QuotaExceededError
            });
            tx.onerror = () => finish({
              kind: "transaction-error",
              name: tx.error && tx.error.name
            });
            tx.onabort = () => finish({
              kind: "abort",
              name: tx.error && tx.error.name
            });
            tx.oncomplete = () => finish({ kind: "complete" });
          });
          const getValue = (db, key) => new Promise((resolve, reject) => {
            const tx = db.transaction("kv", "readonly");
            const request = tx.objectStore("kv").get(key);
            request.onerror = () => reject(`get:${request.error && request.error.name}`);
            request.onsuccess = () => resolve(request.result ?? null);
          });

          const bucket = await navigator.storageBuckets.open("quota-idb", {
            quota: 2048
          });
          const db = await openDb(bucket.indexedDB);
          const small = await putOutcome(db, "small", "ok");
          const beforeLarge = await bucket.estimate();
          const rejected = await putOutcome(db, "large", "x".repeat(8192));
          const afterRejected = await bucket.estimate();
          const smallValue = await getValue(db, "small");
          const largeValue = await getValue(db, "large");
          db.close();
          await navigator.storageBuckets.delete("quota-idb");

          return {
            quota: beforeLarge.quota,
            small: small.kind,
            usagePositive: beforeLarge.usageDetails.indexedDB > 0,
            rejected,
            usageUnchanged: afterRejected.usage === beforeLarge.usage,
            indexedDbUsageUnchanged:
              afterRejected.usageDetails.indexedDB === beforeLarge.usageDetails.indexedDB,
            smallValue,
            largeValue
          };
        })().then(
          value => { globalThis.__storageBucketIndexedDbQuotaProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketIndexedDbQuotaProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket IndexedDB quota probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketIndexedDbQuotaProbe)")
        .expect("storage bucket IndexedDB quota probe should settle");

    assert_eq!(
        result,
        r#"{"quota":2048,"small":"complete","usagePositive":true,"rejected":{"kind":"request","name":"QuotaExceededError","quota":2048,"requestedGreater":true,"dom":true,"quotaError":true},"usageUnchanged":true,"indexedDbUsageUnchanged":true,"smallValue":"ok","largeValue":null}"#
    );
}
#[test]
fn storage_bucket_quota_is_aggregated_across_indexeddb_cache_and_opfs() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://storage-bucket-aggregate-quota.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketAggregateQuotaProbe = "pending";
        (async () => {
          const outcome = async promise => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return error && error.name || typeof error;
            }
          };
          const openDb = factory => new Promise((resolve, reject) => {
            const open = factory.open("aggregate-db", 1);
            open.onerror = () => reject(open.error);
            open.onupgradeneeded = () => open.result.createObjectStore("kv");
            open.onsuccess = () => resolve(open.result);
          });
          const putOutcome = (db, key, value) => new Promise(resolve => {
            let settled = false;
            const finish = value => {
              if (!settled) {
                settled = true;
                resolve(value);
              }
            };
            const tx = db.transaction("kv", "readwrite");
            const request = tx.objectStore("kv").put(value, key);
            request.onerror = () => finish({
              kind: "request",
              name: request.error && request.error.name
            });
            tx.onerror = () => finish({
              kind: "transaction-error",
              name: tx.error && tx.error.name
            });
            tx.onabort = () => finish({
              kind: "abort",
              name: tx.error && tx.error.name
            });
            tx.oncomplete = () => finish({ kind: "complete" });
          });
          const getValue = (db, key) => new Promise((resolve, reject) => {
            const tx = db.transaction("kv", "readonly");
            const request = tx.objectStore("kv").get(key);
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
          });

          const idbBucket = await navigator.storageBuckets.open("idb-opfs", {
            quota: 10000
          });
          const idbRoot = await idbBucket.getDirectory();
          const idbFile = await idbRoot.getFileHandle("data", { create: true });
          const activeWriter = await idbFile.createWritable();
          await activeWriter.write(new Uint8Array(6000));
          const db = await openDb(idbBucket.indexedDB);
          const blockedByActiveOpfs = await putOutcome(
            db,
            "blocked",
            new Uint8Array(5000)
          );
          await activeWriter.abort();
          const storedAfterAbort = await putOutcome(
            db,
            "stored",
            new Uint8Array(5000)
          );
          const storedValue = await getValue(db, "stored");
          const blockedOpfsWriter = await idbFile.createWritable();
          const blockedByIndexedDb = await outcome(
            blockedOpfsWriter.write(new Uint8Array(6000))
          );
          const idbFileSize = (await idbFile.getFile()).size;
          db.close();

          const cacheBucket = await navigator.storageBuckets.open("cache-opfs", {
            quota: 10000
          });
          const cacheRoot = await cacheBucket.getDirectory();
          const committedFile = await cacheRoot.getFileHandle("data", { create: true });
          const committedWriter = await committedFile.createWritable();
          await committedWriter.write(new Uint8Array(6000));
          await committedWriter.close();
          const cache = await cacheBucket.caches.open("entries");
          const blockedByCommittedOpfs = await outcome(
            cache.put("entry", new Response("x".repeat(6000)))
          );
          const absentAfterReject = typeof await cache.match("entry");
          await cacheRoot.removeEntry("data");
          const storedAfterOpfsRemove = await outcome(
            cache.put("entry", new Response("x".repeat(6000)))
          );
          const cacheBackedFile = await cacheRoot.getFileHandle("data", { create: true });
          const cacheBlockedWriter = await cacheBackedFile.createWritable();
          const blockedByCache = await outcome(
            cacheBlockedWriter.write(new Uint8Array(6000))
          );
          const cached = await cache.match("entry");
          const cacheBackedFileSize = (await cacheBackedFile.getFile()).size;

          await navigator.storageBuckets.delete("idb-opfs");
          await navigator.storageBuckets.delete("cache-opfs");
          return {
            blockedByActiveOpfs,
            storedAfterAbort,
            storedByteLength: storedValue && storedValue.byteLength,
            blockedByIndexedDb,
            idbFileSize,
            blockedByCommittedOpfs,
            absentAfterReject,
            storedAfterOpfsRemove,
            blockedByCache,
            cachedTextLength: (await cached.text()).length,
            cacheBackedFileSize
          };
        })().then(
          value => {
            globalThis.__storageBucketAggregateQuotaProbe = JSON.stringify(value);
          },
          error => {
            globalThis.__storageBucketAggregateQuotaProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("aggregate StorageBucket quota probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketAggregateQuotaProbe)")
        .expect("aggregate StorageBucket quota probe should settle");

    assert_eq!(
        result,
        r#"{"blockedByActiveOpfs":{"kind":"request","name":"QuotaExceededError"},"storedAfterAbort":{"kind":"complete"},"storedByteLength":5000,"blockedByIndexedDb":"QuotaExceededError","idbFileSize":0,"blockedByCommittedOpfs":"QuotaExceededError","absentAfterReject":"undefined","storedAfterOpfsRemove":"resolved","blockedByCache":"QuotaExceededError","cachedTextLength":6000,"cacheBackedFileSize":0}"#
    );
}
#[test]
fn storage_bucket_persisted_is_bucket_metadata() {
    let mut vm = new_storage_test_vm("https://storage-bucket-persisted.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketPersistedProbe = "pending";
        (async () => {
          const defaultBucket = await navigator.storageBuckets.open("default");
          const defaultPersisted = await defaultBucket.persisted();
          const defaultPersistRequest = await defaultBucket.persist();
          await navigator.storageBuckets.delete("default");

          const persistedBucket = await navigator.storageBuckets.open("persisted", {
            persisted: true
          });
          const persistedInitial = await persistedBucket.persisted();
          const persistedRequest = await persistedBucket.persist();
          const persistedReopened =
            await (await navigator.storageBuckets.open("persisted")).persisted();
          await navigator.storageBuckets.delete("persisted");

          const recreatedPersisted =
            await (await navigator.storageBuckets.open("persisted")).persisted();
          await navigator.storageBuckets.delete("persisted");

          const toggled = await navigator.storageBuckets.open("toggled", {
            persisted: true
          });
          const toggledInitial = await toggled.persisted();
          const toggledFalse = await navigator.storageBuckets.open("toggled", {
            persisted: false
          });
          const toggledAfterFalse = await toggledFalse.persisted();
          await navigator.storageBuckets.delete("toggled");

          const stringPersisted =
            await (await navigator.storageBuckets.open("string", {
              persisted: "yes"
            })).persisted();
          await navigator.storageBuckets.delete("string");

          return {
            defaultPersisted,
            defaultPersistRequest,
            persistedInitial,
            persistedRequest,
            persistedReopened,
            recreatedPersisted,
            toggledInitial,
            toggledAfterFalse,
            stringPersisted
          };
        })().then(
          value => { globalThis.__storageBucketPersistedProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketPersistedProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket persisted probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketPersistedProbe)")
        .expect("storage bucket persisted probe should settle");

    assert_eq!(
        result,
        r#"{"defaultPersisted":false,"defaultPersistRequest":false,"persistedInitial":true,"persistedRequest":true,"persistedReopened":true,"recreatedPersisted":false,"toggledInitial":true,"toggledAfterFalse":false,"stringPersisted":true}"#
    );
}
#[test]
fn storage_bucket_methods_preserve_stale_error_contracts_after_delete_and_recreate() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://storage-bucket-stale-handle.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketStaleHandleProbe = "pending";
        (async () => {
          const outcome = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return `${error && error.name}:${error instanceof DOMException}`;
            }
          };
          const requestOutcome = (request) => new Promise((resolve) => {
            request.onsuccess = () => {
              if (request.result && typeof request.result.close === "function") {
                request.result.close();
              }
              resolve("success");
            };
            request.onerror = () => {
              resolve(`${request.error && request.error.name}:${request.error instanceof DOMException}`);
            };
          });
          const openDbOutcome = (factory) => requestOutcome(factory.open("stale-db", 1));
          const deleteDbOutcome = (factory) => requestOutcome(factory.deleteDatabase("stale-db"));

          const expiresDate = Date.now() + 60000;
          const bucket = await navigator.storageBuckets.open("stale", { expires: expiresDate });
          const staleFactory = bucket.indexedDB;
          const beforeDeleteMatches = await bucket.expires() === expiresDate;
          await navigator.storageBuckets.delete("stale");

          const persistAfterDelete = await outcome(bucket.persist());
          const persistedAfterDelete = await outcome(bucket.persisted());
          const estimateAfterDelete = await outcome(bucket.estimate());
          const expiresAfterDelete = await outcome(bucket.expires());
          const setExpiresAfterDelete = await outcome(bucket.setExpires(2));
          const getDirectoryAfterDelete = await outcome(bucket.getDirectory());

          const recreatedExpiresDate = Date.now() + 120000;
          const recreated = await navigator.storageBuckets.open("stale", {
            expires: recreatedExpiresDate
          });
          const oldAfterRecreate = await outcome(bucket.expires());
          const oldFactoryOpenAfterRecreate = await openDbOutcome(staleFactory);
          const oldFactoryDeleteAfterRecreate = await deleteDbOutcome(staleFactory);
          const recreatedFactoryOpen = await openDbOutcome(recreated.indexedDB);
          const recreatedFactoryDelete = await deleteDbOutcome(recreated.indexedDB);
          const recreatedExpiresMatches = await recreated.expires() === recreatedExpiresDate;
          await navigator.storageBuckets.delete("stale");

          return {
            beforeDeleteMatches,
            persistAfterDelete,
            persistedAfterDelete,
            estimateAfterDelete,
            expiresAfterDelete,
            setExpiresAfterDelete,
            getDirectoryAfterDelete,
            oldAfterRecreate,
            oldFactoryOpenAfterRecreate,
            oldFactoryDeleteAfterRecreate,
            recreatedFactoryOpen,
            recreatedFactoryDelete,
            recreatedExpiresMatches
          };
        })().then(
          value => { globalThis.__storageBucketStaleHandleProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketStaleHandleProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket stale handle probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketStaleHandleProbe)")
        .expect("storage bucket stale handle probe should settle");

    assert_eq!(
        result,
        r#"{"beforeDeleteMatches":true,"persistAfterDelete":"UnknownError:true","persistedAfterDelete":"UnknownError:true","estimateAfterDelete":"UnknownError:true","expiresAfterDelete":"UnknownError:true","setExpiresAfterDelete":"UnknownError:true","getDirectoryAfterDelete":"InvalidStateError:true","oldAfterRecreate":"UnknownError:true","oldFactoryOpenAfterRecreate":"UnknownError:true","oldFactoryDeleteAfterRecreate":"UnknownError:true","recreatedFactoryOpen":"success","recreatedFactoryDelete":"success","recreatedExpiresMatches":true}"#
    );
}
#[test]
fn opfs_get_unique_id_is_session_stable_and_entry_scoped() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-unique-id.test/");

    vm.exec(
        r#"
        globalThis.__opfsUniqueIdProbe = "pending";
        (async () => {
          const root = await navigator.storage.getDirectory();
          const directory = await root.getDirectoryHandle("directory", { create: true });
          const directoryAgain = await root.getDirectoryHandle("directory");
          const file = await root.getFileHandle("entry", { create: true });
          const fileAgain = await root.getFileHandle("entry");
          const rootId = await root.getUniqueId();
          const directoryId = await directory.getUniqueId();
          const fileId = await file.getUniqueId();
          const writer = await file.createWritable();
          await writer.write("updated");
          await writer.close();
          const fileIdAfterWrite = await file.getUniqueId();
          await root.removeEntry("entry");
          const replacementDirectory = await root.getDirectoryHandle(
            "entry",
            { create: true }
          );
          const replacementDirectoryId = await replacementDirectory.getUniqueId();
          const uuidV4 =
            /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
          return {
            method: typeof FileSystemHandle.prototype.getUniqueId,
            length: FileSystemHandle.prototype.getUniqueId.length,
            rootFormat: uuidV4.test(rootId),
            sameDirectory: await directoryAgain.getUniqueId() === directoryId,
            sameFile: await fileAgain.getUniqueId() === fileId,
            differentEntries: rootId !== directoryId && directoryId !== fileId,
            stableAfterWrite: fileIdAfterWrite === fileId,
            differentKindAtSamePath: replacementDirectoryId !== fileId
          };
        })().then(
          value => { globalThis.__opfsUniqueIdProbe = JSON.stringify(value); },
          error => {
            globalThis.__opfsUniqueIdProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("OPFS unique ID probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsUniqueIdProbe)")
        .expect("OPFS unique ID probe should settle");

    assert_eq!(
        result,
        r#"{"method":"function","length":0,"rootFormat":true,"sameDirectory":true,"sameFile":true,"differentEntries":true,"stableAfterWrite":true,"differentKindAtSamePath":true}"#
    );
}
#[tokio::test]
async fn network_dedicated_worker_storage_access_opfs_handle_clone_reuses_partition_store() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/opfs-clone-worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.onmessage = async event => {
          try {
            const handle = event.data;
            const before = await (await handle.getFile()).text();
            const writer = await handle.createWritable();
            await writer.write("worker bytes");
            await writer.close();
            postMessage({
              brand: handle instanceof FileSystemFileHandle,
              name: handle.name,
              permission: await handle.requestPermission({ mode: "readwrite" }),
              before,
              after: await (await handle.getFile()).text()
            });
          } catch (error) {
            postMessage({ error: `${error && error.name}:${error && error.message}` });
          }
        };
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        globalThis.__opfsWorkerCloneProbe = "pending";
        (async () => {
          const access = await document.requestStorageAccess({ getDirectory: true });
          const root = await access.getDirectory();
          const file = await root.getFileHandle("worker-clone.txt", { create: true });
          const writer = await file.createWritable();
          await writer.write("page bytes");
          await writer.close();

          const worker = new Worker("opfs-clone-worker.js");
          worker.onmessage = async event => {
            globalThis.__opfsWorkerCloneProbe = JSON.stringify({
              ...event.data,
              pageText: await (await file.getFile()).text()
            });
          };
          worker.onmessageerror = () => {
            globalThis.__opfsWorkerCloneProbe = "messageerror";
          };
          worker.onerror = event => {
            globalThis.__opfsWorkerCloneProbe = `error:${event.message}`;
          };
          worker.postMessage(file);
        })().catch(error => {
          globalThis.__opfsWorkerCloneProbe =
            `error:${error && error.name}:${error && error.message}`;
        });
        "#,
    )
    .expect("network Worker OPFS clone probe should schedule");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__opfsWorkerCloneProbe)",
        r#"{"brand":true,"name":"worker-clone.txt","permission":"granted","before":"page bytes","after":"worker bytes","pageText":"worker bytes"}"#,
    )
    .await;

    server
        .await
        .expect("network Worker OPFS clone script server should finish");
}
#[test]
fn opfs_directory_move_reparents_subtrees_and_enforces_hierarchical_locks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-directory-move.test/");

    vm.exec(
        r#"
        globalThis.__opfsDirectoryMoveProbe = "pending";
        (async () => {
          const outcome = async promise => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return error && error.name || typeof error;
            }
          };
          const text = async handle => new Response(await handle.getFile()).text();
          const root = await navigator.storage.getDirectory();
          const source = await root.getDirectoryHandle("source", { create: true });
          const nested = await source.getDirectoryHandle("nested", { create: true });
          const file = await nested.getFileHandle("file.txt", { create: true });
          const writer = await file.createWritable();
          await writer.write("directory subtree");
          await writer.close();

          const sourceWriter = await file.createWritable({ keepExistingData: true });
          const sourceLock = await outcome(source.move("renamed"));
          const sameNameLock = await outcome(source.move("source"));
          await sourceWriter.close();

          const sibling = await root.getFileHandle("sibling.txt", { create: true });
          const siblingWriter = await sibling.createWritable();
          const siblingMove = await outcome(source.move("renamed"));
          await siblingWriter.close();

          const oldNestedAccess = await outcome(nested.getFileHandle("file.txt"));
          const movedNested = await source.getDirectoryHandle("nested");
          const movedFile = await movedNested.getFileHandle("file.txt");
          const child = await source.getDirectoryHandle("child", { create: true });
          const descendantMove = await outcome(source.move(child));
          const rootMove = await outcome(root.move("new-root"));

          const otherBucket = await navigator.storageBuckets.open("other-directory-move");
          const otherRoot = await otherBucket.getDirectory();
          const crossBucket = await outcome(source.move(otherRoot));

          const incoming = await root.getDirectoryHandle("incoming", { create: true });
          const occupied = await root.getDirectoryHandle("occupied", { create: true });
          const occupiedFile = await occupied.getFileHandle("locked.txt", { create: true });
          const occupiedWriter = await occupiedFile.createWritable();
          const destinationLock = await outcome(incoming.move(root, "occupied"));
          await occupiedWriter.close();
          const nonEmptyDestination = await outcome(incoming.move(root, "occupied"));

          const empty = await root.getDirectoryHandle("empty", { create: true });
          await incoming.move(root, "empty");
          const replacementSameEntry = await empty.isSameEntry(incoming);

          const staleDirectory = await root.getDirectoryHandle("stale", { create: true });
          await staleDirectory.remove();
          await root.getFileHandle("stale", { create: true });
          const recreatedAsFile = await outcome(staleDirectory.move("stale-moved"));

          return {
            prototype: [
              typeof FileSystemDirectoryHandle.prototype.move,
              Object.prototype.hasOwnProperty.call(
                FileSystemDirectoryHandle.prototype, "move"),
              Object.prototype.hasOwnProperty.call(FileSystemHandle.prototype, "move")
            ],
            locks: [sourceLock, sameNameLock, siblingMove, destinationLock],
            moved: [
              source.name,
              await root.resolve(source),
              oldNestedAccess,
              await text(movedFile)
            ],
            rejected: [
              descendantMove,
              rootMove,
              crossBucket,
              nonEmptyDestination,
              recreatedAsFile
            ],
            replacement: [incoming.name, await root.resolve(incoming), replacementSameEntry]
          };
        })().then(
          value => { globalThis.__opfsDirectoryMoveProbe = JSON.stringify(value); },
          error => {
            globalThis.__opfsDirectoryMoveProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("OPFS directory move probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsDirectoryMoveProbe)")
        .expect("OPFS directory move probe should settle");

    assert_eq!(
        result,
        r#"{"prototype":["function",true,false],"locks":["NoModificationAllowedError","NoModificationAllowedError","resolved","NoModificationAllowedError"],"moved":["renamed",["renamed"],"NotFoundError","directory subtree"],"rejected":["InvalidModificationError","InvalidModificationError","InvalidModificationError","InvalidModificationError","TypeMismatchError"],"replacement":["empty",["empty"],true]}"#
    );
}
#[test]
fn storage_bucket_delete_revokes_opfs_sessions_and_isolates_same_name_recreate() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-bucket-delete.test/");

    vm.exec(
        r#"
        globalThis.__opfsBucketDeleteProbe = "pending";
        (async () => {
          const outcome = async promise => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return error && error.name || typeof error;
            }
          };
          const bucket = await navigator.storageBuckets.open("revoked");
          const root = await bucket.getDirectory();
          const file = await root.getFileHandle("old.txt", { create: true });
          const committed = await file.createWritable();
          await committed.write("old bucket bytes");
          await committed.close();
          const active = await file.createWritable({ keepExistingData: true });
          await active.write({ type: "write", position: 0, data: "staged" });
          const before = await bucket.estimate();

          await navigator.storageBuckets.delete("revoked");
          const activeCloseAfterDelete = await outcome(active.close());
          const oldRootAccess = await outcome(root.getFileHandle("old.txt"));
          const oldBucketRoot = await outcome(bucket.getDirectory());

          const recreated = await navigator.storageBuckets.open("revoked");
          const recreatedRoot = await recreated.getDirectory();
          const recreatedMissing = await outcome(recreatedRoot.getFileHandle("old.txt"));
          const recreatedKeys = [];
          for await (const key of recreatedRoot.keys()) recreatedKeys.push(key);
          const after = await recreated.estimate();
          await navigator.storageBuckets.delete("revoked");

          return {
            beforeUsagePositive: before.usage > 0,
            beforeFileSystemPositive: before.usageDetails.fileSystem > 0,
            activeCloseAfterDelete,
            oldRootAccess,
            oldBucketRoot,
            recreatedMissing,
            recreatedKeys,
            recreatedUsage: after.usage,
            recreatedFileSystemType: typeof after.usageDetails.fileSystem
          };
        })().then(
          value => { globalThis.__opfsBucketDeleteProbe = JSON.stringify(value); },
          error => {
            globalThis.__opfsBucketDeleteProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("OPFS bucket delete probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsBucketDeleteProbe)")
        .expect("OPFS bucket delete probe should settle");

    assert_eq!(
        result,
        r#"{"beforeUsagePositive":true,"beforeFileSystemPositive":true,"activeCloseAfterDelete":"NotFoundError","oldRootAccess":"NotFoundError","oldBucketRoot":"InvalidStateError","recreatedMissing":"NotFoundError","recreatedKeys":[],"recreatedUsage":0,"recreatedFileSystemType":"undefined"}"#
    );
}
#[test]
fn opfs_handle_permissions_are_fixed_grants_with_webidl_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-permissions.test/");

    vm.exec(
        r#"
globalThis.__opfsPermissionProbe = "pending";
(async () => {
  const outcome = async promise => {
    try {
      return `resolved:${await promise}`;
    } catch (error) {
      return `rejected:${error && error.name}`;
    }
  };
  const root = await navigator.storage.getDirectory();
  const directory = await root.getDirectoryHandle("directory", { create: true });
  const file = await directory.getFileHandle("file", { create: true });
  let getterHits = 0;
  const getterDescriptor = {
    get mode() {
      getterHits++;
      return "readwrite";
    }
  };
  const getterResult = await file.requestPermission(getterDescriptor);
  const cloned = structuredClone(file);

  const bucket = await navigator.storageBuckets.open("permission-stale");
  const stale = await (await bucket.getDirectory()).getFileHandle(
    "file",
    { create: true }
  );
  await navigator.storageBuckets.delete("permission-stale");

  return {
    queryShape: [
      typeof FileSystemHandle.prototype.queryPermission,
      FileSystemHandle.prototype.queryPermission.name,
      FileSystemHandle.prototype.queryPermission.length
    ],
    requestShape: [
      typeof FileSystemHandle.prototype.requestPermission,
      FileSystemHandle.prototype.requestPermission.name,
      FileSystemHandle.prototype.requestPermission.length
    ],
    defaults: [
      await root.queryPermission(),
      await directory.queryPermission(undefined),
      await file.queryPermission(null)
    ],
    modes: [
      await file.queryPermission({ mode: "read" }),
      await file.queryPermission({ mode: "readwrite" }),
      getterResult,
      getterHits
    ],
    requests: [
      await root.requestPermission(),
      await directory.requestPermission({ mode: "read" }),
      await cloned.requestPermission({ mode: "readwrite" })
    ],
    invalid: [
      await outcome(file.queryPermission({ mode: "write" })),
      await outcome(file.requestPermission({ mode: Symbol("read") })),
      await outcome(FileSystemHandle.prototype.queryPermission.call({})),
      await outcome(FileSystemHandle.prototype.requestPermission.call(null))
    ],
    stale: [
      await outcome(stale.queryPermission()),
      await outcome(stale.requestPermission({ mode: "readwrite" }))
    ]
  };
})().then(
  value => { globalThis.__opfsPermissionProbe = JSON.stringify(value); },
  error => {
    globalThis.__opfsPermissionProbe =
      `error:${error && error.name}:${error && error.message}`;
  }
);
        "#,
        None,
    )
    .expect("OPFS permission probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsPermissionProbe)")
        .expect("OPFS permission probe should settle");
    assert_eq!(
        result,
        r#"{"queryShape":["function","queryPermission",0],"requestShape":["function","requestPermission",0],"defaults":["granted","granted","granted"],"modes":["granted","granted","granted",1],"requests":["granted","granted","granted"],"invalid":["rejected:TypeError","rejected:TypeError","rejected:TypeError","rejected:TypeError"],"stale":["resolved:granted","resolved:granted"]}"#
    );
}
#[test]
fn opfs_writable_stream_is_atomic_queued_and_quota_checked() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-writable.test/");

    vm.exec(
        r#"
        globalThis.__opfsWritableProbe = "pending";
        (async () => {
          const outcome = async promise => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return error && error.name || typeof error;
            }
          };
          const text = async handle => new Response(await handle.getFile()).text();
          const root = await navigator.storage.getDirectory();
          const file = await root.getFileHandle("atomic.txt", { create: true });
          const writable = await file.createWritable();
          const writableBrands = [
            writable instanceof FileSystemWritableFileStream,
            writable instanceof WritableStream,
            Object.prototype.toString.call(writable),
            writable.mode,
            writable.locked
          ];
          await writable.write("abc");
          const beforeClose = [(await file.getFile()).size, await text(file)];
          await writable.truncate(5);
          await writable.seek(1);
          await writable.write(new Uint8Array([88, 89]));
          const closeReturns = [...Array(10)].map(() => writable.close());
          const closeReturnsPromises = closeReturns.every(
            value => value && typeof value.then === "function"
          );
          const closeAttempts = await Promise.all(closeReturns.map(
            value => value && typeof value.then === "function"
              ? value.then(() => 1, () => 0)
              : -100
          ));
          const closeSuccessCount = closeAttempts.reduce((sum, value) => sum + value, 0);
          const afterClose = [(await file.getFile()).size, await text(file)];
          const afterCloseWrite = await outcome(writable.write("no"));
          const afterCloseClose = await outcome(writable.close());

          const keep = await file.createWritable({ keepExistingData: true });
          await keep.write({ type: "write", position: 4, data: "z" });
          await keep.close();
          const afterKeep = [(await file.getFile()).size, await text(file)];

          const aborted = await file.createWritable();
          await aborted.write("discarded");
          await aborted.abort("stop");
          const afterAbort = [(await file.getFile()).size, await text(file)];
          const afterAbortClose = await outcome(aborted.close());

          const viaWriter = await file.createWritable();
          const writer = viaWriter.getWriter();
          const directWhileLocked = await outcome(viaWriter.write("blocked"));
          const closeWhileLocked = await outcome(viaWriter.close());
          const abortWhileLocked = await outcome(viaWriter.abort());
          await writer.write("writer");
          await writer.close();
          const afterWriter = await text(file);

          const piped = await file.createWritable();
          const source = new ReadableStream({
            start(controller) {
              controller.enqueue("pipe-");
              controller.enqueue(new Blob(["ok"]));
              controller.close();
            }
          });
          await source.pipeTo(piped);
          const afterPipe = await text(file);

          const exclusive = await file.createWritable({ mode: "exclusive" });
          const exclusiveConflict = await outcome(
            file.createWritable({ mode: "exclusive" })
          );
          await exclusive.close();
          const exclusiveAfterClose = await file.createWritable({ mode: "exclusive" });
          await exclusiveAfterClose.close();

          const invalid = await file.createWritable();
          const missingWriteData = await outcome(invalid.write({ type: "write" }));
          const writeAfterError = await outcome(invalid.write("later"));

          const erroredExclusive = await file.createWritable({ mode: "exclusive" });
          const exclusiveWriteError = await outcome(
            erroredExclusive.write({ type: "write", data: null })
          );
          const exclusiveAfterError = await outcome(
            file.createWritable({ mode: "exclusive" }).then(stream => stream.close())
          );

          const sourceFile = await root.getFileHandle("source.txt", { create: true });
          const sourceWriter = await sourceFile.createWritable();
          await sourceWriter.write("source-data");
          await sourceWriter.close();
          const invalidBlob = await sourceFile.getFile();
          await root.removeEntry("source.txt");
          const invalidBlobTarget = await root.getFileHandle("invalid-blob.txt", { create: true });
          const invalidBlobWriter = await invalidBlobTarget.createWritable();
          const invalidBlobWrite = await outcome(invalidBlobWriter.write(invalidBlob));
          const invalidBlobClose = await outcome(invalidBlobWriter.close());
          const invalidBlobTargetSize = (await invalidBlobTarget.getFile()).size;

          // `tiny` costs 146 + 2 * 4 bytes of virtual entry metadata, leaving
          // exactly four payload bytes under this quota.
          const bucket = await navigator.storageBuckets.open("tiny", { quota: 158 });
          const bucketRoot = await bucket.getDirectory();
          const tiny = await bucketRoot.getFileHandle("tiny", { create: true });
          const overQuota = await tiny.createWritable();
          let quotaError;
          let quotaDetails;
          try {
            await overQuota.write("12345");
            quotaError = "resolved";
          } catch (error) {
            quotaError = error && error.name || typeof error;
            quotaDetails = [
              error instanceof QuotaExceededError,
              error.requested === null,
              error.quota === null
            ];
          }
          const tinySize = (await tiny.getFile()).size;

          return {
            writableBrands,
            beforeClose,
            closeReturnsPromises,
            closeSuccessCount,
            afterClose,
            afterCloseWrite,
            afterCloseClose,
            afterKeep,
            afterAbort,
            afterAbortClose,
            directWhileLocked,
            closeWhileLocked,
            abortWhileLocked,
            afterWriter,
            afterPipe,
            exclusiveConflict,
            missingWriteData,
            writeAfterError,
            exclusiveWriteError,
            exclusiveAfterError,
            invalidBlobWrite,
            invalidBlobClose,
            invalidBlobTargetSize,
            quotaError,
            quotaDetails,
            tinySize
          };
        })().then(
          value => { globalThis.__opfsWritableProbe = JSON.stringify(value); },
          error => {
            globalThis.__opfsWritableProbe =
              `error:${error && error.name}:${error && error.message}`;
          }
        );
        "#,
        None,
    )
    .expect("OPFS writable probe should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__opfsWritableProbe)")
        .expect("OPFS writable probe should settle");

    assert_eq!(
        result,
        r#"{"writableBrands":[true,true,"[object FileSystemWritableFileStream]","siloed",false],"beforeClose":[0,""],"closeReturnsPromises":true,"closeSuccessCount":1,"afterClose":[5,"aXY\u0000\u0000"],"afterCloseWrite":"TypeError","afterCloseClose":"TypeError","afterKeep":[5,"aXY\u0000z"],"afterAbort":[5,"aXY\u0000z"],"afterAbortClose":"TypeError","directWhileLocked":"TypeError","closeWhileLocked":"TypeError","abortWhileLocked":"TypeError","afterWriter":"writer","afterPipe":"pipe-ok","exclusiveConflict":"NoModificationAllowedError","missingWriteData":"SyntaxError","writeAfterError":"SyntaxError","exclusiveWriteError":"TypeError","exclusiveAfterError":"resolved","invalidBlobWrite":"NotFoundError","invalidBlobClose":"TypeError","invalidBlobTargetSize":0,"quotaError":"QuotaExceededError","quotaDetails":[true,true,true],"tinySize":0}"#
    );
}
#[test]
fn retained_storage_bucket_object_uses_bound_owner_after_child_navigation() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://storage-bucket-retained-owner.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketRetainedOwnerProbe = "pending-open";
        (async () => {
          const frame = document.createElement("iframe");
          frame.srcdoc = "<body>child</body>";
          (document.body || document.documentElement || document).appendChild(frame);
          const originalExpires = Date.now() + 60000;
          const bucket = await frame.contentWindow.navigator.storageBuckets.open("retained", {
            quota: 4096,
            expires: originalExpires
          });
          globalThis.__storageBucketRetainedOwnerFrame = frame;
          globalThis.__storageBucketRetainedOwnerBucket = bucket;
          globalThis.__storageBucketRetainedOwnerExpires = originalExpires;
          return "ready";
        })().then(
          value => { globalThis.__storageBucketRetainedOwnerProbe = value; },
          error => { globalThis.__storageBucketRetainedOwnerProbe = `error:${error && error.name}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket retained-owner setup should schedule");

    let setup = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketRetainedOwnerProbe)")
        .expect("storage bucket retained-owner setup should settle");
    assert_eq!(setup, "ready");

    vm.exec(
        r#"
        globalThis.__storageBucketRetainedOwnerFrame.src = "data:text/html,<body>opaque</body>";
        globalThis.__storageBucketRetainedOwnerFrame.removeAttribute("srcdoc");
        "#,
        None,
    )
    .expect("storage bucket retained-owner navigation should queue");
    assert_eq!(
        vm.eval_after_selected_page_tasks("'drained'")
            .expect("child navigation should run through the selected-task dispatcher"),
        "drained"
    );

    vm.exec(
        r#"
        globalThis.__storageBucketRetainedOwnerProbe = "pending-estimate";
        (async () => {
          const frame = globalThis.__storageBucketRetainedOwnerFrame;
          const bucket = globalThis.__storageBucketRetainedOwnerBucket;
          const retiredOutcome = await Promise.race([
            bucket.setExpires(globalThis.__storageBucketRetainedOwnerExpires + 60000).then(
              () => "resolved",
              error => `rejected:${error && error.name}`
            ),
            new Promise(resolve => setTimeout(() => resolve("pending"), 0))
          ]);
          const currentBucket = await navigator.storageBuckets.open("retained");
          const expiresUnchanged =
            await currentBucket.expires() === globalThis.__storageBucketRetainedOwnerExpires;
          await navigator.storageBuckets.delete("retained");
          return {
            frameIsOpaqueToParent: frame.contentDocument === null,
            expiresUnchanged,
            retiredOutcome
          };
        })().then(
          value => { globalThis.__storageBucketRetainedOwnerProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketRetainedOwnerProbe = `error:${error && error.name}`; }
        );
        "#,
        None,
    )
    .expect("storage bucket retained-owner estimate should schedule");

    let result = vm
        .eval_after_selected_page_tasks("String(globalThis.__storageBucketRetainedOwnerProbe)")
        .expect("storage bucket retained-owner probe should settle");

    assert_eq!(
        result,
        r#"{"frameIsOpaqueToParent":true,"expiresUnchanged":true,"retiredOutcome":"pending"}"#
    );
}
#[tokio::test]
async fn third_party_iframe_storage_bucket_manager_uses_partitioned_storage_key() {
    let (child_url, server) = spawn_storage_bucket_partition_child_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let top_url = format!(
        "http://127.0.0.1:{}/storage-bucket-parent.html",
        Url::parse(&child_url)
            .expect("child URL should parse")
            .port()
            .expect("child URL should include a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&top_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child URL should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__storageBucketPartitionMessage = null;
  addEventListener("message", event => {{
    globalThis.__storageBucketPartitionMessage = String(event.data);
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
    ))
    .expect("storage bucket partition setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__storageBucketPartitionMessage !== null)",
        "true",
        "storage bucket partition child result",
    )
    .await;

    assert_eq!(
        vm.eval("globalThis.__storageBucketPartitionMessage || 'missing'")
            .expect("storage bucket partition message should evaluate"),
        r#"{"bucketName":"partitioned-bucket","keys":["partitioned-bucket"]}"#
    );
    let child_origin = Url::parse(&child_url)
        .expect("child URL should parse")
        .origin()
        .ascii_serialization();
    let partitioned_storage_key =
        moli_storage_key::partitioned_storage_key(&child_origin, "http://127.0.0.1");
    assert_eq!(
        vm.storage_bucket_keys_for_test(&child_origin),
        Vec::<String>::new()
    );
    assert_eq!(
        vm.storage_bucket_keys_for_test(&partitioned_storage_key),
        vec!["partitioned-bucket"]
    );

    let request = server
        .await
        .expect("storage bucket partition server should finish");
    assert!(
        request.starts_with("GET /storage-bucket-child.html "),
        "unexpected storage bucket partition child request: {request:?}"
    );
}
#[test]
fn storage_bucket_manager_methods_reject_after_iframe_detaches() {
    let mut vm = new_storage_test_vm("https://storage-buckets-detached.test/");

    vm.exec(
        r#"
        globalThis.__storageBucketsDetachedProbe = "pending";
        (async () => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          const iframe = document.createElement("iframe");
          document.body.appendChild(iframe);
          const manager = iframe.contentWindow.navigator.storageBuckets;
          const parentManager = navigator.storageBuckets;
          await manager.open("iframe-bucket");
          const keysBefore = await manager.keys();
          await manager.delete("iframe-bucket");
          iframe.remove();
          const promiseOutcome = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return error && error.name;
            }
          };
          const [openAfterDetach, keysAfterDetach, deleteAfterDetach] = await Promise.all([
            promiseOutcome(manager.open("iframe-bucket")),
            promiseOutcome(manager.keys()),
            promiseOutcome(manager.delete("iframe-bucket"))
          ]);
          return {
            childManagerIsParentManager: manager === parentManager,
            keysBefore,
            openAfterDetach,
            keysAfterDetach,
            deleteAfterDetach
          };
        })().then(
          value => { globalThis.__storageBucketsDetachedProbe = JSON.stringify(value); },
          error => { globalThis.__storageBucketsDetachedProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage buckets detached probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageBucketsDetachedProbe)")
        .expect("storage buckets detached probe should settle");

    assert_eq!(
        result,
        r#"{"childManagerIsParentManager":false,"keysBefore":["iframe-bucket"],"openAfterDetach":"TypeError","keysAfterDetach":"TypeError","deleteAfterDetach":"TypeError"}"#
    );
}
#[tokio::test]
async fn storage_bucket_manager_methods_reject_in_opaque_origin_frame() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-buckets-opaque.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__storageBucketOpaqueMessages = [];
  addEventListener("message", event => {
    __storageBucketOpaqueMessages.push(String(event.data));
  });

  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.srcdoc = `<script>
    const outcome = async (label, promise) => {
      try {
        await promise;
        parent.postMessage(label + ":fulfilled", "*");
      } catch (error) {
        parent.postMessage(label + ":" + (error && error.name) + ":" + (error instanceof DOMException), "*");
      }
    };
    (async () => {
      await outcome("open", navigator.storageBuckets.open("opaque-origin-bucket"));
      await outcome("keys", navigator.storageBuckets.keys());
      await outcome("delete", navigator.storageBuckets.delete("opaque-origin-bucket"));
    })();
  <\/script>`;
  const host = document.body || document.documentElement || document;
  host.appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("opaque StorageBucket setup should evaluate");
    assert_eq!(setup, "queued");

    for _ in 0..8 {
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let message_count = vm
            .eval("String(__storageBucketOpaqueMessages.length)")
            .expect("opaque StorageBucket message count should evaluate");
        if message_count == "3" {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque StorageBucket child load should advance");
    }

    let result = vm
        .eval("JSON.stringify(__storageBucketOpaqueMessages)")
        .expect("opaque StorageBucket messages should evaluate");
    assert_eq!(
        result,
        r#"["open:SecurityError:true","keys:SecurityError:true","delete:SecurityError:true"]"#
    );
}
#[test]
fn history_state_rejects_webassembly_module_for_storage() {
    let mut vm = new_storage_test_vm("https://example.com/wasm-history");

    let result = vm
        .eval(
            r#"
            (() => {
              const createModule = () => new WebAssembly.Module(
                new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
              );
              const probe = (method, value) => {
                try {
                  history[method](value, "");
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };

              let before = false;
              let after = false;
              const interleaved = probe("pushState", [
                { get x() { before = true; return 1; } },
                createModule(),
                { get x() { after = true; return 2; } }
              ]);

              return [
                probe("pushState", createModule()),
                probe("replaceState", createModule()),
                interleaved,
                before,
                after
              ].join("|");
            })()
            "#,
        )
        .expect("history WebAssembly.Module storage rejection should evaluate");

    assert_eq!(
        result,
        "DataCloneError|DataCloneError|DataCloneError|true|false"
    );
}
#[test]
fn notification_data_rejects_webassembly_module_for_storage() {
    let mut vm = new_storage_test_vm("https://example.com/wasm-notification");

    let result = vm
        .eval(
            r#"
            (() => {
              const createModule = () => new WebAssembly.Module(
                new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
              );
              const probe = (value) => {
                try {
                  new Notification("title", { data: value });
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };

              let before = false;
              let after = false;
              const interleaved = probe([
                { get x() { before = true; return 1; } },
                createModule(),
                { get x() { after = true; return 2; } }
              ]);
              const plain = new Notification("plain", { data: { answer: 42 } });
              const missing = new Notification("missing");
              const explicitUndefined = new Notification(
                "explicit",
                { data: undefined }
              );
              const constructorActions = (() => {
                try {
                  new Notification("actions", {
                    actions: [{ action: "reply", title: "Reply" }]
                  });
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              })();

              return [
                typeof Notification,
                Notification.permission,
                Notification.maxActions,
                typeof Notification.requestPermission,
                plain.title,
                plain.actions.length,
                plain.data.answer,
                missing.data === null,
                explicitUndefined.data === null,
                constructorActions,
                probe(createModule()),
                interleaved,
                before,
                after
              ].join("|");
            })()
            "#,
        )
        .expect("Notification WebAssembly.Module storage rejection should evaluate");

    assert_eq!(
        result,
        "function|default|2|function|plain|0|42|true|true|TypeError|DataCloneError|DataCloneError|true|false"
    );
}
#[test]
fn quota_exceeded_error_is_dom_exception_subclass_with_readonly_slots() {
    let mut vm = new_storage_test_vm("https://quota-exceeded.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const explicit = new QuotaExceededError("full", { requested: 7, quota: 11 });
              const empty = new QuotaExceededError();
              let thrown;
              try {
                crypto.getRandomValues(new Uint8Array(65537));
              } catch (error) {
                thrown = error;
              }

              const requestedDescriptor =
                Object.getOwnPropertyDescriptor(QuotaExceededError.prototype, "requested");
              const quotaDescriptor =
                Object.getOwnPropertyDescriptor(QuotaExceededError.prototype, "quota");
              const domNamed = new DOMException("named", "QuotaExceededError");

              return [
                explicit.name,
                explicit.message,
                explicit.code,
                explicit.requested,
                explicit.quota,
                explicit instanceof QuotaExceededError,
                explicit instanceof DOMException,
                Object.getPrototypeOf(explicit) === QuotaExceededError.prototype,
                Object.getPrototypeOf(QuotaExceededError.prototype) === DOMException.prototype,
                Object.prototype.hasOwnProperty.call(explicit, "requested"),
                typeof requestedDescriptor.get,
                requestedDescriptor.get.name,
                requestedDescriptor.get.length,
                requestedDescriptor.set === undefined,
                requestedDescriptor.enumerable,
                requestedDescriptor.configurable,
                typeof quotaDescriptor.get,
                quotaDescriptor.get.name,
                quotaDescriptor.get.length,
                quotaDescriptor.set === undefined,
                quotaDescriptor.enumerable,
                quotaDescriptor.configurable,
                empty.requested === null,
                empty.quota === null,
                thrown instanceof QuotaExceededError,
                thrown.requested === null,
                thrown.quota === null,
                domNamed instanceof QuotaExceededError,
                domNamed.name,
                domNamed.code
              ].join("|");
            })()
            "#,
        )
        .expect("QuotaExceededError shape probe should evaluate");

    assert_eq!(
        result,
        "QuotaExceededError|full|22|7|11|true|true|true|true|false|function|get requested|0|true|true|true|function|get quota|0|true|true|true|true|true|true|true|true|false|QuotaExceededError|22"
    );
}

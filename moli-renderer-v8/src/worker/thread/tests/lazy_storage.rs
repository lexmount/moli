use super::*;

async fn lazy_diagnostics(
    handle: &WorkerHandle,
) -> crate::worker::handle::WorkerResourceOwnerSlotDiagnostics {
    timeout(TIMEOUT, handle.resource_owner_slot_diagnostics())
        .await
        .expect("timed out waiting for worker lazy-storage diagnostics")
        .expect("worker lazy-storage diagnostics should complete")
}

fn materialization_count(
    diagnostics: &crate::worker::handle::WorkerResourceOwnerSlotDiagnostics,
    name: &str,
) -> usize {
    diagnostics
        .materialized_interfaces
        .iter()
        .find_map(|(candidate, count)| (*candidate == name).then_some(*count))
        .unwrap_or(0)
}

#[tokio::test]
async fn worker_storage_surfaces_materialize_in_independent_stages() {
    ensure_v8();
    let lazy_interface_names =
        crate::context_bootstrap::exposed_interfaces::dedicated_worker_lazy_interface_names_for_test(
        );
    let lazy_interface_names =
        serde_json::to_string(&lazy_interface_names).expect("worker lazy interface names JSON");
    let source = r#"
        const lazyInterfaces = __MOLI_LAZY_INTERFACES__;

        self.onmessage = event => {
          Promise.resolve().then(async () => {
            if (event.data === "navigatorScalar") {
              const workerNavigator = navigator;
              const descriptor =
                Object.getOwnPropertyDescriptor(self, "navigator");
              postMessage({
                phase: "navigatorScalar",
                same: workerNavigator === navigator,
                instance: workerNavigator instanceof WorkerNavigator,
                userAgent: typeof workerNavigator.userAgent === "string",
                dataDescriptor:
                  descriptor.value === workerNavigator &&
                  typeof descriptor.get === "undefined"
              });
              return;
            }

            if (event.data === "crypto") {
              const workerCrypto = crypto;
              postMessage({
                phase: "crypto",
                same: workerCrypto === crypto,
                instance: workerCrypto instanceof Crypto
              });
              return;
            }

            if (event.data === "subtle") {
              const subtle = crypto.subtle;
              postMessage({
                phase: "subtle",
                same: subtle === crypto.subtle,
                instance: subtle instanceof SubtleCrypto
              });
              return;
            }

            if (event.data === "mediaCapabilities") {
              const capabilities = navigator.mediaCapabilities;
              const info = await capabilities.encodingInfo({
                type: "record",
                audio: { contentType: 'audio/webm; codecs="opus"' }
              });
              postMessage({
                phase: "mediaCapabilities",
                same: capabilities === navigator.mediaCapabilities,
                instance: capabilities instanceof MediaCapabilities,
                supportedType: typeof info.supported,
                configurationType: info.configuration.type
              });
              return;
            }

            if (event.data === "storage") {
              const manager = navigator.storage;
              let illegalReceiver;
              try {
                await StorageManager.prototype.getDirectory.call({});
                illegalReceiver = "fulfilled";
              } catch (error) {
                illegalReceiver = error.name;
              }
              const descriptor =
                Object.getOwnPropertyDescriptor(self, "StorageManager");
              postMessage({
                phase: "storage",
                same: manager === navigator.storage,
                instance: manager instanceof StorageManager,
                descriptorIsData:
                  descriptor.value === StorageManager &&
                  typeof descriptor.get === "undefined",
                illegalReceiver
              });
              return;
            }

            if (event.data === "sharedInterfaces") {
              const names = ["URL", "URLSearchParams", "FormData"];
              const constructors = names.map(name => self[name]);
              postMessage({
                phase: "sharedInterfaces",
                stable: names.every(
                  (name, index) => self[name] === constructors[index]
                ),
                dataDescriptors: names.every((name, index) => {
                  const descriptor =
                    Object.getOwnPropertyDescriptor(self, name);
                  return (
                    descriptor.value === constructors[index] &&
                    typeof descriptor.get === "undefined" &&
                    descriptor.enumerable === false &&
                    descriptor.writable === true &&
                    descriptor.configurable === true
                  );
                })
              });
              return;
            }

            if (event.data === "coreInterfaces") {
              const names = [
                "CustomEvent",
                "File",
                "XMLHttpRequest",
                "AbortSignal",
                "DOMException"
              ];
              const constructors = names.map(name => self[name]);
              postMessage({
                phase: "coreInterfaces",
                dataDescriptors: names.every((name, index) => {
                  const descriptor =
                    Object.getOwnPropertyDescriptor(self, name);
                  return (
                    descriptor.value === constructors[index] &&
                    typeof descriptor.get === "undefined" &&
                    descriptor.enumerable === false &&
                    descriptor.writable === true &&
                    descriptor.configurable === true
                  );
                }),
                customEventParent:
                  Object.getPrototypeOf(CustomEvent) === Event &&
                  Object.getPrototypeOf(CustomEvent.prototype) ===
                    Event.prototype,
                fileParent:
                  Object.getPrototypeOf(File) === Blob &&
                  Object.getPrototypeOf(File.prototype) === Blob.prototype,
                xhrParent:
                  Object.getPrototypeOf(XMLHttpRequest) ===
                    XMLHttpRequestEventTarget &&
                  Object.getPrototypeOf(XMLHttpRequest.prototype) ===
                    XMLHttpRequestEventTarget.prototype,
                abortParent:
                  Object.getPrototypeOf(AbortSignal) === EventTarget &&
                  Object.getPrototypeOf(AbortSignal.prototype) ===
                    EventTarget.prototype,
                domExceptionParent:
                  Object.getPrototypeOf(DOMException.prototype) ===
                    Error.prototype
              });
              return;
            }

            if (event.data === "indexedDB") {
              const factory = indexedDB;
              const descriptor =
                Object.getOwnPropertyDescriptor(self, "indexedDB");
              postMessage({
                phase: "indexedDB",
                same: factory === indexedDB,
                instance: factory instanceof IDBFactory,
                dataDescriptor:
                  descriptor.value === factory &&
                  typeof descriptor.get === "undefined"
              });
              return;
            }

            if (event.data === "override") {
              self.StorageEstimate = 17;
              const descriptor =
                Object.getOwnPropertyDescriptor(self, "StorageEstimate");
              postMessage({
                phase: "override",
                value: StorageEstimate,
                descriptorValue: descriptor.value
              });
              return;
            }

            if (event.data === "sync") {
              const constructor = FileSystemSyncAccessHandle;
              postMessage({
                phase: "sync",
                same: constructor === FileSystemSyncAccessHandle,
                name: constructor.name,
                tag:
                  constructor.prototype[Symbol.toStringTag]
              });
              return;
            }

            if (event.data === "buckets") {
              const manager = navigator.storageBuckets;
              postMessage({
                phase: "buckets",
                same: manager === navigator.storageBuckets,
                instance: manager instanceof StorageBucketManager
              });
            }
          }).catch(error => {
            postMessage({
              phase: "error",
              name: error && error.name,
              message: error && error.message
            });
          });
        };

        postMessage({
          phase: "initial",
          present: lazyInterfaces.every(name => name in self),
          own: lazyInterfaces.every(name => Object.hasOwn(self, name)),
          enumerable:
            lazyInterfaces.some(name => Object.keys(self).includes(name)),
          chromiumExposure:
            ["Worker", "XMLHttpRequest", "FileReaderSync"].every(
              name => name in self
            ) &&
            ["CSSRule", "CSSStyleRule", "CSSStyleSheet"].every(
              name => !(name in self)
            )
        });
        "#
    .replace("__MOLI_LAZY_INTERFACES__", &lazy_interface_names);
    let mut handle = spawn_worker(source, "https://worker-lazy-storage.test/worker.js".into());

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"initial","present":true,"own":true,"enumerable":false,"chromiumExposure":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(diagnostics.storage_constructor_materializations, 0);
    assert_eq!(
        diagnostics.materialized_interfaces,
        vec![("Performance", 1), ("EventTarget", 1)],
        "only constructors required by the eagerly exposed performance object may materialize during worker bootstrap"
    );
    assert!(!diagnostics.storage_manager_materialized);
    assert!(!diagnostics.storage_bucket_manager_materialized);
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.post_message(serialize_test_string("navigatorScalar"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"navigatorScalar","same":true,"instance":true,"userAgent":true,"dataDescriptor":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(
        materialization_count(&diagnostics, "WorkerNavigator"),
        1,
        "reading a scalar navigator property should materialize only WorkerNavigator"
    );
    assert_eq!(
        materialization_count(&diagnostics, "MediaCapabilities"),
        0,
        "reading a scalar navigator property must not materialize MediaCapabilities"
    );
    assert!(!diagnostics.storage_manager_materialized);
    assert!(!diagnostics.storage_bucket_manager_materialized);

    handle.post_message(serialize_test_string("mediaCapabilities"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"mediaCapabilities","same":true,"instance":true,"supportedType":"boolean","configurationType":"record"}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(
        materialization_count(&diagnostics, "MediaCapabilities"),
        1,
        "navigator.mediaCapabilities should materialize one shared worker interface"
    );

    handle.post_message(serialize_test_string("crypto"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"crypto","same":true,"instance":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(materialization_count(&diagnostics, "Crypto"), 1);
    assert_eq!(
        materialization_count(&diagnostics, "SubtleCrypto"),
        0,
        "reading worker crypto must not materialize SubtleCrypto"
    );
    assert_eq!(materialization_count(&diagnostics, "CryptoKey"), 0);

    handle.post_message(serialize_test_string("subtle"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"subtle","same":true,"instance":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(materialization_count(&diagnostics, "Crypto"), 1);
    assert_eq!(materialization_count(&diagnostics, "SubtleCrypto"), 1);
    assert_eq!(
        materialization_count(&diagnostics, "CryptoKey"),
        0,
        "CryptoKey must stay lazy until a key object is created"
    );

    handle.post_message(serialize_test_string("coreInterfaces"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"coreInterfaces","dataDescriptors":true,"customEventParent":true,"fileParent":true,"xhrParent":true,"abortParent":true,"domExceptionParent":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    for name in [
        "Event",
        "CustomEvent",
        "Blob",
        "File",
        "EventTarget",
        "XMLHttpRequestEventTarget",
        "XMLHttpRequest",
        "AbortSignal",
        "DOMException",
    ] {
        assert_eq!(
            materialization_count(&diagnostics, name),
            1,
            "{name} should materialize exactly once through the representative worker cohort"
        );
    }
    for name in [
        "Request",
        "MessageChannel",
        "FileReader",
        "AbortController",
        "QuotaExceededError",
    ] {
        assert_eq!(
            materialization_count(&diagnostics, name),
            0,
            "{name} should remain lazy until directly required"
        );
    }
    assert_eq!(diagnostics.storage_constructor_materializations, 0);
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.post_message(serialize_test_string("sharedInterfaces"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"sharedInterfaces","stable":true,"dataDescriptors":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    for name in ["URL", "URLSearchParams", "FormData"] {
        assert_eq!(
            materialization_count(&diagnostics, name),
            1,
            "{name} should materialize exactly once"
        );
    }
    assert_eq!(diagnostics.storage_constructor_materializations, 0);
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.post_message(serialize_test_string("indexedDB"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"indexedDB","same":true,"instance":true,"dataDescriptor":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(materialization_count(&diagnostics, "IDBFactory"), 1);
    for name in [
        "IDBRequest",
        "IDBOpenDBRequest",
        "IDBDatabase",
        "IDBTransaction",
        "IDBObjectStore",
        "IDBIndex",
        "IDBCursor",
        "IDBCursorWithValue",
        "IDBKeyRange",
        "IDBVersionChangeEvent",
    ] {
        assert_eq!(
            materialization_count(&diagnostics, name),
            0,
            "{name} should remain unmaterialized after reading only indexedDB"
        );
    }
    assert_eq!(diagnostics.storage_constructor_materializations, 0);
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.post_message(serialize_test_string("storage"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"storage","same":true,"instance":true,"descriptorIsData":true,"illegalReceiver":"TypeError"}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(
        diagnostics.storage_constructor_materializations, 1,
        "navigator.storage should materialize only StorageManager"
    );
    assert!(diagnostics.storage_manager_materialized);
    assert!(!diagnostics.storage_bucket_manager_materialized);
    assert!(
        !diagnostics.opfs_owner_state_materialized,
        "wrapper access and an illegal OPFS call must not allocate owner state"
    );

    handle.post_message(serialize_test_string("override"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"override","value":17,"descriptorValue":17}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(
        diagnostics.storage_constructor_materializations, 1,
        "assignment before first read must bypass StorageEstimate materialization"
    );
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.post_message(serialize_test_string("sync"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"sync","same":true,"name":"FileSystemSyncAccessHandle","tag":"FileSystemSyncAccessHandle"}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(
        diagnostics.storage_constructor_materializations, 2,
        "the DedicatedWorker-only sync constructor owns its own lazy slot"
    );
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.post_message(serialize_test_string("buckets"));
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"buckets","same":true,"instance":true}"#
    );
    let diagnostics = lazy_diagnostics(&handle).await;
    assert_eq!(
        diagnostics.storage_constructor_materializations, 3,
        "navigator.storageBuckets should materialize only StorageBucketManager"
    );
    assert!(diagnostics.storage_manager_materialized);
    assert!(diagnostics.storage_bucket_manager_materialized);
    assert!(!diagnostics.opfs_owner_state_materialized);

    handle.terminate_and_join();
}

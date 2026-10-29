// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn worker_navigator_uses_loader_identity_for_user_agent_data() {
    ensure_v8();
    const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36";
    let mut config = FetchConfig::default();
    config.set_user_agent(USER_AGENT);
    config.push_default_request_header("Accept-Language", "fr-CA,fr;q=0.8,en;q=0.5");
    let loader = ResourceRequestClient::new(&config).expect("worker loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        const uaData = navigator.userAgentData;
        Promise.all([
            uaData.getHighEntropyValues([]),
            uaData.getHighEntropyValues([
                "architecture",
                "formFactors",
                "fullVersionList",
                "uaFullVersion",
                "unsupported"
            ]),
            uaData.getHighEntropyValues().then(
                () => "resolved",
                error => error && error.name
            )
        ]).then(([empty, selected, missingArgument]) => {
            postMessage({
                userAgent: navigator.userAgent,
                appVersion: navigator.appVersion,
                language: navigator.language,
                languages: Array.from(navigator.languages),
                constructorType: typeof NavigatorUAData,
                constructorOwn: Object.prototype.hasOwnProperty.call(self, "NavigatorUAData"),
                dataType: typeof uaData,
                sameObject: uaData === navigator.userAgentData,
                instance: uaData instanceof NavigatorUAData,
                tag: Object.prototype.toString.call(uaData),
                json: uaData.toJSON(),
                emptyKeys: Object.keys(empty),
                selectedKeys: Object.keys(selected),
                selected,
                missingArgument
            });
            close();
        });
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"userAgent":"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36","appVersion":"5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36","language":"fr-CA","languages":["fr-CA","fr","en"],"constructorType":"function","constructorOwn":true,"dataType":"object","sameObject":false,"instance":true,"tag":"[object NavigatorUAData]","json":{"brands":[{"brand":"Chromium","version":"146"},{"brand":"Not-A.Brand","version":"24"},{"brand":"Google Chrome","version":"146"}],"mobile":false,"platform":"Windows"},"emptyKeys":["brands","mobile","platform"],"selectedKeys":["architecture","brands","formFactors","fullVersionList","mobile","platform","uaFullVersion"],"selected":{"architecture":"x86","brands":[{"brand":"Chromium","version":"146"},{"brand":"Not-A.Brand","version":"24"},{"brand":"Google Chrome","version":"146"}],"formFactors":["Desktop"],"fullVersionList":[{"brand":"Chromium","version":"146.1.2.3"},{"brand":"Not-A.Brand","version":"24.0.0.0"},{"brand":"Google Chrome","version":"146.1.2.3"}],"mobile":false,"platform":"Windows","uaFullVersion":"146.1.2.3"},"missingArgument":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_storage_apis_are_secure_context_only() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const proto = Object.getPrototypeOf(navigator);
        postMessage({
            secure: isSecureContext,
            storageInNavigator: "storage" in navigator,
            storageBucketsInNavigator: "storageBuckets" in navigator,
            serviceWorkerInNavigator: "serviceWorker" in navigator,
            userAgentDataInNavigator: "userAgentData" in navigator,
            storageInProto: Object.prototype.hasOwnProperty.call(proto, "storage"),
            storageBucketsInProto: Object.prototype.hasOwnProperty.call(proto, "storageBuckets"),
            serviceWorkerInProto: Object.prototype.hasOwnProperty.call(proto, "serviceWorker"),
            userAgentDataInProto:
              Object.prototype.hasOwnProperty.call(proto, "userAgentData"),
            storageValueType: typeof navigator.storage,
            storageBucketsValueType: typeof navigator.storageBuckets,
            serviceWorkerValueType: typeof navigator.serviceWorker,
            userAgentDataValueType: typeof navigator.userAgentData,
            storageManagerGlobal: "StorageManager" in self,
            storageEstimateGlobal: "StorageEstimate" in self,
            storageBucketManagerGlobal: "StorageBucketManager" in self,
            storageBucketGlobal: "StorageBucket" in self,
            fileSystemHandleGlobal: "FileSystemHandle" in self,
            fileSystemFileHandleGlobal: "FileSystemFileHandle" in self,
            fileSystemDirectoryHandleGlobal: "FileSystemDirectoryHandle" in self,
            fileSystemWritableFileStreamGlobal:
              "FileSystemWritableFileStream" in self,
            fileSystemSyncAccessHandleGlobal:
              "FileSystemSyncAccessHandle" in self,
        });
        close();
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"secure":false,"storageInNavigator":false,"storageBucketsInNavigator":false,"serviceWorkerInNavigator":false,"userAgentDataInNavigator":false,"storageInProto":false,"storageBucketsInProto":false,"serviceWorkerInProto":false,"userAgentDataInProto":false,"storageValueType":"undefined","storageBucketsValueType":"undefined","serviceWorkerValueType":"undefined","userAgentDataValueType":"undefined","storageManagerGlobal":false,"storageEstimateGlobal":false,"storageBucketManagerGlobal":false,"storageBucketGlobal":false,"fileSystemHandleGlobal":false,"fileSystemFileHandleGlobal":false,"fileSystemDirectoryHandleGlobal":false,"fileSystemWritableFileStreamGlobal":false,"fileSystemSyncAccessHandleGlobal":false}"#
    );
}

#[tokio::test]
async fn worker_cross_origin_isolated_defaults_to_false() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            secure: isSecureContext,
            isolated: crossOriginIsolated,
        });
        close();
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"secure":true,"isolated":false}"#);
}

#[tokio::test]
async fn worker_cross_origin_isolated_reflects_policy_context_capability() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            postMessage({
                secure: isSecureContext,
                isolated: crossOriginIsolated,
            });
            close();
            "#
            .into(),
            "https://example.test/worker/dip.js".into(),
        )
        .with_policy_context(crate::types::SubresourcePolicyContext {
            document_isolation_policy:
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp,
            cross_origin_isolated: true,
            ..Default::default()
        }),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"secure":true,"isolated":true}"#);
}

#[tokio::test]
async fn worker_cross_origin_isolated_does_not_infer_from_dip_policy_context() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            postMessage({
                secure: isSecureContext,
                isolated: crossOriginIsolated,
            });
            close();
            "#
            .into(),
            "https://example.test/worker/dip-only.js".into(),
        )
        .with_policy_context(crate::types::SubresourcePolicyContext {
            document_isolation_policy:
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp,
            ..Default::default()
        }),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"secure":true,"isolated":false}"#);
}

#[tokio::test]
async fn worker_storage_buckets_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        "use strict";
        const manager = navigator.storageBuckets;
        const illegalManagerProbe = StorageBucketManager.prototype.keys.call({}).then(
            () => "resolved",
            error => error && error.name
        );
        const illegalBucketProbe = StorageBucket.prototype.persisted.call({}).then(
            () => "resolved",
            error => error && error.name
        );
        Promise.resolve().then(async () => {
            await manager.open("worker-bucket-b");
            const bucket = await manager.open("worker-bucket-a", {
                durability: "strict",
                quota: 8192,
                persisted: true
            });
            const keysBeforeDelete = await manager.keys();
            const expiresInitial = await bucket.expires();
            const durabilityInitial = await bucket.durability();
            const persistedInitial = await bucket.persisted();
            const expiresDate = Date.now() + 60000;
            await bucket.setExpires(expiresDate);
            const reopened = await manager.open("worker-bucket-a");
            const expiresAfterReopenMatches = await reopened.expires() === expiresDate;
            const durabilityAfterReopen = await reopened.durability();
            const quotaAfterReopen = (await reopened.estimate()).quota;
            const persistedAfterReopen = await reopened.persisted();
            const cache = await reopened.caches.open("worker-cache");
            await cache.put("worker.txt", new Response("worker cache"));
            const cacheMatchText = await (await cache.match("worker.txt")).text();
            const cacheUsageAfterReopen = (await reopened.estimate()).usageDetails.caches > 0;
            await manager.delete("worker-bucket-b");
            const keysAfterDelete = await manager.keys();
            const [illegalManagerError, illegalBucketError] = await Promise.all([
                illegalManagerProbe,
                illegalBucketProbe
            ]);
            postMessage({
                managerCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageBucketManager"),
                bucketCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageBucket"),
                managerTag: Object.prototype.toString.call(manager),
                managerInstanceof: manager instanceof StorageBucketManager,
                managerSameObject: manager === navigator.storageBuckets,
                managerOwnKeys: Object.prototype.hasOwnProperty.call(manager, "keys"),
                openLength: StorageBucketManager.prototype.open.length,
                keysLength: StorageBucketManager.prototype.keys.length,
                deleteLength: StorageBucketManager.prototype.delete.length,
                bucketTag: Object.prototype.toString.call(bucket),
                bucketInstanceof: bucket instanceof StorageBucket,
                bucketName: bucket.name,
                bucketPersistType: typeof bucket.persist,
                bucketPersistedLength: StorageBucket.prototype.persisted.length,
                bucketEstimateLength: StorageBucket.prototype.estimate.length,
                bucketDurabilityLength: StorageBucket.prototype.durability.length,
                bucketSetExpiresLength: StorageBucket.prototype.setExpires.length,
                bucketExpiresLength: StorageBucket.prototype.expires.length,
                bucketGetDirectoryLength: StorageBucket.prototype.getDirectory.length,
                expiresInitial,
                expiresAfterReopenMatches,
                durabilityInitial,
                durabilityAfterReopen,
                quotaAfterReopen,
                persistedInitial,
                persistedAfterReopen,
                cacheMatchText,
                cacheUsageAfterReopen,
                keysBeforeDelete,
                keysAfterDelete,
                illegalManagerError,
                illegalBucketError
            });
            close();
        }).catch(error => {
            postMessage({
                errorName: error && error.name,
                errorMessage: error && error.message
            });
            close();
        });
        "#
        .into(),
        "http://127.0.0.1/worker/storage-buckets.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"managerCtorOwn":true,"bucketCtorOwn":true,"managerTag":"[object StorageBucketManager]","managerInstanceof":true,"managerSameObject":true,"managerOwnKeys":false,"openLength":1,"keysLength":0,"deleteLength":1,"bucketTag":"[object StorageBucket]","bucketInstanceof":true,"bucketName":"worker-bucket-a","bucketPersistType":"function","bucketPersistedLength":0,"bucketEstimateLength":0,"bucketDurabilityLength":0,"bucketSetExpiresLength":1,"bucketExpiresLength":0,"bucketGetDirectoryLength":0,"expiresInitial":null,"expiresAfterReopenMatches":true,"durabilityInitial":"strict","durabilityAfterReopen":"strict","quotaAfterReopen":8192,"persistedInitial":true,"persistedAfterReopen":true,"cacheMatchText":"worker cache","cacheUsageAfterReopen":true,"keysBeforeDelete":["worker-bucket-a","worker-bucket-b"],"keysAfterDelete":["worker-bucket-a"],"illegalManagerError":"TypeError","illegalBucketError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_global_caches_surface_uses_default_bucket() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        "use strict";
        Promise.resolve().then(async () => {
            const cacheStorage = self.caches;
            const sameObject = cacheStorage === self.caches;
            const keysBefore = await cacheStorage.keys();
            const cache = await cacheStorage.open("global-worker-cache");
            await cache.put("worker-global.txt", new Response("global cache"));
            const matched = await cache.match("worker-global.txt");
            const matchedText = await matched.text();
            const storageMatched = await cacheStorage.match("worker-global.txt");
            const storageMatchedText = await storageMatched.text();
            const hasAfterOpen = await cacheStorage.has("global-worker-cache");
            const hasMissing = await cacheStorage.has("missing-cache");
            const keysAfterOpen = await cacheStorage.keys();
            const defaultBucketKeys = await navigator.storageBuckets.keys();
            const deleted = await cacheStorage.delete("global-worker-cache");
            const keysAfterDelete = await cacheStorage.keys();
            await navigator.storageBuckets.delete("default");
            postMessage({
                hasOwnCaches: Object.prototype.hasOwnProperty.call(self, "caches"),
                cacheStorageTag: Object.prototype.toString.call(cacheStorage),
                sameObject,
                keysBefore,
                cacheTag: Object.prototype.toString.call(cache),
                matchedStatus: matched.status,
                matchedText,
                storageMatchedText,
                hasAfterOpen,
                hasMissing,
                keysAfterOpen,
                defaultBucketKeys,
                deleted,
                keysAfterDelete
            });
            close();
        }).catch(error => {
            postMessage({
                errorName: error && error.name,
                errorMessage: error && error.message
            });
            close();
        });
        "#
        .into(),
        "https://example.test/worker/global-caches.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"hasOwnCaches":true,"cacheStorageTag":"[object CacheStorage]","sameObject":true,"keysBefore":[],"cacheTag":"[object Cache]","matchedStatus":200,"matchedText":"global cache","storageMatchedText":"global cache","hasAfterOpen":true,"hasMissing":false,"keysAfterOpen":["global-worker-cache"],"defaultBucketKeys":[],"deleted":true,"keysAfterDelete":[]}"#
    );
}

#[tokio::test]
async fn worker_location_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const beforeHref = location.href;
        location.href = "https://example.com/ignored";
        const proto = Object.getPrototypeOf(location);
        const href = Object.getOwnPropertyDescriptor(proto, "href");
        const toString = Object.getOwnPropertyDescriptor(proto, "toString");
        postMessage({
            locationOwn: Object.prototype.hasOwnProperty.call(self, "location"),
            ctorOwn: Object.prototype.hasOwnProperty.call(self, "WorkerLocation"),
            ctorType: typeof WorkerLocation,
            ctorName: location.constructor && location.constructor.name,
            protoCtor: proto && proto.constructor && proto.constructor.name,
            tag: Object.prototype.toString.call(location),
            stringified: String(location),
            directToString: toString.value.call(location),
            instanceofWorkerLocation: location instanceof WorkerLocation,
            ownHref: Object.prototype.hasOwnProperty.call(location, "href"),
            hrefGetterType: typeof href?.get,
            toStringDescriptor: [
                typeof toString?.value,
                toString?.value?.name,
                toString?.value?.length,
                toString?.enumerable,
                toString?.writable,
                toString?.configurable
            ].join(":"),
            href: location.href,
            origin: location.origin,
            protocol: location.protocol,
            host: location.host,
            hostname: location.hostname,
            port: location.port,
            pathname: location.pathname,
            search: location.search,
            hash: location.hash,
            unchanged: location.href === beforeHref,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?srch%20#hash".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"locationOwn":true,"ctorOwn":true,"ctorType":"function","ctorName":"WorkerLocation","protoCtor":"WorkerLocation","tag":"[object WorkerLocation]","stringified":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","directToString":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","instanceofWorkerLocation":true,"ownHref":false,"hrefGetterType":"function","toStringDescriptor":"function:toString:0:true:true:true","href":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","origin":"http://127.0.0.1:38080","protocol":"http:","host":"127.0.0.1:38080","hostname":"127.0.0.1","port":"38080","pathname":"/worker/main.js","search":"?srch%20","hash":"#hash","unchanged":true}"##
    );
}

#[tokio::test]
async fn worker_location_backing_slot_ignores_reflection_and_spoofing() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const internal = "__moliWorkerLocationData";
        const beforeHref = location.href;
        const beforeStringified = String(location);
        const proto = Object.getPrototypeOf(location);
        const hrefGetter = Object.getOwnPropertyDescriptor(proto, "href").get;
        const toString = Object.getOwnPropertyDescriptor(proto, "toString").value;
        const reflectedBefore = Object.getOwnPropertyNames(location).includes(internal);
        const fakeBacking = {
            href: "https://spoofed.example/worker.js",
            origin: "https://spoofed.example",
            protocol: "https:",
            host: "spoofed.example",
            hostname: "spoofed.example",
            port: "",
            pathname: "/worker.js",
            search: "",
            hash: ""
        };
        Object.defineProperty(location, internal, {
            value: fakeBacking,
            configurable: true
        });
        const fakeReceiver = {};
        Object.defineProperty(fakeReceiver, internal, {
            value: fakeBacking,
            configurable: true
        });
        postMessage({
            reflectedBefore,
            ownAfterSpoof: Object.prototype.hasOwnProperty.call(location, internal),
            hrefAfterSpoof: location.href,
            stringAfterSpoof: String(location),
            getterCallAfterSpoof: hrefGetter.call(location),
            fakeHrefIsUndefined: hrefGetter.call(fakeReceiver) === undefined,
            fakeStringIsUndefined: toString.call(fakeReceiver) === undefined,
            unchanged: location.href === beforeHref && String(location) === beforeStringified,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?srch%20#hash".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"reflectedBefore":false,"ownAfterSpoof":true,"hrefAfterSpoof":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","stringAfterSpoof":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","getterCallAfterSpoof":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","fakeHrefIsUndefined":true,"fakeStringIsUndefined":true,"unchanged":true}"##
    );
}

#[tokio::test]
async fn worker_script_url_slot_ignores_global_reflection_and_spoofing() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const internal = "__moliWorkerCurrentScriptUrl";
        const reflectedBefore = Object.getOwnPropertyNames(globalThis).includes(internal);
        const requestBefore = new Request("./api").url;
        Object.prototype[internal] = "https://prototype-spoof.example/proto.js";
        globalThis[internal] = "https://own-spoof.example/own.js";
        const requestAfter = new Request("./api").url;
        postMessage({
            reflectedBefore,
            ownAfterSpoof: Object.prototype.hasOwnProperty.call(globalThis, internal),
            publicSpoof: globalThis[internal],
            requestBefore,
            requestAfter,
            stable: requestBefore === requestAfter,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?srch%20#hash".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"reflectedBefore":false,"ownAfterSpoof":true,"publicSpoof":"https://own-spoof.example/own.js","requestBefore":"http://127.0.0.1:38080/worker/api","requestAfter":"http://127.0.0.1:38080/worker/api","stable":true}"##
    );
}

#[tokio::test]
async fn worker_location_data_url_uses_null_origin() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            href: location.href,
            origin: location.origin,
            protocol: location.protocol,
            host: location.host,
            pathname: location.pathname,
            search: location.search,
            hash: location.hash,
            stringified: location.toString(),
        });
        close();
        "#
        .into(),
        "data:text/javascript,hello#frag".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"href":"data:text/javascript,hello#frag","origin":"null","protocol":"data:","host":"","pathname":"text/javascript,hello","search":"","hash":"#frag","stringified":"data:text/javascript,hello#frag"}"##
    );
}

#[tokio::test]
async fn worker_location_empty_query_serializes_to_empty_search() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            href: location.href,
            search: location.search,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"href":"http://127.0.0.1:38080/worker/main.js?","search":""}"##
    );
}

#[tokio::test]
async fn worker_location_empty_fragment_serializes_to_empty_hash() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            href: location.href,
            hash: location.hash,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js#".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"href":"http://127.0.0.1:38080/worker/main.js#","hash":""}"##
    );
}

#[tokio::test]
async fn data_url_worker_exposes_indexeddb_but_denies_opaque_origin_access() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let openError = null;
        try {
            indexedDB.open("opaque-worker-db");
        } catch (error) {
            openError = error.name;
        }
        indexedDB.databases().then(
          () => "resolved",
          error => error && error.name
        ).then(databasesError => postMessage({
            present: "indexedDB" in self,
            type: typeof indexedDB,
            openType: typeof indexedDB.open,
            databasesType: typeof indexedDB.databases,
            openError,
            databasesError
        })).finally(() => close());
        "#
        .into(),
        "data:text/javascript,hello".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"present":true,"type":"object","openType":"function","databasesType":"function","openError":"SecurityError","databasesError":"SecurityError"}"#
    );
}

#[tokio::test]
async fn data_url_worker_storage_bucket_api_is_hidden_in_insecure_context() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const proto = Object.getPrototypeOf(navigator);
        postMessage({
            secure: isSecureContext,
            storageInNavigator: "storage" in navigator,
            storageBucketsInNavigator: "storageBuckets" in navigator,
            storageInProto: Object.prototype.hasOwnProperty.call(proto, "storage"),
            storageBucketsInProto: Object.prototype.hasOwnProperty.call(proto, "storageBuckets"),
            storageValueType: typeof navigator.storage,
            storageBucketsValueType: typeof navigator.storageBuckets,
            storageManagerGlobal: "StorageManager" in self,
            storageEstimateGlobal: "StorageEstimate" in self,
            storageBucketManagerGlobal: "StorageBucketManager" in self,
            storageBucketGlobal: "StorageBucket" in self,
            fileSystemHandleGlobal: "FileSystemHandle" in self,
            fileSystemFileHandleGlobal: "FileSystemFileHandle" in self,
            fileSystemDirectoryHandleGlobal: "FileSystemDirectoryHandle" in self,
            fileSystemWritableFileStreamGlobal:
              "FileSystemWritableFileStream" in self,
            fileSystemSyncAccessHandleGlobal:
              "FileSystemSyncAccessHandle" in self,
        });
        close();
        "#
        .into(),
        "data:text/javascript,storage-buckets".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"secure":false,"storageInNavigator":false,"storageBucketsInNavigator":false,"storageInProto":false,"storageBucketsInProto":false,"storageValueType":"undefined","storageBucketsValueType":"undefined","storageManagerGlobal":false,"storageEstimateGlobal":false,"storageBucketManagerGlobal":false,"storageBucketGlobal":false,"fileSystemHandleGlobal":false,"fileSystemFileHandleGlobal":false,"fileSystemDirectoryHandleGlobal":false,"fileSystemWritableFileStreamGlobal":false,"fileSystemSyncAccessHandleGlobal":false}"#
    );
}

#[tokio::test]
async fn third_party_worker_storage_uses_partitioned_storage_key() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let bucket_store = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
    let script_url = "https://worker.example/partitioned-worker-storage.js";
    let top_level_site = "https://app.example";
    let storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &url::Url::parse(script_url).expect("worker script URL should parse"),
        top_level_site.to_owned(),
        None,
    );
    let serialized_storage_key = storage_key.serialized_storage_key();

    let db_name = "partitioned-worker-idb";
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const bucket = await navigator.storageBuckets.open("partitioned-worker-bucket");
                const keys = await navigator.storageBuckets.keys();
                const open = indexedDB.open("partitioned-worker-idb", 7);
                open.onupgradeneeded = () => {
                    open.result.createObjectStore("kv");
                };
                open.onerror = () => {
                    postMessage({ stage: "open", name: open.error && open.error.name });
                    close();
                };
                open.onsuccess = () => {
                    const db = open.result;
                    const tx = db.transaction("kv", "readwrite");
                    tx.objectStore("kv").put("stored", "key");
                    tx.onerror = () => {
                        postMessage({ stage: "tx", name: tx.error && tx.error.name });
                        db.close();
                        close();
                    };
                    tx.oncomplete = () => {
                        db.close();
                        postMessage({
                            bucketName: bucket.name,
                            keys,
                            idbOpen: true
                        });
                        close();
                    };
                };
            })().catch(error => {
                postMessage({ stage: "promise", name: error && error.name });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_key_top_level_site(Some(top_level_site.to_owned()))
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)))
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"bucketName":"partitioned-worker-bucket","keys":["partitioned-worker-bucket"],"idbOpen":true}"#
    );

    assert_eq!(
        manager
            .lock()
            .sqlite_service()
            .unwrap()
            .database_version(serialized_storage_key.as_bytes(), db_name)
            .expect("partitioned IndexedDB version should be readable"),
        Some(7)
    );
    assert_eq!(
        manager
            .lock()
            .sqlite_service()
            .unwrap()
            .database_version(b"https://worker.example", db_name)
            .expect("script-origin IndexedDB version should be readable"),
        None
    );
    assert_eq!(
        bucket_store.lock().keys(&serialized_storage_key),
        vec!["partitioned-worker-bucket".to_owned()]
    );
    assert_eq!(
        bucket_store.lock().keys("https://worker.example"),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn service_worker_storage_uses_explicit_registration_storage_key() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let bucket_store = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
    let script_url = "https://cdn.example/service-worker-storage.js";
    let storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &url::Url::parse(script_url).expect("worker script URL should parse"),
        "https://app.example".to_owned(),
        None,
    );
    let serialized_storage_key = storage_key.serialized_storage_key();

    let db_name = "service-worker-idb";
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const bucket = await navigator.storageBuckets.open("service-worker-bucket");
                const open = indexedDB.open("service-worker-idb", 11);
                open.onupgradeneeded = () => {
                    open.result.createObjectStore("kv");
                };
                open.onerror = () => {
                    throw open.error;
                };
                open.onsuccess = () => {
                    const db = open.result;
                    const tx = db.transaction("kv", "readwrite");
                    tx.objectStore("kv").put("stored", "key");
                    tx.onerror = () => {
                        db.close();
                        throw tx.error;
                    };
                    tx.oncomplete = () => {
                        db.close();
                        if (bucket.name !== "service-worker-bucket") {
                            throw new Error("unexpected bucket name");
                        }
                        skipWaiting();
                    };
                };
            })().catch(error => {
                throw error;
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://cdn.example/").unwrap(),
        })
        .with_api_storage_key(Some(storage_key.clone()))
        .with_broadcast_channel_top_level_site(Some("https://ignored.example".to_owned()))
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)))
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::ServiceWorkerSkipWaiting {
            registration_id,
            version_id,
        } => {
            assert_eq!(
                registration_id,
                crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1)
            );
            assert_eq!(
                version_id,
                crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1)
            );
        }
        other => panic!("expected service worker skipWaiting, got {other:?}"),
    }

    assert_eq!(
        manager
            .lock()
            .sqlite_service()
            .unwrap()
            .database_version(serialized_storage_key.as_bytes(), db_name)
            .expect("registration-key IndexedDB version should be readable"),
        Some(11)
    );
    assert_eq!(
        manager
            .lock()
            .sqlite_service()
            .unwrap()
            .database_version(b"https://cdn.example", db_name)
            .expect("script-origin IndexedDB version should be readable"),
        None
    );
    assert_eq!(
        bucket_store.lock().keys(&serialized_storage_key),
        vec!["service-worker-bucket".to_owned()]
    );
    assert_eq!(
        bucket_store.lock().keys("https://cdn.example"),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn worker_deleted_storage_bucket_handle_rejects_metadata_and_indexeddb() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let bucket_store = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const bucket = await navigator.storageBuckets.open("deleted-worker-bucket", {
                    durability: "strict"
                });
                await navigator.storageBuckets.delete("deleted-worker-bucket");
                const outcome = promise => promise.then(
                    () => "fulfilled",
                    error => error && error.name
                );
                const idb = await new Promise(resolve => {
                    const request = bucket.indexedDB.open("messages");
                    request.onsuccess = () => {
                        request.result.close();
                        resolve("fulfilled");
                    };
                    request.onerror = () => resolve(request.error && request.error.name);
                });
                postMessage({
                    persisted: await outcome(bucket.persisted()),
                    durability: await outcome(bucket.durability()),
                    expires: await outcome(bucket.expires()),
                    setExpires: await outcome(bucket.setExpires(Date.now() + 1000)),
                    idb
                });
                close();
            })().catch(error => {
                postMessage({ stage: "promise", name: error && error.name });
                close();
            });
            "#
            .into(),
            "https://worker.example/deleted-storage-bucket.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)))
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"persisted":"UnknownError","durability":"UnknownError","expires":"UnknownError","setExpires":"UnknownError","idb":"UnknownError"}"#
    );
}

#[tokio::test]
async fn worker_manager_only_partition_binds_named_bucket_indexeddb_quota_owner() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const request = request => new Promise((resolve, reject) => {
                    request.onsuccess = () => resolve(request.result);
                    request.onerror = () => reject(request.error);
                });
                const transaction = transaction => new Promise((resolve, reject) => {
                    transaction.oncomplete = () => resolve();
                    transaction.onabort = transaction.onerror = () => reject(transaction.error);
                });
                const bucket = await navigator.storageBuckets.open("manager-only", {
                    quota: 10000
                });
                const open = bucket.indexedDB.open("quota", 1);
                open.onupgradeneeded = () => open.result.createObjectStore("kv");
                const db = await request(open);
                const tx = db.transaction("kv", "readwrite");
                const committed = transaction(tx);
                tx.objectStore("kv").put(new Uint8Array(5000), "stored");
                await committed;
                const usage = await bucket.estimate();
                const root = await bucket.getDirectory();
                const file = await root.getFileHandle("blocked.bin", { create: true });
                const writer = await file.createWritable();
                let blocked;
                try {
                    await writer.write(new Uint8Array(6000));
                    blocked = "resolved";
                } catch (error) {
                    blocked = error && error.name;
                }
                db.close();
                await navigator.storageBuckets.delete("manager-only");
                postMessage({
                    indexedDbUsagePositive: usage.usageDetails.indexedDB > 0,
                    blocked
                });
                close();
            })().catch(error => {
                postMessage({ stage: "promise", name: error && error.name });
                close();
            });
            "#
            .into(),
            "https://worker.example/manager-only-bucket-owner.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"indexedDbUsagePositive":true,"blocked":"QuotaExceededError"}"#
    );
}

#[tokio::test]
async fn worker_indexed_db_request_chain_does_not_starve_timers() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const open = indexedDB.open("worker-idb-timer-fairness-" + Math.random(), 1);
            open.onupgradeneeded = () => {
                open.result.createObjectStore("store");
            };
            open.onerror = () => {
                postMessage({ stage: "open", name: open.error && open.error.name });
                close();
            };
            open.onsuccess = () => {
                const db = open.result;
                const tx = db.transaction("store", "readonly");
                let keepSpinning = true;
                let spins = 0;
                function finish(status) {
                    keepSpinning = false;
                    db.close();
                    postMessage(status);
                    close();
                }
                function spin() {
                    if (!keepSpinning) {
                        return;
                    }
                    spins += 1;
                    if (spins > 1000) {
                        finish("timer-starved");
                        return;
                    }
                    tx.objectStore("store").get(0).onsuccess = spin;
                }
                setTimeout(() => finish("timer-fired"), 0);
                spin();
            };
            "#
            .into(),
            "https://worker.example/indexeddb-timer-fairness.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""timer-fired""#);
}

#[tokio::test]
async fn http_worker_indexeddb_uses_storage_manager_and_rejects_wasm_module_storage() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let mut handle =
        crate::worker::thread::spawn_worker_with_request_client_and_kind_network_policy_and_broadcast_channel_registry(
            r#"
            const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
            const dbName = "worker-wasm-module-storage-" + Math.random();
            const open = indexedDB.open(dbName, 1);
            open.onupgradeneeded = () => {
                open.result.createObjectStore("store");
            };
            open.onerror = () => {
                postMessage({ stage: "open", name: open.error && open.error.name });
                close();
            };
            open.onsuccess = () => {
                const tx = open.result.transaction("store", "readwrite");
                const store = tx.objectStore("store");
                let thrown = null;
                try {
                    const request = store.put(new WebAssembly.Module(bytes), "module");
                    request.onerror = () => {
                        postMessage({ stage: "put", name: request.error && request.error.name });
                        close();
                    };
                    request.onsuccess = () => {
                        postMessage({ stage: "put", name: "unexpected-success" });
                        close();
                    };
                } catch (error) {
                    thrown = error.name;
                    postMessage({ stage: "put", name: thrown });
                    close();
                }
            };
            "#
            .into(),
            "https://worker.example/indexeddb.js".into(),
            worker_test_request_client(),
            WorkerScriptKind::Classic,
            crate::worker::handle::WorkerNetworkPolicy::default(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            None,
            Some(crate::downgrade_indexed_db_manager(&manager)),
        );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"stage":"put","name":"DataCloneError"}"#
    );
}

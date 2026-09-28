// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn worker_request_constructor_resolves_relative_url_against_worker_script_url() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const request = new Request("./data.txt");
        postMessage(request.url);
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""http://127.0.0.1/worker/data.txt""#
    );
}

#[tokio::test]
async fn blob_worker_request_constructor_rejects_relative_url() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let constructed = false;
        try {
            new Request("./data.txt");
            constructed = true;
        } catch (error) {
            postMessage({ constructed, name: error.name });
            close();
        }
        "#
        .into(),
        "blob:http://127.0.0.1/worker-script".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"constructed":false,"name":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_url_and_search_params_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const url = new URL("./data.txt?x=1&x=2", "http://127.0.0.1/worker/main.js");
        const params = new URLSearchParams("a=1&a=2&b=3");
        postMessage({
            urlCtor: typeof URL,
            searchParamsCtor: typeof URLSearchParams,
            href: url.href,
            origin: url.origin,
            pathname: url.pathname,
            search: url.search,
            xValues: url.searchParams.getAll("x"),
            paramsString: params.toString(),
            canParseRelative: URL.canParse("./next.js", "http://127.0.0.1/worker/main.js"),
            canParseInvalidBase: URL.canParse("./next.js", "not a url"),
            urlTag: Object.prototype.toString.call(url),
            paramsTag: Object.prototype.toString.call(params),
        });
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"urlCtor":"function","searchParamsCtor":"function","href":"http://127.0.0.1/worker/data.txt?x=1&x=2","origin":"http://127.0.0.1","pathname":"/worker/data.txt","search":"?x=1&x=2","xValues":["1","2"],"paramsString":"a=1&a=2&b=3","canParseRelative":true,"canParseInvalidBase":false,"urlTag":"[object URL]","paramsTag":"[object URLSearchParams]"}"#
    );
}

#[tokio::test]
async fn worker_navigator_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        "use strict";
        let assignError = null;
        try {
            navigator.appName = "";
        } catch (error) {
            assignError = error.name;
        }
        const proto = Object.getPrototypeOf(navigator);
        const userAgent = Object.getOwnPropertyDescriptor(proto, "userAgent");
        const storage = navigator.storage;
        const serviceWorker = navigator.serviceWorker;
        const illegalReceiverProbe = StorageManager.prototype.estimate.call({}).then(
            () => "resolved",
            (error) => error && error.name
        );
        const spoofedReceiverProbe = StorageManager.prototype.estimate.call({
            __moliStorageManagerBrand: true
        }).then(
            () => "resolved",
            (error) => error && error.name
        );
        Promise.all([
            storage.persisted(),
            storage.estimate(),
            illegalReceiverProbe,
            spoofedReceiverProbe
        ]).then(([persisted, estimate, illegalReceiverError, spoofedReceiverError]) => {
        postMessage({
            navigatorOwn: Object.prototype.hasOwnProperty.call(self, "navigator"),
            ctorOwn: Object.prototype.hasOwnProperty.call(self, "WorkerNavigator"),
            navigatorType: typeof navigator,
            ctorType: typeof WorkerNavigator,
            ctorName: navigator.constructor && navigator.constructor.name,
            protoCtor: proto && proto.constructor && proto.constructor.name,
            tag: Object.prototype.toString.call(navigator),
            instanceofWorkerNavigator: navigator instanceof WorkerNavigator,
            ownUserAgent: Object.prototype.hasOwnProperty.call(navigator, "userAgent"),
            userAgentGetterType: typeof userAgent?.get,
            appCodeName: navigator.appCodeName,
            appName: navigator.appName,
            appVersionStartsWithWebKit: navigator.appVersion.indexOf("WebKit") === 0,
            platformType: typeof navigator.platform,
            userAgentStartsWithWebKit: navigator.userAgent.indexOf("WebKit") === 0,
            product: navigator.product,
            language: navigator.language,
            languages: Array.from(navigator.languages || []),
            onLineType: typeof navigator.onLine,
            hardwareConcurrencyPositive: navigator.hardwareConcurrency > 0,
            deviceMemoryPositive: typeof navigator.deviceMemory === "number" && navigator.deviceMemory > 0,
            assignError,
            serviceWorkerType: typeof serviceWorker,
            serviceWorkerControllerIsNull: serviceWorker.controller === null,
            serviceWorkerRegisterType: typeof serviceWorker.register,
            serviceWorkerGetRegistrationType: typeof serviceWorker.getRegistration,
            serviceWorkerAddEventListenerType: typeof serviceWorker.addEventListener,
            serviceWorkerOncontrollerchangeIsNull: serviceWorker.oncontrollerchange === null,
            storageCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageManager"),
            storageEstimateCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageEstimate"),
            storageType: typeof storage,
            storageTag: Object.prototype.toString.call(storage),
            storageInstanceof: storage instanceof StorageManager,
            storageOwnEstimate: Object.prototype.hasOwnProperty.call(storage, "estimate"),
            storageOwnPersisted: Object.prototype.hasOwnProperty.call(storage, "persisted"),
            storageOwnPersist: Object.prototype.hasOwnProperty.call(storage, "persist"),
            storageOwnGetDirectory: Object.prototype.hasOwnProperty.call(storage, "getDirectory"),
            storageKeys: Object.keys(storage),
            storageProtoPersisted: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "persisted"),
            storageProtoEstimate: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "estimate"),
            storageProtoPersist: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "persist"),
            storageProtoGetDirectory: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "getDirectory"),
            storagePersistedName: storage.persisted && storage.persisted.name,
            storagePersistedLength: storage.persisted && storage.persisted.length,
            storageEstimateName: storage.estimate && storage.estimate.name,
            storageEstimateLength: storage.estimate && storage.estimate.length,
            storageGetDirectoryName: storage.getDirectory && storage.getDirectory.name,
            storageGetDirectoryLength: storage.getDirectory && storage.getDirectory.length,
            storagePersistType: typeof storage.persist,
            persisted,
            estimateTag: Object.prototype.toString.call(estimate),
            estimateKeys: Object.keys(estimate),
            estimateQuota: estimate.quota,
            estimateUsage: estimate.usage,
            estimateUsageDetailsTag: Object.prototype.toString.call(estimate.usageDetails),
            estimateUsageDetailsKeys: Object.keys(estimate.usageDetails),
            illegalReceiverError,
            spoofedReceiverError,
        });
        close();
        });
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let actual = expect_post_json(msg);
    assert!(actual.contains(r#""languages":["en-US","en"]"#));
    assert_eq!(
        actual.replace(r#""languages":["en-US","en"]"#, r#""languages":["en-US"]"#,),
        r#"{"navigatorOwn":true,"ctorOwn":true,"navigatorType":"object","ctorType":"function","ctorName":"WorkerNavigator","protoCtor":"WorkerNavigator","tag":"[object WorkerNavigator]","instanceofWorkerNavigator":true,"ownUserAgent":false,"userAgentGetterType":"function","appCodeName":"Mozilla","appName":"Netscape","appVersionStartsWithWebKit":false,"platformType":"string","userAgentStartsWithWebKit":false,"product":"Gecko","language":"en-US","languages":["en-US"],"onLineType":"boolean","hardwareConcurrencyPositive":true,"deviceMemoryPositive":true,"assignError":"TypeError","serviceWorkerType":"object","serviceWorkerControllerIsNull":true,"serviceWorkerRegisterType":"function","serviceWorkerGetRegistrationType":"function","serviceWorkerAddEventListenerType":"function","serviceWorkerOncontrollerchangeIsNull":true,"storageCtorOwn":true,"storageEstimateCtorOwn":true,"storageType":"object","storageTag":"[object StorageManager]","storageInstanceof":true,"storageOwnEstimate":false,"storageOwnPersisted":false,"storageOwnPersist":false,"storageOwnGetDirectory":false,"storageKeys":[],"storageProtoPersisted":true,"storageProtoEstimate":true,"storageProtoPersist":false,"storageProtoGetDirectory":true,"storagePersistedName":"persisted","storagePersistedLength":0,"storageEstimateName":"estimate","storageEstimateLength":0,"storageGetDirectoryName":"getDirectory","storageGetDirectoryLength":0,"storagePersistType":"undefined","persisted":false,"estimateTag":"[object StorageEstimate]","estimateKeys":["quota","usage","usageDetails"],"estimateQuota":1073741824,"estimateUsage":0,"estimateUsageDetailsTag":"[object Object]","estimateUsageDetailsKeys":[],"illegalReceiverError":"TypeError","spoofedReceiverError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_css_transforms_use_native_objects_numeric_conversion_and_matrices() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            const unit = (value, name) => new CSSUnitValue(value, name);
            const translation = new CSSTranslate(unit(2.54, 'cm'), unit(10, 'px'));
            const components = [
                translation, new CSSRotate(unit(90, 'deg')), new CSSScale(2, 3),
                new CSSSkew(unit(0, 'deg'), unit(0, 'deg')),
                new CSSSkewX(unit(0, 'deg')), new CSSSkewY(unit(0, 'deg')),
                new CSSPerspective(new CSSKeywordValue('none')),
                new CSSMatrixComponent(new DOMMatrixReadOnly()),
            ];
            const value = new CSSTransformValue(components);
            let branded = false;
            try { CSSTransformComponent.prototype.toMatrix.call(new Proxy(translation, {})); }
            catch (e) { branded = e instanceof TypeError; }
            postMessage(value instanceof CSSStyleValue && value.length === 8 &&
                [...value].every((component, i) => component === components[i] && component instanceof CSSTransformComponent) &&
                translation.toMatrix() instanceof DOMMatrix && translation.toMatrix().e === 96 &&
                value.toMatrix() instanceof DOMMatrix && !value.toMatrix().is2D &&
                typeof CSSStyleValue.parse === 'undefined' && branded);
        } catch (e) { postMessage(String(e)); }
        close();
        "#.into(),
        "https://worker-transforms.test/main.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), "true");
}

use super::*;

#[tokio::test]
async fn service_worker_register_csp_uses_worker_response_policy() {
    const PROBE: &str = r#"
        let outcome, violation;
        const complete = () => {
            if (outcome && violation) postMessage(outcome + '|' + violation);
        };
        addEventListener('securitypolicyviolation', e => {
            violation = [e.effectiveDirective, e.disposition, e.isTrusted, e.target === self].join(':');
            complete();
        });
        navigator.serviceWorker.register('sw.js').then(
            () => { outcome = 'resolved'; complete(); },
            e => { outcome = e.name + ':' + (e instanceof DOMException); complete(); }
        );
    "#;
    let (base, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/blocked-worker.js",
            "text/javascript",
            vec![("Content-Security-Policy", "worker-src 'none'")],
            PROBE,
        ),
        (
            "/app/report-worker.js",
            "text/javascript",
            vec![("Content-Security-Policy-Report-Only", "worker-src 'none'")],
            PROBE,
        ),
        (
            "/app/sw.js",
            "text/javascript",
            Vec::new(),
            "self.oninstall = e => e.waitUntil(skipWaiting());",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/app/page.html"),
        &loader,
    );
    vm.set_response_content_security_policies(&["worker-src 'self'".to_owned()]);
    vm.eval(r#"
        globalThis.__workerCspDone = false;
        (async () => {
            const results = [];
            for (const script of ['blocked-worker.js', 'report-worker.js']) {
                const worker = new Worker(script);
                results.push(await new Promise(resolve => { worker.onmessage = e => resolve(e.data); }));
                worker.terminate();
            }
            globalThis.__workerCspResult = results.join(',');
        })().catch(e => { globalThis.__workerCspResult = 'error:' + e; })
            .finally(() => { globalThis.__workerCspDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__workerCspDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("__workerCspResult").unwrap(),
        "SecurityError:true|worker-src:enforce:true:true,resolved|worker-src:report:true:true"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_register_csp_blocks_before_fetch_or_registration() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (policy, kind) in [
        ("worker-src 'none'", "classic"),
        ("worker-src 'none'", "module"),
        ("child-src 'none'; script-src 'self'", "classic"),
        ("script-src 'none'", "classic"),
        ("default-src 'none'", "module"),
    ] {
        // Keep a listener open so a mistaken fetch remains observable, rather
        // than turning into a network error that resembles a CSP rejection.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (mut vm, runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                &format!("{base}/app/page.html"),
                &loader,
            );
        vm.set_response_content_security_policies(&[policy.to_owned()]);
        vm.eval(&format!(
            "globalThis.__kind = {}",
            serde_json::to_string(kind).unwrap()
        ))
        .unwrap();
        vm.eval(r#"
            globalThis.__cspEvents = [];
            document.addEventListener('securitypolicyviolation', e => {
                __cspEvents.push([e.effectiveDirective, e.disposition,
                    e.isTrusted, e.target === document, e.blockedURI.endsWith('/app/sw.js')].join(':'));
            });
            globalThis.__cspRegisterDone = false;
            navigator.serviceWorker.register('sw.js', {type: __kind}).then(
                () => { globalThis.__cspResult = 'resolved'; },
                e => { globalThis.__cspResult = [e.name, e instanceof DOMException, __cspEvents.length].join(':'); }
            ).then(async () => {
                globalThis.__cspRegistrations = (await navigator.serviceWorker.getRegistrations()).length;
                globalThis.__cspRegisterDone = true;
            });
        "#).unwrap();
        drain_service_worker_test_until_eval_equals(
            &mut vm,
            &runtime,
            &loader,
            "String(__cspRegisterDone && __cspEvents.length === 1)",
            "true",
        )
        .await;
        assert_eq!(
            vm.eval("__cspResult + '|' + __cspEvents + '|' + __cspRegistrations")
                .unwrap(),
            "SecurityError:true:0|worker-src:enforce:true:true:true|0",
            "{policy}, {kind}"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
                .await
                .is_err(),
            "blocked registration fetched its script: {policy}, {kind}"
        );
    }
}

#[tokio::test]
async fn service_worker_register_csp_rechecks_existing_registration_and_type_changes() {
    let (base, server) = spawn_service_worker_response_server(vec![(
        "/app/sw.js",
        "text/javascript",
        "self.addEventListener('install', e => e.waitUntil(skipWaiting()));",
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__warmDone = false;
        (async () => {
            const registration = await navigator.serviceWorker.register('sw.js');
            await __moliTestActivatedServiceWorker();
            const meta = document.createElement('meta');
            meta.httpEquiv = 'Content-Security-Policy'; meta.content = "worker-src 'none'";
            document.head.append(meta);
            const results = [];
            for (const [script, type] of [['sw.js', 'classic'], ['sw.js', 'module'], ['other.js', 'classic']]) {
                results.push(await navigator.serviceWorker.register(script, {type}).then(
                    () => 'resolved', e => e.name + ':' + (e instanceof DOMException)));
            }
            const retained = await navigator.serviceWorker.getRegistration();
            globalThis.__warmResult = results.join('|') + '|' + (retained === registration) + '|' + registration.active.state;
        })().catch(e => { globalThis.__warmResult = 'error:' + e; })
            .finally(() => { globalThis.__warmDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__warmDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("__warmResult").unwrap(),
        "SecurityError:true|SecurityError:true|SecurityError:true|true|activated"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_register_csp_uses_container_owner_and_queues_its_event() {
    let (base, server) = spawn_service_worker_response_server(vec![
        (
            "/app/child.html",
            "text/html",
            "<!doctype html><head></head><body>",
        ),
        (
            "/app/sw.js",
            "text/javascript",
            "self.addEventListener('install', e => e.waitUntil(skipWaiting()));",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/app/page.html"),
        &loader,
    );
    vm.eval(&format!(
        "globalThis.__scriptURL = {}",
        serde_json::to_string(&format!("{base}/app/sw.js")).unwrap()
    ))
    .unwrap();
    vm.eval(r#"
        globalThis.__ownerDone = false;
        (async () => {
            const frame = document.createElement('iframe');
            const loaded = new Promise(resolve => { frame.onload = resolve; });
            frame.src = '/app/child.html';
            document.body.append(frame); await loaded;
            const child = frame.contentWindow;
            globalThis.__ownerEvents = [];
            for (const [label, doc] of [['top', document], ['child', child.document]]) {
                doc.addEventListener('securitypolicyviolation', e => {
                    if (e.target === doc) __ownerEvents.push(label + ':' + e.isTrusted);
                });
            }
            const restrict = doc => {
                const meta = doc.createElement('meta');
                meta.httpEquiv = 'Content-Security-Policy'; meta.content = "worker-src 'none'";
                doc.head.append(meta);
            };
            const register = navigator.serviceWorker.register;
            restrict(document);
            const initial = register.call(child.navigator.serviceWorker, __scriptURL);
            const realms = [initial instanceof child.Promise && !(initial instanceof Promise)];
            const registration = await initial;
            await __moliTestActivatedServiceWorker(child.navigator.serviceWorker);
            restrict(child.document);
            const childPromise = register.call(child.navigator.serviceWorker, __scriptURL);
            realms.push(childPromise instanceof child.Promise && !(childPromise instanceof Promise));
            const childError = await childPromise
                .then(() => 'resolved', e => e.name + ':' + (e instanceof child.DOMException));
            const topPromise = child.navigator.serviceWorker.register.call(navigator.serviceWorker, __scriptURL);
            realms.push(topPromise instanceof Promise && !(topPromise instanceof child.Promise));
            const topError = await topPromise
                .then(() => 'resolved', e => e.name + ':' + (e instanceof DOMException));
            globalThis.__ownerResult = [registration.scope.endsWith('/app/'), childError, topError, realms.every(Boolean)].join('|');
        })().catch(e => { globalThis.__ownerResult = 'error:' + e; })
            .finally(() => { globalThis.__ownerDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__ownerDone && __ownerEvents.length === 2)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("__ownerResult + '|' + __ownerEvents.sort()")
            .unwrap(),
        "true|SecurityError:true|SecurityError:true|true|child:true,top:true"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_register_csp_observes_policy_added_during_author_conversion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for trusted_types in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (mut vm, runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                &format!("{base}/app/page.html"),
                &loader,
            );
        if trusted_types {
            vm.set_response_content_security_policies(&[
                "require-trusted-types-for 'script'".to_owned()
            ]);
        }
        vm.eval(&format!("globalThis.__useTT = {trusted_types}"))
            .unwrap();
        vm.eval(r#"
            const restrict = () => {
                const meta = document.createElement('meta');
                meta.httpEquiv = 'Content-Security-Policy'; meta.content = "worker-src 'none'";
                document.head.append(meta);
            };
            if (__useTT) trustedTypes.createPolicy('default', {createScriptURL: value => { restrict(); return value; }});
            const options = __useTT ? {} : {get scope() { restrict(); return './'; }};
            globalThis.__conversionDone = false;
            navigator.serviceWorker.register('sw.js', options).then(
                () => { globalThis.__conversionResult = 'resolved'; },
                e => { globalThis.__conversionResult = e.name + ':' + (e instanceof DOMException); }
            ).finally(() => { globalThis.__conversionDone = true; });
        "#).unwrap();
        drain_service_worker_test_until_eval_equals(
            &mut vm,
            &runtime,
            &loader,
            "String(__conversionDone)",
            "true",
        )
        .await;
        assert_eq!(
            vm.eval("__conversionResult").unwrap(),
            "SecurityError:true",
            "TT={trusted_types}"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn service_worker_register_csp_reports_each_policy_without_inheriting_it_into_imports() {
    let (base, server) = spawn_service_worker_response_server(vec![
        (
            "/app/sw.js",
            "text/javascript",
            "importScripts('dependency.js'); self.oninstall = e => e.waitUntil(skipWaiting());",
        ),
        (
            "/app/dependency.js",
            "text/javascript",
            "self.imported = true;",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/app/page.html"),
        &loader,
    );
    vm.set_response_content_security_policies(&[
        "worker-src 'self'; child-src 'none'; script-src 'none'; default-src 'none'".to_owned(),
    ]);
    vm.set_response_content_security_report_only_policies(&[
        "worker-src 'none'".to_owned(),
        "child-src 'none'".to_owned(),
    ]);
    vm.eval(r#"
        globalThis.__reports = [];
        document.addEventListener('securitypolicyviolation', e => {
            __reports.push([e.effectiveDirective, e.disposition, e.isTrusted, e.target === document].join(':'));
        });
        globalThis.__reportDone = false;
        (async () => {
            await navigator.serviceWorker.register('sw.js');
            const registration = await __moliTestActivatedServiceWorker();
            globalThis.__reportResult = registration.active.state;
        })().catch(e => { globalThis.__reportResult = 'error:' + e; })
            .finally(() => { globalThis.__reportDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__reportDone && __reports.length === 2)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("__reportResult + '|' + __reports").unwrap(),
        "activated|worker-src:report:true:true,worker-src:report:true:true"
    );
    server.await.unwrap();
}

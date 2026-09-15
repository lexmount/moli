use super::*;

#[tokio::test]
async fn navigator_service_worker_shim_controls_window_fetch() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const path = new URL(event.request.url).pathname;
          event.respondWith(new Response("sw:" + path, {
            status: 202,
            statusText: "Handled by worker",
            headers: {"content-type": "text/plain;charset=UTF-8", "x-worker": "yes"}
          }));
        });
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
            (() => {
              const sw = navigator.serviceWorker;
              globalThis.__serviceWorkerProbe = {
                containerType: typeof sw,
                controllerIsNull: sw.controller === null,
                readyType: typeof sw.ready,
                serviceWorkerConstructorType: typeof ServiceWorker,
                registrationConstructorType: typeof ServiceWorkerRegistration,
                registerType: typeof sw.register,
                registerName: sw.register && sw.register.name,
                registerLength: sw.register && sw.register.length,
                getRegistrationType: typeof sw.getRegistration,
                getRegistrationName: sw.getRegistration && sw.getRegistration.name,
                getRegistrationLength: sw.getRegistration && sw.getRegistration.length,
                getRegistrationsType: typeof sw.getRegistrations,
                getRegistrationsName: sw.getRegistrations && sw.getRegistrations.name,
                getRegistrationsLength: sw.getRegistrations && sw.getRegistrations.length,
                removeEventListenerType: typeof sw.removeEventListener,
                controllerChangeHandlerType: typeof sw.oncontrollerchange,
                controllerChangeLog: []
              };
              sw.addEventListener("controllerchange", (event) => {
                globalThis.__serviceWorkerProbe.controllerChangeLog.push(
                  "listener:" + event.type + ":" + (sw.controller !== null)
                );
              });
              sw.oncontrollerchange = (event) => {
                globalThis.__serviceWorkerProbe.controllerChangeLog.push(
                  "handler:" + event.type + ":" + (sw.controller !== null)
                );
              };
              sw.getRegistration("scope").then((registration) => {
                globalThis.__serviceWorkerProbe.registration = registration === undefined;
              });
              sw.getRegistrations().then((registrations) => {
                globalThis.__serviceWorkerProbe.registrations = [
                  Array.isArray(registrations),
                  registrations.length
                ].join("|");
              });
              sw.register("worker.js#script-fragment", { scope: "./#scope-fragment" })
                .then(async (registration) => {
                  const readyRegistration = await sw.ready;
                  const response = await fetch("api/data.txt");
                  const dataResponse = await fetch("data:text/plain,data-url");
                  let outOfScopeResult;
                  try {
                    const outOfScopeResponse = await fetch("/outside/data.txt");
                    outOfScopeResult =
                      "resolved:" + outOfScopeResponse.statusText + ":" + await outOfScopeResponse.text();
                  } catch (error) {
                    outOfScopeResult = "rejected";
                  }
                  globalThis.__serviceWorkerProbe.afterRegisterController =
                    sw.controller !== null;
                  globalThis.__serviceWorkerProbe.controllerStable =
                    sw.controller === sw.controller;
                  globalThis.__serviceWorkerProbe.controllerMatchesActive =
                    sw.controller === registration.active;
                  globalThis.__serviceWorkerProbe.controllerBrand =
                    sw.controller instanceof ServiceWorker;
                  globalThis.__serviceWorkerProbe.registrationBrand =
                    registration instanceof ServiceWorkerRegistration;
                  globalThis.__serviceWorkerProbe.registrationTag =
                    Object.prototype.toString.call(registration);
                  globalThis.__serviceWorkerProbe.installingBrand =
                    registration.installing instanceof ServiceWorker;
                  globalThis.__serviceWorkerProbe.installingScriptURL =
                    registration.installing && registration.installing.scriptURL;
                  globalThis.__serviceWorkerProbe.installingState =
                    registration.installing && registration.installing.state;
                  globalThis.__serviceWorkerProbe.waitingIsNull =
                    registration.waiting === null;
                  globalThis.__serviceWorkerProbe.activeIsNull =
                    registration.active === null;
                  globalThis.__serviceWorkerProbe.activeBrand =
                    registration.active instanceof ServiceWorker;
                  globalThis.__serviceWorkerProbe.activeScriptURL =
                    registration.active && registration.active.scriptURL;
                  globalThis.__serviceWorkerProbe.activeState =
                    registration.active && registration.active.state;
                  globalThis.__serviceWorkerProbe.registrationScope =
                    registration && registration.scope;
                  globalThis.__serviceWorkerProbe.readyMatchesRegister =
                    readyRegistration === registration;
                  globalThis.__serviceWorkerProbe.responseStatusText =
                    response.statusText;
                  globalThis.__serviceWorkerProbe.responseText =
                    await response.text();
                  globalThis.__serviceWorkerProbe.dataResponseStatusText =
                    dataResponse.statusText;
                  globalThis.__serviceWorkerProbe.dataResponseContentType =
                    dataResponse.headers.get("content-type");
                  globalThis.__serviceWorkerProbe.dataResponseText =
                    await dataResponse.text();
                  globalThis.__serviceWorkerProbe.outOfScopeResult =
                    outOfScopeResult;
                  const controllerBeforeUnregister = sw.controller;
                  globalThis.__serviceWorkerProbe.firstUnregister =
                    await registration.unregister();
                  globalThis.__serviceWorkerProbe.secondUnregister =
                    await registration.unregister();
                  globalThis.__serviceWorkerProbe.controllerRetained =
                    sw.controller === controllerBeforeUnregister;
                  globalThis.__serviceWorkerProbe.controllerStateAfterUnregister =
                    controllerBeforeUnregister && controllerBeforeUnregister.state;
                  globalThis.__serviceWorkerProbe.registrationRemoved =
                    await sw.getRegistration() === undefined;
                  const afterUnregisterResponse = await fetch("after-unregister.txt");
                  globalThis.__serviceWorkerProbe.afterUnregisterText =
                    await afterUnregisterResponse.text();
                }, (error) => {
                  globalThis.__serviceWorkerProbe.registerRejection =
                    error && error.message;
                });
            })()
            "#,
    )
    .expect("service worker shim probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(Object.prototype.hasOwnProperty.call(globalThis.__serviceWorkerProbe, 'afterUnregisterText') && globalThis.__serviceWorkerProbe.controllerChangeLog.length === 2)",
        "true",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__serviceWorkerProbe)")
        .expect("service worker shim promises should settle");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected_scope = format!("{base_url}/app/");
    let expected_fetch_url = format!("{base_url}/app/api/data.txt");
    let expected_out_of_scope_url = format!("{base_url}/outside/data.txt");
    let expected_result = format!(
        r#"{{"containerType":"object","controllerIsNull":true,"readyType":"object","serviceWorkerConstructorType":"function","registrationConstructorType":"function","registerType":"function","registerName":"register","registerLength":1,"getRegistrationType":"function","getRegistrationName":"getRegistration","getRegistrationLength":0,"getRegistrationsType":"function","getRegistrationsName":"getRegistrations","getRegistrationsLength":0,"removeEventListenerType":"function","controllerChangeHandlerType":"object","controllerChangeLog":["listener:controllerchange:true","handler:controllerchange:true"],"registration":true,"registrations":"true|0","afterRegisterController":true,"controllerStable":true,"controllerMatchesActive":true,"controllerBrand":true,"registrationBrand":true,"registrationTag":"[object ServiceWorkerRegistration]","installingBrand":false,"installingScriptURL":null,"installingState":null,"waitingIsNull":true,"activeIsNull":false,"activeBrand":true,"activeScriptURL":"{expected_worker_url}","activeState":"activated","registrationScope":"{expected_scope}","readyMatchesRegister":true,"responseStatusText":"Handled by worker","responseText":"sw:/app/api/data.txt","dataResponseStatusText":"OK","dataResponseContentType":"text/plain","dataResponseText":"data-url","outOfScopeResult":"resolved:Handled by worker:sw:/outside/data.txt","firstUnregister":true,"secondUnregister":false,"controllerRetained":true,"controllerStateAfterUnregister":"activated","registrationRemoved":true,"afterUnregisterText":"sw:/app/after-unregister.txt"}}"#
    );
    assert_eq!(result, expected_result);

    let records = vm
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => Some(record),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(records.iter().any(|record| {
        matches!(
            record.outcome(),
            crate::types::SubresourceNetworkOutcome::Success {
                status: 202,
                status_text,
                final_url,
                ..
            } if status_text.as_deref() == Some("Handled by worker")
                && final_url.as_str() == expected_fetch_url
        )
    }));
    assert!(records.iter().any(|record| {
        matches!(
            record.outcome(),
            crate::types::SubresourceNetworkOutcome::Success {
                status: 200,
                status_text,
                final_url,
                ..
            } if status_text.as_deref().is_none()
                && final_url.as_str() == "data:text/plain,data-url"
        )
    }));
    assert!(records.iter().any(|record| {
        matches!(
            record.outcome(),
            crate::types::SubresourceNetworkOutcome::Success {
                status: 202,
                status_text,
                final_url,
                ..
            } if status_text.as_deref() == Some("Handled by worker")
                && final_url.as_str() == expected_out_of_scope_url
        )
    }));
    server
        .await
        .expect("service worker script server should finish");
}
#[tokio::test]
async fn service_worker_bypass_skips_fetch_dispatch_without_dropping_controller() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const path = new URL(event.request.url).pathname;
              event.respondWith(new Response("worker:" + path));
            });
            "#,
        ),
        (
            "/app/data.txt",
            "text/plain; charset=utf-8",
            "network:/app/data.txt",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("resource request client");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        globalThis.__serviceWorkerBypassProbe = {};
        navigator.serviceWorker.register("worker.js")
          .then(async () => {
            await navigator.serviceWorker.ready;
            const response = await fetch("data.txt");
            globalThis.__serviceWorkerBypassProbe.before = await response.text();
            globalThis.__serviceWorkerBypassProbe.controllerBefore =
              navigator.serviceWorker.controller !== null;
          });
        "#,
    )
    .expect("service worker bypass setup should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(Object.prototype.hasOwnProperty.call(globalThis.__serviceWorkerBypassProbe, 'before'))",
        "true",
    )
    .await;

    vm.set_bypass_service_worker(true);
    vm.eval(
        r#"
        fetch("data.txt").then(async response => {
          globalThis.__serviceWorkerBypassProbe.after = await response.text();
          globalThis.__serviceWorkerBypassProbe.controllerAfter =
            navigator.serviceWorker.controller !== null;
        });
        "#,
    )
    .expect("bypassed service worker fetch should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(Object.prototype.hasOwnProperty.call(globalThis.__serviceWorkerBypassProbe, 'after'))",
        "true",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__serviceWorkerBypassProbe)")
            .expect("service worker bypass result should evaluate"),
        r#"{"before":"worker:/app/data.txt","controllerBefore":true,"after":"network:/app/data.txt","controllerAfter":true}"#
    );
    server
        .await
        .expect("service worker bypass server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_event_preload_response_resolves_undefined_without_preload()
{
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          event.respondWith((async () => {
            const promise = event.preloadResponse;
            const value = await promise;
            return new Response(JSON.stringify({
              hasPromise: promise instanceof Promise,
              samePromise: promise === event.preloadResponse,
              valueType: typeof value,
              isUndefined: value === undefined
            }), {
              status: 203,
              headers: {"content-type": "application/json"}
            });
          })());
        });
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
        (() => {
          globalThis.__serviceWorkerPreloadResponseProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            const response = await fetch("api/preload-response.txt");
            globalThis.__serviceWorkerPreloadResponseProbe = [
              response.status,
              response.headers.get("content-type"),
              await response.text()
            ].join("|");
          })().catch(error => {
            globalThis.__serviceWorkerPreloadResponseProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker preloadResponse probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPreloadResponseProbe)",
        r#"203|application/json|{"hasPromise":true,"samePromise":true,"valueType":"undefined","isUndefined":true}"#,
    )
    .await;

    server
        .await
        .expect("service worker preloadResponse script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_event_request_has_empty_destination_for_window_fetch() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          event.respondWith(new Response([
            "destination=" + event.request.destination,
            "mode=" + event.request.mode,
            "credentials=" + event.request.credentials,
            "redirect=" + event.request.redirect,
            "client=" + (event.clientId.length > 0),
            "resulting=" + event.resultingClientId
          ].join("|")));
        });
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
            (() => {
              globalThis.__serviceWorkerFetchRequestDestinationProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/destination.txt");
                globalThis.__serviceWorkerFetchRequestDestinationProbe =
                  response.status + "|" + await response.text();
              })().catch((error) => {
                globalThis.__serviceWorkerFetchRequestDestinationProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker request destination probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerFetchRequestDestinationProbe)",
        "200|destination=|mode=cors|credentials=same-origin|redirect=follow|client=true|resulting=",
    )
    .await;

    server
        .await
        .expect("service worker destination server should finish");
}
#[tokio::test]
async fn csp_report_fetch_pause_continue_preserves_service_worker_dispatch() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const url = new URL(event.request.url);
          if (url.pathname === "/app/csp-report") {
            event.respondWith(new Response([
              "destination=" + event.request.destination,
              "mode=" + event.request.mode,
              "credentials=" + event.request.credentials,
              "method=" + event.request.method,
              "from=service-worker"
            ].join("|")));
          }
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );
    let report_url = format!("{base_url}/app/csp-report");
    vm.set_response_content_security_policies(&[format!(
        "connect-src 'none'; report-uri {report_url}"
    )]);

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerPausedCspReportProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                globalThis.__serviceWorkerPausedCspReportProbe = "ready";
              })().catch((error) => {
                globalThis.__serviceWorkerPausedCspReportProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker paused CSP report setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPausedCspReportProbe)",
        "ready",
    )
    .await;

    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    vm.eval(
        r#"
            (async () => {
              await fetch("blocked-data").catch(() => {});
              globalThis.__serviceWorkerPausedCspReportProbe = "blocked";
            })().catch((error) => {
              globalThis.__serviceWorkerPausedCspReportProbe =
                "error:" + String(error && error.message);
            })
            "#,
    )
    .expect("paused CSP report trigger should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPausedCspReportProbe)",
        "blocked",
    )
    .await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let paused_report = loop {
        let mut infos = vm.take_pending_subresource_fetch_infos();
        if let Some(index) = infos.iter().position(|info| {
            info.resource_type == crate::types::SubresourceResourceType::CspReport
                && info.url.as_str() == report_url
        }) {
            break infos.remove(index);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for paused CSP report"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    };
    assert_eq!(paused_report.method, "POST");
    assert_eq!(
        paused_report.request_body,
        paused_report
            .request_body_bytes
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned())
    );

    let outcome = vm
        .continue_pending_subresource_fetch(
            paused_report.internal_id,
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .expect("paused CSP report continue should start");
    assert_eq!(
        outcome,
        crate::types::PendingSubresourceContinueOutcome::Started
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        items.extend(vm.take_network_output().into_items());
        if service_worker_csp_report_seen(
            &items,
            &report_url,
            "destination=report|mode=no-cors|credentials=same-origin|method=POST|from=service-worker",
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "continued paused CSP report did not settle through Service Worker: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    server
        .await
        .expect("service worker paused CSP report server should finish");
}
#[tokio::test]
async fn worker_csp_report_fetch_pause_continue_preserves_service_worker_dispatch() {
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            Vec::new(),
            r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const url = new URL(event.request.url);
          if (url.pathname === "/app/client-worker.js") {
            return;
          }
          if (url.pathname === "/app/csp-report") {
            event.respondWith(new Response([
              "destination=" + event.request.destination,
              "mode=" + event.request.mode,
              "credentials=" + event.request.credentials,
              "method=" + event.request.method,
              "from=service-worker",
              "client=" + (event.clientId.length > 0)
            ].join("|")));
          }
        });
        "#,
        ),
        (
            "/app/client-worker.js",
            "text/javascript; charset=utf-8",
            vec![(
                "Content-Security-Policy",
                "connect-src 'none'; report-uri /app/csp-report",
            )],
            r#"
        let violationSeen = false;
        self.addEventListener("securitypolicyviolation", event => {
          violationSeen = event.type === "securitypolicyviolation" &&
            event.effectiveDirective === "connect-src" &&
            event.violatedDirective === "connect-src" &&
            event.blockedURI.endsWith("/app/blocked-data") &&
            event.disposition === "enforce" &&
            event instanceof SecurityPolicyViolationEvent;
        });
        self.onmessage = async () => {
          await fetch("blocked-data").catch(() => {});
          postMessage("blocked:" + violationSeen);
        };
        "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__pausedWorkerCspReportProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                globalThis.__pausedWorkerCspReportProbe = "ready";
              })().catch((error) => {
                globalThis.__pausedWorkerCspReportProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("paused worker CSP report setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__pausedWorkerCspReportProbe)",
        "ready",
    )
    .await;

    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    vm.eval(
        r#"
            (() => {
              const worker = new Worker("client-worker.js");
              worker.onmessage = event => {
                globalThis.__pausedWorkerCspReportProbe = String(event.data);
              };
              worker.onerror = event => {
                globalThis.__pausedWorkerCspReportProbe = "error:" + event.message;
              };
              worker.postMessage("start");
            })()
            "#,
    )
    .expect("paused worker CSP report trigger should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__pausedWorkerCspReportProbe)",
        "blocked:true",
    )
    .await;

    let report_url = format!("{base_url}/app/csp-report");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let paused_report = loop {
        let mut infos = vm.take_pending_subresource_fetch_infos();
        if let Some(index) = infos.iter().position(|info| {
            info.resource_type == crate::types::SubresourceResourceType::CspReport
                && info.url.as_str() == report_url
        }) {
            break infos.remove(index);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for paused worker CSP report"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    };
    assert_eq!(paused_report.method, "POST");

    let outcome = vm
        .continue_pending_subresource_fetch(
            paused_report.internal_id,
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .expect("paused worker CSP report continue should start");
    assert_eq!(
        outcome,
        crate::types::PendingSubresourceContinueOutcome::Started
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        items.extend(vm.take_network_output().into_items());
        if service_worker_csp_report_seen(
            &items,
            &report_url,
            "destination=report|mode=no-cors|credentials=same-origin|method=POST|from=service-worker|client=true",
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "continued paused worker CSP report did not settle through Service Worker: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    server
        .await
        .expect("paused worker CSP report destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_event_request_preserves_window_fetch_policy_metadata() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          event.respondWith(new Response([
            "cache=" + event.request.cache,
            "referrer=" + event.request.referrer,
            "referrerPolicy=" + event.request.referrerPolicy,
            "integrity=" + event.request.integrity,
            "keepalive=" + event.request.keepalive
          ].join("|")));
        });
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
            (() => {
              globalThis.__serviceWorkerFetchRequestPolicyMetadataProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/metadata.txt", {
                  cache: "reload",
                  referrer: "./referrer.html",
                  referrerPolicy: "origin",
                  integrity: "sha256-test",
                  keepalive: true,
                  priority: "low"
                });
                globalThis.__serviceWorkerFetchRequestPolicyMetadataProbe =
                  response.status + "|" + await response.text();
              })().catch((error) => {
                globalThis.__serviceWorkerFetchRequestPolicyMetadataProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker request policy metadata probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerFetchRequestPolicyMetadataProbe)",
        &format!(
            "200|cache=reload|referrer={base_url}/app/referrer.html|referrerPolicy=origin|integrity=sha256-test|keepalive=true"
        ),
    )
    .await;

    server
        .await
        .expect("service worker policy metadata server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_follows_and_filters_synthetic_redirect_response() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const path = new URL(event.request.url).pathname;
          if (path.endsWith("/api/redirect-start.txt")) {
            event.respondWith(Response.redirect("api/redirect-final.txt", 302));
            return;
          }
          if (path.endsWith("/api/redirect-final.txt")) {
            event.respondWith(new Response("synthetic-redirect-final"));
            return;
          }
          if (path.endsWith("/api/manual-start.txt")) {
            event.respondWith(Response.redirect("api/manual-final.txt", 302));
            return;
          }
          if (path.endsWith("/api/manual-final.txt")) {
            event.respondWith(new Response("manual-final-should-not-load"));
            return;
          }
          if (path.endsWith("/api/generated-relative-redirect.txt")) {
            event.respondWith(new Response("", {
              status: 302,
              headers: {location: "relative-final.txt"}
            }));
            return;
          }
          if (path.endsWith("/api/manual-opaqueredirect-follow.txt") ||
              path.endsWith("/api/manual-opaqueredirect-error.txt") ||
              path.endsWith("/api/manual-opaqueredirect-outer-manual.txt")) {
            const redirectUrl = new URL(event.request.url).searchParams.get("url");
            event.respondWith(fetch(redirectUrl, {redirect: "manual"}));
          }
        });
        "#,
    )])
    .await;
    let (redirect_url, redirect_server) = spawn_service_worker_redirect_response_server().await;
    let redirect_url_literal =
        serde_json::to_string(&redirect_url).expect("serialize service worker redirect URL");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(&format!(
        r#"
            (() => {{
              globalThis.__serviceWorkerSyntheticRedirectProbe = "pending";
              (async () => {{
                await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
                await navigator.serviceWorker.ready;
                const response = await fetch("api/redirect-start.txt");
                const manual = await fetch("api/manual-start.txt", {{ redirect: "manual" }});
                const relativeFollow = await fetch(
                  "api/generated-relative-redirect.txt"
                ).then(
                  () => "fulfilled",
                  error => "rejected:" + String(error && error.name)
                );
                const relativeManual = await fetch(
                  "api/generated-relative-redirect.txt",
                  {{ redirect: "manual" }}
                );
                const manualRedirectUrl = "?url=" + encodeURIComponent({redirect_url_literal});
                const manualFollow = await fetch(
                  "api/manual-opaqueredirect-follow.txt" + manualRedirectUrl
                ).then(
                  () => "fulfilled",
                  error => "rejected:" + String(error && error.name)
                );
                const manualError = await fetch(
                  "api/manual-opaqueredirect-error.txt" + manualRedirectUrl,
                  {{ redirect: "error" }}
                ).then(
                  () => "fulfilled",
                  error => "rejected:" + String(error && error.name)
                );
                const manualOuter = await fetch(
                  "api/manual-opaqueredirect-outer-manual.txt" + manualRedirectUrl,
                  {{ redirect: "manual" }}
                );
                globalThis.__serviceWorkerSyntheticRedirectProbe =
                  response.status + "|" + response.redirected + "|" + response.url + "|" + await response.text() +
                  ";manual=" + manual.status + "|" + manual.type + "|" + manual.redirected + "|" + manual.url + "|" + await manual.text() +
                  ";relative-follow=" + relativeFollow +
                  ";relative-manual=" + relativeManual.status + "|" + relativeManual.type + "|" + relativeManual.redirected + "|" + relativeManual.url + "|" + await relativeManual.text() +
                  ";opaqueredirect-follow=" + manualFollow +
                  ";opaqueredirect-error=" + manualError +
                  ";opaqueredirect-manual=" + manualOuter.status + "|" + manualOuter.type + "|" + manualOuter.redirected + "|" + manualOuter.url + "|" + await manualOuter.text();
              }})().catch((error) => {{
                globalThis.__serviceWorkerSyntheticRedirectProbe =
                  "error:" + String(error && error.message);
              }});
            }})()
            "#,
    ))
    .expect("service worker synthetic redirect probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyntheticRedirectProbe)",
        &format!(
            "200|true|{base_url}/app/api/redirect-final.txt|synthetic-redirect-final;\
             manual=0|opaqueredirect|false|{base_url}/app/api/manual-start.txt|;\
             relative-follow=rejected:TypeError;\
             relative-manual=0|opaqueredirect|false|{base_url}/app/api/generated-relative-redirect.txt|;\
             opaqueredirect-follow=rejected:TypeError;\
             opaqueredirect-error=rejected:TypeError;\
             opaqueredirect-manual=0|opaqueredirect|false|{redirect_url}|"
        ),
    )
    .await;

    server
        .await
        .expect("service worker synthetic redirect server should finish");
    redirect_server
        .await
        .expect("service worker redirect response server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_event_request_preserves_worker_fetch_policy_metadata() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const path = new URL(event.request.url).pathname;
          if (path.endsWith("/client-worker.js")) {
            return;
          }
          event.respondWith(new Response([
            "cache=" + event.request.cache,
            "referrer=" + event.request.referrer,
            "referrerPolicy=" + event.request.referrerPolicy,
            "integrity=" + event.request.integrity,
            "keepalive=" + event.request.keepalive
          ].join("|"), {headers: {"x-response": "from-worker"}}));
        });
        "#,
        ),
        (
            "/app/client-worker.js",
            "text/javascript; charset=utf-8",
            r#"
        self.onmessage = async () => {
          try {
            const target = new URL("api/worker-metadata.txt", location.href);
            target.hostname = target.hostname === "127.0.0.1" ? "localhost" : "127.0.0.1";
            const response = await fetch(target, {
              cache: "reload",
              referrer: "./worker-referrer.html",
              referrerPolicy: "origin",
              integrity: "sha256-test",
              keepalive: true,
              priority: "high"
            });
            postMessage([response.status, response.type, response.headers.get("x-response"), await response.text()].join("|"));
          } catch (error) {
            postMessage("error:" + String(error && error.message));
          }
        };
        "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerWorkerFetchRequestPolicyMetadataProbe =
                "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const worker = new Worker("client-worker.js");
                worker.onmessage = event => {
                  globalThis.__serviceWorkerWorkerFetchRequestPolicyMetadataProbe =
                    String(event.data);
                };
                worker.onerror = event => {
                  globalThis.__serviceWorkerWorkerFetchRequestPolicyMetadataProbe =
                    "error:" + event.message;
                };
                worker.postMessage("start");
              })().catch((error) => {
                globalThis.__serviceWorkerWorkerFetchRequestPolicyMetadataProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker worker request policy metadata probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerWorkerFetchRequestPolicyMetadataProbe)",
        &format!(
            "200|basic|from-worker|cache=reload|referrer={base_url}/app/worker-referrer.html|referrerPolicy=origin|integrity=sha256-test|keepalive=true"
        ),
    )
    .await;

    server
        .await
        .expect("service worker worker policy metadata server should finish");
}
#[tokio::test]
async fn navigator_service_worker_intercepts_preload_link_destinations() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        const requests = [];
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const path = new URL(event.request.url).pathname;
          if (path.endsWith("/preload-requests")) {
            event.respondWith(new Response(requests.sort().join("|")));
            return;
          }
          requests.push(path.split("/").pop() + ":" + event.request.destination);
          const expected = path.endsWith("/style.css") ? "style" :
            path.endsWith("/font.woff2") ? "font" :
            path.endsWith("/image.png") ? "image" :
            path.endsWith("/clip.ogg") ? "audio" :
            path.endsWith("/clip.mp4") ? "video" :
            "";
          if (event.request.destination !== expected) {
            event.respondWith(new Response("wrong:" + event.request.destination, {
              status: 500,
              headers: {"content-type": "text/plain"}
            }));
            return;
          }
          if (expected === "style") {
            event.respondWith(new Response("body { color: green; }", {
              headers: {"content-type": "text/css"}
            }));
            return;
          }
          event.respondWith(new Response(expected, {
            headers: {"content-type": "text/plain"}
          }));
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::ALL);
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerPreloadDestinationProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const seen = [];
                const ensureHead = () => {
                  if (document.head) {
                    return document.head;
                  }
                  const html = document.documentElement ||
                    document.appendChild(document.createElement("html"));
                  return html.appendChild(document.createElement("head"));
                };
                const parent = ensureHead();
                const preload = (as, href) => new Promise((resolve) => {
                  const link = document.createElement("link");
                  link.setAttribute("rel", "preload");
                  link.setAttribute("as", as);
                  link.setAttribute("href", href);
                  link.onload = () => {
                    seen.push(as + ":load");
                    resolve();
                  };
                  link.onerror = () => {
                    seen.push(as + ":error");
                    resolve();
                  };
                  parent.appendChild(link);
                });
                // These must produce neither a fetch event nor a terminal link event.
                preload("audio", "clip.ogg");
                preload("video", "clip.mp4");
                await Promise.all([
                  preload("style", "style.css"),
                  preload("font", "font.woff2"),
                  preload("image", "image.png")
                ]);
                const requests = await (await fetch("preload-requests")).text();
                globalThis.__serviceWorkerPreloadDestinationProbe =
                  seen.sort().join("|") + ";" + requests;
              })().catch((error) => {
                globalThis.__serviceWorkerPreloadDestinationProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker preload destination probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPreloadDestinationProbe)",
        "font:load|image:load|style:load;font.woff2:font|image.png:image|style.css:style",
    )
    .await;

    server
        .await
        .expect("service worker preload destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_opaque_fetch_preload_does_not_satisfy_window_xhr() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const url = new URL(event.request.url);
              if (!url.pathname.endsWith("/opaque-response")) {
                return;
              }
              if (event.request.destination !== "") {
                event.respondWith(new Response("wrong-destination:" + event.request.destination, {
                  status: 500,
                  headers: {"content-type": "text/plain"}
                }));
                return;
              }
              if (event.request.mode === "no-cors") {
                event.respondWith(fetch(url.searchParams.get("opaque"), {mode: "no-cors"}));
                return;
              }
              if (event.request.mode !== "cors") {
                event.respondWith(new Response("wrong-mode:" + event.request.mode, {
                  status: 500,
                  headers: {"content-type": "text/plain"}
                }));
                return;
              }
              event.respondWith(fetch(url.searchParams.get("opaque"), {mode: "no-cors"}));
            });
            "#,
    )])
    .await;
    let (cross_base_url, cross_server) = spawn_service_worker_response_server(vec![
        ("/opaque-data.txt", "image/png", "opaque secret"),
        ("/opaque-data.txt", "image/png", "opaque secret"),
    ])
    .await;
    let opaque_url = format!("{cross_base_url}/opaque-data.txt");
    let opaque_url_literal =
        serde_json::to_string(&opaque_url).expect("serialize opaque preload URL");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(&format!(
        r#"
            (() => {{
              globalThis.__serviceWorkerOpaquePreloadXhrProbe = "pending";
              (async () => {{
                await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
                await navigator.serviceWorker.ready;
                const target =
                  "opaque-response?from=preload&opaque=" + encodeURIComponent({opaque_url_literal});
                const preloadResult = await new Promise(resolve => {{
                  const link = document.createElement("link");
                  link.rel = "preload";
                  link.as = "fetch";
                  link.href = target;
                  link.onload = () => resolve("preload:load");
                  link.onerror = () => resolve("preload:error");
                  let parent = document.head;
                  if (!parent) {{
                    const html = document.documentElement ||
                      document.appendChild(document.createElement("html"));
                    parent = html.appendChild(document.createElement("head"));
                  }}
                  parent.appendChild(link);
                }});
                const xhrResult = await new Promise(resolve => {{
                  const xhr = new XMLHttpRequest();
                  xhr.withCredentials = true;
                  xhr.onload = () => resolve([
                    "xhr:load",
                    xhr.readyState,
                    xhr.status,
                    xhr.responseText
                  ].join(":"));
                  xhr.onerror = () => resolve([
                    "xhr:error",
                    xhr.readyState,
                    xhr.status,
                    xhr.responseText
                  ].join(":"));
                  xhr.open("GET", target);
                  xhr.send();
                }});
                globalThis.__serviceWorkerOpaquePreloadXhrProbe =
                  preloadResult + "|" + xhrResult;
              }})().catch((error) => {{
                globalThis.__serviceWorkerOpaquePreloadXhrProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              }});
            }})()
            "#
    ))
    .expect("service worker opaque preload/XHR probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerOpaquePreloadXhrProbe)",
        "preload:load|xhr:error:4:0:",
    )
    .await;

    server
        .await
        .expect("service worker opaque preload/XHR server should finish");
    cross_server
        .await
        .expect("service worker opaque preload/XHR cross server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_handler_fetches_event_request_from_network() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            let fetchCount = 0;
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              event.respondWith((async () => {
                fetchCount += 1;
                const response = await fetch(event.request);
                const body = await response.text();
                return new Response("proxied:" + fetchCount + ":" + body, {
                  status: 203,
                  statusText: "Worker Network Fallback",
                  headers: {"content-type": "text/plain;charset=UTF-8"}
                });
              })());
            });
            "#,
        ),
        (
            "/app/api/network.txt",
            "text/plain; charset=utf-8",
            "network-body",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerFetchEventRequestProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/network.txt");
                globalThis.__serviceWorkerFetchEventRequestProbe = [
                  response.status,
                  response.statusText,
                  response.headers.get("content-type"),
                  await response.text()
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerFetchEventRequestProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker fetch(event.request) probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerFetchEventRequestProbe)",
        "203|Worker Network Fallback|text/plain;charset=UTF-8|proxied:1:network-body",
    )
    .await;

    let expected_fetch_url = format!("{base_url}/app/api/network.txt");
    let records = vm
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => Some(record),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(records.iter().any(|record| {
        matches!(
            record.outcome(),
            crate::types::SubresourceNetworkOutcome::Success {
                status: 203,
                status_text,
                final_url,
                ..
            } if status_text.as_deref() == Some("Worker Network Fallback")
                && final_url.as_str() == expected_fetch_url
        )
    }));
    server
        .await
        .expect("service worker network fallback server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_event_handled_reports_fetch_settlement() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              if (event.data === "bind-handled-port" && event.ports.length > 0) {
                self.handledPort = event.ports[0];
              }
            });
            self.addEventListener("fetch", event => {
              const requestUrl = new URL(event.request.url);
              const testcase = requestUrl.search.split("&")[0];
              if (!testcase.startsWith("?handled-")) {
                return;
              }
              const report = result => {
                if (self.handledPort) {
                  self.handledPort.postMessage(testcase + ":" + result);
                  return;
                }
                clients.get(event.clientId).then(client => {
                  if (client) client.postMessage(testcase + ":" + result);
                });
              };
              event.handled.then(
                () => report("RESOLVED"),
                error => report([
                  "REJECTED",
                  error && error.name,
                  error instanceof DOMException
                ].join(":"))
              );
              if (testcase === "?handled-canceled") {
                event.preventDefault();
              } else if (testcase === "?handled-valid") {
                event.respondWith(new Response("worker-valid", {status: 202}));
              } else if (testcase === "?handled-frame-response") {
                event.respondWith(new Response(
                  "<!doctype html><title>worker frame response</title>" +
                  "<body>" + [
                    "destination=" + event.request.destination,
                    "mode=" + event.request.mode,
                    "client=" + (event.clientId.length > 0),
                    "resulting=" + (event.resultingClientId.length > 0)
                  ].join("|") + "</body>", {
                    status: 203,
                    headers: {"content-type": "text/html; charset=utf-8"}
                  }
                ));
              } else if (testcase === "?handled-frame-opaque") {
                event.respondWith(fetch(requestUrl.searchParams.get("url"), {
                  mode: "no-cors"
                }));
              } else if (testcase === "?handled-invalid") {
                event.respondWith(Promise.resolve("invalid response"));
              } else if (testcase === "?handled-rejected") {
                event.respondWith(Promise.reject(new Error("respondWith rejected")));
              }
            });
            "#,
        ),
        (
            "/app/frame-handled.html?handled-frame-fallback",
            "text/html; charset=utf-8",
            "<!doctype html><title>handled frame fallback</title><body>frame fallback</body>",
        ),
        (
            "/app/api/handled.txt?handled-fallback",
            "text/plain; charset=utf-8",
            "network-fallback",
        ),
    ])
    .await;
    let (cross_base_url, cross_server) = spawn_service_worker_response_server(vec![(
        "/opaque-frame.txt",
        "text/plain; charset=utf-8",
        "opaque-frame-body",
    )])
    .await;
    let cross_fetch_url = format!("{cross_base_url}/opaque-frame.txt");
    let cross_fetch_url_literal =
        serde_json::to_string(&cross_fetch_url).expect("serialize opaque frame URL");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(&format!(
        r#"
            (() => {{
              globalThis.__serviceWorkerFetchEventHandledProbe = "pending";
              (async () => {{
                const registration =
                  await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
                await navigator.serviceWorker.ready;
                const queuedMessages = [];
                const messageResolvers = [];
                const handledChannel = new MessageChannel();
                handledChannel.port1.onmessage = event => {{
                  if (messageResolvers.length > 0) {{
                    messageResolvers.shift()(event.data);
                  }} else {{
                    queuedMessages.push(event.data);
                  }}
                }};
                registration.active.postMessage("bind-handled-port", [handledChannel.port2]);
                navigator.serviceWorker.onmessage = event => {{
                  if (messageResolvers.length > 0) {{
                    messageResolvers.shift()(event.data);
                  }} else {{
                    queuedMessages.push(event.data);
                  }}
                }};
                const nextMessage = () => {{
                  if (queuedMessages.length > 0) {{
                    return Promise.resolve(queuedMessages.shift());
                  }}
                  return new Promise(resolve => messageResolvers.push(resolve));
                }};
                const fetchOutcome = async testcase => {{
                  const fetchResultPromise = fetch("api/handled.txt?" + testcase).then(
                    async response => [
                      "fetch",
                      response.status,
                      await response.text()
                    ].join(":"),
                    error => "error:" + String(error && error.name)
                  );
                  const handledResult = await nextMessage();
                  const fetchResult = await fetchResultPromise;
                  return handledResult + "|" + fetchResult;
                }};
                const frameOutcome = async () => {{
                  const frame = document.createElement("iframe");
                  frame.src = "frame-handled.html?handled-frame-fallback";
                  (document.body || document.documentElement || document).appendChild(frame);
                  globalThis.__serviceWorkerFetchEventHandledProbe = "waiting-frame-message";
                  const handledResult = await nextMessage();
                  globalThis.__serviceWorkerFetchEventHandledProbe =
                    "waiting-frame-document:" + handledResult;
                  for (let i = 0; i < 50; i++) {{
                    const title = frame.contentDocument && frame.contentDocument.title;
                    if (title) return handledResult + "|frame:" + title;
                    await new Promise(resolve => setTimeout(resolve, 0));
                  }}
                  return handledResult + "|frame:" + String(
                    frame.contentDocument && frame.contentDocument.title
                  );
                }};
                const frameResponseOutcome = async () => {{
                  const frame = document.createElement("iframe");
                  frame.src = "frame-handled.html?handled-frame-response";
                  (document.body || document.documentElement || document).appendChild(frame);
                  globalThis.__serviceWorkerFetchEventHandledProbe =
                    "waiting-frame-response-message";
                  const handledResult = await nextMessage();
                  globalThis.__serviceWorkerFetchEventHandledProbe =
                    "waiting-frame-response-document:" + handledResult;
                  for (let i = 0; i < 50; i++) {{
                    const doc = frame.contentDocument;
                    const body = doc && doc.body && doc.body.textContent;
                    if (body) {{
                      return handledResult + "|frame:" + doc.title + ":" + body;
                    }}
                    await new Promise(resolve => setTimeout(resolve, 0));
                  }}
                  const doc = frame.contentDocument;
                  return handledResult + "|frame:" +
                    String(doc && doc.title) + ":" +
                    String(doc && doc.body && doc.body.textContent);
                }};
                const frameOpaqueOutcome = async () => {{
                  const frame = document.createElement("iframe");
                  frame.src = "frame-handled.html?handled-frame-opaque&url=" +
                    encodeURIComponent({cross_fetch_url_literal});
                  (document.body || document.documentElement || document).appendChild(frame);
                  globalThis.__serviceWorkerFetchEventHandledProbe =
                    "waiting-frame-opaque-message";
                  const handledResult = await nextMessage();
                  globalThis.__serviceWorkerFetchEventHandledProbe =
                    "waiting-frame-opaque-document:" + handledResult;
                  for (let i = 0; i < 50; i++) {{
                    const doc = frame.contentDocument;
                    const title = doc && doc.title;
                    const body = doc && doc.body && doc.body.textContent;
                    if (!doc || (!title && !body)) {{
                      return handledResult + "|frame:empty";
                    }}
                    await new Promise(resolve => setTimeout(resolve, 0));
                  }}
                  const doc = frame.contentDocument;
                  return handledResult + "|frame:" +
                    String(doc && doc.title) + ":" +
                    String(doc && doc.body && doc.body.textContent);
                }};
                const results = [];
                results.push(await frameOutcome());
                results.push(await frameResponseOutcome());
                results.push(await frameOpaqueOutcome());
                results.push(await fetchOutcome("handled-fallback"));
                results.push(await fetchOutcome("handled-canceled"));
                results.push(await fetchOutcome("handled-valid"));
                results.push(await fetchOutcome("handled-invalid"));
                results.push(await fetchOutcome("handled-rejected"));
                globalThis.__serviceWorkerFetchEventHandledProbe = results.join(";");
              }})().catch(error => {{
                globalThis.__serviceWorkerFetchEventHandledProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              }});
            }})()
            "#,
    ))
    .expect("service worker FetchEvent.handled probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerFetchEventHandledProbe)",
        concat!(
            "?handled-frame-fallback:RESOLVED|frame:handled frame fallback;",
            "?handled-frame-response:RESOLVED|frame:worker frame response:",
            "destination=iframe|mode=navigate|client=true|resulting=true;",
            "?handled-frame-opaque:REJECTED:NetworkError:true|frame:empty;",
            "?handled-fallback:RESOLVED|fetch:200:network-fallback;",
            "?handled-canceled:REJECTED:NetworkError:true|error:TypeError;",
            "?handled-valid:RESOLVED|fetch:202:worker-valid;",
            "?handled-invalid:REJECTED:NetworkError:true|error:TypeError;",
            "?handled-rejected:REJECTED:NetworkError:true|error:TypeError"
        ),
    )
    .await;

    server
        .await
        .expect("service worker FetchEvent.handled server should finish");
    cross_server
        .await
        .expect("service worker FetchEvent.handled cross server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_abort_reason_reaches_fetch_event_request_signal() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              if (!event.request.url.includes("abort-reason")) {
                return;
              }
              event.respondWith(new Promise(resolve => {
                const signal = event.request.signal;
                clients.get(event.clientId).then(client => {
                  if (client) client.postMessage("fetch-arrived");
                });
                signal.addEventListener("abort", () => {
                  clients.get(event.clientId).then(client => {
                    if (client) {
                      client.postMessage(JSON.stringify({
                        aborted: signal.aborted,
                        name: signal.reason && signal.reason.name,
                        message: signal.reason && signal.reason.message
                      }));
                    }
                  }).then(() => resolve(new Response("ignored-after-abort")));
                });
              }));
            });
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
            (() => {
              globalThis.__serviceWorkerAbortReasonProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const messageResolvers = [];
                navigator.serviceWorker.onmessage = event => {
                  const resolve = messageResolvers.shift();
                  if (resolve) resolve(event.data);
                };
                const nextMessage = () => new Promise(resolve => {
                  messageResolvers.push(resolve);
                });
                const controller = new AbortController();
                const reason = new Error("page-abort");
                const fetchResultPromise = fetch("api/abort-reason.txt", {
                  signal: controller.signal
                }).then(
                  () => "unexpected-resolve",
                  error => [
                    error === reason,
                    error && error.name,
                    error && error.message
                  ].join(":")
                );
                const arrived = await nextMessage();
                controller.abort(reason);
                const fetchResult = await fetchResultPromise;
                const workerReason = await nextMessage();
                globalThis.__serviceWorkerAbortReasonProbe =
                  [arrived, fetchResult, workerReason].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerAbortReasonProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker abort reason probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerAbortReasonProbe)",
        r#"fetch-arrived|true:Error:page-abort|{"aborted":true,"name":"Error","message":"page-abort"}"#,
    )
    .await;

    server
        .await
        .expect("service worker abort reason server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_abort_reason_serializes_on_abort() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              if (!event.request.url.includes("abort-reason-serialization")) {
                return;
              }
              event.respondWith(new Promise(resolve => {
                const signal = event.request.signal;
                clients.get(event.clientId).then(client => {
                  if (client) client.postMessage("fetch-arrived");
                });
                signal.addEventListener("abort", () => {
                  clients.get(event.clientId).then(client => {
                    if (client) {
                      client.postMessage(JSON.stringify({
                        name: signal.reason && signal.reason.name,
                        message: signal.reason && signal.reason.message
                      }));
                    }
                  }).then(() => resolve(new Response("ignored-after-abort")));
                });
              }));
            });
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
            (() => {
              globalThis.__serviceWorkerAbortReasonSerializationProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const messageResolvers = [];
                navigator.serviceWorker.onmessage = event => {
                  const resolve = messageResolvers.shift();
                  if (resolve) resolve(event.data);
                };
                const nextMessage = () => new Promise(resolve => {
                  messageResolvers.push(resolve);
                });
                const controller = new AbortController();
                const reason = new Error("serialization");
                reason.name = "error1";
                const setterHits = [];
                const fetchResultPromise = fetch("api/abort-reason-serialization.txt", {
                  signal: controller.signal
                }).then(
                  () => "unexpected-resolve",
                  error => [
                    error === reason,
                    error && error.name,
                    error && error.message
                  ].join(":")
                );
                const arrived = await nextMessage();
                for (const key of ["name", "message"]) {
                  Object.defineProperty(Object.prototype, key, {
                    configurable: true,
                    set(value) { setterHits.push(`${key}:${typeof value}`); }
                  });
                }
                try {
                  controller.abort(reason);
                } finally {
                  for (const key of ["name", "message"]) {
                    delete Object.prototype[key];
                  }
                }
                const originalName = reason.name;
                reason.name = "error2";
                const workerReason = await nextMessage();
                const fetchResult = await fetchResultPromise;
                globalThis.__serviceWorkerAbortReasonSerializationProbe = [
                  arrived,
                  originalName,
                  workerReason,
                  fetchResult,
                  setterHits.join(",")
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerAbortReasonSerializationProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker abort reason serialization probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerAbortReasonSerializationProbe)",
        r#"fetch-arrived|error1|{"name":"error1","message":"serialization"}|true:error2:serialization|"#,
    )
    .await;

    server
        .await
        .expect("service worker abort reason serialization server should finish");
}
#[tokio::test]
async fn navigator_service_worker_pre_aborted_fetch_does_not_dispatch_fetch_event() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const url = new URL(event.request.url);
              if (!url.pathname.includes("pre-aborted")) {
                return;
              }
              event.waitUntil(clients.get(event.clientId).then(client => {
                if (client) client.postMessage(url.search);
              }));
              event.respondWith(new Response("sw:" + url.search));
            });
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
            (() => {
              globalThis.__serviceWorkerPreAbortedFetchProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const queuedMessages = [];
                const messageResolvers = [];
                navigator.serviceWorker.onmessage = event => {
                  if (messageResolvers.length > 0) {
                    messageResolvers.shift()(event.data);
                  } else {
                    queuedMessages.push(event.data);
                  }
                };
                const nextMessage = () => {
                  if (queuedMessages.length > 0) {
                    return Promise.resolve(queuedMessages.shift());
                  }
                  return new Promise(resolve => messageResolvers.push(resolve));
                };
                const controller = new AbortController();
                controller.abort();
                const abortedResult = await fetch("api/pre-aborted.txt?aborted", {
                  signal: controller.signal
                }).then(
                  () => "unexpected-resolve",
                  error => [
                    error && error.name,
                    error instanceof DOMException,
                    error && error.message
                  ].join(":")
                );
                const noAbortResponse = await fetch("api/pre-aborted.txt?no-abort");
                const firstWorkerMessage = await nextMessage();
                globalThis.__serviceWorkerPreAbortedFetchProbe = [
                  abortedResult,
                  firstWorkerMessage,
                  noAbortResponse.status,
                  await noAbortResponse.text()
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerPreAbortedFetchProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker pre-aborted fetch probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPreAbortedFetchProbe)",
        "AbortError:true:The operation was aborted.|?no-abort|200|sw:?no-abort",
    )
    .await;

    server
        .await
        .expect("service worker pre-aborted fetch server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_response_body_abort_uses_abort_reason() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              if (!event.request.url.includes("body-abort-reason")) {
                return;
              }
              const stream = new ReadableStream({
                start(controller) {
                  self.__bodyAbortReasonController = controller;
                  controller.enqueue(new Uint8Array([65]));
                },
                cancel(reason) {
                  self.__bodyAbortReasonCancel = reason;
                }
              });
              event.respondWith(new Response(stream, {status: 210}));
            });
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
            (() => {
              globalThis.__serviceWorkerBodyAbortReasonProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const controller = new AbortController();
                const reason = new Error("body-abort");
                const response = await fetch("api/body-abort-reason.txt", {
                  signal: controller.signal
                });
                const reader = response.body.getReader();
                const first = await reader.read();
                const bodyPromise = reader.read().then(
                  () => "unexpected-resolve",
                  error => [
                    error === reason,
                    error && error.name,
                    error && error.message
                  ].join(":")
                );
                controller.abort(reason);
                globalThis.__serviceWorkerBodyAbortReasonProbe = [
                  response.status,
                  first.done,
                  new TextDecoder().decode(first.value),
                  await bodyPromise
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerBodyAbortReasonProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker body abort reason probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerBodyAbortReasonProbe)",
        "210|false|A|true:Error:body-abort",
    )
    .await;

    server
        .await
        .expect("service worker body abort reason server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_fetch_resolves_before_body_chunk() {
    let (base_url, server, release_body) = spawn_service_worker_headers_first_body_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerRespondWithFetchHeadersFirstProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/headers-first.txt");
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstProbe = [
                  "resolved",
                  response.status,
                  response.headers.get("content-type"),
                  response.bodyUsed
                ].join("|");
                const body = await response.text();
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstProbe += "|" + body;
              })().catch((error) => {
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker respondWith(fetch()) headers-first probe should evaluate");

    let headers_first_state = "resolved|200|text/plain; charset=utf-8|false";
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let value = vm
            .eval("String(globalThis.__serviceWorkerRespondWithFetchHeadersFirstProbe)")
            .expect("headers-first probe should evaluate");
        if value == headers_first_state {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("service worker respondWith(fetch()) did not resolve headers-first: {value}");
        }
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    release_body
        .send(())
        .expect("headers-first server body release should send");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRespondWithFetchHeadersFirstProbe)",
        "resolved|200|text/plain; charset=utf-8|false|delayed-body",
    )
    .await;

    server
        .await
        .expect("service worker headers-first server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_fetch_body_read_abort_headers_first() {
    let (base_url, server, release_body) = spawn_service_worker_headers_first_body_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerRespondWithFetchHeadersFirstAbortProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const controller = new AbortController();
                const reason = new Error("headers-first-abort");
                const response = await fetch("api/headers-first.txt", {
                  signal: controller.signal
                });
                const reader = response.body.getReader();
                const readPromise = reader.read().then(
                  () => "read:unexpected-resolve",
                  error => [
                    "read",
                    error === reason,
                    error && error.name,
                    error && error.message
                  ].join(":")
                );
                controller.abort(reason);
                const closedResult = await reader.closed.then(
                  () => "closed:unexpected-resolve",
                  error => [
                    "closed",
                    error === reason,
                    error && error.name,
                    error && error.message
                  ].join(":")
                );
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstAbortProbe = [
                  response.status,
                  await readPromise,
                  closedResult
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstAbortProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker respondWith(fetch()) headers-first abort probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRespondWithFetchHeadersFirstAbortProbe)",
        concat!(
            "200|",
            "read:true:Error:headers-first-abort|",
            "closed:true:Error:headers-first-abort"
        ),
    )
    .await;

    let _ = release_body.send(());
    server
        .await
        .expect("service worker headers-first abort server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_fetch_body_methods_abort_headers_first() {
    for method in ["arrayBuffer", "blob", "bytes", "formData", "json", "text"] {
        let (base_url, server, release_body) =
            spawn_service_worker_headers_first_body_server().await;
        let loader =
            ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                &format!("{base_url}/app/page.html"),
                &loader,
            );

        vm.eval(&format!(
            r#"
            (() => {{
              globalThis.__serviceWorkerRespondWithFetchHeadersFirstBodyMethodProbe = "pending";
              (async () => {{
                await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
                await navigator.serviceWorker.ready;
                const method = "{method}";
                const controller = new AbortController();
                const response = await fetch("api/headers-first.txt", {{
                  signal: controller.signal
                }});
                controller.abort();
                const log = [];
                const bodyPromise = response[method]().then(
                  () => log.push(method + ":unexpected-resolve"),
                  error => log.push([
                    method,
                    error && error.name,
                    error instanceof DOMException,
                    error && error.message
                  ].join(":"))
                );
                await Promise.all([
                  bodyPromise,
                  Promise.resolve().then(() => log.push("next-microtask"))
                ]);
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstBodyMethodProbe = [
                  response.status,
                  log.join(">")
                ].join("|");
              }})().catch((error) => {{
                globalThis.__serviceWorkerRespondWithFetchHeadersFirstBodyMethodProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              }});
            }})()
            "#
        ))
        .expect(
            "service worker respondWith(fetch()) headers-first body method probe should evaluate",
        );

        let expected =
            format!("200|{method}:AbortError:true:The operation was aborted.>next-microtask");
        drain_service_worker_test_until_eval_equals(
            &mut vm,
            &browser_context_runtime,
            &loader,
            "String(globalThis.__serviceWorkerRespondWithFetchHeadersFirstBodyMethodProbe)",
            &expected,
        )
        .await;

        let _ = release_body.send(());
        server
            .await
            .expect("service worker headers-first body method server should finish");
    }
}
#[tokio::test]
async fn navigator_service_worker_respond_with_fetch_event_request_stream_body() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              event.respondWith(fetch(event.request));
            });
            "#,
        ),
        (
            "/app/api/direct-stream.txt",
            "text/plain; charset=utf-8",
            "network-stream-body",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerRespondWithFetchStreamProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/direct-stream.txt");
                globalThis.__serviceWorkerRespondWithFetchStreamProbe = [
                  response.status,
                  response.headers.get("content-type"),
                  await response.text()
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerRespondWithFetchStreamProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker respondWith(fetch()) stream probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRespondWithFetchStreamProbe)",
        "200|text/plain; charset=utf-8|network-stream-body",
    )
    .await;

    server
        .await
        .expect("service worker respondWith fetch stream server should finish");
}
#[tokio::test]
async fn navigator_service_worker_invalid_response_body_chunk_errors_reader_not_fetch() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const stream = new ReadableStream({
                start(controller) {
                  Promise.resolve()
                    .then(() => controller.enqueue(new Uint8Array([65])))
                    .then(() => controller.enqueue("not-bytes"));
                }
              });
              event.respondWith(new Response(stream, {status: 209}));
            });
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
            (() => {
              globalThis.__serviceWorkerInvalidChunkProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/invalid-chunk.txt");
                const reader = response.body.getReader();
                const first = await reader.read();
                try {
                  await reader.read();
                  globalThis.__serviceWorkerInvalidChunkProbe = "unexpected-resolve";
                } catch (error) {
                  globalThis.__serviceWorkerInvalidChunkProbe = [
                    "fetch-resolved",
                    response.status,
                    first.done,
                    new TextDecoder().decode(first.value),
                    error && error.name,
                    String(error && error.message).includes("ReadableStream body chunks must be Uint8Array")
                  ].join("|");
                }
              })().catch((error) => {
                globalThis.__serviceWorkerInvalidChunkProbe =
                  "fetch-rejected:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker invalid response body chunk probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerInvalidChunkProbe)",
        "fetch-resolved|209|false|A|TypeError|true",
    )
    .await;

    server
        .await
        .expect("service worker invalid response body chunk server should finish");
}
#[tokio::test]
async fn navigator_service_worker_response_stream_error_rejects_text_not_fetch() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const stream = new ReadableStream({
                start(controller) {
                  Promise.resolve()
                    .then(() => controller.enqueue(new Uint8Array([65])))
                    .then(() => controller.error("stream-broken"));
                }
              });
              event.respondWith(new Response(stream, {status: 208}));
            });
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
            (() => {
              globalThis.__serviceWorkerStreamErrorProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/stream-error.txt");
                try {
                  await response.text();
                  globalThis.__serviceWorkerStreamErrorProbe = "unexpected-text-resolve";
                } catch (error) {
                  globalThis.__serviceWorkerStreamErrorProbe = [
                    "fetch-resolved",
                    response.status,
                    error && error.name,
                    String(error && error.message).includes("stream-broken")
                  ].join("|");
                }
              })().catch((error) => {
                globalThis.__serviceWorkerStreamErrorProbe =
                  "fetch-rejected:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker stream error probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerStreamErrorProbe)",
        "fetch-resolved|208|TypeError|true",
    )
    .await;

    server
        .await
        .expect("service worker response stream error server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_argument_type_controls_fetch_and_xhr_results() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const testcase = new URL(event.request.url).search;
              switch (testcase) {
                case "?response-object":
                  event.respondWith(new Response("direct-body", {status: 211}));
                  break;
                case "?response-promise-object":
                  event.respondWith(Promise.resolve(new Response("promise-body", {status: 212})));
                  break;
                case "?other-value":
                  event.respondWith({not: "a response"});
                  break;
              }
            });
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
            (() => {
              globalThis.__serviceWorkerRespondWithArgumentProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const direct = await fetch("api/data.txt?response-object");
                const promised = await fetch("api/data.txt?response-promise-object");
                let invalidResult;
                try {
                  await fetch("api/data.txt?other-value");
                  invalidResult = "unexpected-resolve";
                } catch (error) {
                  invalidResult = [
                    error && error.name,
                    String(error && error.message).includes("FetchEvent.respondWith requires a Response")
                  ].join(":");
                }
                const runXhr = name => new Promise(resolve => {
                  const xhr = new XMLHttpRequest();
                  xhr.onload = () => resolve([
                    name,
                    "load",
                    xhr.readyState,
                    xhr.status,
                    xhr.statusText,
                    xhr.responseText
                  ].join(":"));
                  xhr.onerror = () => resolve([
                    name,
                    "error",
                    xhr.readyState,
                    xhr.status,
                    xhr.statusText,
                    xhr.responseText
                  ].join(":"));
                  xhr.open("GET", "api/xhr-data.txt?" + name);
                  xhr.send();
                });
                globalThis.__serviceWorkerRespondWithArgumentProbe = [
                  direct.status,
                  await direct.text(),
                  promised.status,
                  await promised.text(),
                  invalidResult,
                  await runXhr("response-object"),
                  await runXhr("response-promise-object"),
                  await runXhr("other-value")
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerRespondWithArgumentProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker respondWith argument probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRespondWithArgumentProbe)",
        concat!(
            "211|direct-body|212|promise-body|TypeError:true|",
            "response-object:load:4:211::direct-body|",
            "response-promise-object:load:4:212::promise-body|",
            "other-value:error:4:0::"
        ),
    )
    .await;

    server
        .await
        .expect("service worker respondWith argument server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_event_network_error_xhr_matrix() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", () => {});
            self.addEventListener("fetch", event => {
              const testcase = new URL(event.request.url).search;
              switch (testcase) {
                case "?reject":
                  event.respondWith(Promise.reject(new Error("rejected")));
                  break;
                case "?prevent-default":
                  event.preventDefault();
                  break;
                case "?prevent-default-and-respond-with":
                  event.preventDefault();
                  break;
                case "?unused-body":
                  event.respondWith(new Response("body"));
                  break;
                case "?used-body": {
                  const response = new Response("body");
                  response.text();
                  event.respondWith(response);
                  break;
                }
                case "?unused-fetched-body":
                  event.respondWith(fetch("other.html").then(response => response));
                  break;
                case "?used-fetched-body":
                  event.respondWith(fetch("other.html").then(response => {
                    response.text();
                    return response;
                  }));
                  break;
                case "?throw-exception":
                  throw new Error("boom");
              }
            });
            self.addEventListener("fetch", () => {});
            self.addEventListener("fetch", event => {
              if (new URL(event.request.url).search === "?prevent-default-and-respond-with") {
                event.respondWith(new Response("responding!"));
              }
            });
            self.addEventListener("fetch", () => {});
            "#,
        ),
        ("/app/other.html", "text/plain", "other-body"),
        ("/app/other.html", "text/plain", "other-body-used"),
        (
            "/app/api/network-error?throw-exception",
            "text/plain",
            "fallback-body",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerNetworkErrorProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const run = name => new Promise(resolve => {
                  const xhr = new XMLHttpRequest();
                  xhr.onload = () => resolve([
                    name,
                    "load",
                    xhr.readyState,
                    xhr.status,
                    xhr.statusText,
                    xhr.responseText
                  ].join(":"));
                  xhr.onerror = () => resolve([
                    name,
                    "error",
                    xhr.readyState,
                    xhr.status,
                    xhr.statusText,
                    xhr.responseText
                  ].join(":"));
                  xhr.open("GET", "api/network-error?" + name);
                  xhr.send();
                });
                const cases = [
                  "prevent-default-and-respond-with",
                  "prevent-default",
                  "reject",
                  "unused-body",
                  "used-body",
                  "unused-fetched-body",
                  "used-fetched-body",
                  "throw-exception"
                ];
                globalThis.__serviceWorkerNetworkErrorProbe =
                  (await cases.reduce(
                    (promise, name) => promise.then(async results => {
                      results.push(await run(name));
                      return results;
                    }),
                    Promise.resolve([])
                  )).join("|");
              })().catch(error => {
                globalThis.__serviceWorkerNetworkErrorProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker network error probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNetworkErrorProbe)",
        concat!(
            "prevent-default-and-respond-with:load:4:200::responding!|",
            "prevent-default:error:4:0::|",
            "reject:error:4:0::|",
            "unused-body:load:4:200::body|",
            "used-body:error:4:0::|",
            "unused-fetched-body:load:4:200:OK:other-body|",
            "used-fetched-body:error:4:0::|",
            "throw-exception:load:4:200:OK:fallback-body"
        ),
    )
    .await;

    server
        .await
        .expect("service worker network error server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_no_cors_fetch_projects_opaque_response() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              event.respondWith(fetch(event.request));
            });
            "#,
    )])
    .await;
    let (cross_base_url, cross_server) = spawn_service_worker_response_server(vec![(
        "/opaque-data.png",
        "image/png",
        "opaque-secret",
    )])
    .await;
    let cross_fetch_url = format!("{cross_base_url}/opaque-data.png");
    let cross_fetch_url_literal =
        serde_json::to_string(&cross_fetch_url).expect("serialize opaque fetch URL");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(&format!(
        r#"
            (() => {{
              globalThis.__serviceWorkerOpaqueRespondWithFetchProbe = "pending";
              (async () => {{
                await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
                await navigator.serviceWorker.ready;
                const response = await fetch({cross_fetch_url_literal}, {{ mode: "no-cors" }});
                const clone = response.clone();
                const bodyUsedBefore = response.bodyUsed;
                const text = await response.text();
                const cloneText = await clone.text();
                globalThis.__serviceWorkerOpaqueRespondWithFetchProbe = [
                  response.type,
                  response.status,
                  response.ok,
                  response.statusText,
                  response.url,
                  response.redirected,
                  response.body === null,
                  Array.from(response.headers).length,
                  bodyUsedBefore,
                  response.bodyUsed,
                  text,
                  clone.type,
                  clone.status,
                  clone.body === null,
                  cloneText
                ].join("|");
              }})().catch((error) => {{
                globalThis.__serviceWorkerOpaqueRespondWithFetchProbe =
                  "error:" + String(error && error.message);
              }});
            }})()
            "#
    ))
    .expect("service worker opaque respondWith fetch probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerOpaqueRespondWithFetchProbe)",
        "opaque|0|false|||false|true|0|false|false||opaque|0|true|",
    )
    .await;

    server
        .await
        .expect("service worker opaque respondWith fetch script server should finish");
    cross_server
        .await
        .expect("service worker opaque respondWith fetch data server should finish");
}
#[tokio::test]
async fn navigator_service_worker_fetch_restarts_stopped_active_worker() {
    let worker_script = r#"
        let fetchCounter = 0;
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          fetchCounter += 1;
          const path = new URL(event.request.url).pathname;
          event.respondWith(new Response("counter:" + fetchCounter + ":" + path, {
            status: 200,
            statusText: "Restart Probe",
            headers: {"content-type": "text/plain;charset=UTF-8"}
          }));
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        worker_script,
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
            (() => {
              globalThis.__serviceWorkerRestartProbe = { phase: "registering" };
              (async () => {
                const registration = await navigator.serviceWorker.register("worker.js", {
                  scope: "./"
                });
                await navigator.serviceWorker.ready;
                const first = await fetch("first.txt");
                const second = await fetch("second.txt");
                globalThis.__serviceWorkerRestartProbe.beforeStop = [
                  await first.text(),
                  await second.text(),
                  registration.active && registration.active.state
                ].join("|");
                globalThis.__serviceWorkerRestartProbe.phase = "beforeStop";
              })().catch((error) => {
                globalThis.__serviceWorkerRestartProbe.phase =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker restart setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRestartProbe.phase)",
        "beforeStop",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__serviceWorkerRestartProbe.beforeStop)")
            .expect("before-stop probe should be readable"),
        r#""counter:1:/app/first.txt|counter:2:/app/second.txt|activated""#
    );

    // Close the script server before restarting: the installed version must
    // start from its stored source without another network request.
    server
        .await
        .expect("service worker restart script server should finish");
    browser_context_runtime.stop_service_worker_hosts_for_test();
    let diagnostics = browser_context_runtime.moli_memory_diagnostics();
    assert_eq!(diagnostics["serviceWorker"]["runningVersions"], 0);
    assert_eq!(diagnostics["serviceWorker"]["stoppedVersions"], 1);

    vm.eval(
        r#"
            (async () => {
              const third = await fetch("third.txt");
              globalThis.__serviceWorkerRestartProbe.afterStop = await third.text();
              globalThis.__serviceWorkerRestartProbe.phase = "afterStop";
            })().catch((error) => {
              globalThis.__serviceWorkerRestartProbe.phase =
                "restart-error:" + String(error && error.message);
            });
            "#,
    )
    .expect("service worker restart fetch should schedule");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRestartProbe.phase)",
        "afterStop",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__serviceWorkerRestartProbe.afterStop)")
            .expect("after-stop probe should be readable"),
        r#""counter:1:/app/third.txt""#
    );
}
#[tokio::test]
async fn navigator_service_worker_event_listeners_do_not_drive_lifecycle_state() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/listener-worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerListenerProbe = "pending";
              (async () => {
                const registration =
                  await navigator.serviceWorker.register("listener-worker.js", { scope: "./" });
                const installing = registration.installing;
                const events = [];
                registration.addEventListener("updatefound", () => events.push("updatefound"));
                installing.addEventListener("statechange", () => {
                  events.push("statechange:" + installing.state);
                });
                await Promise.resolve();
                globalThis.__serviceWorkerListenerProbe = [
                  events.join(","),
                  installing.state,
                  registration.installing === installing,
                  registration.waiting === null,
                  registration.active === null
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerListenerProbe = "error:" + String(error);
              });
            })()
            "#,
    )
    .expect("service worker listener lifecycle probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerListenerProbe !== 'pending')",
        "true",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__serviceWorkerListenerProbe)")
        .expect("service worker listener probe should settle");

    assert_eq!(result, r#""|installing|true|true|true""#);
    server
        .await
        .expect("service worker listener script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_navigation_preload_enabled_during_activate_serves_iframe() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("activate", event => {
              event.waitUntil((async () => {
                const manager = self.registration.navigationPreload;
                await manager.enable();
                await manager.disable();
                await manager.setHeaderValue("activate-preload");
                await manager.enable();
                self.preloadStateDuringActivate = await manager.getState();
              })());
            });
            self.addEventListener("fetch", event => {
              event.respondWith((async () => {
                const response = await event.preloadResponse;
                const result = {
                  duringActivate: self.preloadStateDuringActivate,
                  mode: event.request.mode,
                  destination: event.request.destination,
                  body: await response.text()
                };
                return new Response(
                  "<script>parent.postMessage(" + JSON.stringify(result) + ", '*')</script>",
                  {headers: {"Content-Type": "text/html"}}
                );
              })());
            });
            "#,
        ),
        ("/app/frame.html", "text/plain", "preloaded-body"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );
    vm.eval(
        r#"
        globalThis.preloadIframeResult = "pending";
        (async () => {
          const registration = await navigator.serviceWorker.register("worker.js", {scope: "./"});
          await navigator.serviceWorker.ready;
          const afterActivate = await registration.navigationPreload.getState();
          if (!afterActivate.enabled) throw new Error("activate did not enable preload");
          const iframe = document.createElement("iframe");
          const message = new Promise(resolve => {
            addEventListener("message", event => resolve(event.data), {once: true});
          });
          iframe.src = "frame.html";
          document.body.appendChild(iframe);
          preloadIframeResult = JSON.stringify({afterActivate, response: await message});
          iframe.remove();
          await registration.unregister();
        })().catch(error => { preloadIframeResult = String(error); });
        "#,
    )
    .expect("navigation preload iframe test should schedule");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(preloadIframeResult !== 'pending')",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("preloadIframeResult").unwrap(),
        r#"{"afterActivate":{"enabled":true,"headerValue":"activate-preload"},"response":{"duringActivate":{"enabled":true,"headerValue":"activate-preload"},"mode":"navigate","destination":"iframe","body":"preloaded-body"}}"#
    );
    server
        .await
        .expect("navigation preload server should finish");
}

#[tokio::test]
async fn navigator_service_worker_navigation_preload_state_shared_with_worker() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil((async () => {
                const manager = self.registration.navigationPreload;
                const before = await manager.getState();
                await manager.setHeaderValue("worker-preload");
                const after = await manager.getState();
                event.source.postMessage(JSON.stringify({
                  hasManager: !!manager,
                  instance: manager instanceof NavigationPreloadManager,
                  enable: typeof manager.enable,
                  disable: typeof manager.disable,
                  setHeaderValue: typeof manager.setHeaderValue,
                  getState: typeof manager.getState,
                  before,
                  after
                }));
              })());
            });
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
        (() => {
          globalThis.__serviceWorkerNavigationPreloadProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            const manager = registration.navigationPreload;
            const defaultState = await manager.getState();
            let invalidHeaderError = null;
            try {
              await manager.setHeaderValue("bad\nvalue");
            } catch (error) {
              invalidHeaderError = {
                name: error && error.name,
                isTypeError: error instanceof TypeError
              };
            }
            await manager.enable();
            const enabledState = await manager.getState();
            const workerResult = await new Promise(resolve => {
              sw.onmessage = event => resolve(JSON.parse(event.data));
              registration.active.postMessage("navigation-preload");
            });
            const finalState = await manager.getState();
            globalThis.__serviceWorkerNavigationPreloadProbe = JSON.stringify({
              hasManager: !!manager,
              instance: manager instanceof NavigationPreloadManager,
              enable: typeof manager.enable,
              disable: typeof manager.disable,
              setHeaderValue: typeof manager.setHeaderValue,
              getState: typeof manager.getState,
              defaultState,
              invalidHeaderError,
              enabledState,
              workerResult,
              finalState
            });
          })().catch(error => {
            globalThis.__serviceWorkerNavigationPreloadProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker navigation preload probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNavigationPreloadProbe)",
        r#"{"hasManager":true,"instance":true,"enable":"function","disable":"function","setHeaderValue":"function","getState":"function","defaultState":{"enabled":false,"headerValue":"true"},"invalidHeaderError":{"name":"TypeError","isTypeError":true},"enabledState":{"enabled":true,"headerValue":"true"},"workerResult":{"hasManager":true,"instance":true,"enable":"function","disable":"function","setHeaderValue":"function","getState":"function","before":{"enabled":true,"headerValue":"true"},"after":{"enabled":true,"headerValue":"worker-preload"}},"finalState":{"enabled":true,"headerValue":"worker-preload"}}"#,
    )
    .await;

    server
        .await
        .expect("service worker navigation preload script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_periodic_sync_dispatches_owner_functional_event() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("periodicsync", event => {
              event.waitUntil((async () => {
                const tags = await self.registration.periodicSync.getTags();
                const windows = await clients.matchAll({ includeUncontrolled: true });
                if (windows[0]) {
                  windows[0].postMessage(JSON.stringify({
                    type: event.type,
                    tag: event.tag,
                    hasLastChance: "lastChance" in event,
                    tags: tags.slice().sort().join("|")
                  }));
                }
              })());
            });
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
        (() => {
          globalThis.__serviceWorkerPeriodicSyncDispatchReady = "pending";
          globalThis.__serviceWorkerPeriodicSyncDispatchProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerPeriodicSyncDispatchProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            await registration.periodicSync.register("periodic-event", {
              minInterval: 60000
            });
            const tags = await registration.periodicSync.getTags();
            globalThis.__serviceWorkerPeriodicSyncDispatchReady = JSON.stringify({
              tags
            });
          })().catch(error => {
            globalThis.__serviceWorkerPeriodicSyncDispatchReady =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker periodic sync dispatch setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPeriodicSyncDispatchReady)",
        r#"{"tags":["periodic-event"]}"#,
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__serviceWorkerPeriodicSyncDispatchProbe)")
            .expect("periodic sync dispatch probe should evaluate"),
        "pending",
        "PeriodicSyncManager.register() should not synchronously fake a periodicsync event"
    );

    let scope_url = url::Url::parse(&format!("{base_url}/app/")).unwrap();
    assert!(!browser_context_runtime.dispatch_service_worker_periodic_sync(&scope_url, "missing",));
    assert!(
        browser_context_runtime
            .dispatch_service_worker_periodic_sync(&scope_url, "periodic-event",)
    );

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPeriodicSyncDispatchProbe)",
        r#"{"type":"periodicsync","tag":"periodic-event","hasLastChance":false,"tags":"periodic-event"}"#,
    )
    .await;

    server
        .await
        .expect("service worker periodic sync dispatch script server should finish");
}

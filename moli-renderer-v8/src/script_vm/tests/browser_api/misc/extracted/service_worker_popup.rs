use super::*;

#[tokio::test]
async fn navigator_service_worker_intercepts_popup_csp_report_destination() {
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
          if (url.pathname === "/app/csp-report") {
            event.respondWith((async () => {
              const client = await clients.get(event.clientId);
              const clientPath = client ? new URL(client.url).pathname : "";
              return new Response([
                "destination=" + event.request.destination,
                "mode=" + event.request.mode,
                "credentials=" + event.request.credentials,
                "method=" + event.request.method,
                "client=" + (event.clientId.length > 0),
                "clientPath=" + clientPath
              ].join("|"));
            })());
          }
        });
        "#,
        ),
        (
            "/app/popup.html",
            "text/html; charset=utf-8",
            vec![(
                "Content-Security-Policy",
                "connect-src 'none'; report-uri /app/csp-report",
            )],
            r#"<!doctype html>
        <script>
        let violationSeen = false;
        self.addEventListener("securitypolicyviolation", event => {
          violationSeen = event.type === "securitypolicyviolation" &&
            event.effectiveDirective === "connect-src" &&
            event.violatedDirective === "connect-src" &&
            event.blockedURI.endsWith("/app/blocked-data") &&
            event.disposition === "enforce" &&
            event instanceof SecurityPolicyViolationEvent;
        });
        (async () => {
          await fetch("blocked-data").catch(() => {});
          opener.postMessage("blocked:" + violationSeen, "*");
        })().catch(error => {
          opener.postMessage("error:" + String(error && error.message), "*");
        });
        </script>"#,
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
              globalThis.__serviceWorkerPopupCspReportProbe = "pending";
              addEventListener("message", event => {
                globalThis.__serviceWorkerPopupCspReportProbe = String(event.data);
              });
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                window.open("popup.html");
              })().catch((error) => {
                globalThis.__serviceWorkerPopupCspReportProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker popup CSP report destination setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPopupCspReportProbe)",
        "blocked:true",
    )
    .await;

    let report_url = format!("{base_url}/app/csp-report");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        items.extend(vm.take_network_output().into_items());
        if service_worker_csp_report_seen(
            &items,
            &report_url,
            "destination=report|mode=no-cors|credentials=same-origin|method=POST|client=true|clientPath=/app/popup.html",
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "popup CSP report destination did not settle: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    server
        .await
        .expect("popup CSP report destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_popup_post_message_uses_popup_source_url() {
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
            self.addEventListener("message", event => {
              event.waitUntil(Promise.resolve().then(() => {
                const source = event.source;
                event.source.postMessage([
                  "reply",
                  event.data,
                  event.constructor && event.constructor.name,
                  event instanceof ExtendableMessageEvent,
                  event instanceof ExtendableEvent,
                  event instanceof Event,
                  Object.prototype.toString.call(event),
                  event.origin,
                  event.lastEventId,
                  event.ports.length,
                  event.ports[0] instanceof MessagePort,
                  source && source.constructor && source.constructor.name,
                  source instanceof WindowClient,
                  source instanceof Client,
                  source && source.url,
                  source && source.type,
                  source && source.frameType,
                  source && source.visibilityState,
                  source && source.focused
                ].join("|"));
              }));
            });
            "#,
        ),
        (
            "/app/popup.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>popup</title>",
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
          globalThis.__serviceWorkerPopupSourceProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            const popup = open("popup.html", "sw-source-popup");
            if (!popup) {
              globalThis.__serviceWorkerPopupSourceProbe = "open-null";
            }
            globalThis.__serviceWorkerPopupSourceWindow = popup;
          })().catch(error => {
            globalThis.__serviceWorkerPopupSourceProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker popup source probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        r#"String(globalThis.__serviceWorkerPopupSourceWindow && globalThis.__serviceWorkerPopupSourceWindow.document && globalThis.__serviceWorkerPopupSourceWindow.document.title)"#,
        "popup",
    )
    .await;

    vm.eval(
        r#"
        (() => {
          const popup = globalThis.__serviceWorkerPopupSourceWindow;
          const sw = popup.navigator.serviceWorker;
          sw.onmessage = event => {
            globalThis.__serviceWorkerPopupSourceProbe = event.data;
          };
          const controller = sw.controller;
          if (!controller) {
            globalThis.__serviceWorkerPopupSourceProbe = "no-controller";
            return;
          }
          const channel = new MessageChannel();
          controller.postMessage("from-popup", [channel.port2]);
          globalThis.__serviceWorkerPopupSourcePostResult = [
            typeof sw,
            typeof controller,
            controller && controller.state
          ].join("|");
        })()
        "#,
    )
    .expect("service worker popup controller postMessage should evaluate");

    assert_eq!(
        vm.eval("String(globalThis.__serviceWorkerPopupSourcePostResult)")
            .expect("service worker popup post result should evaluate"),
        "object|object|activated"
    );

    let expected_popup_url = format!("{base_url}/app/popup.html");
    let expected = format!(
        "reply|from-popup|ExtendableMessageEvent|true|true|true|[object ExtendableMessageEvent]|{base_url}||1|true|WindowClient|true|true|{expected_popup_url}|window|top-level|visible|false"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPopupSourceProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker popup source server should finish");
}
#[tokio::test]
async fn service_worker_clients_open_window_request_records_popup_activation() {
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
            "#,
        ),
        (
            "/app/opened.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>opened</title>",
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
          globalThis.__serviceWorkerOpenWindowProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            globalThis.__serviceWorkerOpenWindowProbe = registration.active.state;
          })().catch(error => {
            globalThis.__serviceWorkerOpenWindowProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker openWindow setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerOpenWindowProbe)",
        "activated",
    )
    .await;

    let current_target = vm
        .service_worker_internal_window_client_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )
        .expect("current top-level ServiceWorker client target");
    run_service_worker_clients_open_window_request_task_for_test(
        &mut vm,
        &loader,
        "current Page openWindow request",
        crate::types::ServiceWorkerClientsOpenWindowRequestCompletion {
            host: current_target,
            request_id: 88,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            url: url::Url::parse(&format!("{base_url}/app/opened.html")).unwrap(),
        },
    )
    .await;

    let popups = vm.take_pending_popup_activations();
    assert_eq!(popups.len(), 1);
    assert!(popups[0].popup_id().is_some());
    let popup_id = popups[0].popup_id().expect("openWindow popup id");
    assert_eq!(popups[0].url(), format!("{base_url}/app/opened.html"));
    assert_eq!(popups[0].target_name(), "_blank");
    assert!(matches!(
        popups[0].source(),
        crate::RendererPopupActivationSource::BrowserContext
    ));
    assert_eq!(
        popups[0].disposition(),
        crate::RendererPopupDisposition::Foreground
    );
    assert!(vm.has_pending_lightweight_popup_document_loads());

    drain_service_worker_test_until_popup_loads_settle(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "service worker openWindow",
    )
    .await;

    let clients = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 89,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll {
                options: crate::runtime::ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: crate::runtime::ServiceWorkerClientQueryType::Window,
                },
            },
        });
    let opened = clients
        .clients
        .iter()
        .find(|client| client.url.as_str() == format!("{base_url}/app/opened.html"))
        .expect("opened popup should be registered as a service worker window client");
    assert!(opened.controlled);
    let opened_client_id = opened.id;
    let opened_exposed_client_id = opened.exposed_id.clone();
    let popup_document_owner = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_document_owner(popup_id)
        .expect("openWindow popup document owner");

    run_service_worker_client_focus_request_task_for_test(
        &mut vm,
        &loader,
        "current popup focus request",
        crate::types::ServiceWorkerClientFocusRequestCompletion {
            target: service_worker_window_client_target_for_test(
                opened_client_id,
                crate::native_bridge::WindowDocumentOwner::LightweightPopup(popup_document_owner),
            ),
            request_id: 90,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        },
    )
    .await;
    browser_context_runtime.drain_service_worker_service_lane();
    let focused_clients = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 91,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::Get {
                exposed_client_id: opened_exposed_client_id,
            },
        });
    assert_eq!(focused_clients.clients.len(), 1);
    assert!(focused_clients.clients[0].focused);

    server
        .await
        .expect("service worker openWindow script server should finish");
}
#[tokio::test]
async fn service_worker_popup_client_survives_javascript_reopen() {
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
            "#,
        ),
        (
            "/app/popup.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>popup</title>",
        ),
        (
            "/app/replaced.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>replacement</title>",
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
          globalThis.__serviceWorkerPopupReopenProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            globalThis.__serviceWorkerPopupReopenProbe = registration.active.state;
          })().catch(error => {
            globalThis.__serviceWorkerPopupReopenProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker popup reopen setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPopupReopenProbe)",
        "activated",
    )
    .await;

    let popup_url = format!("{base_url}/app/popup.html");
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");
    let opened = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__serviceWorkerReopenPopup = open({popup_url_literal}, "sw-popup");
  return String(globalThis.__serviceWorkerReopenPopup !== null);
}})()
"#
        ))
        .expect("service worker popup open should evaluate");
    assert_eq!(opened, "true");
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("service worker popup activation id");

    drain_service_worker_test_until_popup_loads_settle(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "service worker popup",
    )
    .await;

    let clients_before = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 96,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll {
                options: crate::runtime::ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: crate::runtime::ServiceWorkerClientQueryType::Window,
                },
            },
        });
    let popup_client_id = clients_before
        .clients
        .iter()
        .find(|client| client.url.as_str() == popup_url)
        .expect("popup should be registered as a service worker window client")
        .id;
    let initial_document_owner = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_document_owner(popup_id)
        .expect("initial service worker popup document owner");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial service worker popup LocalWindow owner");

    let reopened = vm
        .eval(
            r#"
(() => {
  globalThis.__serviceWorkerPopupJavascriptRan = false;
  const reopened = open(
    "javascript:window.opener.__serviceWorkerPopupJavascriptRan = true",
    "sw-popup"
  );
  return [
    reopened === globalThis.__serviceWorkerReopenPopup,
    globalThis.__serviceWorkerReopenPopup.location.href
  ].join("|");
})()
"#,
        )
        .expect("service worker popup javascript reopen should evaluate");
    assert_eq!(reopened, format!("true|{popup_url}"));

    for _ in 0..4 {
        if vm
            .eval("String(globalThis.__serviceWorkerPopupJavascriptRan)")
            .expect("popup javascript reopen flag should evaluate")
            == "true"
        {
            break;
        }
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }
    assert_eq!(
        vm.eval("String(globalThis.__serviceWorkerPopupJavascriptRan)")
            .expect("popup javascript reopen flag should evaluate"),
        "true"
    );

    let clients_after = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 97,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll {
                options: crate::runtime::ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: crate::runtime::ServiceWorkerClientQueryType::Window,
                },
            },
        });
    let popup_after = clients_after
        .clients
        .iter()
        .find(|client| client.id == popup_client_id)
        .expect("javascript: reopen should keep the existing popup service worker client");
    assert_eq!(popup_after.url.as_str(), popup_url);
    assert!(popup_after.controlled);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_lightweight_popup_document_owner(popup_id),
        Some(initial_document_owner),
        "javascript: execution without a string replacement must not rotate popup document identity"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_lightweight_popup_local_window_id(popup_id),
        Some(initial_local_window_id),
        "javascript: execution without replacement must preserve popup LocalWindow identity"
    );

    let replacement_url = format!("{base_url}/app/replaced.html");
    let replacement_url_literal =
        serde_json::to_string(&replacement_url).expect("serialize popup replacement url");
    let replacement_started = vm
        .eval(&format!(
            r#"
(() => {{
  const reopened = open({replacement_url_literal}, "sw-popup");
  return String(reopened === globalThis.__serviceWorkerReopenPopup);
}})()
"#
        ))
        .expect("service worker popup network replacement should evaluate");
    assert_eq!(replacement_started, "true");
    drain_service_worker_test_until_popup_loads_settle(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "service worker popup replacement",
    )
    .await;

    let replacement_document_owner = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_document_owner(popup_id)
        .expect("replacement service worker popup document owner");
    assert_ne!(replacement_document_owner, initial_document_owner);
    let replacement_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("replacement service worker popup LocalWindow owner");
    assert_ne!(replacement_local_window_id, initial_local_window_id);
    run_service_worker_client_focus_request_task_for_test(
        &mut vm,
        &loader,
        "stale popup focus request",
        crate::types::ServiceWorkerClientFocusRequestCompletion {
            target: service_worker_window_client_target_for_test(
                popup_client_id,
                crate::native_bridge::WindowDocumentOwner::LightweightPopup(initial_document_owner),
            ),
            request_id: 98,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        },
    )
    .await;
    browser_context_runtime.drain_service_worker_service_lane();
    let stale_focus = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 99,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::Get {
                exposed_client_id: popup_after.exposed_id.clone(),
            },
        });
    assert_eq!(stale_focus.clients.len(), 1);
    assert!(!stale_focus.clients[0].focused);
    assert_eq!(stale_focus.clients[0].id, popup_client_id);
    assert_eq!(stale_focus.clients[0].url.as_str(), replacement_url);

    run_service_worker_client_focus_request_task_for_test(
        &mut vm,
        &loader,
        "current replacement popup focus request",
        crate::types::ServiceWorkerClientFocusRequestCompletion {
            target: service_worker_window_client_target_for_test(
                popup_client_id,
                crate::native_bridge::WindowDocumentOwner::LightweightPopup(
                    replacement_document_owner,
                ),
            ),
            request_id: 100,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        },
    )
    .await;
    browser_context_runtime.drain_service_worker_service_lane();
    let current_focus = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 101,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::Get {
                exposed_client_id: popup_after.exposed_id.clone(),
            },
        });
    assert_eq!(current_focus.clients.len(), 1);
    assert!(current_focus.clients[0].focused);

    server
        .await
        .expect("service worker popup reopen server should finish");
}
#[tokio::test]
async fn service_worker_clients_open_window_about_blank_request_creates_no_popup() {
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
          globalThis.__serviceWorkerOpenWindowAboutBlankProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            globalThis.__serviceWorkerOpenWindowAboutBlankProbe = registration.active.state;
          })().catch(error => {
            globalThis.__serviceWorkerOpenWindowAboutBlankProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker about:blank openWindow setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerOpenWindowAboutBlankProbe)",
        "activated",
    )
    .await;

    let current_target = vm
        .service_worker_internal_window_client_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )
        .expect("current top-level ServiceWorker client target");
    run_service_worker_clients_open_window_request_task_for_test(
        &mut vm,
        &loader,
        "about:blank openWindow request",
        crate::types::ServiceWorkerClientsOpenWindowRequestCompletion {
            host: current_target,
            request_id: 92,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            url: url::Url::parse("about:blank").unwrap(),
        },
    )
    .await;

    let popups = vm.take_pending_popup_activations();
    assert!(popups.is_empty());
    assert!(!vm.has_pending_lightweight_popup_document_loads());
    browser_context_runtime.drain_service_worker_service_lane();

    server
        .await
        .expect("service worker about:blank openWindow script server should finish");
}
#[tokio::test]
async fn service_worker_clients_open_window_cross_origin_result_stays_null() {
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
            "#,
    )])
    .await;
    let (popup_base_url, popup_server) = spawn_service_worker_response_server(vec![(
        "/app/opened.html",
        "text/html; charset=utf-8",
        "<!doctype html><title>cross origin opened</title>",
    )])
    .await;
    let popup_url = format!("{popup_base_url}/app/opened.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerOpenWindowCrossOriginProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            globalThis.__serviceWorkerOpenWindowCrossOriginProbe = registration.active.state;
          })().catch(error => {
            globalThis.__serviceWorkerOpenWindowCrossOriginProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker cross-origin openWindow setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerOpenWindowCrossOriginProbe)",
        "activated",
    )
    .await;

    let current_target = vm
        .service_worker_internal_window_client_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )
        .expect("current top-level ServiceWorker client target");
    run_service_worker_clients_open_window_request_task_for_test(
        &mut vm,
        &loader,
        "cross-origin openWindow request",
        crate::types::ServiceWorkerClientsOpenWindowRequestCompletion {
            host: current_target,
            request_id: 94,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            url: url::Url::parse(&popup_url).unwrap(),
        },
    )
    .await;

    let popups = vm.take_pending_popup_activations();
    assert_eq!(popups.len(), 1);
    assert!(popups[0].popup_id().is_some());
    assert_eq!(popups[0].url(), popup_url);
    assert_eq!(popups[0].target_name(), "_blank");
    assert!(vm.has_pending_lightweight_popup_document_loads());

    drain_service_worker_test_until_popup_loads_settle(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "cross-origin service worker openWindow",
    )
    .await;

    let clients = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 95,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll {
                options: crate::runtime::ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: crate::runtime::ServiceWorkerClientQueryType::Window,
                },
            },
        });
    assert!(
        clients
            .clients
            .iter()
            .all(|client| client.url.as_str() != popup_url)
    );

    server
        .await
        .expect("service worker cross-origin openWindow script server should finish");
    popup_server
        .await
        .expect("service worker cross-origin popup server should finish");
}
#[tokio::test]
async fn navigator_service_worker_notification_action_navigate_records_popup_activation() {
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
            self.addEventListener("notificationclick", event => {
              event.waitUntil((async () => {
                const windows = await clients.matchAll({ includeUncontrolled: true });
                if (windows[0]) {
                  windows[0].postMessage("clicked:" + event.action);
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
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("notifications".to_owned()),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerNotificationActionNavigateProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerNotificationActionNavigateProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            await registration.showNotification("navigate", {
              actions: [
                { action: "reply", title: "Reply", navigate: "about:blank" }
              ],
              data: { answer: 1 }
            });
            globalThis.__serviceWorkerNotificationActionNavigateProbe = "shown";
          })().catch(error => {
            globalThis.__serviceWorkerNotificationActionNavigateProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker notification action navigate setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNotificationActionNavigateProbe)",
        "shown",
    )
    .await;

    assert!(
        browser_context_runtime.dispatch_service_worker_notification_click(
            &url::Url::parse(&format!("{base_url}/app/")).unwrap(),
            "navigate",
            "reply"
        )
    );

    drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;

    let popups = vm.take_pending_popup_activations();
    assert_eq!(popups.len(), 1);
    assert!(popups[0].popup_id().is_some());
    assert_eq!(popups[0].url(), "about:blank");
    assert_eq!(popups[0].target_name(), "_blank");
    assert!(!vm.has_pending_lightweight_popup_document_loads());
    assert_eq!(
        vm.eval("String(globalThis.__serviceWorkerNotificationActionNavigateProbe)")
            .expect("notification action navigate probe should be readable"),
        "shown"
    );

    server
        .await
        .expect("service worker notification action navigate script server should finish");
}

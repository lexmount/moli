use super::*;

#[tokio::test]
async fn navigator_service_worker_register_applies_script_path_restriction() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            Vec::new(),
            worker_body,
        ),
        (
            "/app/allowed-worker.js",
            "text/javascript; charset=utf-8",
            vec![("Service-Worker-Allowed", "/")],
            worker_body,
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
              globalThis.__serviceWorkerPathRestrictionProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                let rejected;
                try {
                  await sw.register("worker.js", { scope: "/" });
                  rejected = "resolved";
                } catch (error) {
                  const lookup = await sw.getRegistration("/");
                  const all = await sw.getRegistrations();
                  rejected = [
                    "rejected",
                    String(error && error.message).includes("max scope allowed"),
                    lookup === undefined,
                    all.length
                  ].join("|");
                }

                try {
                  const registration = await sw.register("allowed-worker.js", { scope: "/" });
                  const all = await sw.getRegistrations();
                  globalThis.__serviceWorkerPathRestrictionProbe =
                    rejected + "||allowed:" + [registration.scope, all.length].join("|");
                } catch (error) {
                  globalThis.__serviceWorkerPathRestrictionProbe =
                    rejected + "||allowed-rejected:" + (error && error.message);
                }
              })().catch((error) => {
                globalThis.__serviceWorkerPathRestrictionProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker path restriction probe should evaluate");

    let expected = format!("rejected|true|true|0||allowed:{base_url}/|1");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPathRestrictionProbe)",
        &expected,
    )
    .await;

    server
        .await
        .expect("service worker path restriction server should finish");
}
#[tokio::test]
async fn navigator_service_worker_register_identical_main_script_keeps_existing_version() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_body,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_body,
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
              globalThis.__serviceWorkerIdenticalUpdateProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const first = await sw.register("worker.js", { scope: "./" });
                await sw.ready;
                let updatefoundCount = 0;
                first.addEventListener("updatefound", () => {
                  updatefoundCount += 1;
                });
                const second = await sw.register("worker.js", { scope: "./" });
                const all = await sw.getRegistrations();
                globalThis.__serviceWorkerIdenticalUpdateProbe = [
                  updatefoundCount,
                  second.installing === null,
                  second.waiting === null,
                  second.active && second.active.scriptURL,
                  all.length
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerIdenticalUpdateProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker identical update probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!("0|true|true|{expected_worker_url}|1");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerIdenticalUpdateProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker identical update server should finish");
}
#[tokio::test]
async fn navigator_service_worker_register_changed_main_script_fires_updatefound() {
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
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
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
              globalThis.__serviceWorkerChangedUpdateProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const registration = await sw.register("worker.js", { scope: "./" });
                await sw.ready;
                const events = [];
                registration.addEventListener("updatefound", () => {
                  const installing = registration.installing;
                  events.push([
                    "updatefound",
                    installing && installing.scriptURL,
                    installing && installing.state
                  ].join(":"));
                  installing.addEventListener("statechange", () => {
                    events.push("statechange:" + installing.state);
                  });
                });
                const second = await sw.register("worker.js", { scope: "./" });
                let lookup = await sw.getRegistration();
                for (let i = 0; i < 20 && !events.includes("statechange:installed"); i++) {
                  await new Promise(resolve => setTimeout(resolve, 0));
                  lookup = await sw.getRegistration();
                }
                globalThis.__serviceWorkerChangedUpdateProbe = [
                  events.join(","),
                  second.installing === null,
                  second.waiting && second.waiting.scriptURL,
                  second.active && second.active.scriptURL,
                  lookup.waiting && lookup.waiting.scriptURL
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerChangedUpdateProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker changed update probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "updatefound:{expected_worker_url}:installing,statechange:installed|true|{expected_worker_url}|{expected_worker_url}|{expected_worker_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerChangedUpdateProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker changed update server should finish");
}
#[tokio::test]
async fn navigator_service_worker_registration_lifecycle_attributes_reflect_active_runtime() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/lifecycle-worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerLifecycleProbe = "started";
              (async () => {
                const sw = navigator.serviceWorker;
                await sw.register("lifecycle-worker.js", {
                  scope: "./"
                });
                const registration = await sw.ready;
                const newest = registration.active;
                const log = [
                  [
                    "active",
                    newest instanceof ServiceWorker,
                    newest && newest.state,
                    registration.installing === null,
                    registration.waiting === null,
                    registration.active === newest,
                    typeof registration.addEventListener,
                    typeof newest.addEventListener
                  ].join(":")
                ];
                const active = registration.active;
                await Promise.all([
                  new Promise((resolve) => {
                    active.addEventListener("statechange", () => {
                      log.push([
                        "redundant",
                        active.state,
                        registration.installing === null,
                        registration.waiting === null,
                        registration.active === null
                      ].join(":"));
                      if (active.state === "redundant") {
                        resolve();
                      }
                    }, { once: true });
                  }),
                  registration.unregister()
                ]);
                log.push([
                  "after-unregister",
                  registration.installing === null,
                  registration.waiting === null,
                  registration.active === null
                ].join(":"));
                globalThis.__serviceWorkerLifecycleProbe = log.join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerLifecycleProbe = "error:" + String(error);
              });
            })()
            "#,
    )
    .expect("service worker lifecycle probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerLifecycleProbe.includes('after-unregister'))",
        "true",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__serviceWorkerLifecycleProbe)")
        .expect("service worker lifecycle promises should settle");

    assert_eq!(
        result,
        r#""active:true:activated:true:true:true:function:function|redundant:redundant:true:true:true|after-unregister:true:true:true""#
    );
    server
        .await
        .expect("service worker lifecycle script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_update_via_cache_option_reflects_registration() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/app/worker-all.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerUpdateViaCacheProbe = "pending";
              (async () => {
                const first = await navigator.serviceWorker.register("worker.js", {
                  scope: "./",
                  updateViaCache: "none"
                });
                const second = await navigator.serviceWorker.register("worker-all.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
                let invalid = "not-rejected";
                try {
                  await navigator.serviceWorker.register("worker.js", {
                    scope: "./",
                    updateViaCache: "invalid"
                  });
                } catch (error) {
                  invalid = error && error.name;
                }
                globalThis.__serviceWorkerUpdateViaCacheProbe = [
                  first.updateViaCache,
                  second.updateViaCache,
                  invalid
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateViaCacheProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker updateViaCache probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateViaCacheProbe)",
        "all|all|TypeError",
    )
    .await;
    server
        .await
        .expect("service worker updateViaCache script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_update_via_cache_all_uses_fresh_main_script_cache() {
    let cache_dir = service_worker_http_cache_test_root("update-via-cache-all");
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        vec![("Cache-Control", "max-age=60")],
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        "#,
    )])
    .await;
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let browser_context_runtime = crate::runtime::RendererBrowserContextRuntime::new();
    let resource_runtime = browser_context_runtime
        .replace_browser_resource_runtime(crate::network::BrowserResourceRuntimeOwner::new(
            &fetch_config,
            moli_cookie_jar::new_shared_browser_cookie_store(),
        ))
        .expect("cache-enabled browser resource runtime");
    let loader = ResourceRequestClient::from_browser_resource_runtime(resource_runtime);
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
        url::Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
        &loader,
        browser_context_runtime.handle(),
    );
    // Execute register and update with the same Document transport, including
    // its browser-site context used to partition the HTTP cache.
    let loader = vm
        ._context_host
        .borrow()
        .current_main_document_resource_loader()
        .expect("current Document loader")
        .request_client()
        .clone();
    assert!(
        loader
            .browser_resource_runtime()
            .matches_fetch_config(&fetch_config)
    );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerUpdateViaCacheAllProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const registration = await sw.register("worker.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
                await sw.ready;
                let updatefoundCount = 0;
                registration.addEventListener("updatefound", () => {
                  updatefoundCount += 1;
                });
                const second = await sw.register("worker.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
                const updated = await registration.update();
                if (updated !== registration) throw new Error('update identity changed');
                const all = await sw.getRegistrations();
                globalThis.__serviceWorkerUpdateViaCacheAllProbe = [
                  updatefoundCount,
                  second.installing === null,
                  second.waiting === null,
                  second.active && second.active.scriptURL,
                  all.length
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateViaCacheAllProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker updateViaCache all cache probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!("0|true|true|{expected_worker_url}|1");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateViaCacheAllProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker updateViaCache all cache server should finish");
    let _ = std::fs::remove_dir_all(cache_dir);
}
#[tokio::test]
async fn navigator_service_worker_default_update_via_cache_revalidates_fresh_main_script_cache() {
    let cache_dir = service_worker_http_cache_test_root("update-via-cache-imports");
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "max-age=60")],
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "max-age=60")],
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            "#,
        ),
    ])
    .await;
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerUpdateViaCacheImportsProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const registration = await sw.register("worker.js", { scope: "./" });
                await sw.ready;
                const events = [];
                registration.addEventListener("updatefound", () => {
                  const installing = registration.installing;
                  events.push([
                    "updatefound",
                    installing && installing.scriptURL,
                    installing && installing.state
                  ].join(":"));
                  installing.addEventListener("statechange", () => {
                    events.push("statechange:" + installing.state);
                  });
                });
                const second = await sw.register("worker.js", { scope: "./" });
                let lookup = await sw.getRegistration();
                for (let i = 0; i < 20 && !events.includes("statechange:installed"); i++) {
                  await new Promise(resolve => setTimeout(resolve, 0));
                  lookup = await sw.getRegistration();
                }
                globalThis.__serviceWorkerUpdateViaCacheImportsProbe = [
                  events.join(","),
                  second.installing === null,
                  second.waiting && second.waiting.scriptURL,
                  second.active && second.active.scriptURL,
                  lookup.waiting && lookup.waiting.scriptURL
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateViaCacheImportsProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker updateViaCache imports cache probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "updatefound:{expected_worker_url}:installing,statechange:installed|true|{expected_worker_url}|{expected_worker_url}|{expected_worker_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateViaCacheImportsProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker updateViaCache imports cache server should finish");
    let _ = std::fs::remove_dir_all(cache_dir);
}
#[tokio::test]
async fn navigator_service_worker_register_resolves_when_activate_wait_until_rejects() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/activate-reject-worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(Promise.reject(new Error("activate failed")));
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
          globalThis.__serviceWorkerActivateRejectProbe = "pending";
          navigator.serviceWorker.register("activate-reject-worker.js", { scope: "./" })
            .then(
              async registration => {
                const registerState = [
                  Boolean(registration.installing),
                  Boolean(registration.waiting),
                  registration.waiting && registration.waiting.state,
                  Boolean(registration.active)
                ].join(":");
                const readyRegistration = await navigator.serviceWorker.ready;
                globalThis.__serviceWorkerActivateRejectProbe =
                  "resolved:" + registerState + "|" +
                  Boolean(readyRegistration.active) + ":" +
                  (readyRegistration.active && readyRegistration.active.state);
              },
              error => {
                globalThis.__serviceWorkerActivateRejectProbe =
                  "rejected:" + (error && error.message);
              }
            );
        })()
        "#,
    )
    .expect("service worker activate rejection probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerActivateRejectProbe)",
        "resolved:true:false::false|true:activated",
    )
    .await;
    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.redundant_version_count, 0);
    assert_eq!(diagnostics.in_flight_event_count, 0);
    assert_eq!(diagnostics.failed_start_count, 1);
    server
        .await
        .expect("service worker activate rejection script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_update_waits_without_skip_waiting() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker-v1.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker-v2.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(Promise.resolve());
            });
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
          globalThis.__serviceWorkerUpdateProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            await sw.register("worker-v1.js", { scope: "./" });
            await sw.ready;
            const update = await sw.register("worker-v2.js", { scope: "./" });
            let lookup = await sw.getRegistration();
            for (let i = 0; i < 20 && !lookup.waiting; i++) {
              await new Promise(resolve => setTimeout(resolve, 0));
              lookup = await sw.getRegistration();
            }
            globalThis.__serviceWorkerUpdateProbe = [
              "controller",
              sw.controller && sw.controller.scriptURL,
              "returned",
              update.installing && update.installing.scriptURL,
              update.waiting && update.waiting.scriptURL,
              update.active && update.active.scriptURL,
              "lookup",
              lookup.installing === null,
              lookup.waiting && lookup.waiting.scriptURL,
              lookup.active && lookup.active.scriptURL
            ].join("|");
          })().catch(error => {
            globalThis.__serviceWorkerUpdateProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker update waiting probe should evaluate");

    let expected_v1_url = format!("{base_url}/app/worker-v1.js");
    let expected_v2_url = format!("{base_url}/app/worker-v2.js");
    let expected = format!(
        "controller|{expected_v1_url}|returned||{expected_v2_url}|{expected_v1_url}|lookup|true|{expected_v2_url}|{expected_v1_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateProbe)",
        &expected,
    )
    .await;
    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.in_flight_event_count, 0);
    server
        .await
        .expect("service worker update waiting script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_updatefound_and_statechange_are_runtime_driven() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker-v1.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(self.clients.claim());
            });
            "#,
        ),
        (
            "/app/worker-v2.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(self.clients.claim());
            });
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
          globalThis.__serviceWorkerRuntimeEventsProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker-v1.js", { scope: "./" });
            await sw.ready;
            const events = [];
            const expectedV2Url = new URL("worker-v2.js", location.href).href;
            const publishProbe = () => {
              globalThis.__serviceWorkerRuntimeEventsProbe = [
                events.join(","),
                registration.installing === null,
                registration.waiting === null,
                registration.active && registration.active.scriptURL
              ].join("|");
            };
            const controllerChanged = new Promise((resolve, reject) => {
              sw.addEventListener("controllerchange", () => {
                events.push("controllerchange");
                publishProbe();
                const controller = sw.controller;
                if (!controller || controller.scriptURL !== expectedV2Url) {
                  reject(new Error("replacement controllerchange had the wrong controller"));
                  return;
                }
                resolve();
              }, { once: true });
            });
            const activated = new Promise((resolve, reject) => {
              registration.addEventListener("updatefound", () => {
                const installing = registration.installing;
                events.push([
                  "updatefound",
                  installing && installing.scriptURL,
                  installing && installing.state
                ].join(":"));
                publishProbe();
                if (!installing || installing.scriptURL !== expectedV2Url) {
                  reject(new Error("updatefound had the wrong installing worker"));
                  return;
                }
                installing.addEventListener("statechange", () => {
                  events.push("statechange:" + installing.state);
                  publishProbe();
                  if (installing.state === "activated") {
                    resolve();
                  } else if (installing.state === "redundant") {
                    reject(new Error("replacement worker became redundant"));
                  }
                });
              }, { once: true });
            });
            await sw.register("worker-v2.js", { scope: "./" });
            await Promise.all([activated, controllerChanged]);
            publishProbe();
          })().catch(error => {
            globalThis.__serviceWorkerRuntimeEventsProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker runtime event probe should evaluate");

    let expected_v2_url = format!("{base_url}/app/worker-v2.js");
    let expected = format!(
        "updatefound:{expected_v2_url}:installing,statechange:installed,statechange:activating,controllerchange,statechange:activated|true|true|{expected_v2_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRuntimeEventsProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker runtime event script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_sync_register_dispatches_sync_event() {
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
              if (event.data === "register-worker-sync") {
                event.waitUntil(self.registration.sync.register("worker-sync"));
              }
            });
            self.addEventListener("sync", event => {
              event.waitUntil((async () => {
                const tags = await self.registration.sync.getTags();
                const windows = await clients.matchAll({ includeUncontrolled: true });
                if (windows[0]) {
                  windows[0].postMessage(JSON.stringify({
                    type: event.type,
                    tag: event.tag,
                    lastChance: event.lastChance,
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
          globalThis.__serviceWorkerSyncProbe = "pending";
          globalThis.__serviceWorkerSyncRegisterProbe = "pending";
          globalThis.__serviceWorkerSyncMessages = [];
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerSyncMessages.push(event.data);
              globalThis.__serviceWorkerSyncProbe =
                JSON.stringify(globalThis.__serviceWorkerSyncMessages);
              if (globalThis.__serviceWorkerSyncMessages.length === 1) {
                globalThis.__serviceWorkerSyncRegistration.active.postMessage(
                  "register-worker-sync"
                );
              }
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            globalThis.__serviceWorkerSyncRegistration = registration;
            const tagsBefore = await registration.sync.getTags();
            const registerValue = await registration.sync.register("sync-tag");
            globalThis.__serviceWorkerSyncRegisterProbe = JSON.stringify({
              syncType: typeof registration.sync,
              registerType: typeof registration.sync.register,
              getTagsType: typeof registration.sync.getTags,
              tagsBeforeLength: tagsBefore.length,
              registerValue: String(registerValue)
            });
          })().catch(error => {
            globalThis.__serviceWorkerSyncProbe = "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker sync setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncRegisterProbe)",
        r#"{"syncType":"object","registerType":"function","getTagsType":"function","tagsBeforeLength":0,"registerValue":"undefined"}"#,
    )
    .await;
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncProbe)",
        r#"["{\"type\":\"sync\",\"tag\":\"sync-tag\",\"lastChance\":false,\"tags\":\"sync-tag\"}","{\"type\":\"sync\",\"tag\":\"worker-sync\",\"lastChance\":false,\"tags\":\"worker-sync\"}"]"#,
    )
    .await;

    server
        .await
        .expect("service worker sync script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_sync_register_respects_background_sync_permission() {
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
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("background-sync".to_owned()),
        setting: "denied".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerSyncPermissionProbe = "pending";
          (async () => {
            const permission = await navigator.permissions.query({
              name: "background-sync"
            });
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            let registerError = null;
            try {
              await registration.sync.register("denied-sync");
            } catch (error) {
              registerError = {
                name: error && error.name,
                message: error && error.message,
                isDomException: error instanceof DOMException
              };
            }
            const tags = await registration.sync.getTags();
            globalThis.__serviceWorkerSyncPermissionProbe = JSON.stringify({
              permission: permission.state,
              registerError,
              tagsLength: tags.length
            });
          })().catch(error => {
            globalThis.__serviceWorkerSyncPermissionProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker sync permission probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncPermissionProbe)",
        r#"{"permission":"denied","registerError":{"name":"NotAllowedError","message":"Background Sync permission has not been granted.","isDomException":true},"tagsLength":0}"#,
    )
    .await;

    server
        .await
        .expect("service worker sync permission script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_periodic_sync_register_respects_permission() {
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
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("periodic-background-sync".to_owned()),
        setting: "denied".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerPeriodicSyncPermissionProbe = "pending";
          (async () => {
            const permission = await navigator.permissions.query({
              name: "periodic-background-sync"
            });
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            let registerError = null;
            try {
              await registration.periodicSync.register("denied-periodic", {
                minInterval: 60000
              });
            } catch (error) {
              registerError = {
                name: error && error.name,
                message: error && error.message,
                isDomException: error instanceof DOMException
              };
            }
            const tags = await registration.periodicSync.getTags();
            globalThis.__serviceWorkerPeriodicSyncPermissionProbe = JSON.stringify({
              permission: permission.state,
              registerError,
              tagsLength: tags.length
            });
          })().catch(error => {
            globalThis.__serviceWorkerPeriodicSyncPermissionProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker periodic sync permission probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPeriodicSyncPermissionProbe)",
        r#"{"permission":"denied","registerError":{"name":"NotAllowedError","message":"Periodic Background Sync permission has not been granted.","isDomException":true},"tagsLength":0}"#,
    )
    .await;

    server
        .await
        .expect("service worker periodic sync permission script server should finish");
}

#[tokio::test]
async fn navigator_service_worker_register_validates_urls_and_refreshes_cached_workers() {
    let worker_body = "self.addEventListener('install', () => {});";
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "no-store")],
            worker_body,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "no-store")],
            worker_body,
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
              globalThis.__serviceWorkerRegistrationUrlProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const rejection = async (script, options) => {
                  try {
                    await sw.register(script, options);
                    return "resolved";
                  } catch (error) {
                    return [
                      error && error.name,
                      error instanceof TypeError,
                      error instanceof DOMException
                    ].join("|");
                  }
                };
                const encodedSlash = await rejection("worker%2f.js", { scope: "./scope/" });
                const dataScript = await rejection("data:text/javascript,", { scope: "./scope/" });
                const dataScope = await rejection("worker.js", { scope: "data:text/html," });

                const newest = registration =>
                  registration.installing || registration.waiting || registration.active;
                const first = await sw.register("././worker.js", { scope: "./scope/" });
                const firstScriptURL = newest(first) && newest(first).scriptURL;
                const unregistered = await first.unregister();
                const cleared =
                  first.installing === null && first.waiting === null && first.active === null;
                const second = await sw.register("../app/worker.js", { scope: "./scope/" });
                const secondScriptURL = newest(second) && newest(second).scriptURL;

                globalThis.__serviceWorkerRegistrationUrlProbe = JSON.stringify({
                  encodedSlash,
                  dataScript,
                  dataScope,
                  firstScriptURL,
                  unregistered,
                  cleared,
                  secondScriptURL
                });
              })().catch((error) => {
                globalThis.__serviceWorkerRegistrationUrlProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker registration URL probe should evaluate");

    let expected = format!(
        r#"{{"encodedSlash":"TypeError|true|false","dataScript":"TypeError|true|false","dataScope":"TypeError|true|false","firstScriptURL":"{base_url}/app/worker.js","unregistered":true,"cleared":true,"secondScriptURL":"{base_url}/app/worker.js"}}"#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRegistrationUrlProbe)",
        &expected,
    )
    .await;

    server
        .await
        .expect("service worker registration URL server should finish");
}

#[tokio::test]
async fn navigator_service_worker_update_preserves_identity_and_rejects_stale_receivers() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", "// first"),
        ("/app/workers/sw.js", "text/javascript", "// second"),
        ("/app/workers/sw.js", "text/javascript", "// second"),
        (
            "/app/workers/sw.js",
            "text/javascript",
            "// new registration",
        ),
        ("/app/workers/sw.js", "text/javascript", "// new update"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__updateIdentity = 'pending';
      (async () => {
        const activated = async r => {
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
        };
        const errorName = async p => { try { await p; return 'success'; } catch(e) { return e.name; } };
        const register = () => navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        const r = await register();
        let found = 0;
        r.addEventListener('updatefound', () => found++);
        await activated(r);
        const initial = found;
        const receivers = [];
        for (const fake of [{}, Object.create(r), new Proxy(r, {}), null]) {
          const p = Reflect.apply(r.update, fake, []);
          receivers.push(p instanceof Promise && await errorName(p) === 'TypeError');
        }
        const before = r.active;
        const order = [];
        r.addEventListener('updatefound', () => order.push('event'), {once:true});
        const updated = await r.update(); order.push('promise');
        await activated(r);
        const changed = before !== r.active;
        const current = r.active;
        const unchanged = await r.update({get unused(){throw new Error('argument read');}});
        const stable = unchanged === r && current === r.active && found === 2;
        await r.unregister();
        const removed = await errorName(r.update());
        const replacement = await register();
        await activated(replacement);
        const stale = await errorName(r.update());
        const again = await replacement.update();
        await activated(replacement);
        const lookup = await navigator.serviceWorker.getRegistration('workers/client');
        globalThis.__updateIdentity = [r.update.name, r.update.length, initial,
          receivers.every(Boolean), updated === r, changed, order.join(','), stable,
          removed, stale, replacement !== r, again === replacement, lookup === replacement].join('|');
        await replacement.unregister();
      })().catch(e => globalThis.__updateIdentity = 'error:' + e);
    "#).expect("update identity probe");
    drain_service_worker_test_until_eval_equals(&mut vm, &runtime, &loader,
        "String(globalThis.__updateIdentity)",
        "update|0|1|true|true|true|promise,event|true|InvalidStateError|InvalidStateError|true|true|true",
    ).await;
    server.await.expect("update identity server");
}

#[tokio::test]
async fn navigator_service_worker_update_reuses_failed_import_responses() {
    const MAIN: &str = "importScripts('a.js', 'z.js');";
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", MAIN),
        ("/app/workers/a.js", "text/javascript", "// a"),
        ("/app/workers/z.js", "text/javascript", "// z1"),
        ("/app/workers/sw.js", "text/javascript", MAIN),
        ("/app/workers/a.js", "text/html", "missing import"),
        ("/app/workers/z.js", "text/javascript", "// z2"),
        (
            "/app/workers/sw.js",
            "text/javascript",
            "importScripts('z.js');",
        ),
        ("/app/workers/z.js", "text/javascript", "// z3"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__updateImports = 'pending';
      (async () => {
        const activated = async r => {
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
        };
        const r = await navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        await activated(r);
        const before = r.active;
        let found = 0;
        r.addEventListener('updatefound', () => found++);
        let error = 'success';
        try { await r.update(); } catch(e) { error = e.name; }
        const failed = error === 'TypeError' && found === 0 && r.active === before && r.installing === null;
        const updated = await r.update();
        await activated(r);
        globalThis.__updateImports = [failed, updated === r, r.active !== before, found].join('|');
        await r.unregister();
      })().catch(e => globalThis.__updateImports = 'error:' + e);
    "#).expect("update import response probe");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__updateImports)",
        "true|true|true|1",
    )
    .await;
    server
        .await
        .expect("update import server should not refetch failed responses");
}

#[tokio::test]
async fn navigator_service_worker_update_discards_prefetched_but_unused_imports_after_install() {
    const MAIN: &str = r#"
      importScripts('flag.js');
      if (!self.skipOld) importScripts('unused.js');
      onmessage = e => {
        try { importScripts('unused.js'); e.ports[0].postMessage('success'); }
        catch(error) { e.ports[0].postMessage(error.name); }
      };
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", MAIN),
        (
            "/app/workers/flag.js",
            "text/javascript",
            "self.skipOld = false;",
        ),
        ("/app/workers/unused.js", "text/javascript", "// unused"),
        ("/app/workers/sw.js", "text/javascript", MAIN),
        (
            "/app/workers/flag.js",
            "text/javascript",
            "self.skipOld = true;",
        ),
        ("/app/workers/unused.js", "text/javascript", "// unused"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__unusedUpdateImport = 'pending';
      (async () => {
        const activated = async r => {
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
        };
        const r = await navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        await activated(r);
        await r.update(); await activated(r);
        const value = await new Promise(resolve => {
          const c = new MessageChannel();
          c.port1.onmessage = e => { c.port1.close(); resolve(e.data); };
          r.active.postMessage('probe', [c.port2]);
        });
        globalThis.__unusedUpdateImport = value;
        await r.unregister();
      })().catch(e => globalThis.__unusedUpdateImport = 'error:' + e);
    "#).expect("unused update import probe");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__unusedUpdateImport)",
        "NetworkError",
    )
    .await;
    server
        .await
        .expect("prefetched imports should not be fetched again");
}

#[tokio::test]
async fn navigator_service_worker_update_in_worker_preserves_events_and_install_rejection() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", r#"
          const original = registration;
          const seen = [];
          let installError;
          original.addEventListener('updatefound', () => seen.push('before'));
          original.onupdatefound = function(e) {
            seen.push([this === original, e.target === original, e.currentTarget === original,
              e instanceof Event, e.isTrusted, e.bubbles, e.cancelable].join(':'));
          };
          original.addEventListener('updatefound', () => seen.push('after'));
          oninstall = e => e.waitUntil(original.update().then(
            () => installError = 'success', e => installError = e.name));
          onactivate = () => seen.push('activate');
          onmessage = e => {
            if (e.data === 'sample') e.ports[0].postMessage(seen.join(',') + '|' + installError);
            else e.waitUntil(original.update().then(r => e.ports[0].postMessage(r === original)));
          };
          Object.defineProperty(self, 'registration', {get(){throw new Error('public registration read');}});
        "#),
        ("/app/workers/sw.js", "text/javascript", "// updated"),
    ]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(
        r#"
      globalThis.__workerUpdate = 'pending';
      (async () => {
        const r = await navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        const w = r.installing;
        await new Promise(resolve => w.addEventListener('statechange', () => {
          if (w.state === 'activated') resolve();
        }));
        const message = data => new Promise(resolve => {
          const c = new MessageChannel();
          c.port1.onmessage = e => { c.port1.close(); resolve(e.data); };
          w.postMessage(data, [c.port2]);
        });
        const first = await message('sample');
        const same = await message('update');
        globalThis.__workerUpdate = first + '|' + same;
        await r.unregister();
      })().catch(e => globalThis.__workerUpdate = 'error:' + e);
    "#,
    )
    .expect("worker update probe");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__workerUpdate)",
        "before,true:true:true:true:true:false:false,after,activate|InvalidStateError|true",
    )
    .await;
    server.await.expect("worker update server");
}

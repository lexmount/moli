use super::*;

#[tokio::test]
async fn navigator_service_worker_update_check_failure_rejects_register_with_type_error() {
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
              event.waitUntil(Promise.resolve());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/plain",
            vec![("X-Content-Type-Options", "nosniff")],
            "not javascript",
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
              globalThis.__serviceWorkerUpdateFailureProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                try {
                  await navigator.serviceWorker.register("worker.js", { scope: "./" });
                  globalThis.__serviceWorkerUpdateFailureProbe = "resolved";
                } catch (error) {
                  globalThis.__serviceWorkerUpdateFailureProbe = JSON.stringify({
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    isDomException: error instanceof DOMException,
                    messageIncludesNosniff: String(error && error.message).includes("nosniff")
                  });
                }
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateFailureProbe =
                  "setup-error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker update failure probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateFailureProbe)",
        r#"{"name":"TypeError","isTypeError":true,"isDomException":false,"messageIncludesNosniff":true}"#,
    )
    .await;

    server
        .await
        .expect("service worker update failure server should finish");
}
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
                await __moliTestActivatedServiceWorker(sw);
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
                await __moliTestActivatedServiceWorker(sw);
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
                {
                  // ready may resolve while activate (and its claim) is still pending.
                  const active = (await sw.ready).active;
                  if (active.state !== "activated") {
                    await new Promise(resolve => active.addEventListener("statechange", () => {
                      if (active.state === "activated") resolve();
                    }));
                  }
                }
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
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
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
                {
                  const worker = registration.active;
                  if (worker.state !== "activated") {
                    await new Promise(resolve => worker.addEventListener("statechange", () => {
                      if (worker.state === "activated") resolve();
                    }));
                  }
                }
                let updatefoundCount = 0;
                registration.addEventListener("updatefound", () => {
                  updatefoundCount += 1;
                });
                const second = await sw.register("worker.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
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
                await __moliTestActivatedServiceWorker(sw);
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
                {
                  // ready may resolve while activate (and its claim) is still pending.
                  const active = (await navigator.serviceWorker.ready).active;
                  if (active.state !== "activated") {
                    await new Promise(resolve => active.addEventListener("statechange", () => {
                      if (active.state === "activated") resolve();
                    }));
                  }
                }
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
            await __moliTestActivatedServiceWorker(sw);
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
            const initiallyControlled = new Promise(resolve => {
              sw.addEventListener("controllerchange", resolve, { once: true });
            });
            const registration = await sw.register("worker-v1.js", { scope: "./" });
            await sw.ready;
            {
              // ready may resolve while activate (and its claim) is still pending.
              const active = (await sw.ready).active;
              if (active.state !== "activated") {
                await new Promise(resolve => active.addEventListener("statechange", () => {
                  if (active.state === "activated") resolve();
                }));
              }
            }
            // Native activation and the Window controllerchange task can settle
            // separately. Consume the initial event before observing replacement.
            await initiallyControlled;
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
async fn navigator_service_worker_module_bootstrap_claim_rejects_and_activation_claim_completes() {
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/claim-bootstrap.js",
        "text/javascript; charset=utf-8",
        Vec::new(),
        r#"
        const startupClaim = clients.claim().then(
          () => "resolved", error => error.name
        );
        self.addEventListener("activate", event => {
          event.waitUntil(startupClaim.then(() => clients.claim()));
        });
        self.addEventListener("message", event => {
          event.waitUntil(startupClaim.then(result => event.ports[0].postMessage(result)));
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__claimBootstrapProbe = "pending";
      (async () => {
        await navigator.serviceWorker.register("claim-bootstrap.js", { scope: "./", type: "module" });
        const registration = await navigator.serviceWorker.ready;
        const worker = registration.active;
        if (worker.state !== "activated") {
          await new Promise(resolve => worker.addEventListener("statechange", () => {
            if (worker.state === "activated") resolve();
          }));
        }
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        worker.postMessage("startup-claim", [channel.port2]);
        globalThis.__claimBootstrapProbe = (await result) + ":" + worker.state + ":" + String(navigator.serviceWorker.controller !== null);
        channel.port1.close();
      })().catch(error => {
        globalThis.__claimBootstrapProbe = "error:" + error.name + ":" + error.message;
      });
    "#).expect("module claim bootstrap probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__claimBootstrapProbe)",
        "InvalidStateError:activated:true",
    )
    .await;
    server
        .await
        .expect("module worker script should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_skip_waiting_waits_for_activate_completion() {
    for reject_activation in [false, true] {
        let (base_url, server) = spawn_service_worker_response_server(vec![
            ("/app/initial.js", "text/javascript; charset=utf-8", r#"
              self.addEventListener('activate', event => event.waitUntil(clients.claim()));
            "#),
            ("/app/waiting.js", "text/javascript; charset=utf-8", r#"
              let sawActivate = false;
              let activationFinished = false;
              let nestedSkipFinished = false;
              let skipSettled = false;
              let finishActivate;
              let reply;
              self.addEventListener('activate', event => {
                sawActivate = true;
                // An activating worker's nested call must not wait on itself.
                event.waitUntil(skipWaiting().then(() => { nestedSkipFinished = true; }));
                event.waitUntil(new Promise((resolve, reject) => {
                  finishActivate = shouldReject => {
                    activationFinished = true;
                    if (shouldReject) reject(new Error('activation gate rejected'));
                    else resolve();
                  };
                }));
                reply.postMessage({kind: 'activate-started'});
              });
              self.addEventListener('message', event => {
                if (event.data === 'skip') {
                  reply = event.ports[0];
                  event.waitUntil(skipWaiting().then(result => {
                    skipSettled = true;
                    reply.postMessage({kind: 'result', value: [
                      typeof result, activationFinished, nestedSkipFinished
                    ].join(':')});
                  }));
                } else if (event.data === 'status') {
                  event.ports[0].postMessage([sawActivate, skipSettled, activationFinished].join(':'));
                } else if (event.data.kind === 'release') {
                  finishActivate(event.data.reject);
                }
              });
            "#),
        ]).await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                &format!("{base_url}/app/page.html"),
                &loader,
            );
        vm.eval(&format!(
            "globalThis.rejectActivationGate = {reject_activation};"
        ))
        .unwrap();
        vm.eval(r#"
          globalThis.__skipWaitingDone = false;
          (async () => {
            const sw = navigator.serviceWorker;
            await sw.register('initial.js', {scope: './'});
            await __moliTestActivatedServiceWorker(sw);
            if (!sw.controller) await new Promise(resolve => sw.addEventListener('controllerchange', resolve, {once: true}));
            const registration = await sw.register('waiting.js', {scope: './'});
            const worker = registration.installing || registration.waiting;
            if (worker.state !== 'installed') await new Promise(resolve => {
              worker.addEventListener('statechange', () => { if (worker.state === 'installed') resolve(); });
            });
            const channel = new MessageChannel();
            let startActivation;
            let finishSkip;
            const started = new Promise(resolve => { startActivation = resolve; });
            const result = new Promise(resolve => { finishSkip = resolve; });
            channel.port1.onmessage = event => {
              if (event.data.kind === 'activate-started') startActivation();
              if (event.data.kind === 'result') finishSkip(event.data.value);
            };
            worker.postMessage('skip', [channel.port2]);
            await started;
            const probe = new MessageChannel();
            const status = new Promise(resolve => { probe.port1.onmessage = event => resolve(event.data); });
            worker.postMessage('status', [probe.port2]);
            const beforeRelease = await status;
            // Release activation independently of skipWaiting's completion.
            worker.postMessage({kind: 'release', reject: rejectActivationGate});
            globalThis.__skipWaitingResult = beforeRelease + '|' + (await result);
            channel.port1.close();
            probe.port1.close();
          })().catch(error => {
            globalThis.__skipWaitingResult = 'error:' + error.name + ':' + error.message;
          }).finally(() => { globalThis.__skipWaitingDone = true; });
        "#).unwrap();
        drain_service_worker_test_until_eval_equals(
            &mut vm,
            &runtime,
            &loader,
            "String(globalThis.__skipWaitingDone)",
            "true",
        )
        .await;
        assert_eq!(
            vm.eval("String(globalThis.__skipWaitingResult)").unwrap(),
            "true:false:false|undefined:true:true",
            "reject activation: {reject_activation}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn navigator_service_worker_module_bootstrap_skip_waiting_completes_install() {
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/skip-bootstrap.js",
        "text/javascript; charset=utf-8",
        Vec::new(),
        r#"
        const startupSkip = skipWaiting().then(value => String(value));
        self.addEventListener("install", event => event.waitUntil(startupSkip));
        self.addEventListener("activate", event => event.waitUntil(clients.claim()));
        self.addEventListener("message", event => {
          event.waitUntil(startupSkip.then(value => event.ports[0].postMessage(value)));
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__skipBootstrap = "pending";
      (async () => {
        const sw = navigator.serviceWorker;
        await sw.register("skip-bootstrap.js", { scope: "./", type: "module" });
        const registration = await __moliTestActivatedServiceWorker(sw);
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        registration.active.postMessage("startup", [channel.port2]);
        globalThis.__skipBootstrap = (await result) + ":" + registration.active.state + ":" + String(sw.controller !== null);
        channel.port1.close();
      })().catch(error => {
        globalThis.__skipBootstrap = "error:" + error.name + ":" + error.message;
      });
    "#).expect("module startup skipWaiting probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__skipBootstrap)",
        "undefined:activated:true",
    )
    .await;
    server
        .await
        .expect("module worker script should be fetched");
}

#[tokio::test]
async fn waiting_replacement_lifecycle_projects_registration_slots() {
    let active = r#"
      self.addEventListener('activate', event => event.waitUntil(clients.claim()));
    "#;
    let waiting = "self.addEventListener('install', event => event.waitUntil(Promise.resolve()));";
    let replacement = "// replacement\nself.addEventListener('install', event => event.waitUntil(Promise.resolve()));";
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/active.js", "text/javascript; charset=utf-8", active),
        ("/app/waiting.js", "text/javascript; charset=utf-8", waiting),
        (
            "/app/replacement.js",
            "text/javascript; charset=utf-8",
            replacement,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, context) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.replacementObservations = [];
      globalThis.replacementFinished = false;
      (async () => {
        const container = navigator.serviceWorker;
        const registration = await container.register('active.js', {scope: './'});
        await container.ready;
        const active = registration.active;
        if (active.state !== 'activated') await new Promise(resolve => {
          active.addEventListener('statechange', () => { if (active.state === 'activated') resolve(); });
        });
        if (!container.controller) await new Promise(resolve => {
          container.addEventListener('controllerchange', resolve, {once: true});
        });
        await container.register('waiting.js', {scope: './'});
        const previous = registration.installing || registration.waiting;
        if (!previous) throw new Error('the controlled page must keep a candidate worker');
        if (previous.state !== 'installed') await new Promise(resolve => {
          previous.addEventListener('statechange', () => { if (previous.state === 'installed') resolve(); });
        });
        previous.addEventListener('statechange', () => {
          if (previous.state !== 'redundant') return;
          const installing = registration.installing;
          replacementObservations.push([
            'redundant', registration.waiting === previous,
            installing && new URL(installing.scriptURL).pathname,
            installing && installing.state, registration.active === active
          ]);
          if (installing) installing.addEventListener('statechange', () => {
            if (installing.state === 'installed') replacementObservations.push([
              'installed', registration.installing === null,
              registration.waiting === installing, registration.active === active
            ]);
          });
        });
        await container.register('replacement.js', {scope: './'});
        replacementFinished = true;
      })().catch(error => { globalThis.replacementFailure = String(error); });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm, &context, &loader,
        "JSON.stringify({done: replacementFinished, events: replacementObservations, error: globalThis.replacementFailure || null})",
        r#"{"done":true,"events":[["redundant",true,"/app/replacement.js","installing",true],["installed",true,true,true]],"error":null}"#,
    ).await;
    server.await.unwrap();
}

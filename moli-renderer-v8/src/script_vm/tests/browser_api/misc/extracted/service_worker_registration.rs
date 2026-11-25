use super::*;

#[tokio::test]
async fn service_worker_get_registration_separates_api_base_url_from_native_origin() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/other/worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__registrationBaseDone = false;
        (async () => {
            const sw = navigator.serviceWorker;
            const app = await sw.register('worker.js');
            const other = await sw.register('/other/worker.js');
            const checks = [await sw.getRegistration() === app];
            const base = document.createElement('base');
            base.href = other.scope;
            document.head.appendChild(base);
            checks.push(await sw.getRegistration() === other);
            checks.push(await sw.getRegistration('page.html#fragment') === other);
            base.href = 'https://different-origin.test/';
            checks.push(await sw.getRegistration(app.scope + 'page.html#fragment') === app);
            const errorName = promise => promise.then(() => 'resolved', error => error.name);
            checks.push(await errorName(sw.getRegistration('page.html')));
            checks.push(await errorName(sw.getRegistration('http://[')));
            checks.push(await errorName(sw.getRegistration('data:text/html,opaque')));
            checks.push(await sw.getRegistration({toString() {
                base.href = other.scope;
                return 'page.html#converted';
            }}) === other);
            Object.defineProperty(document, 'baseURI', {get() { throw new Error('author baseURI'); }});
            Object.defineProperty(document, 'URL', {get() { throw new Error('author URL'); }});
            checks.push(await sw.getRegistration() === other);
            globalThis.__registrationBase = checks.join('|');
        })().catch(error => { __registrationBase = 'error:' + error.name + ':' + error.message; })
          .finally(() => { __registrationBaseDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__registrationBaseDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(__registrationBase)").unwrap(),
        "true|true|true|true|SecurityError|TypeError|SecurityError|true|true"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_get_registration_uses_inherited_blank_popup_and_srcdoc_settings() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/other/worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__registrationInheritedDone = false;
        globalThis.__registrationInheritedPhase = 'register';
        (async () => {
            const sw = navigator.serviceWorker;
            const app = await sw.register('worker.js');
            const other = await sw.register('/other/worker.js');
            const checks = [];
            const setBase = (realm, href) => {
                const doc = realm.document;
                const html = doc.documentElement || doc.appendChild(doc.createElement('html'));
                const head = doc.head || html.insertBefore(doc.createElement('head'), html.firstChild);
                const base = doc.createElement('base');
                base.href = href;
                head.appendChild(base);
                return base;
            };
            __registrationInheritedPhase = 'blank';
            const blank = document.createElement('iframe');
            document.body.appendChild(blank);
            const child = blank.contentWindow;
            const childSw = child.navigator.serviceWorker;
            checks.push((await childSw.getRegistration(app.scope + 'page.html#fragment')).scope === app.scope);
            checks.push((await childSw.getRegistration()).scope === app.scope);
            checks.push((await childSw.getRegistration('deep/page.html')).scope === app.scope);
            const childBase = setBase(child, other.scope);
            checks.push((await childSw.getRegistration()).scope === other.scope);
            checks.push((await sw.getRegistration.call(childSw, 'page.html')).scope === other.scope);
            checks.push((await childSw.getRegistration.call(sw, 'page.html')).scope === app.scope);
            childBase.href = 'https://different-origin.test/';
            checks.push((await childSw.getRegistration(app.scope + 'page.html')).scope === app.scope);
            checks.push(await childSw.getRegistration('page.html').then(() => 'resolved', error => error.name));
            __registrationInheritedPhase = 'popup';
            const popup = open('about:blank', 'get-registration-settings');
            const popupSw = popup.navigator.serviceWorker;
            checks.push((await popupSw.getRegistration()).scope === app.scope);
            const popupBase = setBase(popup, other.scope);
            checks.push((await popupSw.getRegistration('page.html#fragment')).scope === other.scope);
            popupBase.href = 'https://different-origin.test/';
            checks.push((await popupSw.getRegistration(app.scope + 'page.html')).scope === app.scope);
            checks.push(await popupSw.getRegistration('page.html').then(() => 'resolved', error => error.name));
            __registrationInheritedPhase = 'srcdoc';
            const srcdoc = document.createElement('iframe');
            const loaded = new Promise(resolve => { srcdoc.onload = resolve; });
            srcdoc.srcdoc = '<!doctype html><base href="' + other.scope + '"><title>lookup</title>';
            document.body.appendChild(srcdoc);
            await loaded;
            const srcdocSw = srcdoc.contentWindow.navigator.serviceWorker;
            checks.push((await srcdocSw.getRegistration('page.html#fragment')).scope === other.scope);
            checks.push((await srcdocSw.getRegistration()).scope === other.scope);
            checks.push((await srcdocSw.getRegistration(app.scope + 'page.html')).scope === app.scope);
            globalThis.__registrationInherited = checks.join('|');
        })().catch(error => { __registrationInherited = __registrationInheritedPhase + ':error:' + error.name + ':' + error.message; })
          .finally(() => { __registrationInheritedDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__registrationInheritedDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(__registrationInherited)").unwrap(),
        "true|true|true|true|true|true|true|SecurityError|true|true|true|SecurityError|true|true|true"
    );
    server.await.unwrap();
}

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
                const installed = new Promise((resolve, reject) => {
                  registration.addEventListener("updatefound", () => {
                    const installing = registration.installing;
                    events.push([
                      "updatefound",
                      installing && installing.scriptURL,
                      installing && installing.state
                    ].join(":"));
                    if (!installing) {
                      reject(new Error("updatefound had no installing worker"));
                      return;
                    }
                    installing.addEventListener("statechange", () => {
                      events.push("statechange:" + installing.state);
                      if (installing.state === "installed") {
                        resolve();
                      } else if (installing.state === "redundant") {
                        reject(new Error("replacement worker became redundant"));
                      }
                    });
                  }, { once: true });
                });
                const second = await sw.register("worker.js", { scope: "./" });
                await installed;
                const lookup = await sw.getRegistration();
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
                await sw.ready;
                const events = [];
                const installed = new Promise((resolve, reject) => {
                  registration.addEventListener("updatefound", () => {
                    const installing = registration.installing;
                    events.push([
                      "updatefound",
                      installing && installing.scriptURL,
                      installing && installing.state
                    ].join(":"));
                    if (!installing) {
                      reject(new Error("updatefound had no installing worker"));
                      return;
                    }
                    installing.addEventListener("statechange", () => {
                      events.push("statechange:" + installing.state);
                      if (installing.state === "installed") {
                        resolve();
                      } else if (installing.state === "redundant") {
                        reject(new Error("replacement worker became redundant"));
                      }
                    });
                  }, { once: true });
                });
                const second = await sw.register("worker.js", { scope: "./" });
                await installed;
                const lookup = await sw.getRegistration();
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
async fn service_worker_registration_update_resolves_for_identical_script_and_validates_receiver() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
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
          globalThis.__swUpdateIdenticalDone = false;
          globalThis.__swUpdateIdenticalProbe = "pending";
          (async () => {
            const reg = await navigator.serviceWorker.register("worker.js", { scope: "./" });
            let updateFoundCount = 0;
            reg.addEventListener("updatefound", () => {
              updateFoundCount += 1;
            });
            const updateFn = reg.update;
            const updated = await reg.update();
            let invalidReceiver = "resolved";
            let syncThrew = false;
            try {
              const p = updateFn.call({});
              try {
                await p;
              } catch (error) {
                invalidReceiver = error instanceof TypeError ? "TypeError" : String(error);
              }
            } catch (_error) {
              syncThrew = true;
            }
            globalThis.__swUpdateIdenticalProbe = JSON.stringify({
              type: typeof updateFn,
              name: updateFn && updateFn.name,
              length: updateFn && updateFn.length,
              sameRegistration: updated === reg,
              updateFoundCount,
              invalidReceiver,
              syncThrew
            });
          })().catch(error => {
            globalThis.__swUpdateIdenticalProbe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swUpdateIdenticalDone = true;
          });
        })()
        "#,
    )
    .expect("service worker update identical probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swUpdateIdenticalDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swUpdateIdenticalProbe)")
            .unwrap(),
        r#"{"type":"function","name":"update","length":0,"sameRegistration":true,"updateFoundCount":0,"invalidReceiver":"TypeError","syncThrew":false}"#
    );

    server
        .await
        .expect("service worker update identical server should receive both requests");
}

#[tokio::test]
async fn service_worker_registration_update_installs_changed_script_and_fires_updatefound() {
    let worker_v1 = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("message", event => {
          event.ports[0].postMessage("v1");
        });
    "#;
    let worker_v2 = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("message", event => {
          event.ports[0].postMessage("v2");
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v1,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v2,
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
          globalThis.__swUpdateChangedDone = false;
          globalThis.__swUpdateChangedProbe = "pending";
          const askWorker = worker => new Promise(resolve => {
            const channel = new MessageChannel();
            channel.port1.onmessage = event => resolve(event.data);
            worker.postMessage("ping", [channel.port2]);
          });
          (async () => {
            const reg = await navigator.serviceWorker.register("worker.js", { scope: "./" });
            await navigator.serviceWorker.ready;
            const beforeVersion = await askWorker(reg.active);
            let updateFoundCount = 0;
            const controllerChanged = new Promise(resolve => {
              navigator.serviceWorker.addEventListener("controllerchange", () => resolve(true), { once: true });
            });
            reg.addEventListener("updatefound", () => {
              updateFoundCount += 1;
            });
            const updated = await reg.update();
            await controllerChanged;
            const afterVersion = await askWorker(reg.active);
            globalThis.__swUpdateChangedProbe = JSON.stringify({
              beforeVersion,
              sameRegistration: updated === reg,
              updateFoundCount,
              afterVersion
            });
          })().catch(error => {
            globalThis.__swUpdateChangedProbe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swUpdateChangedDone = true;
          });
        })()
        "#,
    )
    .expect("service worker update changed probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swUpdateChangedDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swUpdateChangedProbe)")
            .unwrap(),
        r#"{"beforeVersion":"v1","sameRegistration":true,"updateFoundCount":1,"afterVersion":"v2"}"#
    );

    server
        .await
        .expect("service worker update changed server should finish");
}

#[tokio::test]
async fn service_worker_registration_update_rejects_on_script_error_and_after_unregister() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
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
          globalThis.__swUpdateErrorDone = false;
          globalThis.__swUpdateErrorProbe = "pending";
          (async () => {
            const reg = await navigator.serviceWorker.register("worker.js", { scope: "./" });
            await navigator.serviceWorker.ready;
            if (typeof reg.update !== "function") {
              throw new Error("missing reg.update");
            }
            let scriptError = "resolved";
            try {
              await reg.update();
            } catch (error) {
              scriptError = (
                error instanceof TypeError
                && !(error instanceof DOMException)
                && !String(error.message).includes("is not a function")
              )
                ? "TypeError"
                : String(error && error.name) + ":" + String(error && error.message);
            }
            const activePreserved = reg.active !== null;
            const unregistered = await reg.unregister();
            let afterUnregister = "resolved";
            try {
              await reg.update();
            } catch (error) {
              afterUnregister = (
                error instanceof TypeError
                && !(error instanceof DOMException)
                && !String(error.message).includes("is not a function")
              )
                ? "TypeError"
                : String(error && error.name) + ":" + String(error && error.message);
            }
            const lookupAfter = await navigator.serviceWorker.getRegistration();
            globalThis.__swUpdateErrorProbe = JSON.stringify({
              scriptError,
              activePreserved,
              unregistered,
              afterUnregister,
              stillUnregistered: lookupAfter === undefined
            });
          })().catch(error => {
            globalThis.__swUpdateErrorProbe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swUpdateErrorDone = true;
          });
        })()
        "#,
    )
    .expect("service worker update error probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swUpdateErrorDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swUpdateErrorProbe)").unwrap(),
        r#"{"scriptError":"TypeError","activePreserved":true,"unregistered":true,"afterUnregister":"TypeError","stillUnregistered":true}"#
    );

    server
        .await
        .expect("service worker update error server should finish");
}

#[tokio::test]
async fn service_worker_registration_update_uses_receiver_realm_when_borrowed() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        worker_body,
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
          globalThis.__swUpdateRealmDone = false;
          globalThis.__swUpdateRealmProbe = "pending";
          (async () => {
            const parentReg = await navigator.serviceWorker.register("worker.js", { scope: "./" });
            await navigator.serviceWorker.ready;
            const frame = document.createElement("iframe");
            document.body.appendChild(frame);
            const child = frame.contentWindow;
            const childReg = await child.navigator.serviceWorker.getRegistration();
            await parentReg.unregister();
            const promise = parentReg.update.call(childReg);
            const promiseInChildRealm =
              promise instanceof child.Promise && !(promise instanceof window.Promise);
            const errorInChildRealm = await promise.then(
              () => false,
              error => error instanceof child.TypeError && !(error instanceof window.TypeError)
            );
            globalThis.__swUpdateRealmProbe = JSON.stringify({
              promiseInChildRealm,
              errorInChildRealm
            });
          })().catch(error => {
            globalThis.__swUpdateRealmProbe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swUpdateRealmDone = true;
          });
        })()
        "#,
    )
    .expect("service worker update realm probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swUpdateRealmDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swUpdateRealmProbe)").unwrap(),
        r#"{"promiseInChildRealm":true,"errorInChildRealm":true}"#
    );

    server
        .await
        .expect("service worker update realm server should finish");
}

#[tokio::test]
async fn service_worker_global_scope_registration_update_rejects_while_installing_and_updates_when_active()
 {
    let worker_v1 = r#"
        let installUpdateOutcome = "not-run";
        self.addEventListener("install", event => {
          event.waitUntil(
            Promise.resolve().then(() => {
              if (typeof self.registration.update !== "function") {
                throw new Error("missing self.registration.update");
              }
              return self.registration.update();
            }).then(
              () => { installUpdateOutcome = "resolved"; },
              error => {
                installUpdateOutcome = (
                  error instanceof TypeError
                  && String(error.message).includes("installing")
                )
                  ? "TypeError"
                  : String(error && error.name) + ":" + String(error && error.message);
              }
            ).then(() => self.skipWaiting())
          );
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("message", event => {
          if (event.data === "update") {
            Promise.resolve().then(() => self.registration.update()).then(
              updated => event.ports[0].postMessage(JSON.stringify({
                installUpdateOutcome,
                sameRegistration: updated === self.registration,
                outcome: "resolved"
              })),
              error => event.ports[0].postMessage(JSON.stringify({
                installUpdateOutcome,
                outcome: "rejected:" + (error && error.name) + ":" + (error && error.message)
              }))
            );
          }
        });
    "#;
    let worker_v2 = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("message", event => {
          event.ports[0].postMessage("v2");
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v1,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v2,
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
          globalThis.__swWorkerSelfUpdateDone = false;
          globalThis.__swWorkerSelfUpdateProbe = "pending";
          const askWorker = (worker, payload) => new Promise(resolve => {
            const channel = new MessageChannel();
            channel.port1.onmessage = event => resolve(event.data);
            worker.postMessage(payload, [channel.port2]);
          });
          (async () => {
            const reg = await navigator.serviceWorker.register("worker.js", { scope: "./" });
            await navigator.serviceWorker.ready;
            const controllerChanged = new Promise(resolve => {
              navigator.serviceWorker.addEventListener("controllerchange", () => resolve(true), { once: true });
            });
            const workerReply = JSON.parse(await askWorker(reg.active, "update"));
            if (workerReply.outcome !== "resolved") {
              globalThis.__swWorkerSelfUpdateProbe = JSON.stringify(workerReply);
              return;
            }
            await controllerChanged;
            const afterVersion = await askWorker(reg.active, "ping");
            globalThis.__swWorkerSelfUpdateProbe = JSON.stringify({
              installUpdateOutcome: workerReply.installUpdateOutcome,
              sameRegistration: workerReply.sameRegistration,
              outcome: workerReply.outcome,
              afterVersion
            });
          })().catch(error => {
            globalThis.__swWorkerSelfUpdateProbe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swWorkerSelfUpdateDone = true;
          });
        })()
        "#,
    )
    .expect("service worker self update probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swWorkerSelfUpdateDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swWorkerSelfUpdateProbe)")
            .unwrap(),
        r#"{"installUpdateOutcome":"TypeError","sameRegistration":true,"outcome":"resolved","afterVersion":"v2"}"#
    );

    server
        .await
        .expect("service worker self update server should finish");
}

#[tokio::test]
async fn service_worker_global_scope_top_level_update_succeeds_after_worker_restart() {
    let worker_v1 = r#"
        const topLevelUpdatePromise = self.registration.update().then(
          updated => ({
            outcome: "resolved",
            sameRegistration: updated === self.registration
          }),
          error => ({
            outcome: "rejected",
            name: error && error.name,
            message: String(error && error.message)
          })
        );
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("message", event => {
          event.waitUntil(
            topLevelUpdatePromise.then(result => {
              event.ports[0].postMessage(JSON.stringify({
                version: "v1",
                topLevelUpdate: result
              }));
            })
          );
        });
    "#;
    let worker_v2 = r#"
        self.addEventListener("install", event => {
          event.waitUntil(self.skipWaiting());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("message", event => {
          event.ports[0].postMessage(JSON.stringify({ version: "v2" }));
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v1,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v1,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_v2,
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
          globalThis.__swRestartStep1Done = false;
          globalThis.__swRestartStep1Probe = "pending";
          globalThis.__swRestartAskWorker = worker => new Promise(resolve => {
            const channel = new MessageChannel();
            channel.port1.onmessage = event => resolve(JSON.parse(event.data));
            worker.postMessage("status", [channel.port2]);
          });
          (async () => {
            const reg = await navigator.serviceWorker.register("worker.js", { scope: "./" });
            await navigator.serviceWorker.ready;
            globalThis.__swRestartReg = reg;
            const initialReply = await globalThis.__swRestartAskWorker(reg.active);
            globalThis.__swRestartStep1Probe = JSON.stringify({
              version: initialReply.version,
              outcome: initialReply.topLevelUpdate.outcome,
              name: initialReply.topLevelUpdate.name,
              mentionsInstalling: String(initialReply.topLevelUpdate.message).includes("installing")
            });
          })().catch(error => {
            globalThis.__swRestartStep1Probe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swRestartStep1Done = true;
          });
        })()
        "#,
    )
    .expect("step 1 probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swRestartStep1Done)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swRestartStep1Probe)").unwrap(),
        r#"{"version":"v1","outcome":"rejected","name":"TypeError","mentionsInstalling":true}"#
    );

    // Stop the active worker so the next message restarts it while already installed.
    browser_context_runtime
        .service_worker_runtime()
        .stop_all_running_hosts_for_test();

    vm.eval(
        r#"
        (() => {
          globalThis.__swRestartStep2Done = false;
          globalThis.__swRestartStep2Probe = "pending";
          (async () => {
            const reg = globalThis.__swRestartReg;
            const controllerChanged = new Promise(resolve => {
              navigator.serviceWorker.addEventListener("controllerchange", () => resolve(true), { once: true });
            });
            const restartedReply = await globalThis.__swRestartAskWorker(reg.active);
            await controllerChanged;
            const updatedReply = await globalThis.__swRestartAskWorker(reg.active);
            globalThis.__swRestartStep2Probe = JSON.stringify({
              restartedVersion: restartedReply.version,
              topLevelOutcome: restartedReply.topLevelUpdate.outcome,
              topLevelSameRegistration: restartedReply.topLevelUpdate.sameRegistration,
              afterVersion: updatedReply.version
            });
          })().catch(error => {
            globalThis.__swRestartStep2Probe =
              "error:" + (error && error.name) + ":" + (error && error.message);
          }).finally(() => {
            globalThis.__swRestartStep2Done = true;
          });
        })()
        "#,
    )
    .expect("step 2 probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__swRestartStep2Done)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__swRestartStep2Probe)").unwrap(),
        r#"{"restartedVersion":"v1","topLevelOutcome":"resolved","topLevelSameRegistration":true,"afterVersion":"v2"}"#
    );

    server
        .await
        .expect("service worker restart update server should finish");
}

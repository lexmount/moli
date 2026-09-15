use super::*;

#[tokio::test]
async fn navigator_service_worker_intercepts_csp_report_destination() {
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
              "method=" + event.request.method
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
              globalThis.__serviceWorkerCspReportProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                await fetch("blocked-data").catch(() => {});
                globalThis.__serviceWorkerCspReportProbe = "blocked";
              })().catch((error) => {
                globalThis.__serviceWorkerCspReportProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker CSP report destination setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerCspReportProbe)",
        "blocked",
    )
    .await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        items.extend(vm.take_network_output().into_items());
        if service_worker_csp_report_seen(
            &items,
            &report_url,
            "destination=report|mode=no-cors|credentials=same-origin|method=POST",
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "service worker CSP report destination did not settle: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    server
        .await
        .expect("service worker CSP report destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_intercepts_worker_csp_report_destination() {
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
              globalThis.__serviceWorkerWorkerCspReportProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const worker = new Worker("client-worker.js");
                worker.onmessage = event => {
                  globalThis.__serviceWorkerWorkerCspReportProbe =
                    String(event.data);
                };
                worker.onerror = event => {
                  globalThis.__serviceWorkerWorkerCspReportProbe =
                    "error:" + event.message;
                };
                worker.postMessage("start");
              })().catch((error) => {
                globalThis.__serviceWorkerWorkerCspReportProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker worker CSP report destination setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerWorkerCspReportProbe)",
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
            "destination=report|mode=no-cors|credentials=same-origin|method=POST|client=true",
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker CSP report destination did not settle: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    server
        .await
        .expect("worker CSP report destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_does_not_intercept_invalid_no_cors_redirect_mode() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        let count = 0;
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const path = new URL(event.request.url).pathname;
          if (path.endsWith("/api/target")) {
            count++;
            event.respondWith(new Response("intercepted"));
          } else if (path.endsWith("/api/count")) {
            event.respondWith(new Response(String(count)));
          }
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );
    vm.eval(
        r#"
        globalThis.noCorsRedirectResult = "pending";
        (async () => {
          const assert = (value, label) => { if (!value) throw new Error(label); };
          await navigator.serviceWorker.register("worker.js", {scope: "./"});
          await navigator.serviceWorker.ready;
          const remote = new URL("api/target", location.href);
          remote.hostname = location.hostname === "localhost" ? "127.0.0.1" : "localhost";
          for (const redirect of ["manual", "error"]) {
            let rejected = false;
            try { await fetch(remote, {mode: "no-cors", redirect}); }
            catch (error) { rejected = error instanceof TypeError; }
            assert(rejected, "cross-origin fetch must reject " + redirect);
            const response = await fetch("api/target", {mode: "no-cors", redirect});
            assert(response.status === 200 && await response.text() === "intercepted", "same-origin fetch reaches Service Worker");
          }
          const count = await (await fetch("api/count")).text();
          assert(count === "2", "only same-origin requests reach Service Worker: " + count);
          noCorsRedirectResult = "ok";
        })().catch(error => { noCorsRedirectResult = String(error.stack || error); });
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "noCorsRedirectResult",
        "ok",
    )
    .await;
    server.await.unwrap();
}
#[tokio::test]
async fn navigator_service_worker_intercepts_window_xhr() {
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
          event.respondWith((async () => {
            return new Response(JSON.stringify({
              url: new URL(event.request.url).pathname,
              method: event.request.method,
              destination: event.request.destination,
              mode: event.request.mode,
              credentials: event.request.credentials,
              client: event.clientId.length > 0,
              resulting: event.resultingClientId,
              header: event.request.headers.get("x-test"),
              body: await event.request.text()
            }), {
              status: 201,
              statusText: "XHR handled",
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
              globalThis.__serviceWorkerXhrProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const xhr = new XMLHttpRequest();
                xhr.open("POST", "api/xhr.txt");
                xhr.setRequestHeader("X-Test", "yes");
                xhr.onload = () => {
                  globalThis.__serviceWorkerXhrProbe = [
                    xhr.status,
                    xhr.statusText,
                    xhr.getResponseHeader("content-type"),
                    xhr.responseText
                  ].join("|");
                };
                xhr.onerror = () => {
                  globalThis.__serviceWorkerXhrProbe = "error";
                };
                xhr.send("xhr-body");
              })().catch((error) => {
                globalThis.__serviceWorkerXhrProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker XHR probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerXhrProbe)",
        r#"201|XHR handled|application/json|{"url":"/app/api/xhr.txt","method":"POST","destination":"","mode":"cors","credentials":"same-origin","client":true,"resulting":"","header":"yes","body":"xhr-body"}"#,
    )
    .await;

    server
        .await
        .expect("service worker XHR server should finish");
}
#[tokio::test]
async fn navigator_service_worker_invalid_response_header_fails_window_xhr() {
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
          if (!event.request.url.includes("sample?test")) {
            return;
          }
          event.respondWith(new Promise(resolve => {
            const headers = new Headers();
            headers.append("foo", "foo");
            headers.append("foo", "b\0r");
            resolve(new Response("hello world", {headers}));
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
              globalThis.__serviceWorkerInvalidHeaderXhrProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const xhr = new XMLHttpRequest();
                xhr.onload = () => {
                  globalThis.__serviceWorkerInvalidHeaderXhrProbe =
                    "load:" + xhr.status + ":" + xhr.responseText;
                };
                xhr.onerror = () => {
                  globalThis.__serviceWorkerInvalidHeaderXhrProbe = [
                    "error",
                    xhr.readyState,
                    xhr.status,
                    xhr.statusText,
                    xhr.responseText
                  ].join("|");
                };
                xhr.open("POST", "sample?test");
                xhr.send("test string");
              })().catch((error) => {
                globalThis.__serviceWorkerInvalidHeaderXhrProbe =
                  "exception:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker invalid header XHR probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerInvalidHeaderXhrProbe)",
        "error|4|0||",
    )
    .await;

    server
        .await
        .expect("service worker invalid response header server should finish");
}
#[tokio::test]
async fn navigator_service_worker_intercepts_element_resource_destinations_once() {
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
          const expected = path.endsWith("/hero.png") ? "image" :
            path.endsWith("/captions/en.vtt") ? "track" :
            path.endsWith("/media/sound.ogg") ? "audio" :
            path.endsWith("/media/clip.mp4") ? "video" :
            "";
          if (event.request.destination !== expected) {
            event.respondWith(new Response("wrong:" + event.request.destination, {
              status: 500,
              headers: {"content-type": "text/plain"}
            }));
            return;
          }
          event.respondWith(new Response("parser:" + expected + ":" + path, {
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
              globalThis.__serviceWorkerElementResourceProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const root = document.body || document.documentElement ||
                  document.appendChild(document.createElement("html"));
                const img = document.createElement("img");
                img.setAttribute("src", "hero.png");
                const audio = document.createElement("audio");
                const audioSource = document.createElement("source");
                audioSource.setAttribute("src", "media/sound.ogg");
                audio.appendChild(audioSource);
                const video = document.createElement("video");
                video.setAttribute("src", "media/clip.mp4");
                const track = document.createElement("track");
                track.setAttribute("src", "captions/en.vtt");
                track.setAttribute("default", "");
                video.appendChild(track);
                root.appendChild(img);
                root.appendChild(audio);
                root.appendChild(video);
                globalThis.__serviceWorkerElementResourceProbe = "ready";
              })().catch((error) => {
                globalThis.__serviceWorkerElementResourceProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker parser-discovered destination setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerElementResourceProbe)",
        "ready",
    )
    .await;

    assert!(
        vm.has_pending_image_network_requests(),
        "the image element owner must bind its exact sequence before fetch"
    );

    let expected_image_url = format!("{base_url}/app/hero.png");
    let expected_track_url = format!("{base_url}/app/captions/en.vtt");
    let expected_audio_url = format!("{base_url}/app/media/sound.ogg");
    let expected_video_url = format!("{base_url}/app/media/clip.mp4");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        items.extend(vm.take_network_output().into_items());

        let resource_seen = |resource_type: crate::types::SubresourceResourceType,
                             expected_url: &str,
                             expected_body: &str| {
            if items.iter().any(|item| {
                matches!(
                    item,
                    crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                        if record.resource_type() == resource_type
                            && record.url().as_str() == expected_url
                            && matches!(
                                record.outcome(),
                                crate::types::SubresourceNetworkOutcome::Success {
                                    status: 200,
                                    response_body,
                                    ..
                                } if response_body.diagnostic_bytes().as_ref() == expected_body.as_bytes()
                            )
                )
            }) {
                return true;
            }
            let handle = items.iter().find_map(|item| {
                let crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request) =
                    item
                else {
                    return None;
                };
                (request.resource_type() == resource_type && request.url().as_str() == expected_url)
                    .then(|| request.handle())
            });
            handle.is_some_and(|handle| {
                items.iter().any(|item| {
                    matches!(
                        item,
                        crate::types::ScriptNetworkOutputItem::SubresourceResponseStarted(response)
                            if response.handle() == handle
                                && response.status() == 200
                                && response.final_url().as_str() == expected_url
                    )
                }) && items.iter().any(|item| {
                    matches!(
                        item,
                        crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(body)
                            if body.handle() == handle
                                && matches!(
                                    body.result(),
                                    crate::types::SubresourceBodyFinishedResult::Ready(response_body)
                                        if response_body.diagnostic_bytes().as_ref() == expected_body.as_bytes()
                                )
                    )
                })
            })
        };

        if resource_seen(
            crate::types::SubresourceResourceType::Image,
            &expected_image_url,
            "parser:image:/app/hero.png",
        ) && resource_seen(
            crate::types::SubresourceResourceType::TextTrack,
            &expected_track_url,
            "parser:track:/app/captions/en.vtt",
        ) && resource_seen(
            crate::types::SubresourceResourceType::Audio,
            &expected_audio_url,
            "parser:audio:/app/media/sound.ogg",
        ) && resource_seen(
            crate::types::SubresourceResourceType::Video,
            &expected_video_url,
            "parser:video:/app/media/clip.mp4",
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "element-owned service worker network events did not settle: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }
    assert!(
        !vm.has_pending_image_network_requests(),
        "the image body terminal must consume the sequence's network state"
    );
    for resource_type in [
        crate::types::SubresourceResourceType::Image,
        crate::types::SubresourceResourceType::Audio,
        crate::types::SubresourceResourceType::TextTrack,
        crate::types::SubresourceResourceType::Video,
    ] {
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(
                    item,
                    crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                        if request.resource_type() == resource_type
                ))
                .count(),
            1,
            "element resource selection must be the only image/media/track request producer"
        );
    }

    let image_requests = items
        .iter()
        .filter_map(|item| {
            let crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item
            else {
                return None;
            };
            (request.resource_type() == crate::types::SubresourceResourceType::Image
                && request.url().as_str() == expected_image_url)
                .then_some(request)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        image_requests.len(),
        1,
        "DOM image fetch must not be duplicated"
    );
    assert_eq!(
        image_requests[0].request_initiator_type(),
        crate::types::SubresourceRequestInitiatorType::Script,
        "the element-owned image pipeline should replace parser scanning"
    );

    server
        .await
        .expect("service worker element-resource destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_intercepts_stylesheet_font_face_destination() {
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
          const expected = path.endsWith("/style.css") ? "style" :
            path.endsWith("/fonts/demo.woff2") ? "font" :
            "";
          if (event.request.destination !== expected) {
            event.respondWith(new Response("wrong:" + event.request.destination, {
              status: 500,
              headers: {"content-type": "text/plain"}
            }));
            return;
          }
          if (expected === "style") {
            event.respondWith(new Response(`
              @font-face {
                font-family: Demo;
                src: local("Demo"), url("fonts/demo.woff2") format("woff2");
              }
              body { font-family: Demo; }
            `, {
              headers: {"content-type": "text/css"}
            }));
            return;
          }
          event.respondWith(new Response("stylesheet-font:" + expected + ":" + path, {
            headers: {"content-type": "text/plain"}
          }));
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Font, true);
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerStylesheetFontProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const ensureHead = () => {
                  if (document.head) {
                    return document.head;
                  }
                  const html = document.documentElement ||
                    document.appendChild(document.createElement("html"));
                  return html.appendChild(document.createElement("head"));
                };
                const parent = ensureHead();
                const outcome = await new Promise((resolve) => {
                  const link = document.createElement("link");
                  link.setAttribute("rel", "stylesheet");
                  link.setAttribute("href", "style.css");
                  link.onload = () => resolve("link:load");
                  link.onerror = () => resolve("link:error");
                  parent.appendChild(link);
                });
                globalThis.__serviceWorkerStylesheetFontProbe = outcome;
              })().catch((error) => {
                globalThis.__serviceWorkerStylesheetFontProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker stylesheet font destination setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerStylesheetFontProbe)",
        "link:load",
    )
    .await;

    let expected_font_url = format!("{base_url}/app/fonts/demo.woff2");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        items.extend(vm.take_network_output().into_items());

        let font_handle = items.iter().find_map(|item| {
            let crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item
            else {
                return None;
            };
            (request.resource_type() == crate::types::SubresourceResourceType::Font
                && request.url().as_str() == expected_font_url)
                .then(|| request.handle())
        });
        let font_seen = font_handle.is_some_and(|handle| {
            items.iter().any(|item| {
                let crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) = item
                else {
                    return false;
                };
                record.request_handle() == Some(handle)
                    && record.url().as_str() == expected_font_url
                    && record.request_initiator_type()
                        == crate::types::SubresourceRequestInitiatorType::Css
                    && matches!(
                        record.outcome(),
                        crate::types::SubresourceNetworkOutcome::Success {
                            final_url,
                            status: 200,
                            response_body,
                            ..
                        } if final_url.as_str() == expected_font_url
                            && response_body.diagnostic_bytes().as_ref()
                                == b"stylesheet-font:font:/app/fonts/demo.woff2"
                    )
            })
        });
        if font_seen {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "stylesheet font service worker network events did not settle: {items:?}"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    assert!(
        !items.iter().any(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                    if request.resource_type() == crate::types::SubresourceResourceType::Image
                        && request.url().as_str() == expected_font_url
            )
        }),
        "stylesheet font URL should not also be reported as an image request: {items:?}"
    );

    server
        .await
        .expect("service worker stylesheet font destination server should finish");
}
#[tokio::test]
async fn navigator_service_worker_intercepts_connected_stylesheet_link() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r##"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        self.addEventListener("fetch", event => {
          const path = new URL(event.request.url).pathname;
          if (!path.endsWith("/style.css")) {
            return;
          }
          if (event.request.destination !== "style") {
            event.respondWith(new Response("wrong:" + event.request.destination, {
              status: 500,
              headers: {"content-type": "text/plain"}
            }));
            return;
          }
          event.respondWith(new Response(
            "#sw-style-target { color: rgb(0, 0, 255); }",
            { headers: {"content-type": "text/css"} }
          ));
        });
        "##,
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
              globalThis.__serviceWorkerStylesheetLinkProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const ensureHead = () => {
                  if (document.head) {
                    return document.head;
                  }
                  const html = document.documentElement ||
                    document.appendChild(document.createElement("html"));
                  return html.appendChild(document.createElement("head"));
                };
                const ensureBody = () => {
                  if (document.body) {
                    return document.body;
                  }
                  const html = document.documentElement ||
                    document.appendChild(document.createElement("html"));
                  return html.appendChild(document.createElement("body"));
                };
                const target = document.createElement("div");
                target.id = "sw-style-target";
                ensureBody().appendChild(target);
                const link = document.createElement("link");
                link.setAttribute("rel", "stylesheet");
                link.setAttribute("href", "style.css");
                const outcome = await new Promise((resolve) => {
                  link.onload = () => resolve("load");
                  link.onerror = () => resolve("error");
                  ensureHead().appendChild(link);
                });
                let rules = "";
                try {
                  rules = link.sheet.cssRules[0].cssText;
                } catch (error) {
                  rules = "throw:" + error.name;
                }
                globalThis.__serviceWorkerStylesheetLinkProbe = [
                  outcome,
                  getComputedStyle(target).color,
                  rules
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerStylesheetLinkProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker stylesheet link setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerStylesheetLinkProbe)",
        "load|rgb(0, 0, 255)|#sw-style-target { color: rgb(0, 0, 255); }",
    )
    .await;

    server
        .await
        .expect("service worker stylesheet link server should finish");
}
#[tokio::test]
async fn navigator_service_worker_no_respond_with_fallback_and_rejected_response_failures() {
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
              const testcase = new URL(event.request.url).search;
              if (testcase === "?wait-until-only") {
                event.waitUntil(Promise.resolve());
                return;
              }
              if (testcase === "?reject") {
                event.respondWith(Promise.reject(new Error("rejected")));
              }
            });
            "#,
        ),
        (
            "/app/api/fallback.txt?empty-listener",
            "text/plain; charset=utf-8",
            "fallback-fetch-empty",
        ),
        (
            "/app/api/fallback.txt?wait-until-only",
            "text/plain; charset=utf-8",
            "fallback-xhr-wait",
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
              globalThis.__serviceWorkerFallbackFailureProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const fetchFallback = await fetch("api/fallback.txt?empty-listener");
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
                  xhr.open("GET", "api/fallback.txt?" + name);
                  xhr.send();
                });
                let rejectedFetch;
                try {
                  await fetch("api/failure.txt?reject");
                  rejectedFetch = "unexpected-resolve";
                } catch (error) {
                  rejectedFetch = String(error && error.name);
                }
                globalThis.__serviceWorkerFallbackFailureProbe = [
                  fetchFallback.status,
                  fetchFallback.statusText,
                  await fetchFallback.text(),
                  await runXhr("wait-until-only"),
                  rejectedFetch,
                  await runXhr("reject")
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerFallbackFailureProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker fallback/failure probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerFallbackFailureProbe)",
        concat!(
            "200|OK|fallback-fetch-empty|",
            "wait-until-only:load:4:200:OK:fallback-xhr-wait|",
            "TypeError|",
            "reject:error:4:0::"
        ),
    )
    .await;

    server
        .await
        .expect("service worker fallback/failure server should finish");
}
#[tokio::test]
async fn navigator_service_worker_response_body_reader_closed_rejects_after_abort() {
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
              if (!event.request.url.includes("reader-closed-abort")) {
                return;
              }
              const stream = new ReadableStream({
                start(controller) {
                  controller.enqueue(new Uint8Array([65]));
                }
              });
              event.respondWith(new Response(stream, {status: 212}));
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
              globalThis.__serviceWorkerReaderClosedAbortProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const controller = new AbortController();
                const response = await fetch("api/reader-closed-abort.txt", {
                  signal: controller.signal
                });
                const reader = response.body.getReader();
                const first = await reader.read();
                controller.abort();
                const readResult = await reader.read().then(
                  () => "read:unexpected-resolve",
                  error => [
                    "read",
                    error && error.name,
                    error instanceof DOMException,
                    error && error.message
                  ].join(":")
                );
                const closed = reader.closed;
                const closedSame = reader.closed === closed;
                const closedResult = await closed.then(
                  () => "closed:unexpected-resolve",
                  error => [
                    "closed",
                    error && error.name,
                    error instanceof DOMException,
                    error && error.message
                  ].join(":")
                );
                globalThis.__serviceWorkerReaderClosedAbortProbe = [
                  response.status,
                  first.done,
                  new TextDecoder().decode(first.value),
                  readResult,
                  "closed-same:" + closedSame,
                  closedResult
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerReaderClosedAbortProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker reader.closed abort probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerReaderClosedAbortProbe)",
        concat!(
            "212|false|A|",
            "read:AbortError:true:The operation was aborted.|",
            "closed-same:true|",
            "closed:AbortError:true:The operation was aborted."
        ),
    )
    .await;

    server
        .await
        .expect("service worker reader.closed abort server should finish");
}
#[tokio::test]
async fn navigator_service_worker_response_body_methods_reject_after_abort() {
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
              if (!event.request.url.includes("body-method-abort")) {
                return;
              }
              const stream = new ReadableStream({
                start(controller) {
                  controller.enqueue(new Uint8Array([97, 61, 49, 38, 98, 61, 116, 119, 111]));
                }
              });
              event.respondWith(new Response(stream, {
                status: 211,
                headers: [["Content-Type", "application/x-www-form-urlencoded"]]
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
              globalThis.__serviceWorkerBodyMethodsAbortProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const methods = ["arrayBuffer", "blob", "bytes", "formData", "json", "text"];
                const results = [];
                for (const method of methods) {
                  const controller = new AbortController();
                  const response = await fetch("api/body-method-abort.txt?" + method, {
                    signal: controller.signal
                  });
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
                  results.push(log.join(">"));
                }
                globalThis.__serviceWorkerBodyMethodsAbortProbe = results.join("|");
              })().catch(error => {
                globalThis.__serviceWorkerBodyMethodsAbortProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker body methods abort probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerBodyMethodsAbortProbe)",
        concat!(
            "arrayBuffer:AbortError:true:The operation was aborted.>next-microtask|",
            "blob:AbortError:true:The operation was aborted.>next-microtask|",
            "bytes:AbortError:true:The operation was aborted.>next-microtask|",
            "formData:AbortError:true:The operation was aborted.>next-microtask|",
            "json:AbortError:true:The operation was aborted.>next-microtask|",
            "text:AbortError:true:The operation was aborted.>next-microtask"
        ),
    )
    .await;

    server
        .await
        .expect("service worker body methods abort server should finish");
}
#[tokio::test]
async fn navigator_service_worker_synthetic_stream_response_resolves_before_body_chunk() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            let pendingController;
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              const url = new URL(event.request.url);
              if (url.pathname.endsWith("/api/synthetic-stream.txt")) {
                const stream = new ReadableStream({
                  start(controller) {
                    pendingController = controller;
                  },
                  cancel() {
                    pendingController = undefined;
                  }
                });
                event.respondWith(new Response(stream, {
                  status: 213,
                  headers: [
                    ["Content-Type", "text/plain; charset=utf-8"],
                    ["x-synthetic-stream", "yes"]
                  ]
                }));
                return;
              }
              if (url.pathname.endsWith("/api/release-synthetic-stream.txt")) {
                const controller = pendingController;
                pendingController = undefined;
                if (controller) {
                  controller.enqueue(new Uint8Array([100, 101, 108, 97, 121, 101, 100]));
                  controller.close();
                  event.respondWith(new Response("released"));
                } else {
                  event.respondWith(new Response("missing-controller", { status: 500 }));
                }
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
              globalThis.__serviceWorkerSyntheticStreamHeadersFirstProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const response = await fetch("api/synthetic-stream.txt");
                const releaseResponse = await fetch("api/release-synthetic-stream.txt");
                globalThis.__serviceWorkerSyntheticStreamHeadersFirstProbe = [
                  response.status,
                  response.headers.get("x-synthetic-stream"),
                  response.body instanceof ReadableStream,
                  await releaseResponse.text(),
                  await response.text()
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerSyntheticStreamHeadersFirstProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker synthetic stream headers-first probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyntheticStreamHeadersFirstProbe)",
        "213|yes|true|released|delayed",
    )
    .await;

    server
        .await
        .expect("service worker synthetic stream headers-first server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_propagation_and_throw_browser_matrix() {
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

            const propagationOrder = [];

            self.addEventListener("fetch", event => {
              const testcase = new URL(event.request.url).search;
              if (testcase === "?stops-propagation") {
                propagationOrder.push("first");
                event.respondWith(Promise.resolve().then(() => new Response(
                  propagationOrder.join(","),
                  {status: 214}
                )));
                return;
              }
              if (testcase === "?throws-after") {
                event.respondWith(new Response("intercepted", {status: 215}));
                throw new Error("after-respond");
              }
              if (testcase === "?double") {
                let secondResult = "unset";
                event.respondWith(Promise.resolve().then(() => new Response(
                  secondResult,
                  {status: 216}
                )));
                try {
                  event.respondWith(new Response("second"));
                  secondResult = "resolved";
                } catch (error) {
                  secondResult = error && error.name;
                }
              }
              if (testcase === "?microtask") {
                Promise.resolve().then(() => {
                  event.respondWith(new Response("microtask", {status: 217}));
                });
              }
              if (testcase === "?task") {
                const clientId = event.clientId;
                setTimeout(() => {
                  let result = "unset";
                  try {
                    event.respondWith(new Response("task-response"));
                    result = "resolved";
                  } catch (error) {
                    result = error && error.name;
                  }
                  clients.get(clientId).then(client => {
                    if (client) client.postMessage("task:" + result);
                  });
                }, 0);
              }
            });

            self.addEventListener("fetch", event => {
              if (new URL(event.request.url).search === "?stops-propagation") {
                propagationOrder.push("second");
              }
            });
            "#,
        ),
        (
            "/app/api/respondwith?task",
            "text/plain; charset=utf-8",
            "task-fallback",
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
              globalThis.__serviceWorkerRespondWithPropagationProbe = "pending";
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
                const run = async testcase => {
                  const response = await fetch("api/respondwith" + testcase);
                  return [response.status, await response.text()].join(":");
                };
                const runTask = async () => {
                  const responsePromise = fetch("api/respondwith?task");
                  const message = await nextMessage();
                  const response = await responsePromise;
                  return [response.status, await response.text(), message].join(":");
                };
                globalThis.__serviceWorkerRespondWithPropagationProbe = [
                  await run("?stops-propagation"),
                  await run("?throws-after"),
                  await run("?double"),
                  await run("?microtask"),
                  await runTask()
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerRespondWithPropagationProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker respondWith propagation probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRespondWithPropagationProbe)",
        "214:first|215:intercepted|216:InvalidStateError|217:microtask|200:task-fallback:task:InvalidStateError",
    )
    .await;

    server
        .await
        .expect("service worker respondWith propagation server should finish");
}
#[tokio::test]
async fn navigator_service_worker_respond_with_wait_until_lifetime_browser_matrix() {
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

            function waitUntilResult(event) {
              try {
                event.waitUntil(Promise.resolve());
                return "OK";
              } catch (error) {
                return (error && error.name) + ":" + (error instanceof DOMException);
              }
            }

            function report(clientId, message) {
              clients.get(clientId).then(client => {
                if (client) client.postMessage(message);
              });
            }

            function newTaskResponse(body) {
              return new Promise(resolve => {
                setTimeout(() => resolve(new Response(body)), 0);
              });
            }

            self.addEventListener("fetch", event => {
              const step = new URL(event.request.url).search.slice(1);
              if (step === "pending") {
                let resolveResponse;
                const response = new Promise(resolve => {
                  resolveResponse = resolve;
                });
                event.respondWith(response);
                setTimeout(() => {
                  report(event.clientId, step + ":" + waitUntilResult(event));
                  resolveResponse(new Response(step, {status: 218}));
                }, 0);
                return;
              }
              if (step === "same-turn") {
                const response = newTaskResponse(step);
                event.respondWith(response);
                response.then(() => {
                  report(event.clientId, step + ":" + waitUntilResult(event));
                });
                return;
              }
              if (step === "extra-microtask") {
                const response = newTaskResponse(step);
                event.respondWith(response);
                response.then(() => Promise.resolve().then(() => {
                  report(event.clientId, step + ":" + waitUntilResult(event));
                }));
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
              globalThis.__serviceWorkerRespondWithWaitUntilProbe = "pending";
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
                const run = async step => {
                  const responsePromise = fetch("api/waituntil?" + step);
                  const message = await nextMessage();
                  const response = await responsePromise;
                  return [response.status, await response.text(), message].join(":");
                };
                globalThis.__serviceWorkerRespondWithWaitUntilProbe = [
                  await run("pending"),
                  await run("same-turn"),
                  await run("extra-microtask")
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerRespondWithWaitUntilProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker respondWith waitUntil probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRespondWithWaitUntilProbe)",
        "218:pending:pending:OK|200:same-turn:same-turn:OK|200:extra-microtask:extra-microtask:InvalidStateError:true",
    )
    .await;

    server
        .await
        .expect("service worker respondWith waitUntil server should finish");
}
#[tokio::test]
async fn navigator_service_worker_responds_with_body_accessed_default_and_basic_responses() {
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

            function maybeClone(response, cloneMode) {
              if (cloneMode === "clone-response") {
                return response.clone();
              }
              if (cloneMode === "clone-unused") {
                response.clone();
              }
              return response;
            }

            function assertUsableBody(response, label) {
              const body = response.body;
              if (!body || response.bodyUsed || body.locked) {
                throw new Error(label + ":" + [
                  Boolean(body),
                  response.bodyUsed,
                  body && body.locked
                ].join("/"));
              }
            }

            async function passThroughCacheIfNeeded(event, response, cacheMode) {
              if (cacheMode !== "cache") {
                return response;
              }
              const cacheName = event.request.url;
              await self.caches.delete(cacheName);
              const cache = await self.caches.open(cacheName);
              await cache.put(event.request, response);
              const matched = await cache.match(event.request.url);
              matched.body;
              await self.caches.delete(cacheName);
              return matched;
            }

            self.addEventListener("fetch", event => {
              const url = new URL(event.request.url);
              if (!url.pathname.endsWith("/TestRequest")) {
                return;
              }
              const responsePromise = url.searchParams.get("type") === "basic"
                ? fetch("respond-with-body-accessed-response.jsonp")
                : Promise.resolve(new Response("callback('OK');", {
                    status: 213,
                    headers: {"content-type": "application/javascript"}
                  }));
              event.respondWith(responsePromise.then(async response => {
                assertUsableBody(response, "original");
                const selected = maybeClone(response, url.searchParams.get("clone"));
                assertUsableBody(selected, "selected");
                const finalResponse = await passThroughCacheIfNeeded(
                  event,
                  selected,
                  url.searchParams.get("passThroughCache")
                );
                assertUsableBody(finalResponse, "final");
                return finalResponse;
              }));
            });
            "#,
        ),
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "callback('OK');",
        ),
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "callback('OK');",
        ),
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "callback('OK');",
        ),
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "callback('OK');",
        ),
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "callback('OK');",
        ),
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "callback('OK');",
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
              globalThis.__serviceWorkerBodyAccessedResponseProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const run = async (type, clone, cacheMode) => {
                  delete globalThis.__serviceWorkerBodyAccessedCallback;
                  const response = await fetch(
                    "TestRequest?type=" + type +
                    "&clone=" + clone +
                    "&passThroughCache=" + cacheMode
                  );
                  const scriptText = await response.text();
                  Function("callback", scriptText)(value => {
                    globalThis.__serviceWorkerBodyAccessedCallback = value;
                  });
                  return [
                    type,
                    clone,
                    cacheMode,
                    response.status,
                    response.headers.get("content-type"),
                    globalThis.__serviceWorkerBodyAccessedCallback
                  ].join(":");
                };
                const runType = async (type, cacheMode) => [
                  await run(type, "none", cacheMode),
                  await run(type, "clone-response", cacheMode),
                  await run(type, "clone-unused", cacheMode)
                ].join(",");
                globalThis.__serviceWorkerBodyAccessedResponseProbe = [
                  await runType("default", "direct"),
                  await runType("default", "cache"),
                  await runType("basic", "direct"),
                  await runType("basic", "cache")
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerBodyAccessedResponseProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker body-accessed response probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerBodyAccessedResponseProbe)",
        concat!(
            "default:none:direct:213:application/javascript:OK,",
            "default:clone-response:direct:213:application/javascript:OK,",
            "default:clone-unused:direct:213:application/javascript:OK|",
            "default:none:cache:213:application/javascript:OK,",
            "default:clone-response:cache:213:application/javascript:OK,",
            "default:clone-unused:cache:213:application/javascript:OK|",
            "basic:none:direct:200:application/javascript:OK,",
            "basic:clone-response:direct:200:application/javascript:OK,",
            "basic:clone-unused:direct:200:application/javascript:OK|",
            "basic:none:cache:200:application/javascript:OK,",
            "basic:clone-response:cache:200:application/javascript:OK,",
            "basic:clone-unused:cache:200:application/javascript:OK"
        ),
    )
    .await;

    server
        .await
        .expect("service worker body-accessed response server should finish");
}
#[tokio::test]
async fn navigator_service_worker_push_manager_tracks_subscription() {
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
        permission: serde_json::Value::String("notifications".to_owned()),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerPushManagerProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            const pushManager = registration.pushManager;
            const permission = await pushManager.permissionState();
            const before = await pushManager.getSubscription();
            const subscription = await pushManager.subscribe({ userVisibleOnly: true });
            const beforeUnsubscribe = await pushManager.getSubscription();
            const pushJsonSetterHits = [];
            const pushJsonNames = ["endpoint", "expirationTime", "options"];
            for (const name of pushJsonNames) {
              Object.defineProperty(Object.prototype, name, {
                configurable: true,
                get() { return undefined; },
                set(value) {
                  const receiverKind = this === subscription ? "subscription" : "plain";
                  pushJsonSetterHits.push(`${receiverKind}:${name}`);
                  Object.defineProperty(this, name, {
                    configurable: true,
                    enumerable: true,
                    writable: true,
                    value
                  });
                }
              });
            }
            let subscriptionJson;
            try {
              subscriptionJson = subscription.toJSON();
            } finally {
              for (const name of pushJsonNames) {
                delete Object.prototype[name];
              }
            }
            const jsonEndpointDescriptor =
              Object.getOwnPropertyDescriptor(subscriptionJson, "endpoint");
            const unsubscribed = await subscription.unsubscribe();
            const after = await pushManager.getSubscription();
            globalThis.__serviceWorkerPushManagerProbe = JSON.stringify({
              hasPushManager: !!pushManager,
              subscribe: typeof pushManager.subscribe,
              getSubscription: typeof pushManager.getSubscription,
              permissionState: typeof pushManager.permissionState,
              permission,
              before,
              endpoint: subscription.endpoint,
              expirationTime: subscription.expirationTime,
              userVisibleOnly: subscription.options.userVisibleOnly,
              applicationServerKey: subscription.options.applicationServerKey,
              toJSONEndpoint: subscriptionJson.endpoint,
              toJSONEndpointDescriptor: jsonEndpointDescriptor,
              toJSONSetterHits: pushJsonSetterHits
                .filter(hit => hit.startsWith("plain:")),
              unsubscribe: typeof subscription.unsubscribe,
              unsubscribed,
              beforeUnsubscribeEndpoint: beforeUnsubscribe && beforeUnsubscribe.endpoint,
              beforeUnsubscribeUserVisibleOnly:
                beforeUnsubscribe && beforeUnsubscribe.options.userVisibleOnly,
              after
            });
          })().catch(error => {
            globalThis.__serviceWorkerPushManagerProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker push manager probe should evaluate");

    let expected = r#"{"hasPushManager":true,"subscribe":"function","getSubscription":"function","permissionState":"function","permission":"granted","before":null,"endpoint":"https://moli.invalid/service-worker/push/1","expirationTime":null,"userVisibleOnly":true,"applicationServerKey":null,"toJSONEndpoint":"https://moli.invalid/service-worker/push/1","toJSONEndpointDescriptor":{"value":"https://moli.invalid/service-worker/push/1","writable":true,"enumerable":true,"configurable":true},"toJSONSetterHits":[],"unsubscribe":"function","unsubscribed":true,"beforeUnsubscribeEndpoint":"https://moli.invalid/service-worker/push/1","beforeUnsubscribeUserVisibleOnly":true,"after":null}"#;
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPushManagerProbe)",
        expected,
    )
    .await;
    server
        .await
        .expect("service worker push manager script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_sync_failure_retries_with_last_chance() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            let syncAttempts = 0;
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("sync", event => {
              syncAttempts += 1;
              if (!event.lastChance) {
                event.waitUntil(Promise.reject(new Error("retry sync")));
                return;
              }
              event.waitUntil((async () => {
                const tags = await self.registration.sync.getTags();
                const windows = await clients.matchAll({ includeUncontrolled: true });
                if (windows[0]) {
                  windows[0].postMessage(JSON.stringify({
                    tag: event.tag,
                    lastChance: event.lastChance,
                    attempts: syncAttempts,
                    tags: tags.join("|")
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
          globalThis.__serviceWorkerSyncRetryProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerSyncRetryProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            await registration.sync.register("retry-sync");
          })().catch(error => {
            globalThis.__serviceWorkerSyncRetryProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker sync retry setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncRetryProbe)",
        r#"{"tag":"retry-sync","lastChance":true,"attempts":2,"tags":"retry-sync"}"#,
    )
    .await;

    server
        .await
        .expect("service worker sync retry script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_periodic_sync_uses_owner_store() {
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
                if (!self.registration.periodicSync ||
                    typeof self.registration.periodicSync.register !== "function" ||
                    typeof self.registration.periodicSync.getTags !== "function" ||
                    typeof self.registration.periodicSync.unregister !== "function") {
                  throw new Error("missing periodicSync surface");
                }
                const before = await self.registration.periodicSync.getTags();
                await self.registration.periodicSync.register("worker-periodic", {
                  minInterval: 1234
                });
                const mid = await self.registration.periodicSync.getTags();
                await self.registration.periodicSync.unregister("page-periodic");
                const after = await self.registration.periodicSync.getTags();
                event.source.postMessage(JSON.stringify({
                  before,
                  mid,
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
          globalThis.__serviceWorkerPeriodicSyncProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            const permission = await navigator.permissions.query({
              name: "periodic-background-sync"
            });
            if (!registration.periodicSync ||
                typeof registration.periodicSync.register !== "function" ||
                typeof registration.periodicSync.getTags !== "function" ||
                typeof registration.periodicSync.unregister !== "function") {
              throw new Error("missing page periodicSync surface");
            }
            await registration.periodicSync.register("page-periodic", {
              minInterval: 60000
            });
            const pageTags = await registration.periodicSync.getTags();
            const workerResult = await new Promise(resolve => {
              sw.onmessage = event => resolve(JSON.parse(event.data));
              registration.active.postMessage("periodic-sync");
            });
            const finalTags = await registration.periodicSync.getTags();
            globalThis.__serviceWorkerPeriodicSyncProbe = JSON.stringify({
              permission: permission.state,
              pageTags,
              workerResult,
              finalTags
            });
          })().catch(error => {
            globalThis.__serviceWorkerPeriodicSyncProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker periodic sync probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPeriodicSyncProbe)",
        r#"{"permission":"granted","pageTags":["page-periodic"],"workerResult":{"before":["page-periodic"],"mid":["page-periodic","worker-periodic"],"after":["worker-periodic"]},"finalTags":["worker-periodic"]}"#,
    )
    .await;

    server
        .await
        .expect("service worker periodic sync script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_show_notification_records_clickable_notification() {
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
                  windows[0].postMessage(JSON.stringify({
                    title: event.notification && event.notification.title,
                    tag: event.notification && event.notification.tag,
                    actions: event.notification && event.notification.actions &&
                      Array.from(event.notification.actions).map(action => [
                        action.action,
                        action.title,
                        action.icon
                      ].join(":")).join(","),
                    answer: event.notification && event.notification.data && event.notification.data.answer,
                    action: event.action,
                    body: event.notification && event.notification.body,
                    icon: event.notification && event.notification.icon,
                    image: event.notification && event.notification.image,
                    badge: event.notification && event.notification.badge,
                    dir: event.notification && event.notification.dir,
                    lang: event.notification && event.notification.lang,
                    vibrate: event.notification && Array.from(event.notification.vibrate).join("/"),
                    timestamp: event.notification && event.notification.timestamp,
                    renotify: event.notification && event.notification.renotify,
                    silent: event.notification && event.notification.silent,
                    requireInteraction: event.notification && event.notification.requireInteraction
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
          globalThis.__serviceWorkerShowNotificationProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerShowNotificationProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            globalThis.__serviceWorkerShowRegistration = registration;
            try {
              await registration.showNotification("denied");
              globalThis.__serviceWorkerShowNotificationProbe = "denied:resolved";
            } catch (error) {
              globalThis.__serviceWorkerShowNotificationProbe = "denied:" + error.name;
            }
          })().catch(error => {
            globalThis.__serviceWorkerShowNotificationProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker showNotification denied probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerShowNotificationProbe)",
        "denied:TypeError",
    )
    .await;

    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("notifications".to_owned()),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);
    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerShowNotificationProbe = "show-pending";
          (async () => {
            const registration = globalThis.__serviceWorkerShowRegistration;
            await registration.showNotification("first", {
              tag: "same",
              actions: [{ action: "old", title: "Old", icon: "/old.png" }],
              data: { answer: 1 }
            });
            await registration.showNotification("second", {
              tag: "same",
              body: "Second body",
              icon: "/icon.png",
              image: "/image.png",
              badge: "/badge.png",
              dir: "rtl",
              lang: "fr",
              vibrate: [10, 20],
              timestamp: 123456,
              renotify: true,
              silent: true,
              requireInteraction: true,
              actions: [
                {
                  action: "reply",
                  title: "Reply",
                  icon: "/reply.png",
                  navigate: "./reply.html"
                },
                { action: "archive", title: "Archive", icon: "/archive.png" },
                { action: "ignored", title: "Ignored", icon: "/ignored.png" }
              ],
              data: { answer: 2 }
            });
            await registration.showNotification("loose", {
              data: { answer: 3 }
            });
            const notificationInitSetterHits = [];
            const poisonedNames = [
              "data",
              "tag",
              "dir",
              "lang",
              "body",
              "icon",
              "image",
              "badge",
              "vibrate",
              "timestamp",
              "renotify",
              "silent",
              "requireInteraction",
              "action",
              "title",
              "navigate"
            ];
            for (const name of poisonedNames) {
              Object.defineProperty(Object.prototype, name, {
                configurable: true,
                get() { return undefined; },
                set(value) {
                  const receiverKind = this instanceof Notification ? "notification" : "plain";
                  notificationInitSetterHits.push(`${receiverKind}:${name}`);
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
              const tagged = await registration.getNotifications({ tag: "same" });
              const allBeforeClose = await registration.getNotifications();
              tagged[0].close();
              const taggedAfterClose = await registration.getNotifications({ tag: "same" });
              const allAfterClose = await registration.getNotifications();
              await registration.showNotification("hello", {
                body: "Hello body",
                icon: "/hello.png",
                image: "/hello-image.png",
                badge: "/hello-badge.png",
                dir: "ltr",
                lang: "en",
                vibrate: 30,
                timestamp: 987654,
                renotify: false,
                silent: false,
                requireInteraction: true,
                actions: [{ action: "open", title: "Open", icon: "/open.png" }],
                data: { answer: 42 }
              });
              globalThis.__serviceWorkerShowNotificationProbe = JSON.stringify({
                taggedLength: tagged.length,
                taggedTitle: tagged[0] && tagged[0].title,
                taggedTag: tagged[0] && tagged[0].tag,
                taggedActions: tagged[0] && Array.from(tagged[0].actions).map(action => [
                  action.action,
                  action.title,
                  action.icon
                ].join(":")).join(","),
                taggedNavigate: tagged[0] && tagged[0].actions[0] &&
                  tagged[0].actions[0].navigate === new URL("./reply.html", location.href).href,
                taggedOptions: tagged[0] && [
                  tagged[0].body,
                  tagged[0].icon,
                  tagged[0].image,
                  tagged[0].badge,
                  tagged[0].dir,
                  tagged[0].lang,
                  Array.from(tagged[0].vibrate).join("/"),
                  tagged[0].timestamp,
                  tagged[0].renotify,
                  tagged[0].silent,
                  tagged[0].requireInteraction
                ].join("|"),
                taggedAnswer: tagged[0] && tagged[0].data && tagged[0].data.answer,
                taggedOwnDataDescriptor:
                  (tagged[0] && Object.getOwnPropertyDescriptor(tagged[0], "data")) ?? null,
                taggedActionOwnTitleDescriptor: tagged[0] && tagged[0].actions[0] &&
                  Object.getOwnPropertyDescriptor(tagged[0].actions[0], "title"),
                taggedClose: tagged[0] && typeof tagged[0].close,
                allBeforeCloseLength: allBeforeClose.length,
                looseTitle: allBeforeClose.find(notification => !notification.tag).title,
                taggedAfterCloseLength: taggedAfterClose.length,
                allAfterCloseLength: allAfterClose.length,
                notificationInitSetterHits: notificationInitSetterHits
                  .filter(hit => hit.startsWith("plain:"))
              });
            } finally {
              for (const name of poisonedNames) {
                delete Object.prototype[name];
              }
            }
          })().catch(error => {
            globalThis.__serviceWorkerShowNotificationProbe = "show-error:" + error.name;
          });
        })()
        "#,
    )
    .expect("service worker showNotification granted probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerShowNotificationProbe)",
        r#"{"taggedLength":1,"taggedTitle":"second","taggedTag":"same","taggedActions":"reply:Reply:/reply.png,archive:Archive:/archive.png","taggedNavigate":true,"taggedOptions":"Second body|/icon.png|/image.png|/badge.png|rtl|fr|10/20|123456|true|true|true","taggedAnswer":2,"taggedOwnDataDescriptor":null,"taggedActionOwnTitleDescriptor":{"value":"Reply","writable":true,"enumerable":true,"configurable":true},"taggedClose":"function","allBeforeCloseLength":2,"looseTitle":"loose","taggedAfterCloseLength":0,"allAfterCloseLength":1,"notificationInitSetterHits":[]}"#,
    )
    .await;

    assert!(
        browser_context_runtime.dispatch_service_worker_notification_click(
            &url::Url::parse(&format!("{base_url}/app/")).unwrap(),
            "hello",
            "open"
        )
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerShowNotificationProbe)",
        r#"{"title":"hello","tag":"","actions":"open:Open:/open.png","answer":42,"action":"open","body":"Hello body","icon":"/hello.png","image":"/hello-image.png","badge":"/hello-badge.png","dir":"ltr","lang":"en","vibrate":"30","timestamp":987654,"renotify":false,"silent":false,"requireInteraction":true}"#,
    )
    .await;

    server
        .await
        .expect("service worker showNotification script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_notificationclose_removes_record_without_focus_grant() {
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
            self.addEventListener("notificationclose", event => {
              event.waitUntil((async () => {
                const remaining = await self.registration.getNotifications({ tag: "closed" });
                const windows = await clients.matchAll({ includeUncontrolled: true });
                let focusError = null;
                try {
                  if (windows[0]) {
                    await windows[0].focus();
                  }
                } catch (error) {
                  focusError = {
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                  };
                }
                if (windows[0]) {
                  windows[0].postMessage(JSON.stringify({
                    type: event.type,
                    title: event.notification && event.notification.title,
                    tag: event.notification && event.notification.tag,
                    body: event.notification && event.notification.body,
                    answer: event.notification && event.notification.data &&
                      event.notification.data.answer,
                    action: event.action,
                    remainingLength: remaining.length,
                    focusError
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
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("notifications".to_owned()),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerNotificationCloseProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerNotificationCloseProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            await registration.showNotification("closing", {
              tag: "closed",
              body: "Close body",
              data: { answer: 9 }
            });
            globalThis.__serviceWorkerNotificationCloseProbe = "shown";
          })().catch(error => {
            globalThis.__serviceWorkerNotificationCloseProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker notificationclose setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNotificationCloseProbe)",
        "shown",
    )
    .await;

    assert!(
        browser_context_runtime.dispatch_service_worker_notification_close(
            &url::Url::parse(&format!("{base_url}/app/")).unwrap(),
            "closing",
        )
    );

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNotificationCloseProbe)",
        r#"{"type":"notificationclose","title":"closing","tag":"closed","body":"Close body","answer":9,"action":"","remainingLength":0,"focusError":{"name":"InvalidAccessError","message":"Not allowed to focus a window.","isDomException":true}}"#,
    )
    .await;

    server
        .await
        .expect("service worker notificationclose script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_registry_queries_are_scope_based() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/other/worker.js"]).await;
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
              globalThis.__serviceWorkerRegistryProbe = { started: true };
              (async () => {
                const app = await sw.register("worker.js", { scope: "./" });
                const other = await sw.register("/other/worker.js", { scope: "/other/" });
                const current = await sw.getRegistration();
                const appLookup = await sw.getRegistration("deep/page.html#fragment");
                const otherLookup = await sw.getRegistration("/other/page.html#fragment");
                const missing = await sw.getRegistration("/missing/page.html");
                const allBefore = await sw.getRegistrations();
                const firstUnregister = await other.unregister();
                const secondUnregister = await other.unregister();
                const otherAfter = await sw.getRegistration("/other/page.html");
                const allAfter = await sw.getRegistrations();
                globalThis.__serviceWorkerRegistryProbe = {
                  currentScope: current && current.scope,
                  appLookupScope: appLookup && appLookup.scope,
                  otherLookupScope: otherLookup && otherLookup.scope,
                  missingIsUndefined: missing === undefined,
                  allBefore: allBefore.map((registration) => registration.scope).sort().join("|"),
                  firstUnregister,
                  secondUnregister,
                  otherAfterIsUndefined: otherAfter === undefined,
                  allAfter: allAfter.map((registration) => registration.scope).join("|"),
                  controllerScriptURL: sw.controller && sw.controller.scriptURL,
                  appStillRegistered: app instanceof ServiceWorkerRegistration
                };
              })().catch((error) => {
                globalThis.__serviceWorkerRegistryProbe = { error: String(error) };
              });
            })()
            "#,
    )
    .expect("service worker registry query setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(Object.prototype.hasOwnProperty.call(globalThis.__serviceWorkerRegistryProbe, 'allAfter'))",
        "true",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__serviceWorkerRegistryProbe)")
        .expect("service worker registry query promises should settle");

    let app_scope = format!("{base_url}/app/");
    let other_scope = format!("{base_url}/other/");
    let expected_result = format!(
        r#"{{"currentScope":"{app_scope}","appLookupScope":"{app_scope}","otherLookupScope":"{other_scope}","missingIsUndefined":true,"allBefore":"{app_scope}|{other_scope}","firstUnregister":true,"secondUnregister":false,"otherAfterIsUndefined":true,"allAfter":"{app_scope}","controllerScriptURL":null,"appStillRegistered":true}}"#
    );
    assert_eq!(result, expected_result);
    server
        .await
        .expect("service worker registry script server should finish");
}
#[test]
fn navigator_service_worker_argument_errors_reject_with_the_original_exception() {
    let mut vm = new_storage_test_vm("https://service-worker-webidl.test/");
    vm.eval(
        r#"
(() => {
  const sw = navigator.serviceWorker;
  const marker = {sentinel: true};
  globalThis.serviceWorkerArgumentFailures = [];
  globalThis.serviceWorkerArgumentRejections = 0;
  function check(callback, expected) {
    const promise = callback();
    if (!(promise instanceof Promise)) serviceWorkerArgumentFailures.push('not a Promise');
    promise.then(
      () => serviceWorkerArgumentFailures.push('resolved'),
      error => {
        serviceWorkerArgumentRejections++;
        if (expected === marker ? error !== marker : !(error instanceof TypeError)) {
          serviceWorkerArgumentFailures.push('wrong rejection reason');
        }
      }
    );
  }
  check(() => sw.register(), TypeError);
  check(() => sw.register(Symbol('script')), TypeError);
  check(() => sw.register('https://[', {scope: Symbol('scope')}), TypeError);
  check(() => sw.getRegistration({toString() { throw marker; }}), marker);
  const steps = ['script', 'scope-get', 'scope-string', 'type-get', 'type-string', 'cache-get', 'cache-string'];
  for (let index = 0; index < steps.length; index++) {
    const log = [];
    const step = name => { log.push(name); if (name === steps[index]) throw marker; };
    check(() => sw.register(
      {toString() { step('script'); return 'https://['; }},
      {
        get scope() { step('scope-get'); return {toString() { step('scope-string'); return './'; }}; },
        get type() { step('type-get'); return {toString() { step('type-string'); return 'classic'; }}; },
        get updateViaCache() { step('cache-get'); return {toString() { step('cache-string'); return 'imports'; }}; }
      }
    ), marker);
    if (JSON.stringify(log) !== JSON.stringify(steps.slice(0, index + 1))) {
      serviceWorkerArgumentFailures.push('conversion order: ' + log);
    }
  }
})()
"#,
    )
    .expect("ServiceWorkerContainer conversion errors should return Promises");
    assert_eq!(
        vm.eval("JSON.stringify([serviceWorkerArgumentRejections, serviceWorkerArgumentFailures])")
            .unwrap(),
        "[11,[]]"
    );
}
#[tokio::test]
async fn navigator_service_worker_url_arguments_follow_webidl_and_origin_rules() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/worker.js", "/resources/worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              const sw = navigator.serviceWorker;
              const rejectionName = promise => promise.then(
                () => "resolved",
                error => error && error.name
              );
              globalThis.__serviceWorkerUrlArgumentProbe = { state: "pending" };
              (async () => {
                const registration = await sw.register("/worker.js", { scope: "null" });
                const nullClientMatches = await sw.getRegistration(null) === registration;
                const crossOrigin = await rejectionName(
                  sw.getRegistration("http://example.com/")
                );
                const invalidClientUrl = await rejectionName(
                  sw.getRegistration("https://[")
                );
                const nullScope = await rejectionName(
                  sw.register("/resources/worker.js", { scope: null })
                );
                const nullType = await rejectionName(
                  sw.register("/worker.js", { type: null })
                );
                const nullUpdateViaCache = await rejectionName(
                  sw.register("/worker.js", { updateViaCache: null })
                );
                const primitiveOptions = await rejectionName(
                  sw.register("/worker.js", 1)
                );
                const symbolClient = await rejectionName(
                  sw.getRegistration(Symbol("client"))
                );
                const unregistered = await registration.unregister();
                globalThis.__serviceWorkerUrlArgumentProbe = {
                  state: "done",
                  nullClientMatches,
                  crossOrigin,
                  invalidClientUrl,
                  nullScope,
                  nullType,
                  nullUpdateViaCache,
                  primitiveOptions,
                  symbolClient,
                  unregistered
                };
              })().catch(error => {
                globalThis.__serviceWorkerUrlArgumentProbe = {
                  state: "error",
                  error: String(error)
                };
              });
            })()
            "#,
    )
    .expect("service worker URL argument probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUrlArgumentProbe.state !== 'pending')",
        "true",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__serviceWorkerUrlArgumentProbe)")
            .expect("service worker URL argument result should evaluate"),
        r#"{"state":"done","nullClientMatches":true,"crossOrigin":"SecurityError","invalidClientUrl":"TypeError","nullScope":"SecurityError","nullType":"TypeError","nullUpdateViaCache":"TypeError","primitiveOptions":"TypeError","symbolClient":"TypeError","unregistered":true}"#
    );
    server
        .await
        .expect("service worker URL argument script server should finish");
}

#[tokio::test]
async fn navigator_service_worker_update_check_preserves_mime_security_error() {
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
        r#"{"name":"SecurityError","isTypeError":false,"isDomException":true,"messageIncludesNosniff":true}"#,
    )
    .await;

    server
        .await
        .expect("service worker update failure server should finish");
}

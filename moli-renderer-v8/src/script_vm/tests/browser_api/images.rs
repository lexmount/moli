use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const ONE_BY_ONE_GIF: &[u8] = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;";

fn one_by_one_gif_response_body() -> crate::runtime::RendererSyntheticResponseBody {
    crate::runtime::RendererSyntheticResponseBody::from_bytes(ONE_BY_ONE_GIF.to_vec())
}

async fn spawn_image_response_server(
    status_line: &'static str,
    content_type: &'static str,
    body: &'static [u8],
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind image response server");
    let addr = listener.local_addr().expect("image response server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept image request");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let count = stream.read(&mut chunk).await.expect("read image request");
            assert_ne!(count, 0, "image request ended before its header");
            request.extend_from_slice(&chunk[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&request).into_owned();
        let _ = request_tx.send(request);
        let head = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(head.as_bytes())
            .await
            .expect("write image response head");
        stream
            .write_all(body)
            .await
            .expect("write image response body");
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn wait_for_and_apply_image_completions(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
) {
    wait_for_one_page_resource_completion_executor_test_turn(page, "image request").await;
    while page
        .apply_one_page_resource_terminal_owner_admission()
        .expect("ready image resource completion should apply through its Page owner")
    {}
}

async fn run_next_image_event_task(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    label: &str,
) {
    assert!(
        !page.has_ready_timeout(),
        "{label}: image terminal event must not create a synthetic Page timer"
    );
    wait_for_image_event_task(page, label).await;
    assert!(
        page.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            loader,
        )
        .await
        .unwrap_or_else(|error| panic!("{label}: {error:#}")),
        "{label}: DOM-manipulation source should retain the image event task"
    );
}

async fn wait_for_image_event_task(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    label: &str,
) {
    assert!(
        {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if page.has_ready_dom_manipulation_family_for_test(
                    PageDomManipulationTestFamily::ImageLoadEvent,
                ) {
                    break true;
                }
                let arrived =
                    tokio::time::timeout_at(deadline, page.wait_for_task_executor_work_arrival())
                        .await
                        .unwrap_or_else(|_| panic!("{label}: image event task did not arrive"));
                if !arrived {
                    break false;
                }
            }
        },
        "{label}: image event task route closed before publishing its task"
    );
}

#[tokio::test]
async fn render_image_decode_requires_both_real_layout_and_image_fetch() {
    let cases = [
        (moli_page_types::LayoutPolicy::OnDemand, true, true),
        (moli_page_types::LayoutPolicy::OnDemand, false, false),
        (moli_page_types::LayoutPolicy::Mock, true, false),
        (moli_page_types::LayoutPolicy::Mock, false, false),
    ];

    for (index, (layout_policy, image_fetch_enabled, expect_pixels)) in
        cases.into_iter().enumerate()
    {
        let loader =
            ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
        loader.set_image_fetch_enabled(image_fetch_enabled);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            &format!("https://image-decode-gate-{index}.test/"),
            &loader,
        );
        vm.set_layout_policy(layout_policy);
        vm.eval(
            r#"
            (() => {
              const image = document.createElement("img");
              image.id = "decode-gate-image";
              globalThis.__decodeGateEvent = "pending";
              image.onload = () => { globalThis.__decodeGateEvent = "load"; };
              image.onerror = () => { globalThis.__decodeGateEvent = "error"; };
              (document.body || document.documentElement || document).appendChild(image);
              image.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
            })()
            "#,
        )
        .expect("decode gate image should start");

        run_next_image_event_task(&mut vm, &loader, "decode gate image event").await;
        assert_eq!(
            vm.eval("globalThis.__decodeGateEvent")
                .expect("decode gate event should be readable"),
            "load"
        );
        let image = vm
            .document_runtime
            .get_element_by_id("decode-gate-image")
            .expect("decode gate image handle");
        let ready = vm
            ._context_host
            .borrow()
            .ready_image_for_layout(image)
            .expect("successful local image retains metadata");
        assert_eq!((ready.intrinsic_width, ready.intrinsic_height), (1.0, 1.0));
        assert_eq!(
            ready.pixels.is_some(),
            expect_pixels,
            "layout={layout_policy:?}, image_fetch_enabled={image_fetch_enabled}"
        );
    }
}

#[tokio::test]
async fn image_fetch_enabled_queues_empty_source_error_after_microtask_checkpoint() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://empty-image-source.test/page.html",
        &loader,
    );

    let before_listener = vm
        .eval(
            r#"
            (() => {
              const image = document.createElement("img");
              (document.body || document.documentElement || document).appendChild(image);
              globalThis.__emptySourceImage = image;
              globalThis.__emptySourceResult = "pending";
              image.src = "";
              return globalThis.__emptySourceResult;
            })()
            "#,
        )
        .expect("empty image source setup should evaluate");
    assert_eq!(before_listener, "pending");

    vm.eval(
        r#"
        (() => {
          __emptySourceImage.addEventListener("error", () => {
            globalThis.__emptySourceResult = "error";
          });
        })()
        "#,
    )
    .expect("empty image source listener should install");

    run_next_image_event_task(&mut vm, &loader, "empty image source error").await;
    assert_eq!(
        vm.eval("globalThis.__emptySourceResult")
            .expect("empty image source result should evaluate"),
        "error"
    );
}

#[tokio::test]
async fn image_fetch_enabled_discards_stale_queued_terminal_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stale-image-event.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          (document.body || document.documentElement || document).appendChild(image);
          globalThis.__staleImage = image;
          globalThis.__staleImageEvents = [];
          image.src = "";
        })()
        "#,
    )
    .expect("empty image source should queue its update");

    vm.eval(
        r#"
        (() => {
          __staleImage.addEventListener("error", () => __staleImageEvents.push("error"));
          __staleImage.addEventListener("load", () => __staleImageEvents.push("load"));
          __staleImage.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==";
        })()
        "#,
    )
    .expect("replacement image source should queue its update");

    run_next_image_event_task(&mut vm, &loader, "stale empty-source image event").await;
    assert_eq!(
        vm.eval("globalThis.__staleImageEvents.join(',')")
            .expect("stale image events should evaluate"),
        ""
    );

    run_next_image_event_task(&mut vm, &loader, "replacement image event").await;
    assert_eq!(
        vm.eval("globalThis.__staleImageEvents.join(',')")
            .expect("replacement image events should evaluate"),
        "load"
    );
}

#[tokio::test]
async fn image_current_src_switches_only_when_the_update_microtask_selects_a_current_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let (mut vm, _resource_completion_queue) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(
            "https://image-current-request.test/page.html",
            &loader,
        );

    let before_update = vm
        .eval(
            r#"
            (() => {
              const picture = document.createElement("picture");
              const nonMatchingSource = document.createElement("source");
              nonMatchingSource.media = "not all";
              nonMatchingSource.srcset = "data:,a";
              picture.append(nonMatchingSource);

              const matchingSource = document.createElement("source");
              matchingSource.media = "all";
              matchingSource.srcset = "data:,b";
              picture.append(matchingSource);

              const image = document.createElement("img");
              globalThis.__imageCurrentRequestObservations = [];
              image.src = "data:,c";
              const beforeUpdate = image.currentSrc;
              queueMicrotask(() => {
                __imageCurrentRequestObservations.push(`first:${image.currentSrc}`);
                picture.append(image);
                __imageCurrentRequestObservations.push(`inserted:${image.currentSrc}`);
                queueMicrotask(() => {
                  __imageCurrentRequestObservations.push(`second:${image.currentSrc}`);
                });
              });
              return beforeUpdate;
            })()
            "#,
        )
        .expect("image current request setup should evaluate");
    assert_eq!(before_update, "");
    assert_eq!(
        vm.eval("globalThis.__imageCurrentRequestObservations.join('|')")
            .expect("image current request observations should evaluate"),
        "first:data:,c|inserted:data:,c|second:data:,b"
    );
}

#[tokio::test]
async fn image_current_src_is_empty_while_a_replacement_request_is_pending() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let (mut vm, _resource_completion_queue) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(
            "https://image-current-request.test/page.html",
            &loader,
        );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));

    vm.eval(
        r#"
        (() => {
          globalThis.__pendingCurrentRequestImage = new Image();
          __pendingCurrentRequestImage.src = "data:,old";
        })()
        "#,
    )
    .expect("initial synthetic image request should evaluate");
    assert_eq!(
        vm.eval("globalThis.__pendingCurrentRequestImage.currentSrc")
            .expect("initial current source should evaluate"),
        "data:,old"
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const before = __pendingCurrentRequestImage.currentSrc;
              __pendingCurrentRequestImage.src = "replacement.gif";
              return `${before}|${__pendingCurrentRequestImage.currentSrc}`;
            })()
            "#,
        )
        .expect("replacement image request should evaluate"),
        "data:,old|data:,old"
    );
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        vm.eval("globalThis.__pendingCurrentRequestImage.currentSrc")
            .expect("pending current source should evaluate"),
        ""
    );

    vm.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("replacement image request should fulfill");
    assert_eq!(
        vm.eval("globalThis.__pendingCurrentRequestImage.currentSrc")
            .expect("completed current source should evaluate"),
        "https://image-current-request.test/replacement.gif"
    );
}

#[tokio::test]
async fn picture_source_and_image_tree_mutations_reselect_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let (mut vm, _resource_completion_queue) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(
            "https://picture-source-removal.test/page.html",
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.append(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.append(document.createElement("body"));
          }
          const picture = document.createElement("picture");
          const source1 = document.createElement("source");
          const source2 = document.createElement("source");
          const source3 = document.createElement("source");
          const source4 = document.createElement("source");
          const source5 = document.createElement("source");
          const image1 = document.createElement("img");
          const image2 = document.createElement("img");
          source1.srcset = "data:,s1";
          source2.srcset = "data:,s2";
          source3.srcset = "data:,s3";
          source4.srcset = "data:,s4";
          source5.srcset = "data:,s5";
          image1.src = "data:,img1";
          image2.src = "data:,img2";
          picture.append(source1, source2, source3, image1, image2, source4, source5);
          const host = document.body;
          host.append(picture);
          globalThis.__pictureSourceMutation = {
            host,
            picture,
            source1,
            source2,
            source3,
            source4,
            source5,
            image1,
            image2
          };
        })()
        "#,
    )
    .expect("picture source mutation setup should evaluate");

    let current_sources = |vm: &mut ScriptVm| {
        vm.eval(
            "[__pictureSourceMutation.image1.currentSrc, \
             __pictureSourceMutation.image2.currentSrc].join(',')",
        )
        .expect("picture image current sources should evaluate")
    };
    assert_eq!(current_sources(&mut vm), "data:,s1,data:,s1");

    vm.eval("__pictureSourceMutation.source1.remove()")
        .expect("first picture source removal should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,s2,data:,s2");

    vm.eval("__pictureSourceMutation.source2.remove()")
        .expect("second picture source removal should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,s3,data:,s3");

    vm.eval("__pictureSourceMutation.host.append(__pictureSourceMutation.source3)")
        .expect("third picture source move should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,img1,data:,img2");

    vm.eval("__pictureSourceMutation.source4.remove()")
        .expect("trailing picture source removal should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,img1,data:,img2");

    vm.eval("__pictureSourceMutation.host.append(__pictureSourceMutation.source5)")
        .expect("trailing picture source move should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,img1,data:,img2");

    vm.eval("__pictureSourceMutation.picture.prepend(__pictureSourceMutation.source2)")
        .expect("second picture source reinsertion should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,s2,data:,s2");

    vm.eval("__pictureSourceMutation.picture.prepend(__pictureSourceMutation.source1)")
        .expect("first picture source reinsertion should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,s1,data:,s1");

    vm.eval("__pictureSourceMutation.source1.replaceWith(document.createElement('div'))")
        .expect("first picture source replacement should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,s2,data:,s2");

    vm.eval("__pictureSourceMutation.image1.remove()")
        .expect("first picture image removal should evaluate");
    assert_eq!(current_sources(&mut vm), "data:,img1,data:,s2");
}

#[tokio::test]
async fn image_fetch_enabled_dispatches_http_error_after_resource_timing() {
    let (base_url, request_rx, server) =
        spawn_image_response_server("404 Not Found", "text/html", b"missing").await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{base_url}/page.html"),
        &loader,
    );

    let before = vm
        .eval(
            r#"
            (() => {
              const image = document.createElement("img");
              const host = document.body || document.documentElement || document;
              host.appendChild(image);
              const expectedUrl = new URL("missing.png", location.href).href;
              globalThis.__imageNetworkResult = "pending";
              image.addEventListener("error", () => {
                const entries = performance.getEntriesByName(expectedUrl);
                globalThis.__imageNetworkResult = [
                  "error",
                  entries.length,
                  entries[0]?.responseStatus
                ].join(":");
              });
              image.src = "missing.png";
              return globalThis.__imageNetworkResult;
            })()
            "#,
        )
        .expect("image network setup should evaluate");
    assert_eq!(before, "pending");

    wait_for_and_apply_image_completions(&mut vm).await;

    assert_eq!(
        vm.eval("globalThis.__imageNetworkResult")
            .expect("image network terminal result should evaluate"),
        "pending",
        "network completion must only enqueue the element event task"
    );
    run_next_image_event_task(&mut vm, &loader, "HTTP image error event").await;

    let after = vm
        .eval("globalThis.__imageNetworkResult")
        .expect("image network result should evaluate");
    assert_eq!(after, "error:1:404");
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .expect("image server should receive a request")
        .expect("image request capture should remain connected");
    assert!(request.starts_with("GET /missing.png HTTP/1.1\r\n"));
    server.await.expect("image response server should finish");
}

#[tokio::test]
async fn image_fetch_enabled_dispatches_load_after_resource_timing() {
    let (base_url, request_rx, server) =
        spawn_image_response_server("200 OK", "image/gif", ONE_BY_ONE_GIF).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{base_url}/page.html"),
        &loader,
    );

    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          (document.body || document.documentElement || document).appendChild(image);
          const expectedUrl = new URL("image.png", location.href).href;
          globalThis.__imageNetworkResult = "pending";
          image.addEventListener("load", () => {
            const entries = performance.getEntriesByName(expectedUrl);
            globalThis.__imageNetworkResult = [
              "load",
              entries.length,
              entries[0]?.responseStatus
            ].join(":");
          });
          image.src = "image.png";
        })()
        "#,
    )
    .expect("image load setup should evaluate");

    wait_for_and_apply_image_completions(&mut vm).await;

    assert_eq!(
        vm.eval("globalThis.__imageNetworkResult")
            .expect("image network terminal result should evaluate"),
        "pending",
        "network completion must only enqueue the element event task"
    );
    run_next_image_event_task(&mut vm, &loader, "HTTP image load event").await;

    assert_eq!(
        vm.eval("globalThis.__imageNetworkResult")
            .expect("image load result should evaluate"),
        "load:1:200"
    );
    let request = request_rx
        .await
        .expect("image request capture should remain connected");
    assert!(request.starts_with("GET /image.png HTTP/1.1\r\n"));
    server.await.expect("image response server should finish");
}

#[tokio::test]
async fn image_fetch_enabled_rejects_http_success_with_corrupt_image_bytes() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://corrupt-image.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));

    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          globalThis.__corruptImage = image;
          globalThis.__corruptImageEvents = [];
          image.addEventListener("load", () => __corruptImageEvents.push("load"));
          image.addEventListener("error", () => __corruptImageEvents.push("error"));
          image.src = "corrupt.jpg";
          (document.body || document.documentElement || document).appendChild(image);
        })()
        "#,
    )
    .expect("corrupt image request should start");
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);

    vm.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/jpeg".to_owned())],
        crate::runtime::RendererSyntheticResponseBody::from_bytes(
            b"<html>not an image</html>".to_vec(),
        ),
    )
    .expect("corrupt image response should complete");

    assert_eq!(
        vm.eval("__corruptImageEvents.join('|')")
            .expect("corrupt image terminal trace should evaluate"),
        "",
        "interception completion must not inline-dispatch the error event"
    );
    run_next_image_event_task(&mut vm, &loader, "corrupt image error event").await;

    assert_eq!(
        vm.eval("__corruptImageEvents.join('|') + ':' + __corruptImage.complete")
            .expect("corrupt image result should evaluate"),
        "error:true"
    );
}

#[tokio::test]
async fn inserting_completed_detached_image_does_not_restart_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://image-insertion.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));

    vm.eval(
        r#"
        (() => {
          const image = new Image();
          globalThis.__completedDetachedImage = image;
          globalThis.__completedDetachedImageEvents = [];
          image.addEventListener("load", () => __completedDetachedImageEvents.push("load"));
          image.addEventListener("error", () => __completedDetachedImageEvents.push("error"));
          image.src = "https://image-insertion.test/image.gif";
        })()
        "#,
    )
    .expect("detached image request should start");
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    vm.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("detached image response should complete");

    vm.eval(
        r#"
        (() => {
          const root = document.documentElement ||
            document.appendChild(document.createElement("html"));
          const head = document.head ||
            root.insertBefore(document.createElement("head"), root.firstChild);
          const body = document.body || root.appendChild(document.createElement("body"));
          const base = document.createElement("base");
          base.href = "https://bogus-image-base.test/";
          head.appendChild(base);
          body.appendChild(__completedDetachedImage);
        })()
        "#,
    )
    .expect("completed detached image should insert");

    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "ordinary insertion must not restart the completed request"
    );
    run_next_image_event_task(&mut vm, &loader, "detached image load event").await;
    assert_eq!(
        vm.eval("__completedDetachedImageEvents.join('|')")
            .expect("detached image events should evaluate"),
        "load"
    );
}

#[tokio::test]
async fn image_update_microtask_freezes_request_after_observing_live_base_url() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://image-update-base.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));

    vm.eval(
        r#"
        (() => {
          const image = new Image();
          globalThis.__baseSelectedImage = image;
          globalThis.__baseSelectedImageEvents = [];
          image.onload = () => __baseSelectedImageEvents.push("load");
          image.onerror = () => __baseSelectedImageEvents.push("error");
          image.src = "green.png";

          const root = document.documentElement ||
            document.appendChild(document.createElement("html"));
          const head = document.head ||
            root.insertBefore(document.createElement("head"), root.firstChild);
          const base = document.createElement("base");
          base.href = "/resources/";
          head.appendChild(base);
          globalThis.__baseSelectedImageBase = base;
        })()
        "#,
    )
    .expect("image and base should update in one script turn");

    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].url.as_str(),
        "https://image-update-base.test/resources/green.png",
        "URL selection must happen in the queued image-update microtask"
    );
    vm.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("base-selected image response should complete");

    vm.eval(
        r#"
        (() => {
          __baseSelectedImageBase.href = "/bogus/";
          (document.body || document.documentElement).appendChild(__baseSelectedImage);
        })()
        "#,
    )
    .expect("completed detached image should insert after a later base change");
    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "ordinary insertion must preserve the request selected by the update microtask"
    );

    run_next_image_event_task(&mut vm, &loader, "base-selected image event").await;
    assert_eq!(
        vm.eval(
            r#"__baseSelectedImageEvents.join("|") + ":" +
               performance.getEntriesByName(
                 "https://image-update-base.test/resources/green.png"
               ).length"#,
        )
        .expect("base-selected image result should evaluate"),
        "load:1"
    );
}

#[tokio::test]
async fn removed_lazy_image_suppresses_in_flight_terminal_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://removed-lazy-image.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));

    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          globalThis.__removedLazyImage = image;
          globalThis.__removedLazyImageEvents = [];
          image.loading = "lazy";
          image.addEventListener("load", () => __removedLazyImageEvents.push("load"));
          image.addEventListener("error", () => __removedLazyImageEvents.push("error"));
          image.src = "image.gif";
          (document.body || document.documentElement || document).appendChild(image);
        })()
        "#,
    )
    .expect("lazy image request should start");
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("near lazy-image layout refresh should succeed")
    );
    vm.perform_owner_lane_task_microtask_checkpoints()
        .expect("layout task should finish its image-update microtask checkpoint");
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);

    vm.eval("__removedLazyImage.remove()")
        .expect("lazy image should be removed");
    vm.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("removed lazy image response should complete");

    run_next_image_event_task(&mut vm, &loader, "removed lazy image terminal").await;

    assert_eq!(
        vm.eval("__removedLazyImageEvents.join('|')")
            .expect("removed lazy image events should evaluate"),
        ""
    );
}

#[tokio::test]
async fn image_fetch_disabled_keeps_synthetic_event_without_network_request() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind disabled image probe server");
    let base_url = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("disabled image probe server addr")
    );
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{base_url}/page.html"),
        &loader,
    );

    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          (document.body || document.documentElement || document).appendChild(image);
          globalThis.__imageDisabledResult = "pending";
          image.addEventListener("load", () => {
            globalThis.__imageDisabledResult = "load";
          });
          image.addEventListener("error", () => {
            globalThis.__imageDisabledResult = "error";
          });
          image.src = "not-fetched.png";
        })()
        "#,
    )
    .expect("disabled image setup should evaluate");
    run_next_image_event_task(&mut vm, &loader, "disabled-fetch image event").await;

    assert_eq!(
        vm.eval("globalThis.__imageDisabledResult")
            .expect("disabled image result should evaluate"),
        "load"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "default image policy must not issue an HTTP request"
    );
}

#[tokio::test]
async fn replacing_image_source_suppresses_stale_network_completion() {
    let (slow_base_url, slow_request_rx, slow_server) =
        spawn_image_response_server("200 OK", "image/gif", ONE_BY_ONE_GIF).await;
    let (fast_base_url, fast_request_rx, fast_server) =
        spawn_image_response_server("200 OK", "image/gif", ONE_BY_ONE_GIF).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{slow_base_url}/page.html"),
        &loader,
    );

    vm.eval(&format!(
        r#"
        (() => {{
          const image = document.createElement("img");
          (document.body || document.documentElement || document).appendChild(image);
          globalThis.__imageReplacementEvents = [];
          globalThis.__replacementImage = image;
          image.addEventListener("load", () => {{
            globalThis.__imageReplacementEvents.push("load:" + image.currentSrc);
          }});
          image.addEventListener("error", () => {{
            globalThis.__imageReplacementEvents.push("error:" + image.currentSrc);
          }});
          image.src = {slow_url:?};
        }})()
        "#,
        slow_url = format!("{slow_base_url}/slow.png"),
    ))
    .expect("first image source should evaluate");
    slow_request_rx
        .await
        .expect("first image request should reach its server");
    slow_server
        .await
        .expect("first image response server should finish");

    vm.eval(&format!(
        "globalThis.__replacementImage.src = {:?}",
        format!("{fast_base_url}/fast.png")
    ))
    .expect("replacement image source should evaluate");
    let image = vm
        .document_runtime
        .query_selector(None, "img")
        .expect("replacement image selector")
        .expect("replacement image handle");
    fast_request_rx
        .await
        .expect("replacement image request should reach its server");
    fast_server
        .await
        .expect("replacement image response server should finish");
    for _ in 0..2 {
        wait_for_and_apply_image_completions(&mut vm).await;
        if vm
            ._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some_and(|pending| pending.network_request_id().is_none())
        {
            break;
        }
    }
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some_and(|pending| pending.network_request_id().is_none()),
        "the exact replacement request must reach a terminal state"
    );
    assert_eq!(
        vm.eval("globalThis.__imageReplacementEvents.join('|')")
            .expect("pre-event replacement trace should evaluate"),
        "",
        "resource completion must not inline-dispatch the replacement event"
    );
    run_next_image_event_task(&mut vm, &loader, "replacement network image event").await;
    assert_eq!(
        vm.eval("globalThis.__imageReplacementEvents.join('|')")
            .expect("replacement events should evaluate"),
        format!("load:{fast_base_url}/fast.png")
    );
    assert_eq!(
        vm.eval(&format!(
            "performance.getEntriesByName({:?}).length + ':' + performance.getEntriesByName({:?}).length",
            format!("{slow_base_url}/slow.png"),
            format!("{fast_base_url}/fast.png"),
        ))
        .expect("replacement resource timing counts should evaluate"),
        "0:1"
    );
}

#[tokio::test]
async fn replacing_image_source_discards_queued_stale_decode_pixels() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          image.id = "stale-decode-image";
          globalThis.__staleDecodeEvents = [];
          image.addEventListener("load", () => {
            globalThis.__staleDecodeEvents.push(`load:${image.currentSrc}`);
          });
          image.addEventListener("error", () => {
            globalThis.__staleDecodeEvents.push(`error:${image.currentSrc}`);
          });
          (document.body || document.documentElement || document).appendChild(image);
          image.src = "/images/old.png";
        })()
        "#,
    )
    .expect("old image source should evaluate");
    let old_request = vm.take_pending_subresource_fetch_infos();
    assert_eq!(old_request.len(), 1);
    let old_pixels =
        moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 255]).expect("valid old image pixels");
    let old_png = moli_image::encode_png(&old_pixels).expect("old PNG should encode");
    vm.fulfill_pending_subresource_fetch(
        old_request[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/png".to_owned())],
        crate::runtime::RendererSyntheticResponseBody::from_bytes(old_png.bytes),
    )
    .expect("old image response should fulfill");
    wait_for_image_event_task(&mut vm, "old image decode completion").await;

    vm.eval("document.getElementById('stale-decode-image').src = '/images/new.png'")
        .expect("replacement image source should evaluate");
    let new_request = vm.take_pending_subresource_fetch_infos();
    assert_eq!(new_request.len(), 1);
    let image = vm
        .document_runtime
        .get_element_by_id("stale-decode-image")
        .expect("stale decode image handle");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("stale image decode task should retire cleanly"),
        "the queued stale decode task must remain owned by its task source"
    );
    assert!(
        vm._context_host
            .borrow()
            .ready_image_for_layout(image)
            .is_none(),
        "old decoded pixels must not commit into the replacement resource slot"
    );
    assert_eq!(
        vm.eval("globalThis.__staleDecodeEvents.join('|')")
            .expect("stale decode event trace should evaluate"),
        "",
        "retiring the stale decode completion must not dispatch an element event"
    );

    let new_pixels = moli_image::RgbaImage::try_new(1, 1, vec![0, 0, 255, 255])
        .expect("valid replacement image pixels");
    let new_png = moli_image::encode_png(&new_pixels).expect("new PNG should encode");
    vm.fulfill_pending_subresource_fetch(
        new_request[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/png".to_owned())],
        crate::runtime::RendererSyntheticResponseBody::from_bytes(new_png.bytes),
    )
    .expect("replacement image response should fulfill");
    run_next_image_event_task(&mut vm, &loader, "replacement image decode completion").await;

    assert_eq!(
        vm.eval("globalThis.__staleDecodeEvents.join('|')")
            .expect("replacement image event trace should evaluate"),
        "load:https://example.test/images/new.png"
    );
    let ready = vm
        ._context_host
        .borrow()
        .ready_image_for_layout(image)
        .and_then(|ready| ready.pixels)
        .expect("replacement image pixels should be ready");
    assert_eq!(ready.rgba.as_slice(), &[0, 0, 255, 255]);
}

#[tokio::test]
async fn image_request_interception_fulfills_through_image_continuation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.test/page.html",
        &loader,
    );
    vm.set_extra_http_headers(&[("X-Image-Request".to_owned(), "intercepted".to_owned())]);
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          (document.body || document.documentElement || document).appendChild(image);
          const expectedUrl = new URL("paused.png", location.href).href;
          globalThis.__interceptedImageResult = "pending";
          image.addEventListener("error", () => {
            const entries = performance.getEntriesByName(expectedUrl);
            globalThis.__interceptedImageResult = [
              "error",
              entries.length,
              entries[0]?.responseStatus
            ].join(":");
          });
          image.src = expectedUrl;
        })()
        "#,
    )
    .expect("intercepted image setup should evaluate");
    let mut pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    let pending = pending.remove(0);
    assert_eq!(
        pending.resource_type,
        crate::types::SubresourceResourceType::Image
    );
    assert_eq!(pending.url.as_str(), "https://example.test/paused.png");
    assert_eq!(
        pending.request_headers,
        vec![("X-Image-Request".to_owned(), "intercepted".to_owned())],
        "interception must observe the request headers frozen at request start"
    );
    vm.fulfill_pending_subresource_fetch(
        pending.internal_id,
        404,
        vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        crate::runtime::RendererSyntheticResponseBody::empty(),
    )
    .expect("intercepted image request should fulfill");

    assert_eq!(
        vm.eval("globalThis.__interceptedImageResult")
            .expect("intercepted image terminal result should evaluate"),
        "pending",
        "interception completion must only enqueue the element event task"
    );
    run_next_image_event_task(&mut vm, &loader, "intercepted image event").await;

    assert_eq!(
        vm.eval("globalThis.__interceptedImageResult")
            .expect("intercepted image result should evaluate"),
        "error:1:404"
    );
}

#[tokio::test]
async fn image_decode_quick_attach_distinguishes_uncached_and_ready_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));

    vm.eval(
        r#"
        (() => {
          const picture = document.createElement("picture");
          (document.body || document.documentElement || document).appendChild(picture);
          const image = new Image();
          image.id = "first-shared-decode";
          image.src = "/images/green.png?shared-decode";
          globalThis.__firstSharedDecode = "pending";
          image.decode().then(
            () => { globalThis.__firstSharedDecode = "resolve"; },
            error => { globalThis.__firstSharedDecode = `reject:${error.name}`; }
          );
          picture.appendChild(image);
        })()
        "#,
    )
    .expect("first image decode should evaluate");
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        vm.eval("globalThis.__firstSharedDecode")
            .expect("uncached quick-attach image decode state should evaluate"),
        "reject:EncodingError"
    );
    vm.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("in-flight image response should fulfill");
    run_next_image_event_task(&mut vm, &loader, "in-flight image load event").await;
    assert_eq!(
        vm.eval("globalThis.__firstSharedDecode")
            .expect("first image decode result should evaluate"),
        "reject:EncodingError"
    );
    let first_image = vm
        .document_runtime
        .get_element_by_id("first-shared-decode")
        .expect("first shared image handle");
    let first_pixels = vm
        ._context_host
        .borrow()
        .ready_image_for_layout(first_image)
        .and_then(|ready| ready.pixels)
        .expect("first shared image pixels");

    vm.eval(
        r#"
        (() => {
          const picture = document.createElement("picture");
          (document.body || document.documentElement || document).appendChild(picture);
          const image = new Image();
          image.id = "second-shared-decode";
          image.src = "/images/green.png?shared-decode";
          globalThis.__secondSharedDecode = "pending";
          image.decode().then(
            () => { globalThis.__secondSharedDecode = "resolve"; },
            error => { globalThis.__secondSharedDecode = `reject:${error.name}`; }
          );
          picture.appendChild(image);
        })()
        "#,
    )
    .expect("second image decode should evaluate");

    assert_eq!(
        vm.eval("globalThis.__secondSharedDecode")
            .expect("second image decode result should evaluate"),
        "resolve"
    );
    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "an active ready resource must not start another fetch or decode"
    );
    let second_image = vm
        .document_runtime
        .get_element_by_id("second-shared-decode")
        .expect("second shared image handle");
    let second_pixels = vm
        ._context_host
        .borrow()
        .ready_image_for_layout(second_image)
        .and_then(|ready| ready.pixels)
        .expect("second shared image pixels");
    assert!(
        std::sync::Arc::ptr_eq(&first_pixels, &second_pixels),
        "same exact request must share one decoded RGBA allocation"
    );
}

#[tokio::test]
async fn changing_image_cross_origin_restarts_intercepted_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        (() => {
          const image = document.createElement("img");
          (document.body || document.documentElement || document).appendChild(image);
          globalThis.__crossOriginImage = image;
          globalThis.__crossOriginImageEvents = [];
          image.addEventListener("load", () => {
            globalThis.__crossOriginImageEvents.push(
              `load:${image.crossOrigin}:${performance.getEntriesByName(image.currentSrc).length}`
            );
          });
          image.src = "same-origin.png";
        })()
        "#,
    )
    .expect("initial image request should evaluate");
    let first = vm.take_pending_subresource_fetch_infos();
    assert_eq!(first.len(), 1);

    vm.eval("globalThis.__crossOriginImage.crossOrigin = 'anonymous'")
        .expect("crossOrigin mutation should evaluate");
    let second = vm.take_pending_subresource_fetch_infos();
    assert_eq!(
        second.len(),
        1,
        "crossOrigin must restart the image request"
    );
    assert_ne!(first[0].internal_id, second[0].internal_id);
    assert!(
        vm.fulfill_pending_subresource_fetch(
            first[0].internal_id,
            200,
            vec![("Content-Type".to_owned(), "image/png".to_owned())],
            crate::runtime::RendererSyntheticResponseBody::empty(),
        )
        .is_err(),
        "the replaced request must no longer be fulfillable"
    );

    vm.eval("globalThis.__crossOriginImage.crossOrigin = 'invalid-token'")
        .expect("equivalent crossOrigin mutation should evaluate");
    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "equivalent anonymous CORS states must not restart the request"
    );

    vm.fulfill_pending_subresource_fetch(
        second[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("replacement image request should fulfill");
    assert_eq!(
        vm.eval("globalThis.__crossOriginImageEvents.join('|')")
            .expect("pre-event replacement image trace should evaluate"),
        ""
    );
    run_next_image_event_task(&mut vm, &loader, "cross-origin replacement image event").await;
    assert_eq!(
        vm.eval("globalThis.__crossOriginImageEvents.join('|')")
            .expect("replacement image event should evaluate"),
        "load:anonymous:1"
    );
}

#[tokio::test]
async fn changing_picture_source_restarts_intercepted_image_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        (() => {
          const picture = document.createElement("picture");
          const source = document.createElement("source");
          const image = document.createElement("img");
          source.srcset = "first.png";
          image.src = "fallback.png";
          picture.append(source, image);
          (document.body || document.documentElement || document).appendChild(picture);
          globalThis.__pictureSource = source;
          globalThis.__pictureImageEvents = [];
          image.addEventListener("load", () => {
            globalThis.__pictureImageEvents.push(
              `load:${image.currentSrc}:${performance.getEntriesByName(image.currentSrc).length}`
            );
          });
        })()
        "#,
    )
    .expect("initial picture image request should evaluate");
    let first = vm.take_pending_subresource_fetch_infos();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].url.as_str(), "https://example.test/first.png");

    vm.eval("globalThis.__pictureSource.srcset = 'second.png'")
        .expect("picture source mutation should evaluate");
    let second = vm.take_pending_subresource_fetch_infos();
    assert_eq!(
        second.len(),
        1,
        "source mutation must restart its picture image request"
    );
    assert_ne!(first[0].internal_id, second[0].internal_id);
    assert_eq!(second[0].url.as_str(), "https://example.test/second.png");

    vm.fulfill_pending_subresource_fetch(
        second[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), "image/gif".to_owned())],
        one_by_one_gif_response_body(),
    )
    .expect("replacement picture image request should fulfill");
    assert_eq!(
        vm.eval("globalThis.__pictureImageEvents.join('|')")
            .expect("pre-event picture image trace should evaluate"),
        ""
    );
    run_next_image_event_task(&mut vm, &loader, "picture replacement image event").await;
    assert_eq!(
        vm.eval("globalThis.__pictureImageEvents.join('|')")
            .expect("replacement picture image event should evaluate"),
        "load:https://example.test/second.png:1"
    );
}

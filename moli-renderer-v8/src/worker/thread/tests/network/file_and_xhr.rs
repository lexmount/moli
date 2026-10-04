// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn worker_filelist_interface_object_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let constructError = null;
        try {
            new FileList();
        } catch (error) {
            constructError = error && error.name;
        }
        postMessage({
            ctorOwn: Object.prototype.hasOwnProperty.call(self, "FileList"),
            ctorType: typeof FileList,
            ctorName: FileList.name,
            constructError,
            itemType: typeof FileList.prototype.item,
            lengthGetterType: typeof Object.getOwnPropertyDescriptor(
                FileList.prototype,
                "length",
            ).get,
            iterType: typeof FileList.prototype[Symbol.iterator],
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
        r#"{"ctorOwn":true,"ctorType":"function","ctorName":"FileList","constructError":"TypeError","itemType":"function","lengthGetterType":"function","iterType":"function"}"#
    );
}

#[tokio::test]
async fn worker_blob_surface_supports_response_blob_and_blob_url_import_scripts() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            const response = new Response("hello from worker blob", {
                headers: { "Content-Type": "text/plain;charset=utf-8" }
            });
            const blob = await response.blob();
            const blobUrl = URL.createObjectURL(new Blob([
                "postMessage({ imported: true, objectUrlType: typeof URL.createObjectURL });"
            ], { type: "text/javascript" }));
            postMessage({
                blobCtor: typeof Blob,
                blobTag: Object.prototype.toString.call(blob),
                size: blob.size,
                type: blob.type,
                text: await blob.text(),
                blobUrlPrefix: blobUrl.startsWith("blob:http://127.0.0.1/"),
            });
            importScripts(blobUrl);
            URL.revokeObjectURL(blobUrl);
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let first = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(first),
        r#"{"blobCtor":"function","blobTag":"[object Blob]","size":22,"type":"text/plain;charset=utf-8","text":"hello from worker blob","blobUrlPrefix":true}"#
    );

    let second = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(second),
        r#"{"imported":true,"objectUrlType":"function"}"#
    );
}

#[tokio::test]
async fn worker_file_and_filereader_surface_reads_file_asynchronously() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            const file = new File(["hello worker file"], "note.txt", {
                type: "text/plain",
                lastModified: 123
            });
            const fileText = await file.text();
            const reader = new FileReader();
            const events = [];
            reader.onloadstart = () => events.push("loadstart");
            reader.addEventListener("progress", () => events.push("progress"));
            reader.onload = () => {
                events.push(`load:${reader.result}`);
            };
            reader.onloadend = () => {
                events.push(`loadend:${reader.readyState}`);
                postMessage({
                    fileCtor: typeof File,
                    readerCtor: typeof FileReader,
                    fileTag: Object.prototype.toString.call(file),
                    fileInstanceofBlob: file instanceof Blob,
                    readerInstanceofEventTarget: reader instanceof EventTarget,
                    fileName: file.name,
                    fileLastModified: file.lastModified,
                    fileType: file.type,
                    fileText,
                    constants: [FileReader.EMPTY, FileReader.LOADING, FileReader.DONE],
                    events,
                });
                close();
            };
            events.push(`before:${reader.readyState}`);
            reader.readAsText(file);
            events.push(`after:${reader.readyState}:${reader.result === null}`);
        })();
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
        r#"{"fileCtor":"function","readerCtor":"function","fileTag":"[object File]","fileInstanceofBlob":true,"readerInstanceofEventTarget":true,"fileName":"note.txt","fileLastModified":123,"fileType":"text/plain","fileText":"hello worker file","constants":[0,1,2],"events":["before:0","after:1:true","loadstart","progress","load:hello worker file","loadend:2"]}"#
    );
}

#[tokio::test]
async fn worker_filereader_abort_suppresses_late_load_from_pending_queue() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const reader = new FileReader();
            const events = [];
            reader.onload = () => events.push("load");
            reader.onabort = () => events.push("abort");
            reader.onloadend = () => events.push("loadend");
            reader.readAsText(new File(["cancel"], "cancel.txt"));
            reader.abort();
            setTimeout(() => {
                postMessage({
                    readerReadyState: reader.readyState,
                    resultIsNull: reader.result === null,
                    events,
                });
                close();
            }, 0);
        })();
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
        r#"{"readerReadyState":2,"resultIsNull":true,"events":["abort","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_does_not_expose_response_xml() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const xhr = new XMLHttpRequest();
        postMessage({
            instanceHasResponseXml: "responseXML" in xhr,
            prototypeHasResponseXml: Object.prototype.hasOwnProperty.call(
                XMLHttpRequest.prototype,
                "responseXML"
            ),
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
        r#"{"instanceHasResponseXml":false,"prototypeHasResponseXml":false}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_uses_worker_script_base_url_and_event_target_listeners() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "hello from worker xhr".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onreadystatechange = (event) => {
                events.push(`prop-rs:${event.type}:${xhr.readyState}`);
            };
            xhr.onload = () => {
                events.push(`prop-load:${xhr.status}`);
            };
            xhr.onloadend = () => {
                events.push(`prop-loadend:${xhr.readyState}`);
            };
            xhr.addEventListener('readystatechange', (event) => {
                events.push(`rs:${event.type}:${xhr.readyState}`);
            });
            xhr.addEventListener('load', () => {
                events.push(`load:${xhr.status}:${xhr.responseText}`);
            });
            xhr.addEventListener('loadend', () => {
                events.push(`loadend:${xhr.readyState}`);
            });
            xhr.addEventListener('loadend', () => {
                postMessage({
                    ctor: typeof XMLHttpRequest,
                    eventTarget: xhr instanceof XMLHttpRequestEventTarget,
                    uploadTag: Object.prototype.toString.call(xhr.upload),
                    status: xhr.status,
                    url: xhr.responseURL,
                    text: xhr.responseText,
                    readyState: xhr.readyState,
                    events,
                });
                close();
            });
            xhr.open('GET', './data.txt');
            xhr.send();
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"ctor":"function","eventTarget":true,"uploadTag":"[object XMLHttpRequestUpload]","status":200,"url":"{base_url}/assets/data.txt","text":"hello from worker xhr","readyState":4,"events":["prop-rs:readystatechange:1","rs:readystatechange:1","prop-rs:readystatechange:2","rs:readystatechange:2","prop-rs:readystatechange:3","rs:readystatechange:3","prop-rs:readystatechange:4","rs:readystatechange:4","prop-load:200","load:200:hello from worker xhr","prop-loadend:4","loadend:4"]}}"#
        )
    );
    server.await.expect("worker xhr server should finish");
}

#[tokio::test]
async fn worker_xhr_request_stage_interception_can_fulfill_synthetic_response() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            xhr.onloadend = () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    header: xhr.getResponseHeader("x-worker-xhr-intercept"),
                    text: xhr.responseText,
                });
                close();
            };
            xhr.open("POST", "http://example.test/intercepted-worker-xhr");
            xhr.send("payload");
        };
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader"),
        WorkerNetworkPolicy {
            network_partition_key: Some("credentialless-worker-xhr".to_owned()),
            ..WorkerNetworkPolicy::default()
        },
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr pause, got {pending:?}");
    };
    assert!(pending.info.network_request_handle.is_none());
    assert_eq!(pending.info.resource_type, SubresourceResourceType::Xhr);
    assert_eq!(
        pending.info.url.as_str(),
        "http://example.test/intercepted-worker-xhr"
    );
    assert_eq!(pending.info.request_body.as_deref(), Some("payload"));
    assert_eq!(
        pending.network_partition_key.as_deref(),
        Some("credentialless-worker-xhr")
    );

    let request = pending_worker_xhr_continue(pending.fetch_id, 31, &pending.info, false);
    handle.fulfill_pending_xhr(
        request,
        204,
        vec![
            ("content-type".to_owned(), b"text/plain".to_vec()),
            (
                "x-worker-xhr-intercept".to_owned(),
                b"request-stage".to_vec(),
            ),
        ],
        RendererSyntheticResponseBody::from_bytes(b"fulfilled-worker-xhr".to_vec()),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"readyState":4,"status":204,"header":"request-stage","text":"fulfilled-worker-xhr"}"#
    );
}

#[tokio::test]
async fn worker_sync_xhr_request_stage_interception_reports_explicit_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener("readystatechange", () => events.push("readystatechange:" + xhr.readyState));
            xhr.addEventListener("loadstart", () => events.push("loadstart"));
            xhr.addEventListener("error", () => events.push("error"));
            xhr.addEventListener("loadend", () => events.push("loadend"));
            xhr.open("GET", "http://example.test/sync-worker-xhr-intercepted", false);
            let error = null;
            try {
                xhr.send();
            } catch (caught) {
                error = {
                    name: caught && caught.name,
                    message: caught && caught.message,
                    isDomException: caught instanceof DOMException,
                };
            }
            postMessage({
                error,
                readyState: xhr.readyState,
                status: xhr.status,
                responseText: xhr.responseText,
                events,
            });
            close();
        };
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader"),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("worker channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            WorkerToParentMessage::PendingSubresourceFetch(pending) => {
                panic!("sync worker XHR should not be paused for interception: {pending:?}")
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("sync worker XHR interception should record network failure");
    assert_eq!(
        record.url().as_str(),
        "http://example.test/sync-worker-xhr-intercepted"
    );
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "Synchronous XMLHttpRequest interception is not supported"
    ));
    assert_eq!(
        post.expect("sync worker XHR interception should post final surface"),
        r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'http://example.test/sync-worker-xhr-intercepted'.","isDomException":true},"readyState":4,"status":0,"responseText":"","events":["readystatechange:1"]}"#
    );
}

#[tokio::test]
async fn worker_xhr_response_stage_interception_pauses_before_done() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/xhr.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "origin-worker-xhr".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const readyStates = [];
            xhr.onreadystatechange = () => readyStates.push(xhr.readyState);
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    header: xhr.getResponseHeader("x-worker-xhr-response-stage"),
                    text: xhr.responseText,
                    readyStates,
                });
                close();
            };
            xhr.open("GET", "./xhr.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr pause, got {pending:?}");
    };
    let request = pending_worker_xhr_continue(pending.fetch_id, 37, &pending.info, true);
    handle.continue_pending_xhr(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected worker xhr response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 37);
    assert_eq!(info.resource_type, SubresourceResourceType::Xhr);
    assert_eq!(info.response_status, 200);
    assert_eq!(
        info.response_body.try_bytes().unwrap().as_ref(),
        b"origin-worker-xhr"
    );

    handle.continue_pending_xhr_response(
        request,
        Some(206),
        Some(vec![
            ("content-type".to_owned(), b"text/plain".to_vec()),
            (
                "x-worker-xhr-response-stage".to_owned(),
                b"continued".to_vec(),
            ),
        ]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":206,"header":"continued","text":"origin-worker-xhr","readyStates":[1,2,3,4]}"#
    );
    server
        .await
        .expect("worker xhr response-stage server should finish");
}

#[tokio::test]
async fn worker_sync_xhr_timeout_cancels_fetch_and_throws_without_progress_events() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind slow sync worker xhr server");
    let addr = listener.local_addr().expect("slow sync worker xhr addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept slow sync worker xhr request");
        read_http_request_head(&mut stream)
            .await
            .expect("read slow sync worker xhr request");
        tokio::time::sleep(Duration::from_millis(250)).await;
        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nslow";
        let _ = stream.write_all(response.as_bytes()).await;
    });
    let base_url = format!("http://{addr}");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener("readystatechange", () => events.push("readystatechange:" + xhr.readyState));
            xhr.addEventListener("loadstart", () => events.push("loadstart"));
            xhr.addEventListener("timeout", () => events.push("timeout:" + xhr.readyState + ":" + xhr.status));
            xhr.addEventListener("error", () => events.push("error"));
            xhr.addEventListener("load", () => events.push("load"));
            xhr.addEventListener("loadend", () => events.push("loadend:" + xhr.readyState + ":" + xhr.status));
            xhr.timeout = 30;
            xhr.open("GET", "./slow.txt", false);
            let error = null;
            try {
                xhr.send();
            } catch (caught) {
                error = {
                    name: caught && caught.name,
                    message: caught && caught.message,
                    isDomException: caught instanceof DOMException,
                };
            }
            postMessage({
                error,
                readyState: xhr.readyState,
                status: xhr.status,
                responseText: xhr.responseText,
                events,
            });
            close();
        })();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("worker channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("sync worker XHR timeout should record network failure");
    assert_eq!(record.url().as_str(), format!("{base_url}/worker/slow.txt"));
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    let outcome = record.outcome();
    assert!(
        matches!(
            outcome,
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text == "Synchronous XMLHttpRequest timed out after 30 ms"
        ),
        "expected sync worker XHR timeout failure, got {outcome:?}"
    );
    assert_eq!(
        post.expect("sync worker XHR timeout should post final surface"),
        format!(
            r#"{{"error":{{"name":"TimeoutError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load '{base_url}/worker/slow.txt'.","isDomException":true}},"readyState":4,"status":0,"responseText":"","events":["readystatechange:1"]}}"#
        )
    );
    timeout(TIMEOUT, server)
        .await
        .expect("slow sync worker xhr server should finish within test timeout")
        .expect("slow sync worker xhr server should finish");
}

#[tokio::test]
async fn worker_sync_xhr_allows_response_types_and_omits_progress_events() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
          const probe = (type, setBeforeOpen) => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onreadystatechange = () => events.push(`readystatechange:${xhr.readyState}`);
            for (const eventType of ["loadstart", "progress", "load", "loadend"]) {
              xhr.addEventListener(eventType, event => events.push(
                `${eventType}:${event.loaded}:${event.total}:${event.lengthComputable}`
              ));
              xhr.upload.addEventListener(eventType, event => events.push(
                `upload.${eventType}:${event.loaded}:${event.total}:${event.lengthComputable}`
              ));
            }
            if (setBeforeOpen) xhr.responseType = type;
            const url = type === "json"
              ? "data:application/json,%7B%22value%22%3A7%7D"
              : "data:text/plain,ok";
            xhr.open("GET", url, false);
            if (!setBeforeOpen) xhr.responseType = type;
            xhr.send();
            let response;
            if (xhr.responseType === "arraybuffer") response = xhr.response.byteLength;
            else if (xhr.responseType === "blob") response = `${xhr.response.size}:${xhr.response.type}`;
            else if (xhr.responseType === "json") response = xhr.response.value;
            else response = xhr.responseText;
            return {
              type,
              setBeforeOpen,
              responseType: xhr.responseType,
              response,
              readyState: xhr.readyState,
              status: xhr.status,
              events,
            };
          };
          const matrix = [];
          for (const setBeforeOpen of [true, false]) {
            for (const type of ["arraybuffer", "blob", "json", "text", "document"]) {
              matrix.push(probe(type, setBeforeOpen));
            }
          }
          postMessage(matrix);
          close();
        })();
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );

    let observed = recv_post_json(&mut handle).await;
    let observed: serde_json::Value =
        serde_json::from_str(&observed).expect("worker responseType matrix should be JSON");
    let matrix = observed
        .as_array()
        .expect("worker responseType matrix should be an array");
    assert_eq!(matrix.len(), 10);
    for entry in matrix {
        let requested_type = entry["type"]
            .as_str()
            .expect("requested responseType should be a string");
        let effective_type = if requested_type == "document" {
            ""
        } else {
            requested_type
        };
        assert_eq!(entry["responseType"], effective_type);
        assert_eq!(entry["readyState"], 4);
        assert_eq!(entry["status"], 200);
        let loaded = if requested_type == "json" { 11 } else { 2 };
        assert_eq!(
            entry["events"],
            serde_json::json!([
                "readystatechange:1",
                "readystatechange:4",
                format!("load:{loaded}:{loaded}:true"),
                format!("loadend:{loaded}:{loaded}:true")
            ])
        );
        match requested_type {
            "arraybuffer" => assert_eq!(entry["response"], 2),
            "blob" => assert_eq!(entry["response"], "2:text/plain"),
            "json" => {
                assert_eq!(entry["response"], 7);
            }
            "text" | "document" => assert_eq!(entry["response"], "ok"),
            other => panic!("unexpected responseType matrix entry: {other}"),
        }
    }
}

#[tokio::test]
async fn worker_xhr_response_stage_continue_preserves_large_spooled_body() {
    ensure_v8();
    let body = "x".repeat(1024 * 1024 + 17);
    let expected_len = body.len();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/large-xhr.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        body,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    length: xhr.responseText.length,
                    first: xhr.responseText.slice(0, 1),
                    last: xhr.responseText.slice(-1),
                });
                close();
            };
            xhr.open("GET", "./large-xhr.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage large worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected large worker xhr pause, got {pending:?}");
    };
    let request = pending_worker_xhr_continue(pending.fetch_id, 67, &pending.info, true);
    handle.continue_pending_xhr(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage large worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected large worker xhr response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 67);
    assert_eq!(
        info.response_body
            .read_chunk(expected_len - 1, 1)
            .expect("spooled body tail should be readable"),
        b"x"
    );

    handle.continue_pending_xhr_response(request, None, None);

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(r#"{{"status":200,"length":{expected_len},"first":"x","last":"x"}}"#)
    );
    server
        .await
        .expect("large worker xhr response-stage server should finish");
}

#[tokio::test]
async fn worker_xhr_auth_required_then_continue_with_auth_resolves() {
    ensure_v8();
    let (base_url, server) =
        spawn_basic_auth_http_server("/worker/xhr-auth.txt", "worker-xhr-area", "xhr-secret", 2)
            .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const readyStates = [];
            xhr.onreadystatechange = () => readyStates.push(xhr.readyState);
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    text: xhr.responseText,
                    readyStates,
                });
                close();
            };
            xhr.open("GET", "./xhr-auth.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker xhr auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_xhr_continue(pending.fetch_id, 41, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_xhr(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker xhr auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 41);
    assert_eq!(info.resource_type, SubresourceResourceType::Xhr);
    assert_eq!(info.challenge.source, "Server");
    assert_eq!(info.challenge.scheme, "basic");
    assert_eq!(info.challenge.realm, "worker-xhr-area");
    assert!(!info.intercept_response);
    assert_initial_worker_auth_network_headers(info.network_request_headers.as_deref());

    request.auth = Some(server_basic_auth_credentials());
    handle.continue_pending_xhr(request);

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr auth-success network record")
        .expect("worker channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert_initial_worker_auth_network_headers(record.network_request_headers());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Success { status, .. } if *status == 200
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":200,"text":"xhr-secret","readyStates":[1,2,3,4]}"#
    );
    server.await.expect("worker xhr auth server should finish");
}

#[tokio::test]
async fn worker_xhr_auth_required_then_fail_errors_without_exposing_challenge_body() {
    ensure_v8();
    let (base_url, server) =
        spawn_basic_auth_http_server("/worker/xhr-auth.txt", "worker-xhr-area", "xhr-secret", 1)
            .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onerror = () => events.push("error");
            xhr.onload = () => events.push("load");
            xhr.onloadend = () => {
                events.push("loadend");
                postMessage({
                    status: xhr.status,
                    readyState: xhr.readyState,
                    text: xhr.responseText,
                    events,
                });
                close();
            };
            xhr.open("GET", "./xhr-auth.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker xhr auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_xhr_continue(pending.fetch_id, 43, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_xhr(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker xhr auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 43);
    assert_eq!(info.challenge.realm, "worker-xhr-area");

    handle.fail_pending_xhr_auth(request, "worker xhr auth aborted".to_owned());

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":0,"readyState":4,"text":"","events":["error","loadend"]}"#
    );
    server
        .await
        .expect("worker xhr auth-fail server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_arraybuffer_preserves_response_bytes() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.bin",
        "HTTP/1.1 200 OK",
        "application/octet-stream",
        "A\0B".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            xhr.responseType = "arraybuffer";
            xhr.onloadend = () => {
                let responseTextState = "not-checked";
                try {
                    void xhr.responseText;
                    responseTextState = "readable";
                } catch (error) {
                    responseTextState = error && error.name;
                }
                postMessage({
                    status: xhr.status,
                    bytes: Array.from(new Uint8Array(xhr.response)).join(","),
                    responseTextState,
                });
                close();
            };
            xhr.open("GET", "./data.bin");
            xhr.send();
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"status":200,"bytes":"65,0,66","responseTextState":"InvalidStateError"}"#
    );
    server
        .await
        .expect("worker xhr arraybuffer server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_upload_dispatches_completion_events() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/upload",
        "HTTP/1.1 200 OK",
        "application/json",
        r#"{"ok":true}"#.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const payload = "upload=alpha&count=2";
            const uploadEvents = [];
            const uploadOrder = [];
            xhr.onloadstart = () => uploadOrder.push("xhr:loadstart");
            ["loadstart", "progress", "load", "loadend"].forEach((type) => {
                xhr.upload.addEventListener(type, (event) => {
                    uploadOrder.push(`listener-before:${event.type}`);
                });
                xhr.upload[`on${type}`] = (event) => {
                    uploadOrder.push(`handler:${event.type}`);
                };
                xhr.upload.addEventListener(type, (event) => {
                    uploadEvents.push([
                        event.type,
                        event.target === xhr.upload,
                        event.currentTarget === xhr.upload,
                        event.lengthComputable,
                        event.loaded,
                        event.total,
                    ].join(":"));
                    uploadOrder.push(`listener-after:${event.type}`);
                });
            });
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    response: xhr.responseText,
                    uploadEvents,
                    uploadOrder,
                });
                close();
            };
            xhr.open("POST", "./upload");
            xhr.setRequestHeader("Content-Type", "text/plain;charset=utf-8");
            xhr.send(payload);
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"status":200,"response":"{\"ok\":true}","uploadEvents":["loadstart:true:true:true:0:20","progress:true:true:true:20:20","load:true:true:true:20:20","loadend:true:true:true:20:20"],"uploadOrder":["xhr:loadstart","listener-before:loadstart","handler:loadstart","listener-after:loadstart","listener-before:progress","handler:progress","listener-after:progress","listener-before:load","handler:load","listener-after:load","listener-before:loadend","handler:loadend","listener-after:loadend"]}"#
    );
    server
        .await
        .expect("worker xhr upload server should finish");
}

#[tokio::test]
async fn worker_xhr_upload_listener_preflight_survives_sync_and_interception() {
    ensure_v8();
    for mode in ["async", "sync", "intercept", "none", "late", "clear"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/upload", listener.local_addr().unwrap());
        let requires_preflight = !matches!(mode, "none" | "late");
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let methods: &[&str] = if requires_preflight {
                &["OPTIONS", "POST"]
            } else {
                &["POST"]
            };
            for method in methods {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                let mut byte = [0; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                assert!(
                    head.starts_with(&format!("{method} /upload HTTP/1.1\r\n")),
                    "{mode}: {head}"
                );
                if *method == "OPTIONS" {
                    let lower = head.to_ascii_lowercase();
                    assert!(lower.contains("access-control-request-method: post\r\n"));
                    assert!(!lower.contains("access-control-request-headers:"));
                } else {
                    let mut body = [0; 7];
                    socket.read_exact(&mut body).await.unwrap();
                    assert_eq!(&body, b"payload");
                }
                socket.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
            }
        });
        let mut config = FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let loader = ResourceRequestClient::new(&config).unwrap();
        let mut handle = spawn_worker_with_request_client(
            format!(
                r#"
            onmessage = () => {{
                const xhr = new XMLHttpRequest();
                const mode = {mode:?};
                const listener = () => {{}};
                if (mode !== "none" && mode !== "late") xhr.upload.addEventListener("custom", listener);
                xhr.onloadstart = () => {{
                    if (mode === "late") xhr.upload.addEventListener("custom", listener);
                    if (mode === "clear") xhr.upload.removeEventListener("custom", listener);
                }};
                const finish = () => {{ postMessage([xhr.status, xhr.responseText]); close(); }};
                xhr.open("POST", {url:?}, mode !== "sync");
                if (mode !== "sync") xhr.onloadend = finish;
                xhr.send("payload");
                if (mode === "sync") finish();
            }};
        "#
            ),
            "http://origin.test/worker.js".to_owned(),
            loader,
        );
        if mode == "intercept" {
            handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
        }
        handle.post_message(serialize_test_string("go"));
        if mode == "intercept" {
            let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
            let WorkerToParentMessage::PendingSubresourceFetch(pending) = message else {
                panic!("expected intercepted XHR, got {message:?}");
            };
            handle.continue_pending_xhr(pending_worker_xhr_continue(
                pending.fetch_id,
                47,
                &pending.info,
                false,
            ));
        }
        assert_eq!(recv_post_json(&mut handle).await, "[200,\"ok\"]", "{mode}");
        timeout(TIMEOUT, server).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn worker_xmlhttprequest_bad_port_errors_before_transport() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.addEventListener('loadend', () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                });
                close();
            });
            xhr.open('GET', 'http://example.test:25/blocked-port');
            xhr.send();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(record.url().as_str(), "http://example.test:25/blocked-port");
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("blocked bad port")
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_blocked_url_reports_error_after_loadstart() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.addEventListener('loadend', () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                });
                close();
            });
            xhr.open('GET', 'http://example.test/blocked/worker-xhr');
            xhr.send();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        vec!["http://example.test/blocked/*".to_owned()],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/blocked/worker-xhr"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "net::ERR_BLOCKED_BY_CLIENT"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_offline_reports_error_after_loadstart() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.addEventListener('loadend', () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                });
                close();
            });
            xhr.open('GET', 'http://example.test/offline/worker-xhr');
            xhr.send();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        WorkerNetworkPolicy {
            network_offline: true,
            ..WorkerNetworkPolicy::default()
        },
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/offline/worker-xhr"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "Network emulation offline"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_connection_refused_reports_error_after_loadstart() {
    ensure_v8();
    let (base_url, server) =
        spawn_connection_drop_http_server("/worker-xhr-connection-refused").await;
    let url = format!("{base_url}/worker-xhr-connection-refused");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }
    let record = network.expect("worker XHR connection failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
    ));
    assert_eq!(
        post.expect("worker XHR connection failure should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
    server
        .await
        .expect("worker XHR connection-drop server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_file_url_rejects_before_interception_or_transport() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {
                events.push('loadend');
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    events,
                });
                close();
            });
            xhr.open('GET', 'file:///moli-policy-must-not-open');
            xhr.send();
        };
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        match timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker file XHR rejection")
            .expect("worker channel closed")
        {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unsupported worker XHR must not reach interception: {other:?}"),
        }
    }

    let record = network.expect("worker file XHR should record a network failure");
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert_eq!(
        record.outcome(),
        &SubresourceNetworkOutcome::Failure {
            error_text: "URL scheme \"file\" is not supported.".to_owned(),
        }
    );
    assert_eq!(
        post.expect("worker file XHR should expose a network error surface"),
        r#"{"readyState":4,"status":0,"responseURL":"","responseText":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn synchronous_worker_xhr_file_url_throws_network_error_without_progress_events() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.open('GET', 'file:///moli-policy-must-not-open', false);
            let error = null;
            try {
                xhr.send();
            } catch (caught) {
                error = {
                    name: caught && caught.name,
                    message: caught && caught.message,
                    isDomException: caught instanceof DOMException,
                };
            }
            postMessage({
                error,
                events,
                readyState: xhr.readyState,
                status: xhr.status,
            });
            close();
        };
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        match timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for synchronous worker file XHR rejection")
            .expect("worker channel closed")
        {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unsupported synchronous worker XHR reached interception: {other:?}"),
        }
    }

    let record = network.expect("synchronous worker file XHR should record a network failure");
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert_eq!(
        record.outcome(),
        &SubresourceNetworkOutcome::Failure {
            error_text: "URL scheme \"file\" is not supported.".to_owned(),
        }
    );
    assert_eq!(
        post.expect("synchronous worker file XHR should throw NetworkError"),
        r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'file:///moli-policy-must-not-open'.","isDomException":true},"events":["readystatechange:1"],"readyState":4,"status":0}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_dns_failure_reports_error_after_loadstart() {
    ensure_v8();
    let url = "http://moli-dns-failure.invalid./worker-xhr-dns-failure";
    let url_literal = serde_json::to_string(url).expect("serialize worker xhr url");
    let loader =
        ResourceRequestClient::new(&dns_failure_fetch_config()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for worker XHR DNS failure output (post={}, network={})",
                    post.is_some(),
                    network.is_some()
                )
            })
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }
    let record = network.expect("worker XHR DNS failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
        panic!(
            "expected worker XHR DNS failure, got {:?}",
            record.outcome()
        );
    };
    assert!(
        error_text.to_ascii_lowercase().contains("resolv"),
        "expected DNS-resolution error text, got {error_text:?}"
    );
    assert_eq!(
        post.expect("worker XHR DNS failure should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_redirect_loop_reports_error_after_loadstart() {
    ensure_v8();
    let (base_url, server) = spawn_redirect_loop_http_server("/worker-xhr-loop").await;
    let url = format!("{base_url}/worker-xhr-loop");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    server
        .await
        .expect("worker xhr redirect-loop server should finish");
    let record = network.expect("worker XHR redirect loop should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("redirect limit exceeded")
    ));
    assert_eq!(
        post.expect("worker XHR redirect loop should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_cross_origin_redirect_without_cors_reports_error_after_loadstart() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_without_cors_http_servers(
            "/worker-xhr-cors-redirect-deny",
            "/worker-xhr-cors-denied-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-xhr-cors-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        format!("{source_base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker XHR CORS redirect source server should finish");
    target_server
        .await
        .expect("worker XHR CORS redirect target server should finish");
    let record = network.expect("worker XHR CORS redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == crate::network_host::FAILED_ERROR_TEXT
    ));
    assert_eq!(
        post.expect("worker XHR CORS redirect should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_cross_origin_redirect_final_url_obeys_connect_src() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_with_cors_http_servers(
            "/worker-xhr-csp-redirect-deny",
            "/worker-xhr-csp-target",
            "worker-xhr-csp-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-xhr-csp-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            const violations = [];
            addEventListener("securitypolicyviolation", event => {{
                violations.push({{
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition,
                }});
            }});
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                    violations,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
            ),
            format!("{source_base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_content_security_policies(vec!["connect-src 'self'".to_owned()]),
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker XHR CSP redirect source server should finish");
    target_server
        .await
        .expect("worker XHR CSP redirect target server should finish");
    let record = network.expect("worker XHR CSP redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Content Security Policy")
    ));
    assert_eq!(
        post.expect("worker XHR CSP redirect should post final surface"),
        format!(
            r#"{{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"violations":[{{"blockedURI":"{url}","effectiveDirective":"connect-src","disposition":"enforce"}}]}}"#
        )
    );
}

#[tokio::test]
async fn worker_termination_releases_native_websocket_transport() {
    ensure_v8();
    let (url, server) =
        moli_websocket::test_support::spawn_transport_retirement_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker network client");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
            globalThis.socket = new WebSocket({url:?});
            socket.onopen = () => postMessage('opened');
            socket.onerror = () => postMessage('error');
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );
    assert_eq!(recv_post_json(&mut handle).await, r#""opened""#);
    handle.terminate_and_join();
    server
        .await
        .expect("terminating Worker releases its native socket");
}

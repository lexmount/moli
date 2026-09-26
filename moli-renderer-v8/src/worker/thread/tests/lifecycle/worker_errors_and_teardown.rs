use super::*;

#[tokio::test]
async fn worker_setinterval() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let count = 0;
        let id = setInterval(function() {
            count++;
            postMessage(count);
            if (count >= 3) {
                clearInterval(id);
                close();
            }
        }, 50);
        "#
        .into(),
        "test://setinterval".into(),
    );

    for expected in 1..=3 {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(expect_post_json(msg), expected.to_string());
    }
}

#[tokio::test]
async fn worker_message_listener_receives_data_without_onmessage() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        addEventListener("message", function(event) {
            postMessage(`listener:${event.data}`);
            close();
        });
        "#
        .into(),
        "test://message_listener".into(),
    );

    handle.post_message(serialize_test_string("ping"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""listener:ping""#);
}

#[tokio::test]
async fn worker_message_listener_receives_messageevent_instance() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        addEventListener("message", function(event) {
            postMessage({
                isMessageEvent: event instanceof MessageEvent,
                typeString: Object.prototype.toString.call(event),
                data: event.data
            });
            close();
        });
        "#
        .into(),
        "test://message_event_instance".into(),
    );

    handle.post_message(serialize_test_string("ping"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"isMessageEvent":true,"typeString":"[object MessageEvent]","data":"ping"}"#
    );
}

#[tokio::test]
async fn file_reader_sync_declared_methods_preserve_descriptors_and_reads() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const methods = [
            "readAsText",
            "readAsDataURL",
            "readAsArrayBuffer",
            "readAsBinaryString"
        ];
        const descriptors = methods.map(name => {
            const descriptor =
                Object.getOwnPropertyDescriptor(FileReaderSync.prototype, name);
            return [
                name,
                typeof descriptor?.value,
                descriptor?.value?.name,
                descriptor?.value?.length,
                descriptor?.enumerable,
                descriptor?.writable,
                descriptor?.configurable
            ].join(":");
        });
        const reader = new FileReaderSync();
        const blob = new Blob(["abc"], { type: "text/plain" });
        const buffer = reader.readAsArrayBuffer(blob);
        postMessage({
            descriptors,
            text: reader.readAsText(blob),
            dataUrlOk:
                reader.readAsDataURL(blob) === "data:text/plain;base64,YWJj",
            binary: reader.readAsBinaryString(blob),
            byteLength: buffer.byteLength
        });
        close();
        "#
        .into(),
        "test://filereadersync-declared-methods".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"descriptors":["readAsText:function:readAsText:1:true:true:true","readAsDataURL:function:readAsDataURL:1:true:true:true","readAsArrayBuffer:function:readAsArrayBuffer:1:true:true:true","readAsBinaryString:function:readAsBinaryString:1:true:true:true"],"text":"abc","dataUrlOk":true,"binary":"abc","byteLength":3}"#
    );
}

#[tokio::test]
async fn worker_unsupported_worker_related_constructors_throw_not_supported() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const describeUnsupported = (name, construct) => {
            let errorName = "";
            let errorCode = 0;
            let errorMessage = "";
            try {
                construct();
            } catch (error) {
                errorName = error && error.name;
                errorCode = error && error.code;
                errorMessage = error && error.message;
            }
            const ctor = globalThis[name];
            return [
                typeof ctor,
                ctor && ctor.name,
                ctor && ctor.length,
                Object.getPrototypeOf(ctor) === EventTarget,
                Object.getPrototypeOf(ctor.prototype) === EventTarget.prototype,
                Object.prototype.toString.call(Object.create(ctor.prototype)),
                errorName,
                errorCode,
                errorMessage
            ].join("|");
        };

        postMessage({
            eventSource: describeUnsupported(
                "EventSource",
                () => new EventSource("/events")
            ),
            sharedWorker: typeof SharedWorker
        });
        close();
        "#
        .into(),
        "test://unsupported_worker_related_constructors".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"eventSource":"function|EventSource|1|true|true|[object EventSource]|NotSupportedError|9|This constructor is not implemented in dedicated workers yet.","sharedWorker":"undefined"}"#
    );
}

#[tokio::test]
async fn nested_worker_drops_message_dispatched_before_listener_is_added() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const child = new Worker(
            "data:text/javascript," +
            encodeURIComponent(`
                onmessage = () => postMessage('after-listener');
                postMessage('early');
                throw new Error('early-dispatched');
            `)
        );
        child.onerror = event => {
            event.preventDefault();
            child.onerror = null;
            child.onmessage = event => {
                postMessage(`child:${event.data}`);
                close();
            };
            child.postMessage("go");
        };
        "#
        .into(),
        "test://nested_worker_no_replay".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""child:after-listener""#);
}

#[tokio::test]
async fn worker_global_unhandledrejection_event_dispatches_after_microtask_checkpoint() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onunhandledrejection = event => {
            const defaultPreventedBefore = event.defaultPrevented;
            event.preventDefault();
            postMessage({
                type: event.type,
                reason: event.reason,
                promise: event.promise instanceof Promise,
                cancelable: event.cancelable,
                defaultPreventedBefore,
                defaultPreventedAfter: event.defaultPrevented
            });
            close();
        };
        Promise.reject("worker-boom");
        "#
        .into(),
        "test://worker_unhandledrejection".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"type":"unhandledrejection","reason":"worker-boom","promise":true,"cancelable":true,"defaultPreventedBefore":false,"defaultPreventedAfter":true}"#
    );
}

#[tokio::test]
async fn worker_global_unhandledrejection_fallback_event_preserves_event_shape() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        self.PromiseRejectionEvent = undefined;
        onunhandledrejection = event => {
            const defaultPreventedBefore = event.defaultPrevented;
            event.preventDefault();
            postMessage({
                type: event.type,
                reason: event.reason,
                promise: event.promise instanceof Promise,
                defaultPreventedBefore,
                defaultPreventedAfter: event.defaultPrevented,
                preventDefaultType: typeof event.preventDefault,
                preventDefaultEnumerable: Object.prototype.propertyIsEnumerable.call(event, "preventDefault"),
                defaultPreventedEnumerable: Object.prototype.propertyIsEnumerable.call(event, "defaultPrevented"),
                typeEnumerable: Object.prototype.propertyIsEnumerable.call(event, "type")
            });
            close();
        };
        Promise.reject("fallback-worker-boom");
        "#
        .into(),
        "test://worker_unhandledrejection_fallback_event".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"type":"unhandledrejection","reason":"fallback-worker-boom","promise":true,"defaultPreventedBefore":false,"defaultPreventedAfter":true,"preventDefaultType":"function","preventDefaultEnumerable":true,"defaultPreventedEnumerable":true,"typeEnumerable":true}"#
    );
}

#[tokio::test]
async fn worker_global_rejectionhandled_event_dispatches_for_late_handler() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const rejected = Promise.reject("late-worker-boom");
        onunhandledrejection = event => {
            event.preventDefault();
            setTimeout(() => rejected.catch(() => {}), 0);
        };
        onrejectionhandled = event => {
            postMessage({
                type: event.type,
                reason: event.reason,
                samePromise: event.promise === rejected,
                cancelable: event.cancelable
            });
            close();
        };
        "#
        .into(),
        "test://worker_rejectionhandled".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"type":"rejectionhandled","reason":"late-worker-boom","samePromise":true,"cancelable":false}"#
    );
}

#[tokio::test]
async fn worker_handler_added_during_unhandled_rejection_suppresses_rejectionhandled() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const events = [];
        const rejected = Promise.reject("handled-during-worker-notification");
        onunhandledrejection = event => {
            event.preventDefault();
            events.push(event.type);
            rejected.catch(reason => {
                events.push(reason === "handled-during-worker-notification"
                    ? "handler"
                    : "wrong-reason");
            });
            setTimeout(() => {
                postMessage(events);
                close();
            }, 0);
        };
        onrejectionhandled = event => events.push(event.type);
        "#
        .into(),
        "test://worker_handled_during_unhandledrejection".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"["unhandledrejection","handler"]"#
    );
}

#[tokio::test]
async fn worker_unhandledrejection_notification_allows_one_message_port_turn() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const events = [];
        let rejected;
        addEventListener("unhandledrejection", event => {
            if (event.promise === rejected) {
                event.preventDefault();
                events.push(event.type);
            }
        });
        const channel = new MessageChannel();
        channel.port1.onmessage = () => {
            rejected.catch(() => {});
            setTimeout(() => {
                postMessage(events);
                close();
            }, 10);
        };
        rejected = Promise.reject("handled-before-notification");
        channel.port2.postMessage("attach");
        "#
        .into(),
        "test://worker_unhandledrejection_one_port_turn".into(),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#"[]"#);
}

#[tokio::test]
async fn worker_unhandledrejection_notification_precedes_nested_message_port_turn() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let rejected;
        addEventListener("unhandledrejection", event => {
            if (event.promise === rejected) {
                event.preventDefault();
                postMessage(event.type);
                close();
            }
        });
        const first = new MessageChannel();
        first.port1.onmessage = () => {
            const second = new MessageChannel();
            second.port1.onmessage = () => rejected.catch(() => {});
            second.port2.postMessage("too-late");
        };
        rejected = Promise.reject("nested-turn");
        first.port2.postMessage("queue-nested");
        "#
        .into(),
        "test://worker_unhandledrejection_nested_port_turn".into(),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""unhandledrejection""#);
}

#[tokio::test]
async fn worker_create_image_bitmap_rejects_invalid_blob_with_dom_exception() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            try {
                await createImageBitmap(new Blob());
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    isDomException: error instanceof DOMException
                });
            }
            close();
        })();
        "#
        .into(),
        "test://worker_create_image_bitmap".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"name":"InvalidStateError","isDomException":true}"#
    );
}

#[tokio::test]
async fn worker_message_port_transfers_readable_stream() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const stream = new ReadableStream({
            start(controller) {
                controller.enqueue("a");
                controller.close();
            }
        });
        const channel = new MessageChannel();
        channel.port1.onmessage = async event => {
            const reader = event.data.getReader();
            const first = await reader.read();
            const second = await reader.read();
            postMessage({
                originalLocked: stream.locked,
                instance: event.data instanceof ReadableStream,
                value: first.value,
                firstDone: first.done,
                secondDone: second.done
            });
            close();
        };
        channel.port2.postMessage(stream, [stream]);
        "#
        .into(),
        "test://worker_message_port_readable_stream_transfer".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"originalLocked":true,"instance":true,"value":"a","firstDone":false,"secondDone":true}"#
    );
}

#[tokio::test]
async fn shared_worker_global_unhandledrejection_event_dispatches_after_connect_task() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                onunhandledrejection = event => {
                    if (event.type === "unhandledrejection" &&
                        event.reason === "shared-boom" &&
                        event.promise instanceof Promise &&
                        event.cancelable) {
                        event.preventDefault();
                        close();
                    }
                };
                Promise.reject("shared-boom");
            };
            "#
            .into(),
            "https://app.example/shared-worker.js".into(),
        )
        .with_global_kind(super::super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        }),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn worker_fetch_csp_block_dispatches_securitypolicyviolation_event() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            fetch("https://api.example/data").catch(error => {
                postMessage({
                    events,
                    error: error && error.message
                });
                close();
            });
            "#
            .into(),
            "https://app.example/worker.js".into(),
        )
        .with_content_security_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"events":[{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"https://api.example/data","documentURI":"https://app.example/worker.js","originalPolicy":"connect-src 'none'","disposition":"enforce","instance":true}],"error":"fetch: blocked by Content Security Policy for `https://api.example/data`."}"#
    );
}

#[tokio::test]
async fn worker_fetch_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "report-only fetch ok".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker fetch report-only loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const events = [];
                addEventListener("securitypolicyviolation", event => {
                    events.push({
                        type: event.type,
                        effectiveDirective: event.effectiveDirective,
                        violatedDirective: event.violatedDirective,
                        blockedURI: event.blockedURI,
                        documentURI: event.documentURI,
                        originalPolicy: event.originalPolicy,
                        disposition: event.disposition,
                        instance: event instanceof SecurityPolicyViolationEvent
                    });
                });
                const response = await fetch("./data.txt");
                postMessage({ events, text: await response.text() });
                close();
            })().catch(error => {
                postMessage({ error: String(error), stack: error && error.stack });
                close();
            });
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader)
        .with_content_security_report_only_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"{base_url}/worker/data.txt","documentURI":"{script_url}","originalPolicy":"connect-src 'none'","disposition":"report","instance":true}}],"text":"report-only fetch ok"}}"#
        )
    );
    server
        .await
        .expect("worker fetch report-only server should finish");
}

#[tokio::test]
async fn worker_fetch_csp_report_uri_posts_violation_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker CSP report server");
    let addr = listener.local_addr().expect("worker CSP report addr");
    let base_url = format!("http://{addr}");
    let (request_tx, request_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker CSP report request");
        let request = read_http_request_with_body(&mut stream)
            .await
            .expect("read worker CSP report request");
        let _ = request_tx.send(request);
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write worker CSP report response");
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker CSP report loader");
    let script_url = format!("{base_url}/worker/main.js");
    let blocked_url = format!("{base_url}/worker/blocked.txt");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                try {
                    await fetch("./blocked.txt");
                    postMessage({ status: "unexpected" });
                } catch (error) {
                    postMessage({ name: error && error.name });
                }
                close();
            })();
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader.clone())
        .with_content_security_policies(vec![
            "connect-src 'none'; report-uri /csp-report".to_owned(),
        ]),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#"{"name":"TypeError"}"#);
    let request = timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for worker CSP report")
        .expect("worker CSP report capture channel closed");
    server
        .await
        .expect("worker CSP report server should finish");
    assert!(request.starts_with("POST /csp-report HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/csp-report")
    );
    let body = request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("worker CSP report request should contain body");
    let body: serde_json::Value =
        serde_json::from_str(body).expect("worker CSP report body should be JSON");
    assert_eq!(body["csp-report"]["document-uri"], script_url);
    assert_eq!(body["csp-report"]["blocked-uri"], blocked_url);
    assert_eq!(body["csp-report"]["effective-directive"], "connect-src");
    assert_eq!(body["csp-report"]["violated-directive"], "connect-src");
    assert_eq!(body["csp-report"]["disposition"], "enforce");
}

#[tokio::test]
async fn worker_fetch_csp_report_to_posts_reporting_api_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker CSP report-to server");
    let addr = listener.local_addr().expect("worker CSP report-to addr");
    let base_url = format!("http://{addr}");
    let (request_tx, request_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker CSP report-to request");
        let request = read_http_request_with_body(&mut stream)
            .await
            .expect("read worker CSP report-to request");
        let _ = request_tx.send(request);
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write worker CSP report-to response");
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker CSP report-to loader");
    let script_url = format!("{base_url}/worker/main.js");
    let blocked_url = format!("{base_url}/worker/blocked.txt");
    let reporting_endpoints =
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &[(
                "Reporting-Endpoints".to_owned(),
                b"csp=\"/report-to\"".to_vec(),
            )],
            &url::Url::parse(&script_url).expect("script url"),
        );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                try {
                    await fetch("./blocked.txt");
                    postMessage({ status: "unexpected" });
                } catch (error) {
                    postMessage({ name: error && error.name });
                }
                close();
            })();
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader.clone())
        .with_content_security_policies(vec![
            "connect-src 'none'; report-uri /legacy; report-to csp".to_owned(),
        ])
        .with_content_security_reporting_endpoints(reporting_endpoints),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#"{"name":"TypeError"}"#);
    let request = timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for worker CSP report-to")
        .expect("worker CSP report-to capture channel closed");
    server
        .await
        .expect("worker CSP report-to server should finish");
    assert!(request.starts_with("POST /report-to HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/reports+json")
    );
    let body = request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("worker CSP report-to request should contain body");
    let body: serde_json::Value =
        serde_json::from_str(body).expect("worker CSP report-to body should be JSON");
    assert_eq!(body[0]["type"], "csp-violation");
    assert_eq!(body[0]["url"], script_url);
    assert_eq!(body[0]["body"]["documentURL"], script_url);
    assert_eq!(body[0]["body"]["blockedURL"], blocked_url);
    assert_eq!(body[0]["body"]["effectiveDirective"], "connect-src");
    assert_eq!(
        body[0]["body"]["originalPolicy"],
        "connect-src 'none'; report-uri /legacy; report-to csp"
    );
    assert_eq!(body[0]["body"]["disposition"], "enforce");
}

#[tokio::test]
async fn worker_xhr_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "report-only xhr ok".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr report-only loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            const xhr = new XMLHttpRequest();
            xhr.onload = () => {
                postMessage({ events, status: xhr.status, text: xhr.responseText });
                close();
            };
            xhr.onerror = () => {
                postMessage({ events, error: "xhr-error" });
                close();
            };
            xhr.open("GET", "./data.txt");
            xhr.send();
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader)
        .with_content_security_report_only_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"{base_url}/worker/data.txt","documentURI":"{script_url}","originalPolicy":"connect-src 'none'","disposition":"report","instance":true}}],"status":200,"text":"report-only xhr ok"}}"#
        )
    );
    server
        .await
        .expect("worker xhr report-only server should finish");
}

#[tokio::test]
async fn shared_worker_fetch_csp_block_dispatches_securitypolicyviolation_event() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                addEventListener("securitypolicyviolation", event => {
                    if (event.type === "securitypolicyviolation" &&
                        event.effectiveDirective === "connect-src" &&
                        event.blockedURI === "https://api.example/data" &&
                        event instanceof SecurityPolicyViolationEvent) {
                        close();
                    }
                });
                fetch("https://api.example/data").catch(() => {});
            };
            "#
            .into(),
            "https://app.example/shared-worker.js".into(),
        )
        .with_global_kind(super::super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        })
        .with_content_security_policies(vec!["connect-src 'none'".to_owned()]),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn nested_worker_unhandled_error_routes_through_parent_worker_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesBoom: String(message).includes("child-boom"),
                filenameIncludesDataUrl: String(filename).includes("data:text/javascript"),
                errorIsUndefined: error === undefined
            });
            close();
            return true;
        };
        new Worker("data:text/javascript,throw%20new%20Error('child-boom')");
        "#
        .into(),
        "test://nested_worker_error_parent_onerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"messageIncludesBoom":true,"filenameIncludesDataUrl":true,"errorIsUndefined":true}"#
    );
}

#[tokio::test]
async fn nested_worker_script_load_failure_is_async_error_event() {
    ensure_v8();
    // Produce a network failure locally instead of depending on DNS failure
    // timing for example.test to deliver the asynchronous error event.
    let (base_url, server) = spawn_connection_drop_http_server("/missing-child.js").await;
    let mut handle = spawn_worker(
        r#"
        let result = "not-run";
        let globalErrors = 0;
        onerror = () => { ++globalErrors; return true; };
        try {
            const child = new Worker("missing-child.js");
            child.onerror = event => {
                event.preventDefault();
                const observation = {
                    constructed: result === "constructed",
                    type: event.type,
                    intrinsicEvent: Object.getPrototypeOf(event) === Event.prototype,
                    target: event.target === child,
                    trusted: event.isTrusted,
                    flags: [event.bubbles, event.cancelable, event.composed, event.defaultPrevented],
                    hasErrorDetails: ['message', 'filename', 'lineno', 'colno', 'error'].some(name => name in event)
                };
                setTimeout(() => {
                    postMessage({ ...observation, globalErrors });
                    close();
                }, 0);
            };
            result = "constructed";
        } catch (error) {
            postMessage({ threw: error.name });
            close();
        }
        "#
        .into(),
        format!("{base_url}/parent.js"),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"constructed":true,"type":"error","intrinsicEvent":true,"target":true,"trusted":true,"flags":[false,false,false,false],"hasErrorDetails":false,"globalErrors":0}"#
    );
    timeout(TIMEOUT, server)
        .await
        .expect("timed out waiting for nested worker script request")
        .expect("nested worker script server should finish");
}

#[tokio::test]
async fn nested_worker_parse_errors_use_intrinsic_events_without_global_propagation() {
    ensure_v8();
    for child_type in ["classic", "module"] {
        let mut handle = spawn_worker(
            r#"
            const originalEvent = Event;
            let globalErrors = 0;
            let authorReads = 0;
            onerror = () => { ++globalErrors; return true; };
            const child = new Worker("data:text/javascript,function%20(", { type: "CHILD_TYPE" });
            child.onerror = event => {
                event.preventDefault();
                const observation = {
                    type: event.type,
                    intrinsicEvent: Object.getPrototypeOf(event) === originalEvent.prototype,
                    target: event.target === child,
                    trusted: event.isTrusted,
                    flags: [event.bubbles, event.cancelable, event.composed, event.defaultPrevented],
                    hasErrorDetails: ['message', 'filename', 'lineno', 'colno', 'error'].some(name => name in event)
                };
                setTimeout(() => {
                    postMessage({ ...observation, globalErrors, authorReads });
                    close();
                }, 0);
            };
            Object.defineProperty(globalThis, "Event", {
                configurable: true,
                get() { ++authorReads; throw new Error("author Event getter"); }
            });
            "#
            .replace("CHILD_TYPE", child_type),
            "test://nested_worker_parse_error".into(),
        );
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"{"type":"error","intrinsicEvent":true,"target":true,"trusted":true,"flags":[false,false,false,false],"hasErrorDetails":false,"globalErrors":0,"authorReads":0}"#,
            "{child_type} child bootstrap must not invoke author hooks or parent onerror"
        );
    }
}

#[tokio::test]
async fn worker_onmessage_exception_routes_through_worker_global_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesBoom: String(message).includes("boom-message"),
                filename,
                lineno,
                colno,
                errorMessage: error && error.message
            });
            close();
            return true;
        };
        onmessage = function() {
            throw new Error("boom-message");
        };
        "#
        .into(),
        "test://message_onerror".into(),
    );

    handle.post_message(serialize_test_string("go"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(msg);
    assert!(
        json.contains(r#""messageIncludesBoom":true"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""errorMessage":"boom-message""#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""filename":"test://message_onerror""#),
        "json: {json}"
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn worker_error_listener_receives_errorevent_instance() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        addEventListener("error", function(event) {
            postMessage({
                isErrorEvent: event instanceof ErrorEvent,
                typeString: Object.prototype.toString.call(event),
                isTrusted: event.isTrusted,
                message: event.message,
                filename: event.filename
            });
            event.preventDefault();
            close();
        });
        onmessage = function() {
            throw new Error("boom-message");
        };
        "#
        .into(),
        "test://error_event_instance".into(),
    );

    handle.post_message(serialize_test_string("go"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"isErrorEvent":true,"typeString":"[object ErrorEvent]","isTrusted":true,"message":"Uncaught Error: boom-message","filename":"test://error_event_instance"}"#
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn worker_domexception_error_event_message_preserves_name_and_message() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesName: String(message).includes("TypeError"),
                messageIncludesMessage: String(message).includes("dom-boom"),
                errorName: error && error.name,
                errorMessage: error && error.message
            });
            close();
            return true;
        };
        onmessage = function() {
            throw new DOMException("dom-boom", "TypeError");
        };
        "#
        .into(),
        "test://domexception_onerror".into(),
    );

    handle.post_message(serialize_test_string("go"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"messageIncludesName":true,"messageIncludesMessage":true,"errorName":"TypeError","errorMessage":"dom-boom"}"#
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn worker_error_report_ignores_throwing_accessors() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        throw {
            get name() { throw new Error("name getter should stay local"); },
            get message() { throw new Error("message getter should stay local"); },
            get stack() { throw new Error("stack getter should stay local"); },
        };
        "#
        .into(),
        "test://throwing_error_accessors".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error {
            message, filename, ..
        } => {
            assert!(
                message.contains("Uncaught"),
                "message should come from the original report: {message}"
            );
            assert_eq!(filename, "test://throwing_error_accessors");
        }
        WorkerToParentMessage::Post(_) => panic!("expected worker error"),
        WorkerToParentMessage::SubresourceNetwork(_)
        | WorkerToParentMessage::PendingSubresourceFetch(_)
        | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
        | WorkerToParentMessage::SubresourceContinue(_)
        | WorkerToParentMessage::WebSocketSubresource(_)
        | WorkerToParentMessage::WebSocketLifecycle(_)
        | WorkerToParentMessage::WebSocketFrame(_)
        | WorkerToParentMessage::Console(_)
        | WorkerToParentMessage::RuntimeInspectorMessages(_)
        | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
        | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
        | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
        | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
        | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
        | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
        | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
        | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
        | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
        | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
        | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
        | WorkerToParentMessage::ServiceWorkerShowNotification(_)
        | WorkerToParentMessage::ServiceWorkerGetNotifications(_)
        | WorkerToParentMessage::ServiceWorkerSyncRegistration(_)
        | WorkerToParentMessage::ServiceWorkerSyncGetTags(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_)
        | WorkerToParentMessage::ServiceWorkerCloseNotification(_)
        | WorkerToParentMessage::ServiceWorkerClientMessage(_)
        | WorkerToParentMessage::ServiceWorkerWorkerMessage(_)
        | WorkerToParentMessage::ServiceWorkerClientQuery(_)
        | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
        | WorkerToParentMessage::ServiceWorkerClientFocus(_)
        | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
        | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
        | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
        | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
        | WorkerToParentMessage::RuntimeInspectorResponse(_)
        | WorkerToParentMessage::SharedWorkerClosed => panic!("expected worker error"),
    }
}

#[tokio::test]
async fn worker_timer_exception_routes_through_worker_global_onerror_and_interval_continues() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let tick = 0;
        let intervalId = setInterval(function() {
            tick += 1;
            if (tick === 1) {
                throw new Error("tick-boom");
            }
            postMessage(`tick:${tick}`);
            clearInterval(intervalId);
            close();
        }, 10);

        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                phase: "onerror",
                messageIncludesBoom: String(message).includes("tick-boom"),
                filename,
                lineno,
                colno,
                errorMessage: error && error.message
            });
            return true;
        };
        "#
        .into(),
        "test://timer_onerror".into(),
    );

    let first = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(first);
    assert!(json.contains(r#""phase":"onerror""#), "json: {json}");
    assert!(
        json.contains(r#""messageIncludesBoom":true"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""errorMessage":"tick-boom""#),
        "json: {json}"
    );

    let second = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(second), r#""tick:2""#);

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected interval worker to close cleanly");
}

#[tokio::test]
async fn worker_queue_microtask_runs_after_current_stack() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const order = [];
        queueMicrotask(() => {
            order.push("microtask");
            postMessage(order.join(","));
        });
        order.push("sync");
        "#
        .into(),
        "test://queue_microtask".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""sync,microtask""#);
}

#[tokio::test]
async fn worker_queue_microtask_uses_typed_callback_fifo_and_error_reporting() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const events = [];
        const conversionErrors = [];
        for (const invoke of [
            () => queueMicrotask(),
            () => queueMicrotask(null),
            () => queueMicrotask({})
        ]) {
            try {
                invoke();
                conversionErrors.push("missing");
            } catch (error) {
                conversionErrors.push(error.name);
            }
        }

        onerror = (_message, _source, _line, _column, error) => {
            events.push(`error:${error && error.name}`);
            return true;
        };
        const callback = new Proxy(
            function() {
                "use strict";
                events.push(`callback:${this === undefined}:${arguments.length}`);
                queueMicrotask(() => events.push("nested"));
            },
            {
                apply(target, receiver, args) {
                    events.push(`apply:${receiver === undefined}:${args.length}`);
                    return Reflect.apply(target, receiver, args);
                }
            }
        );
        queueMicrotask(callback);
        Promise.resolve().then(() => events.push("promise"));
        queueMicrotask(() => events.push("second"));

        const revoked = Proxy.revocable(function() {}, {});
        revoked.revoke();
        let revokedAccepted = true;
        try {
            queueMicrotask(revoked.proxy);
        } catch {
            revokedAccepted = false;
        }
        queueMicrotask(() => {
            queueMicrotask(() => {
                postMessage({ conversionErrors, revokedAccepted, events });
                close();
            });
        });
        events.push("sync");
        "#
        .into(),
        "test://queue_microtask_webidl".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"conversionErrors":["TypeError","TypeError","TypeError"],"revokedAccepted":true,"events":["sync","apply:true:0","callback:true:0","promise","second","error:TypeError","nested"]}"#
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(
        next.is_none(),
        "worker should close after the exact microtask run"
    );
}

#[tokio::test]
async fn worker_top_level_exception_routes_through_worker_global_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesBoom: String(message).includes("top-boom"),
                filename,
                lineno,
                colno,
                errorMessage: error && error.message
            });
            close();
            return true;
        };
        throw new Error("top-boom");
        "#
        .into(),
        "test://top_level_onerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(msg);
    assert!(
        json.contains(r#""messageIncludesBoom":true"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""errorMessage":"top-boom""#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""filename":"test://top_level_onerror""#),
        "json: {json}"
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn unhandled_classic_worker_top_level_exception_is_runtime_phase() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        throw new Error("top-level-runtime");
        "#
        .into(),
        "test://top_level_runtime_phase".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, phase, .. } => {
            assert!(
                message.contains("top-level-runtime"),
                "expected top-level-runtime, got {message:?}"
            );
            assert_eq!(phase, WorkerErrorPhase::Runtime);
        }
        other => panic!("expected worker error, got {other:?}"),
    }
}

#[tokio::test]
async fn multiple_concurrent_workers() {
    ensure_v8();
    let mut handles: Vec<_> = (0..3)
        .map(|i| {
            spawn_worker(
                format!(r#"postMessage("worker_{i}");"#),
                format!("test://concurrent_{i}"),
            )
        })
        .collect();

    let mut received = Vec::new();
    for h in handles.iter_mut() {
        let msg = timeout(TIMEOUT, h.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        received.push(expect_post_json(msg));
    }

    received.sort();
    assert_eq!(
        received,
        vec![r#""worker_0""#, r#""worker_1""#, r#""worker_2""#,]
    );
}

#[tokio::test]
async fn worker_console_does_not_crash() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        console.log("hello");
        console.warn("warning");
        console.error("error");
        console.info("info");
        console.debug("debug");
        console.trace("trace");
        console.time("t");
        console.timeEnd("t");
        postMessage("done");
        "#
        .into(),
        "test://console".into(),
    );

    let mut console_messages = Vec::new();
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match msg {
            WorkerToParentMessage::Console(message) => console_messages.push(message.message),
            WorkerToParentMessage::Post(_) => break,
            other => panic!("unexpected worker console probe message: {other:?}"),
        }
    }
    assert_eq!(
        console_messages,
        [
            "log: hello",
            "warn: warning",
            "error: error",
            "info: info",
            "debug: debug",
            "trace: trace"
        ]
    );
}

#[tokio::test]
async fn worker_console_does_not_invoke_error_prepare_stack_trace() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let accessed = false;
        const originalPrepareStackTrace = Error.prepareStackTrace;
        try {
          Error.prepareStackTrace = () => {
            accessed = true;
            return "detected";
          };
          console.log(new Error(""));
          const afterConsole = accessed;
          void new Error("explicit stack access").stack;
          postMessage(`${afterConsole}|${accessed}`);
        } finally {
          Error.prepareStackTrace = originalPrepareStackTrace;
        }
        "#
        .into(),
        "test://console_prepare_stack_trace".into(),
    );

    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::Console(_) => {}
            WorkerToParentMessage::Post(payload) => {
                assert_eq!(stringify_payload(&payload), r#""false|true""#);
                break;
            }
            other => panic!("unexpected worker console probe message: {other:?}"),
        }
    }
}

#[tokio::test]
async fn worker_self_reference() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        self.postMessage(self === globalThis);
        "#
        .into(),
        "test://self_ref".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "true");
}

#[tokio::test]
async fn worker_empty_script_exits_on_drop() {
    ensure_v8();
    let handle = spawn_worker(
        r#"
        // intentionally empty — no onmessage, no timers
        "#
        .into(),
        "test://empty".into(),
    );

    // Worker stays alive (per spec) until the parent drops the handle.
    // Drop sends Terminate, then thread exits.
    drop(handle);
    // If we reach here without hanging, the test passes.
}

#[tokio::test]
async fn worker_syntax_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        function( { broken syntax
        "#
        .into(),
        "test://syntax_error".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(matches!(msg, WorkerToParentMessage::Error { .. }));
}

#[tokio::test]
async fn worker_drop_terminates() {
    ensure_v8();
    let handle = spawn_worker(
        r#"
        onmessage = function(e) {
            postMessage("alive");
        };
        "#
        .into(),
        "test://drop_terminate".into(),
    );

    drop(handle);
    // If we reach here without hanging, the test passes.
}

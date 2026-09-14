use super::*;
use crate::runtime::RendererNetworkOutputItem;
use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};

#[tokio::test]
async fn intercepted_fetch_failure_retains_physical_head_and_prefix() {
    intercepted_failure(SubresourceResourceType::Fetch).await;
}

#[tokio::test]
async fn intercepted_xhr_failure_retains_physical_head_and_prefix() {
    intercepted_failure(SubresourceResourceType::Xhr).await;
}

async fn intercepted_failure(resource_type: SubresourceResourceType) {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_request_head(&mut stream).await.unwrap();
        assert!(request.starts_with("GET /partial "));
        stream.write_all(b"HTTP/1.1 200 OK\r\nX-Physical: retained\r\nContent-Length: 6\r\nConnection: close\r\n\r\npre").await.unwrap();
    });
    let script = match resource_type {
        SubresourceResourceType::Fetch => {
            "onmessage=async()=>{try{const r=await fetch('/partial');await r.text();postMessage('resolved');}catch(e){postMessage('rejected');}close();};"
        }
        SubresourceResourceType::Xhr => {
            "onmessage=()=>{const x=new XMLHttpRequest();x.open('GET','/partial');x.onload=()=>{postMessage('resolved');close();};x.onerror=()=>{postMessage('rejected');close();};x.send();};"
        }
        _ => unreachable!(),
    };
    let mut worker = spawn_worker_with_request_client(
        script.into(),
        format!("{origin}/worker.js"),
        ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
    );
    worker.set_fetch_subresource_interception(true, Some(resource_type));
    worker.post_message(serialize_test_string("go"));
    let mut items = Vec::new();
    let mut posts = Vec::new();
    timeout(TIMEOUT, async {
        while let Some(message) = worker.recv().await {
            match message {
                WorkerToParentMessage::Network(observation) => {
                    let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                        panic!("resource receipt")
                    };
                    items.push(item.clone());
                }
                WorkerToParentMessage::FetchInterception(pause) => {
                    assert!(
                        matches!(
                            pause.stage(),
                            crate::runtime::RendererWorkerFetchStage::Request(_)
                        ),
                        "a failed transport cannot enter a response decision"
                    );
                    continue_worker_request(&pause, true, false).await;
                }
                WorkerToParentMessage::Post(payload) => posts.push(stringify_payload(&payload)),
                other => panic!("unexpected Worker output: {other:?}"),
            }
        }
    })
    .await
    .expect("Worker must settle the failed request and close");
    worker.terminate_and_join();
    server.await.unwrap();
    assert_eq!(posts, ["\"rejected\""]);
    assert_eq!(
        items.len(),
        3,
        "admission, physical head and one failed terminal: {items:?}"
    );
    let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = items[0].as_ref() else {
        panic!("request admission first")
    };
    let ScriptNetworkOutputItem::SubresourceResponseStarted(head) = items[1].as_ref() else {
        panic!("the failed stream must retain its physical head")
    };
    assert_eq!(head.handle(), start.handle());
    assert_eq!(head.status(), 200);
    assert!(
        head.response_headers()
            .iter()
            .any(|(name, value)| name == "x-physical" && value == "retained")
    );
    let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) = items[2].as_ref() else {
        panic!("one terminal last")
    };
    assert_eq!(terminal.handle(), start.handle());
    let SubresourceBodyFinishedResult::FailedWithPartialBody { partial_body, .. } =
        terminal.result()
    else {
        panic!("the failed stream must retain its received bytes")
    };
    assert_eq!(partial_body.clone_body_bytes(), b"pre");
}

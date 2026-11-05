use super::*;
use crate::protocol_server::{AppState, serve_listener};
use futures_util::{SinkExt, StreamExt};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask,
    runtime::{NavigationRuntimeConfig, storage_partition::StoragePartitionState},
};
use moli_fetch::FetchConfig;
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

type TestCdpSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct TestCdpTargetSession {
    target_id: String,
    session_id: String,
}

async fn recv_until_match(
    socket: &mut TestCdpSocket,
    predicate: impl Fn(&Value) -> bool,
) -> Vec<Value> {
    timeout(Duration::from_secs(30), async {
        let mut messages = Vec::new();
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                let message = serde_json::from_str(&text).unwrap();
                let done = predicate(&message);
                messages.push(message);
                if done {
                    return messages;
                }
            }
        }
    })
    .await
    .expect("browser response or lifecycle event")
}

async fn send_cdp_command(
    socket: &mut TestCdpSocket,
    id: u64,
    method: &str,
    session: Option<&str>,
    params: Value,
) -> Vec<Value> {
    let mut command = json!({"id": id, "method": method, "params": params});
    if let Some(session) = session {
        command["sessionId"] = json!(session);
    }
    socket
        .send(Message::Text(command.to_string().into()))
        .await
        .unwrap();
    recv_until_match(socket, |message| message["id"] == id).await
}

fn response_by_id(messages: &[Value], id: u64) -> &Value {
    messages.iter().find(|message| message["id"] == id).unwrap()
}

async fn cdp_create_browser_context(socket: &mut TestCdpSocket, id: u64) -> String {
    let messages =
        send_cdp_command(socket, id, "Target.createBrowserContext", None, json!({})).await;
    response_by_id(&messages, id)["result"]["browserContextId"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn cdp_create_attached_target(
    socket: &mut TestCdpSocket,
    id: u64,
    context: &str,
) -> TestCdpTargetSession {
    let messages = send_cdp_command(
        socket,
        id,
        "Target.createTarget",
        None,
        json!({"browserContextId": context, "url": "about:blank"}),
    )
    .await;
    let target_id = response_by_id(&messages, id)["result"]["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let messages = send_cdp_command(
        socket,
        id + 1,
        "Target.attachToTarget",
        None,
        json!({"targetId": target_id, "flatten": true}),
    )
    .await;
    let session_id = response_by_id(&messages, id + 1)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    TestCdpTargetSession {
        target_id,
        session_id,
    }
}

async fn navigate_fixture_document(
    socket: &mut TestCdpSocket,
    id: u64,
    target: &TestCdpTargetSession,
    url: &str,
) {
    for (id, method, params) in [
        (90, "Page.enable", json!({})),
        (
            91,
            "Page.setLifecycleEventsEnabled",
            json!({"enabled": true}),
        ),
    ] {
        let response = send_cdp_command(socket, id, method, Some(&target.session_id), params).await;
        assert!(response_by_id(&response, id)["error"].is_null());
    }
    let response = send_cdp_command(
        socket,
        id,
        "Page.navigate",
        Some(&target.session_id),
        json!({"url": url}),
    )
    .await;
    let loader = response_by_id(&response, id)["result"]["loaderId"]
        .as_str()
        .unwrap();
    let parsed = |message: &Value| {
        message["sessionId"] == target.session_id
            && message["method"] == "Page.lifecycleEvent"
            && message["params"]["loaderId"] == loader
            && message["params"]["name"] == "DOMContentLoaded"
    };
    if !response.iter().any(parsed) {
        recv_until_match(socket, parsed).await;
    }
}

#[derive(Debug)]
struct ReadAcePageTask;
impl moli_core::ace_page_task::AcePageTask for ReadAcePageTask {
    fn kind(&self) -> moli_core::ace_page_task::AcePageTaskKind {
        moli_core::ace_page_task::AcePageTaskKind::ReadOnly
    }
    fn run(
        self: Box<Self>,
        host: &mut dyn moli_core::ace_page_task::PageExtensionHost,
    ) -> anyhow::Result<moli_core::ace_page_task::AcePageTaskOutput> {
        Ok(moli_core::ace_page_task::AcePageTaskOutput {
            value: json!({"url":host.url(), "id":0, "result":null, "error":"payload"}),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedder_tasks_share_targets_with_cdp_and_handles_are_revoked() {
    let (server, state) = instance(None).await;
    let runtime = BrowserRuntimeHandle {
        directory: state.cdp_agent_host_directory.clone(),
        registry: state.cdp_owner_registry.clone(),
        control: state.control.clone(),
    };
    let initial = runtime.targets().await.unwrap();
    let initial_id = initial[0].target_id.as_ref().unwrap().as_str();
    runtime
        .run_ace_page_task(initial_id, Box::new(ReadAcePageTask))
        .await
        .expect("initial page tasks need no CDP attachment");
    let mut socket = browser(&state).await;
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &context).await;
    let url = "data:text/html,<p>Embedder task</p>";
    navigate_fixture_document(&mut socket, 4, &target, url).await;
    assert!(runtime.targets().await.unwrap().iter().any(|info| {
        info.target_id
            .as_ref()
            .is_some_and(|id| id.as_str() == target.target_id)
    }));
    let result = runtime
        .run_ace_page_task(&target.target_id, Box::new(ReadAcePageTask))
        .await
        .unwrap();
    assert_eq!(result.value["url"], url);
    assert_eq!(result.value["id"], 0);
    assert!(result.value["result"].is_null());
    assert_eq!(result.value["error"], "payload");
    assert_eq!(
        result.before.source_target_id.as_deref(),
        Some(target.target_id.as_str())
    );
    let evaluated = send_cdp_command(
        &mut socket,
        5,
        "Runtime.evaluate",
        Some(&target.session_id),
        json!({"expression":"document.querySelector('p').textContent", "returnByValue":true}),
    )
    .await;
    assert_eq!(
        response_by_id(&evaluated, 5)["result"]["result"]["value"],
        "Embedder task"
    );
    let detached = send_cdp_command(
        &mut socket,
        6,
        "Target.detachFromTarget",
        None,
        json!({"sessionId":target.session_id}),
    )
    .await;
    assert!(response_by_id(&detached, 6)["error"].is_null());
    assert!(
        runtime
            .run_ace_page_task(&target.target_id, Box::new(ReadAcePageTask))
            .await
            .is_ok(),
        "embedder tasks are independent of CDP attachments"
    );
    let closed = send_cdp_command(
        &mut socket,
        7,
        "Target.closeTarget",
        None,
        json!({"targetId":target.target_id}),
    )
    .await;
    assert!(response_by_id(&closed, 7)["error"].is_null());
    assert_eq!(
        runtime
            .run_ace_page_task(&target.target_id, Box::new(ReadAcePageTask))
            .await
            .unwrap_err()
            .code,
        404
    );
    state.control.request(InstanceCommand::Shutdown);
    joined(server).await;
    assert_eq!(runtime.targets().await.unwrap_err().code, 404);
}

#[derive(Debug)]
struct HeldAcePageTask {
    entered: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
    runs: Arc<std::sync::atomic::AtomicUsize>,
}
impl moli_core::ace_page_task::AcePageTask for HeldAcePageTask {
    fn kind(&self) -> moli_core::ace_page_task::AcePageTaskKind {
        moli_core::ace_page_task::AcePageTaskKind::ReadOnly
    }
    fn run(
        self: Box<Self>,
        _: &mut dyn moli_core::ace_page_task::PageExtensionHost,
    ) -> anyhow::Result<moli_core::ace_page_task::AcePageTaskOutput> {
        let _ = self.entered.send(());
        self.release.recv_timeout(Duration::from_secs(5))?;
        self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(moli_core::ace_page_task::AcePageTaskOutput { value: json!({}) })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_an_embedder_waiter_does_not_cancel_or_repeat_admitted_work() {
    let (server, state) = instance(None).await;
    let runtime = BrowserRuntimeHandle {
        directory: state.cdp_agent_host_directory.clone(),
        registry: state.cdp_owner_registry.clone(),
        control: state.control.clone(),
    };
    let mut socket = browser(&state).await;
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &context).await;
    navigate_fixture_document(&mut socket, 4, &target, "data:text/html,<p>Held task</p>").await;
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let task = HeldAcePageTask {
        entered,
        release: held,
        runs: runs.clone(),
    };
    let caller = runtime.clone();
    let target_id = target.target_id.clone();
    let waiter =
        tokio::spawn(async move { caller.run_ace_page_task(&target_id, Box::new(task)).await });
    timeout(Duration::from_secs(5), started)
        .await
        .unwrap()
        .unwrap();
    waiter.abort();
    let _ = waiter.await;
    release.send(()).unwrap();
    runtime
        .run_ace_page_task(&target.target_id, Box::new(ReadAcePageTask))
        .await
        .unwrap();
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    state.control.request(InstanceCommand::Shutdown);
    joined(server).await;
}

async fn instance(
    profile_dir: Option<PathBuf>,
) -> (tokio::task::JoinHandle<anyhow::Result<()>>, AppState) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owned instance");
    let addr = listener.local_addr().expect("owned instance address");
    let state = AppState::new_with_storage_partition_and_runtime_config(
        addr,
        Arc::new(
            StoragePartitionState::open(profile_dir.as_deref()).expect("owned storage partition"),
        ),
        NavigationRuntimeConfig::new(
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::OnDemand,
        )
        .with_debugger_enabled(true),
        crate::config::DEFAULT_SCREENCAST_INTERVAL_MS,
        BrowserInstanceControl::new(),
    )
    .expect("owned app state");
    let serving = state.clone();
    let task = tokio::spawn(async move { serve_listener(listener, serving).await });
    (task, state)
}

async fn browser(state: &AppState) -> TestCdpSocket {
    connect_async(state.browser_ws_url.as_str())
        .await
        .expect("connect owned browser")
        .0
}

async fn joined(task: tokio::task::JoinHandle<anyhow::Result<()>>) {
    timeout(Duration::from_secs(7), task)
        .await
        .expect("instance exits within total shutdown budget")
        .expect("server task joins")
        .expect("server completes shutdown without a timeout or join error");
}

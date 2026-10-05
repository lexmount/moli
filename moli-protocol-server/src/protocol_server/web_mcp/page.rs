use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rmcp::model::Tool;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::catalog::{Catalog, SiteTool};
use crate::{
    cdp_frontend::CdpFrontendEndpoint,
    cdp_writer::{CdpOutputReceiver, channel_sink},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(super) struct WebMcpPage(Arc<PageHandle>);

struct PageHandle {
    endpoint: CdpFrontendEndpoint,
    frontend_id: u64,
    target_id: String,
    state: Arc<PageState>,
    reader: tokio::task::AbortHandle,
}

struct CommandWaiter {
    tx: oneshot::Sender<Value>,
    invocation: bool,
}

struct PendingCommand<'a> {
    state: &'a PageState,
    id: u64,
    retain_for_ack: bool,
}

impl Drop for PendingCommand<'_> {
    fn drop(&mut self) {
        if !self.retain_for_ack {
            self.state.commands.lock().remove(&self.id);
        }
    }
}

struct PageState {
    commands: Mutex<HashMap<u64, CommandWaiter>>,
    next_command: AtomicU64,
    catalog: Mutex<Catalog>,
    changed: watch::Sender<u64>,
    responses: broadcast::Sender<Value>,
    closed: CancellationToken,
}

impl Drop for PageHandle {
    fn drop(&mut self) {
        self.state.closed.cancel();
        self.endpoint.detach_target(self.frontend_id);
        self.reader.abort();
    }
}

impl WebMcpPage {
    pub(super) async fn open(endpoint: CdpFrontendEndpoint, url: &str) -> Result<Self> {
        url::Url::parse(url).context("WebMCP requires an absolute URL")?;
        endpoint.ensure_default_target_published().await?;
        let target_id = moli_protocol::DEFAULT_CDP_PAGE_TARGET_ID.to_owned();
        let (sink, output) = channel_sink();
        let frontend_id = endpoint.attach_target(target_id.clone(), sink).await?;
        let (changed, _) = watch::channel(0);
        let (responses, _) = broadcast::channel(1024);
        let state = Arc::new(PageState {
            commands: Mutex::new(HashMap::new()),
            next_command: AtomicU64::new(1),
            catalog: Mutex::new(Catalog::default()),
            changed,
            responses,
            closed: CancellationToken::new(),
        });
        let reader = tokio::spawn(read_output(
            endpoint.clone(),
            frontend_id,
            target_id.clone(),
            state.clone(),
            output,
        ))
        .abort_handle();
        let page = Self(Arc::new(PageHandle {
            endpoint,
            frontend_id,
            target_id,
            state,
            reader,
        }));
        page.command("Page.enable", json!({})).await?;
        page.command("Runtime.enable", json!({})).await?;
        page.command("WebMCP.enable", json!({})).await?;
        let previous = page.command("Page.getFrameTree", json!({})).await?;
        let navigation = page.command("Page.navigate", json!({"url":url})).await?;
        if let Some(error) = navigation["errorText"].as_str() {
            bail!("WebMCP navigation failed: {error}");
        }
        loop {
            // Page.navigate can acknowledge before the requested Document
            // commits. The previous about:blank page may already be complete.
            let tree = page.command("Page.getFrameTree", json!({})).await?;
            if navigation["loaderId"].is_string()
                && tree["frameTree"]["frame"]["loaderId"]
                    == previous["frameTree"]["frame"]["loaderId"]
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            let ready = page
                .command(
                    "Runtime.evaluate",
                    json!({
                        "expression":"document.readyState", "returnByValue":true
                    }),
                )
                .await?;
            if matches!(
                ready["result"]["value"].as_str(),
                Some("interactive" | "complete")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(page)
    }

    pub(super) fn target_id(&self) -> &str {
        &self.0.target_id
    }

    pub(super) fn changes(&self) -> watch::Receiver<u64> {
        self.0.state.changed.subscribe()
    }

    pub(super) fn closed(&self) -> &CancellationToken {
        &self.0.state.closed
    }

    pub(super) fn tools(&self) -> Result<Vec<Tool>> {
        if self.closed().is_cancelled() {
            bail!("WebMCP page is closed");
        }
        Ok(self.0.state.catalog.lock().list())
    }

    pub(super) fn find(&self, name: &str) -> Option<SiteTool> {
        self.0.state.catalog.lock().find(name)
    }

    pub(super) async fn command(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.0.state.next_command.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut commands = self.0.state.commands.lock();
            if self.closed().is_cancelled() {
                bail!("WebMCP page is closed");
            }
            commands.insert(
                id,
                CommandWaiter {
                    tx,
                    invocation: method == "WebMCP.invokeTool",
                },
            );
        }
        let mut pending = PendingCommand {
            state: &self.0.state,
            id,
            retain_for_ack: false,
        };
        let request = json!({"id":id,"method":method,"params":params}).to_string();
        let result = tokio::time::timeout(COMMAND_TIMEOUT, async {
            if !self.0.endpoint.command(self.0.frontend_id, request).await {
                bail!("WebMCP page owner stopped");
            }
            pending.retain_for_ack = method == "WebMCP.invokeTool";
            tokio::select! {
                biased;
                _ = self.closed().cancelled() => bail!("WebMCP page is closed"),
                reply = rx => Ok(reply.context("WebMCP page stopped before command reply")?),
            }
        })
        .await;
        let reply = result.context("WebMCP command timed out")??;
        if let Some(error) = reply.get("error") {
            bail!("{method}: {error}");
        }
        Ok(reply["result"].clone())
    }

    pub(super) async fn invoke(
        &self,
        tool: SiteTool,
        arguments: Value,
        auto_submit: bool,
    ) -> Result<Value> {
        // Subscribe before scheduling: a synchronous tool may finish before
        // the invocationId command reply reaches this frontend.
        let mut responses = self.0.state.responses.subscribe();
        let reply = self
            .command(
                "WebMCP.invokeTool",
                json!({
                    "frameId":tool.frame_id,"toolName":tool.name,"input":arguments
                }),
            )
            .await?;
        let id = reply["invocationId"]
            .as_str()
            .context("missing WebMCP invocationId")?
            .to_owned();
        let mut guard = CleanupGuard {
            page: self.clone(),
            method: "WebMCP.cancelInvocation",
            params: Some(json!({"invocationId":id})),
        };
        if auto_submit
            && !tool.autosubmit
            && let Some(node_id) = tool.backend_node_id
        {
            tokio::select! {
                biased;
                response = self.wait_response(&mut responses, &id) => {
                    let response = response?;
                    guard.params = None;
                    return Ok(response);
                }
                confirmed = self.confirm_form(node_id) => confirmed?,
            }
        }
        let response = self.wait_response(&mut responses, &id).await?;
        guard.params = None;
        Ok(response)
    }

    async fn wait_response(
        &self,
        responses: &mut broadcast::Receiver<Value>,
        id: &str,
    ) -> Result<Value> {
        loop {
            let response = tokio::select! {
                biased;
                _ = self.closed().cancelled() => bail!("WebMCP page is closed"),
                response = responses.recv() => response.context("lost WebMCP invocation response")?,
            };
            if response["invocationId"] == id {
                return Ok(response);
            }
        }
    }

    async fn confirm_form(&self, node_id: u64) -> Result<()> {
        let node = self
            .command("DOM.resolveNode", json!({"backendNodeId":node_id}))
            .await?;
        let object_id = node["object"]["objectId"]
            .as_str()
            .context("tool form is unavailable")?;
        let _release = CleanupGuard {
            page: self.clone(),
            method: "Runtime.releaseObject",
            params: Some(json!({"objectId":object_id})),
        };
        loop {
            let result = self.command("Runtime.callFunctionOn", json!({
            "objectId":object_id, "returnByValue":true,
            "functionDeclaration": "function() {\n\
                if (!this.matches(':tool-form-active')) return false;\n\
                const button = Array.from(this.elements).find(el => el.matches(':tool-submit-active'));\n\
                if (button) {\n\
                    if (button.matches(':disabled')) throw new Error('Tool submit button is disabled');\n\
                    button.click();\n\
                } else { this.requestSubmit(); }\n\
                return true;\n\
            }"
            })).await?;
            if let Some(error) = result.get("exceptionDetails") {
                bail!("form confirmation failed: {error}");
            }
            if result["result"]["value"] == true {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

struct CleanupGuard {
    page: WebMcpPage,
    method: &'static str,
    params: Option<Value>,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Some(params) = self.params.take() {
            let page = self.page.clone();
            let method = self.method;
            tokio::spawn(async move {
                let _ = page.command(method, params).await;
            });
        }
    }
}

async fn read_output(
    endpoint: CdpFrontendEndpoint,
    frontend_id: u64,
    target_id: String,
    state: Arc<PageState>,
    mut output: CdpOutputReceiver,
) {
    loop {
        let raw = tokio::select! {
            biased;
            _ = endpoint.wait_for_shutdown() => break,
            message = output.recv() => match message { Some(message) => message, None => break },
        };
        let Ok(message) = serde_json::from_str::<Value>(&raw) else {
            break;
        };
        if let Some(id) = message["id"].as_u64() {
            let waiter = state.commands.lock().remove(&id);
            if let Some(waiter) = waiter {
                let invocation_id = waiter
                    .invocation
                    .then(|| {
                        message["result"]["invocationId"]
                            .as_str()
                            .map(str::to_owned)
                    })
                    .flatten();
                if waiter.tx.send(message).is_err()
                    && let Some(invocation_id) = invocation_id
                {
                    // The MCP request was dropped while native scheduling was
                    // still acknowledging it. Cancel even without a caller.
                    let id = state.next_command.fetch_add(1, Ordering::Relaxed);
                    let _ = endpoint
                        .command(
                            frontend_id,
                            json!({
                                "id":id,"method":"WebMCP.cancelInvocation",
                                "params":{"invocationId":invocation_id}
                            })
                            .to_string(),
                        )
                        .await;
                }
            }
            continue;
        }
        let params = &message["params"];
        let changed = match message["method"].as_str() {
            Some("WebMCP.toolsAdded") => {
                let mut catalog = state.catalog.lock();
                let mut changed = false;
                for raw in params["tools"].as_array().into_iter().flatten() {
                    changed |= catalog.add(raw, &target_id);
                }
                changed
            }
            Some("WebMCP.toolsRemoved") => {
                let mut catalog = state.catalog.lock();
                let mut changed = false;
                for raw in params["tools"].as_array().into_iter().flatten() {
                    changed |= catalog.remove(raw);
                }
                changed
            }
            Some("Page.frameDetached") => params["frameId"]
                .as_str()
                .is_some_and(|frame| state.catalog.lock().remove_frame(frame)),
            Some("Runtime.executionContextsCleared") => state.catalog.lock().clear(),
            Some("WebMCP.toolResponded") => {
                let _ = state.responses.send(params.clone());
                if let Some(object_id) = params["exception"]["objectId"].as_str() {
                    // MCP exposes the exception description, not its inspector handle.
                    let id = state.next_command.fetch_add(1, Ordering::Relaxed);
                    let _ = endpoint.command(frontend_id, json!({
                        "id":id, "method":"Runtime.releaseObject", "params":{"objectId":object_id}
                    }).to_string()).await;
                }
                false
            }
            Some("Inspector.detached") => break,
            _ => false,
        };
        if changed {
            state.changed.send_modify(|version| *version += 1);
        }
    }
    state.closed.cancel();
    state.commands.lock().clear();
}

pub(super) fn response_error(response: &Value) -> String {
    response["errorText"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| response["exception"]["description"].as_str())
        .or_else(|| response["status"].as_str())
        .unwrap_or("WebMCP invocation failed")
        .to_owned()
}

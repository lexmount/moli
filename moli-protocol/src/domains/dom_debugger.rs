use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererDomDebuggerDomBreakpointResolution,
    RendererDomDebuggerEventListener, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerEventListenersResolution, RendererDomDebuggerXhrBreakpoint,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::domains::actions::DomDebuggerAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) struct PendingDomDebuggerCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    pending: PendingPageCommand,
    operation: CompletedDomDebuggerOperation,
}

pub(crate) struct CompletedDomDebuggerCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    completed: Result<CompletedPageCommand, String>,
    operation: CompletedDomDebuggerOperation,
}

#[derive(Clone)]
enum CompletedDomDebuggerOperation {
    GetEventListeners,
    ConfigureEventListenerBreakpoint {
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    },
    ConfigureXhrBreakpoint {
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    },
    ConfigureDomBreakpoint,
}

pub(crate) enum DomDebuggerCommandTaskStep {
    Pending(PendingDomDebuggerCommandDispatch),
    Complete(CommandOutputPlan),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetEventListenersParams {
    object_id: String,
    #[serde(default = "default_depth")]
    depth: i32,
    #[serde(default)]
    pierce: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventListenerBreakpointParams {
    event_name: String,
    #[serde(default)]
    target_name: Option<String>,
}

#[derive(Deserialize)]
struct XhrBreakpointParams {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DomBreakpointParams {
    node_id: u32,
    r#type: String,
}

fn default_depth() -> i32 {
    1
}

impl PendingDomDebuggerCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedDomDebuggerCommandDispatch {
        CompletedDomDebuggerCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
            operation: self.operation,
        }
    }
}

impl CompletedDomDebuggerCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

pub(crate) fn try_start_dom_debugger_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> DomDebuggerCommandTaskStep {
    let action = match cmd.parse_action::<DomDebuggerAction>() {
        Some(action) => action,
        None => {
            return DomDebuggerCommandTaskStep::Complete(CommandOutputPlan::error(
                -32601,
                "UnknownMethod",
            ));
        }
    };
    if let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id) {
        return DomDebuggerCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
    }
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let pending_and_operation = match action {
        DomDebuggerAction::GetEventListeners => {
            let params = match cmd.get_params::<GetEventListenersParams>() {
                Ok(Some(params)) => params,
                _ => return invalid_parameters(),
            };
            conn.loaded_page_mut_for_protocol_access(cmd.session_id)
                .and_then(|page| {
                    page.start_dom_debugger_get_event_listeners(
                        renderer_inspector_session_id,
                        params.object_id,
                        params.depth,
                        params.pierce,
                    )
                    .map(|pending| (pending, CompletedDomDebuggerOperation::GetEventListeners))
                    .map_err(|error| error.to_string())
                })
        }
        DomDebuggerAction::SetEventListenerBreakpoint
        | DomDebuggerAction::RemoveEventListenerBreakpoint => {
            let params = match cmd.get_params::<EventListenerBreakpointParams>() {
                Ok(Some(params)) => params,
                _ => return invalid_parameters(),
            };
            if params.event_name.is_empty() {
                return DomDebuggerCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "Event name is empty",
                ));
            }
            let breakpoint = RendererDomDebuggerEventListenerBreakpoint::new(
                params.event_name,
                params.target_name,
            );
            let enabled = action == DomDebuggerAction::SetEventListenerBreakpoint;
            conn.loaded_page_mut_for_protocol_access(cmd.session_id)
                .and_then(|page| {
                    page.start_dom_debugger_configure_event_listener_breakpoint(
                        renderer_inspector_session_id,
                        breakpoint.clone(),
                        enabled,
                    )
                    .map(|pending| {
                        (
                            pending,
                            CompletedDomDebuggerOperation::ConfigureEventListenerBreakpoint {
                                breakpoint,
                                enabled,
                            },
                        )
                    })
                    .map_err(|error| error.to_string())
                })
        }
        DomDebuggerAction::SetXHRBreakpoint | DomDebuggerAction::RemoveXHRBreakpoint => {
            let params = match cmd.get_params::<XhrBreakpointParams>() {
                Ok(Some(params)) => params,
                _ => return invalid_parameters(),
            };
            let breakpoint = RendererDomDebuggerXhrBreakpoint::new(params.url);
            let enabled = action == DomDebuggerAction::SetXHRBreakpoint;
            conn.loaded_page_mut_for_protocol_access(cmd.session_id)
                .and_then(|page| {
                    page.start_dom_debugger_configure_xhr_breakpoint(
                        renderer_inspector_session_id,
                        breakpoint.clone(),
                        enabled,
                    )
                    .map(|pending| {
                        (
                            pending,
                            CompletedDomDebuggerOperation::ConfigureXhrBreakpoint {
                                breakpoint,
                                enabled,
                            },
                        )
                    })
                    .map_err(|error| error.to_string())
                })
        }
        DomDebuggerAction::SetDOMBreakpoint | DomDebuggerAction::RemoveDOMBreakpoint => {
            let params = match cmd.get_params::<DomBreakpointParams>() {
                Ok(Some(params)) => params,
                _ => return invalid_parameters(),
            };
            let enabled = action == DomDebuggerAction::SetDOMBreakpoint;
            conn.loaded_page_mut_for_protocol_access(cmd.session_id)
                .and_then(|page| {
                    page.start_dom_debugger_configure_dom_breakpoint(
                        renderer_inspector_session_id,
                        params.node_id,
                        params.r#type,
                        enabled,
                    )
                    .map(|pending| {
                        (
                            pending,
                            CompletedDomDebuggerOperation::ConfigureDomBreakpoint,
                        )
                    })
                    .map_err(|error| error.to_string())
                })
        }
    };
    match pending_and_operation {
        Ok((pending, operation)) => {
            DomDebuggerCommandTaskStep::Pending(PendingDomDebuggerCommandDispatch {
                command_id: cmd.id,
                owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
                pending,
                operation,
            })
        }
        Err(message) => {
            DomDebuggerCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message))
        }
    }
}

fn invalid_parameters() -> DomDebuggerCommandTaskStep {
    DomDebuggerCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "Invalid parameters"))
}

pub(crate) fn complete_pending_dom_debugger_command(
    conn: &mut CdpConnection,
    completed: CompletedDomDebuggerCommandDispatch,
) -> CommandOutputPlan {
    let session_id = completed.owner_scope.session_id().map(str::to_owned);
    match completed.operation {
        CompletedDomDebuggerOperation::GetEventListeners => {
            let resolution = completed.completed.and_then(|completion| {
                conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner_scope)
                    .and_then(|page| {
                        page.finish_dom_debugger_get_event_listeners(completion)
                            .map_err(|error| error.to_string())
                    })
            });
            match resolution {
                Ok(RendererDomDebuggerEventListenersResolution::Found(listeners)) => {
                    CommandOutputPlan::result(json!({
                        "listeners": listeners.into_iter().map(listener_payload).collect::<Vec<_>>()
                    }))
                }
                Ok(RendererDomDebuggerEventListenersResolution::InvalidRemoteObjectId(message))
                | Err(message) => CommandOutputPlan::error(-32000, message),
            }
        }
        CompletedDomDebuggerOperation::ConfigureEventListenerBreakpoint {
            breakpoint,
            enabled,
        } => {
            let completion = completed.completed.and_then(|completion| {
                conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner_scope)
                    .and_then(|page| {
                        page.finish_unit_runtime_page_command(
                            completion,
                            "DOMDebugger event listener breakpoint",
                        )
                        .map_err(|error| error.to_string())
                    })
            });
            if let Err(message) = completion {
                return CommandOutputPlan::error(-32000, message);
            }
            let recorded = conn.with_target_devtools_session_state_for_session_mut(
                session_id.as_deref(),
                |state| {
                    if enabled {
                        state
                            .dom_debugger_event_listener_breakpoints
                            .insert(breakpoint);
                    } else {
                        state
                            .dom_debugger_event_listener_breakpoints
                            .remove(&breakpoint);
                    }
                },
            );
            if recorded.is_none() {
                return CommandOutputPlan::error(-32000, "NoSuchTarget");
            }
            CommandOutputPlan::result(json!({}))
        }
        CompletedDomDebuggerOperation::ConfigureXhrBreakpoint {
            breakpoint,
            enabled,
        } => {
            let completion = completed.completed.and_then(|completion| {
                conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner_scope)
                    .and_then(|page| {
                        page.finish_unit_runtime_page_command(
                            completion,
                            "DOMDebugger XHR breakpoint",
                        )
                        .map_err(|error| error.to_string())
                    })
            });
            if let Err(message) = completion {
                return CommandOutputPlan::error(-32000, message);
            }
            let recorded = conn.with_target_devtools_session_state_for_session_mut(
                session_id.as_deref(),
                |state| {
                    if enabled {
                        state.dom_debugger_xhr_breakpoints.insert(breakpoint);
                    } else {
                        state.dom_debugger_xhr_breakpoints.remove(&breakpoint);
                    }
                },
            );
            if recorded.is_none() {
                return CommandOutputPlan::error(-32000, "NoSuchTarget");
            }
            CommandOutputPlan::result(json!({}))
        }
        CompletedDomDebuggerOperation::ConfigureDomBreakpoint => {
            let resolution = completed.completed.and_then(|completion| {
                conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner_scope)
                    .and_then(|page| {
                        page.finish_dom_debugger_configure_dom_breakpoint(completion)
                            .map_err(|error| error.to_string())
                    })
            });
            match resolution {
                Ok(RendererDomDebuggerDomBreakpointResolution::Configured) => {
                    CommandOutputPlan::result(json!({}))
                }
                Ok(RendererDomDebuggerDomBreakpointResolution::NodeNotFound) => {
                    CommandOutputPlan::error(-32000, "Could not find node with given id")
                }
                Ok(RendererDomDebuggerDomBreakpointResolution::UnknownType(breakpoint_type)) => {
                    CommandOutputPlan::error(
                        -32000,
                        format!("Unknown DOM breakpoint type: {breakpoint_type}"),
                    )
                }
                Err(message) => CommandOutputPlan::error(-32000, message),
            }
        }
    }
}

fn listener_payload(listener: RendererDomDebuggerEventListener) -> Value {
    let mut payload = Map::from_iter([
        ("type".to_owned(), Value::String(listener.event_type)),
        ("useCapture".to_owned(), Value::Bool(listener.use_capture)),
        ("passive".to_owned(), Value::Bool(listener.passive)),
        ("once".to_owned(), Value::Bool(listener.once)),
        ("scriptId".to_owned(), Value::String(listener.script_id)),
        ("lineNumber".to_owned(), json!(listener.line_number)),
        ("columnNumber".to_owned(), json!(listener.column_number)),
    ]);
    if let Some(handler) = listener.handler {
        payload.insert("handler".to_owned(), handler);
    }
    if let Some(original_handler) = listener.original_handler {
        payload.insert("originalHandler".to_owned(), original_handler);
    }
    if let Some(backend_node_id) = listener.backend_node_id {
        payload.insert("backendNodeId".to_owned(), json!(backend_node_id));
    }
    Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use serde_json::{Value, json};

    use crate::{
        conn::{BrowserContext, CdpCommandTaskStep},
        testing::TestContext,
    };

    // Full-workspace CI runs these renderer-owner tests alongside CPU-heavy
    // suites. Keep the guard diagnostic, but allow the same scheduling
    // headroom as the test scheduler's external-input waits.
    const DOM_DEBUGGER_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

    async fn command(ctx: &mut TestContext, message: Value, command_id: u64) -> Value {
        tokio::time::timeout(DOM_DEBUGGER_COMMAND_TIMEOUT, ctx.process_async(message))
            .await
            .unwrap_or_else(|_| {
                panic!("timed out processing DOMDebugger test command {command_id}")
            });
        ctx.take_response_by_id(command_id)
    }

    async fn run_pausing_evaluate_with_observed_resumes(
        ctx: &mut TestContext,
        evaluate: Value,
        resumes: Vec<Value>,
    ) {
        let evaluate_id = evaluate["id"]
            .as_u64()
            .expect("Runtime.evaluate test command must have a numeric id");
        // Keep the pending Runtime.evaluate receiver alive while the test
        // scheduler observes the real renderer publication. Dispatching the
        // interruptible resume before that publication is racy: the interrupt
        // channel can overtake the evaluate request and resume an idle
        // inspector, leaving the later pause without a matching command.
        let evaluate_step = ctx.conn.start_command_dispatch(&evaluate.to_string());
        let CdpCommandTaskStep::Pending(evaluate_pending) = evaluate_step else {
            panic!("a pausing Runtime.evaluate must remain pending until Debugger.resume");
        };
        let mut messages = Vec::new();

        for resume in resumes {
            let resume_id = resume["id"]
                .as_u64()
                .expect("Debugger.resume test command must have a numeric id");
            let resume_session_id = resume.get("sessionId").cloned();
            let paused = ctx
                .wait_for_scheduler_message("pause preceding Debugger.resume", |message| {
                    message["method"] == json!("Debugger.paused")
                        && match &resume_session_id {
                            Some(session_id) => message.get("sessionId") == Some(session_id),
                            None => message.get("sessionId").is_none(),
                        }
                })
                .await;
            messages.push(paused);

            let response_start = ctx.sent.len();
            let resume_step = ctx.conn.start_command_dispatch(&resume.to_string());
            let (mut resume_messages, scheduler_events) = tokio::time::timeout(
                DOM_DEBUGGER_COMMAND_TIMEOUT,
                ctx.complete_command_task_step_for_test(resume_step),
            )
            .await
            .expect("Debugger.resume should complete after its matching pause");
            assert!(scheduler_events.is_empty(), "{scheduler_events:?}");
            if !resume_messages
                .iter()
                .any(|message| message["id"] == json!(resume_id))
            {
                ctx.wait_for_test_command_response(resume_id, response_start)
                    .await;
                resume_messages.push(ctx.take_response_by_id(resume_id));
            }
            messages.append(&mut resume_messages);
        }

        let evaluate_completed =
            tokio::time::timeout(DOM_DEBUGGER_COMMAND_TIMEOUT, evaluate_pending.wait())
                .await
                .expect("Debugger.resume should release the paused Runtime.evaluate");
        let evaluate_step = ctx
            .conn
            .complete_pending_command_dispatch(evaluate_completed)
            .await;
        let (mut evaluate_messages, scheduler_events) = tokio::time::timeout(
            DOM_DEBUGGER_COMMAND_TIMEOUT,
            ctx.complete_command_task_step_for_test(evaluate_step),
        )
        .await
        .expect("resumed Runtime.evaluate should complete");
        assert!(scheduler_events.is_empty(), "{scheduler_events:?}");
        if !evaluate_messages
            .iter()
            .any(|message| message["id"] == json!(evaluate_id))
        {
            if !ctx
                .sent
                .iter()
                .any(|message| message["id"] == json!(evaluate_id))
            {
                let response_start = ctx.sent.len();
                ctx.wait_for_test_command_response(evaluate_id, response_start)
                    .await;
            }
            evaluate_messages.push(ctx.take_response_by_id(evaluate_id));
        }
        messages.append(&mut evaluate_messages);
        ctx.sent.extend(messages);
    }

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut browser_context = BrowserContext::new("BID-1".into());
        browser_context.set_active_target_id("TID-1".to_owned());
        browser_context.set_target_url("data:text/html,dom-debugger-test".to_owned());
        browser_context.attach_active_session("SID-1".to_owned());
        ctx.conn.browser_context = Some(browser_context);
        ctx.install_navigation_fixture_for_session_owner(
            &format!("data:text/html,{html}"),
            Some("SID-1"),
        )
        .await;
    }

    async fn evaluate_object(
        ctx: &mut TestContext,
        id: u64,
        expression: &str,
        object_group: Option<&str>,
    ) -> String {
        let mut params = json!({ "expression": expression });
        if let Some(object_group) = object_group {
            params["objectGroup"] = json!(object_group);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": params
        }))
        .await;
        let response = ctx.take_response_by_id(id);
        response["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_else(|| panic!("Runtime.evaluate should return an object: {response:?}"))
            .to_owned()
    }

    async fn dom_node_id(ctx: &mut TestContext, id: u64, selector: &str) -> u32 {
        let document = command(
            ctx,
            json!({
                "id": id,
                "method": "DOM.getDocument",
                "sessionId": "SID-1",
                "params": { "depth": 1 }
            }),
            id,
        )
        .await;
        let root_node_id = document["result"]["root"]["nodeId"]
            .as_u64()
            .expect("DOM.getDocument root node id") as u32;
        let query_id = id + 1;
        let query = command(
            ctx,
            json!({
                "id": query_id,
                "method": "DOM.querySelector",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "selector": selector }
            }),
            query_id,
        )
        .await;
        query["result"]["nodeId"]
            .as_u64()
            .unwrap_or_else(|| panic!("DOM.querySelector should find {selector}: {query:?}"))
            as u32
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dom_breakpoints_validate_nodes_pause_before_mutations_and_remove_cleanly() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><main id=root><section id=middle><span>old</span></section></main>",
        )
        .await;

        let enable = command(
            &mut ctx,
            json!({
                "id": 80,
                "method": "Debugger.enable",
                "sessionId": "SID-1"
            }),
            80,
        )
        .await;
        assert!(enable.get("error").is_none(), "{enable:?}");
        let root_node_id = dom_node_id(&mut ctx, 81, "#root").await;

        let missing_precedes_type = command(
            &mut ctx,
            json!({
                "id": 83,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": 999999, "type": "bogus" }
            }),
            83,
        )
        .await;
        assert_eq!(
            missing_precedes_type["error"]["message"],
            json!("Could not find node with given id")
        );

        let invalid_type = command(
            &mut ctx,
            json!({
                "id": 84,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "bogus" }
            }),
            84,
        )
        .await;
        assert_eq!(
            invalid_type["error"]["message"],
            json!("Unknown DOM breakpoint type: bogus")
        );

        let set = command(
            &mut ctx,
            json!({
                "id": 85,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "subtree-modified" }
            }),
            85,
        )
        .await;
        assert_eq!(set["result"], json!({}), "{set:?}");

        let output_start = ctx.sent.len();
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 86,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "middle.appendChild(document.createElement('b')); true",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 87,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let mut saw_target_path = false;
        let paused = ctx
            .wait_for_scheduler_message("DOM mutation breakpoint pause", |message| {
                if message["method"] == json!("DOM.setChildNodes")
                    && message["sessionId"] == json!("SID-1")
                {
                    saw_target_path = true;
                    return false;
                }
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("DOM")
            })
            .await;
        assert!(
            saw_target_path,
            "unbound mutation target path must precede Debugger.paused: {:?}",
            &ctx.sent[output_start..]
        );
        assert_eq!(paused["params"]["reason"], json!("DOM"), "{paused:?}");
        assert_eq!(
            paused["params"]["data"]["nodeId"],
            json!(root_node_id),
            "{paused:?}"
        );
        assert_eq!(
            paused["params"]["data"]["type"],
            json!("subtree-modified"),
            "{paused:?}"
        );
        assert_eq!(paused["params"]["data"]["insertion"], json!(true));
        let middle_node_id = paused["params"]["data"]["targetNodeId"]
            .as_u64()
            .filter(|node_id| *node_id > 0)
            .unwrap_or_else(|| panic!("subtree pause should bind the target node: {paused:?}"))
            as u32;

        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 180,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "(() => { const fragment = document.createDocumentFragment(); fragment.appendChild(document.createElement('i')); fragment.appendChild(document.createElement('u')); middle.appendChild(fragment); return true; })()",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 181,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let fragment_pause = ctx
            .wait_for_scheduler_message("DocumentFragment insertion DOM pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("DOM")
            })
            .await;
        assert_eq!(
            fragment_pause["params"]["data"]["insertion"],
            json!(true),
            "DocumentFragment insertion should report one insertion batch pause"
        );
        let fragment_result = ctx.take_response_by_id(180);
        assert_eq!(
            fragment_result["result"]["result"]["value"],
            json!(true),
            "DocumentFragment insertion should complete after one resume: {fragment_result:?}"
        );
        let remove = command(
            &mut ctx,
            json!({
                "id": 88,
                "method": "DOMDebugger.removeDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "subtree-modified" }
            }),
            88,
        )
        .await;
        assert_eq!(remove["result"], json!({}), "{remove:?}");
        let unpaused = command(
            &mut ctx,
            json!({
                "id": 89,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "middle.appendChild(document.createElement('i')); true",
                    "returnByValue": true
                }
            }),
            89,
        )
        .await;
        assert_eq!(unpaused["result"]["result"]["value"], json!(true));

        let initial_attribute = command(
            &mut ctx,
            json!({
                "id": 90,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "root.setAttribute('data-value', 'same'); true",
                    "returnByValue": true
                }
            }),
            90,
        )
        .await;
        assert_eq!(initial_attribute["result"]["result"]["value"], json!(true));
        let set_attribute = command(
            &mut ctx,
            json!({
                "id": 91,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "attribute-modified" }
            }),
            91,
        )
        .await;
        assert_eq!(set_attribute["result"], json!({}), "{set_attribute:?}");
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 92,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "root.setAttribute('data-value', 'same'); true",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 93,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let attribute_pause = ctx
            .wait_for_scheduler_message("same-value attribute DOM breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("DOM")
            })
            .await;
        assert_eq!(
            attribute_pause["params"]["data"],
            json!({
                "nodeId": root_node_id,
                "type": "attribute-modified"
            }),
            "{attribute_pause:?}"
        );
        let remove_attribute = command(
            &mut ctx,
            json!({
                "id": 94,
                "method": "DOMDebugger.removeDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "attribute-modified" }
            }),
            94,
        )
        .await;
        assert_eq!(
            remove_attribute["result"],
            json!({}),
            "{remove_attribute:?}"
        );

        let set_subtree = command(
            &mut ctx,
            json!({
                "id": 95,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "subtree-modified" }
            }),
            95,
        )
        .await;
        assert_eq!(set_subtree["result"], json!({}), "{set_subtree:?}");
        let set_node_removed = command(
            &mut ctx,
            json!({
                "id": 98,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": middle_node_id, "type": "node-removed" }
            }),
            98,
        )
        .await;
        assert_eq!(
            set_node_removed["result"],
            json!({}),
            "{set_node_removed:?}"
        );
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 99,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "globalThis.removedMiddle = middle; middle.remove(); true",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 100,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let removal_pause = ctx
            .wait_for_scheduler_message("direct node-removed DOM breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("DOM")
            })
            .await;
        assert_eq!(
            removal_pause["params"]["data"],
            json!({
                "nodeId": middle_node_id,
                "type": "node-removed"
            }),
            "a direct node-removed breakpoint wins over an ancestor subtree breakpoint: \
             {removal_pause:?}"
        );
        let remove_subtree = command(
            &mut ctx,
            json!({
                "id": 101,
                "method": "DOMDebugger.removeDOMBreakpoint",
                "sessionId": "SID-1",
                "params": { "nodeId": root_node_id, "type": "subtree-modified" }
            }),
            101,
        )
        .await;
        assert_eq!(remove_subtree["result"], json!({}), "{remove_subtree:?}");
        let detached_breakpoint_does_not_return = command(
            &mut ctx,
            json!({
                "id": 102,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "root.appendChild(removedMiddle); removedMiddle.remove(); true",
                    "returnByValue": true
                }
            }),
            102,
        )
        .await;
        assert_eq!(
            detached_breakpoint_does_not_return["result"]["result"]["value"],
            json!(true)
        );

        let rebound_root_node_id = dom_node_id(&mut ctx, 103, "#root").await;
        let set_before_disable = command(
            &mut ctx,
            json!({
                "id": 105,
                "method": "DOMDebugger.setDOMBreakpoint",
                "sessionId": "SID-1",
                "params": {
                    "nodeId": rebound_root_node_id,
                    "type": "attribute-modified"
                }
            }),
            105,
        )
        .await;
        assert_eq!(
            set_before_disable["result"],
            json!({}),
            "{set_before_disable:?}"
        );
        let disable = command(
            &mut ctx,
            json!({
                "id": 106,
                "method": "DOM.disable",
                "sessionId": "SID-1"
            }),
            106,
        )
        .await;
        assert_eq!(disable["result"], json!({}), "{disable:?}");
        let after_disable = command(
            &mut ctx,
            json!({
                "id": 107,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "root.setAttribute('data-disabled', '1'); true",
                    "returnByValue": true
                }
            }),
            107,
        )
        .await;
        assert_eq!(after_disable["result"]["result"]["value"], json!(true));
    }

    async fn get_listeners(
        ctx: &mut TestContext,
        id: u64,
        object_id: &str,
        extra_params: Value,
    ) -> Vec<Value> {
        let mut params = json!({ "objectId": object_id });
        params
            .as_object_mut()
            .expect("params should be an object")
            .extend(
                extra_params
                    .as_object()
                    .expect("extra params should be an object")
                    .clone(),
            );
        ctx.process_async(json!({
            "id": id,
            "method": "DOMDebugger.getEventListeners",
            "sessionId": "SID-1",
            "params": params
        }))
        .await;
        let response = ctx.take_response_by_id(id);
        response["result"]["listeners"]
            .as_array()
            .unwrap_or_else(|| panic!("DOMDebugger should return listeners: {response:?}"))
            .clone()
    }

    fn listeners_by_type(listeners: &[Value]) -> HashMap<&str, &Value> {
        listeners
            .iter()
            .map(|listener| {
                (
                    listener["type"]
                        .as_str()
                        .expect("listener should have a type"),
                    listener,
                )
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_event_listeners_matches_chromium_node_depth_and_listener_shape() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<main id=root><button id=child><span id=grand></span></button></main>",
        )
        .await;
        let root_id = evaluate_object(
            &mut ctx,
            1,
            r#"(() => {
                const root = document.querySelector('#root');
                const child = document.querySelector('#child');
                const grand = document.querySelector('#grand');
                function removed() {}
                function duplicate() {}
                root.addEventListener('removed', removed);
                root.removeEventListener('removed', removed);
                root.addEventListener('duplicate', duplicate);
                root.addEventListener('duplicate', duplicate);
                root.addEventListener('root-bubble', function rootBubble() {});
                root.addEventListener('root-capture', function rootCapture() {}, {
                    capture: true,
                    passive: true,
                    once: true
                });
                root.onclick = function rootProperty() {};
                root.addEventListener('group-a', function groupAFirst() {});
                root.addEventListener('group-b', function groupB() {});
                root.addEventListener('group-a', function groupASecond() {});
                child.addEventListener('child-listener', function childListener() {});
                grand.addEventListener('grand-listener', function grandListener() {});
                const shadowHost = document.createElement('section');
                root.append(shadowHost);
                const shadowChild = shadowHost.attachShadow({mode: 'open'}).appendChild(
                    document.createElement('i')
                );
                shadowChild.addEventListener('shadow-listener', function shadowListener() {});
                return root;
            })()"#,
            None,
        )
        .await;

        ctx.process_async(json!({
            "id": 30,
            "method": "Page.getFrameTree",
            "sessionId": "SID-1"
        }))
        .await;
        let frame_tree = ctx.take_response_by_id(30);
        let frame_id = frame_tree["result"]["frameTree"]["frame"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("Page.getFrameTree should return a frame id: {frame_tree:?}"))
            .to_owned();
        ctx.process_async(json!({
            "id": 31,
            "method": "Page.createIsolatedWorld",
            "sessionId": "SID-1",
            "params": {
                "frameId": frame_id,
                "worldName": "dom-debugger-listener-world"
            }
        }))
        .await;
        let isolated_world = ctx.take_response_by_id(31);
        let isolated_context_id = isolated_world["result"]["executionContextId"]
            .as_i64()
            .unwrap_or_else(|| {
                panic!("Page.createIsolatedWorld should return a context: {isolated_world:?}")
            });
        ctx.process_async(json!({
            "id": 32,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "contextId": isolated_context_id,
                "expression": "document.querySelector('#root').addEventListener('isolated-listener', function isolatedListener() {})"
            }
        }))
        .await;
        assert!(ctx.take_response_by_id(32).get("error").is_none());

        let default_listeners = get_listeners(&mut ctx, 2, &root_id, json!({})).await;
        assert_eq!(
            default_listeners
                .iter()
                .map(|listener| listener["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "root-capture",
                "duplicate",
                "root-bubble",
                "click",
                "group-a",
                "group-a",
                "group-b",
            ],
            "Chromium groups listeners by event-type insertion order after capture partitioning"
        );
        let by_type = listeners_by_type(&default_listeners);
        assert!(by_type.contains_key("duplicate"));
        assert!(by_type.contains_key("root-bubble"));
        assert!(by_type.contains_key("click"));
        assert!(!by_type.contains_key("removed"));
        assert!(!by_type.contains_key("child-listener"));
        assert!(!by_type.contains_key("isolated-listener"));
        assert_eq!(by_type["root-capture"]["useCapture"], json!(true));
        assert_eq!(by_type["root-capture"]["passive"], json!(true));
        assert_eq!(by_type["root-capture"]["once"], json!(true));
        let root_backend_id = default_listeners[0]["backendNodeId"]
            .as_u64()
            .expect("node listener should have a backendNodeId");
        for listener in &default_listeners {
            assert_eq!(listener["backendNodeId"], json!(root_backend_id));
            assert!(listener["scriptId"].is_string());
            assert!(listener["lineNumber"].is_i64());
            assert!(listener["columnNumber"].is_i64());
            assert!(listener.get("handler").is_none());
            assert!(listener.get("originalHandler").is_none());
        }

        let depth_two = get_listeners(&mut ctx, 3, &root_id, json!({ "depth": 2 })).await;
        let by_type = listeners_by_type(&depth_two);
        assert!(by_type.contains_key("child-listener"));
        assert!(!by_type.contains_key("grand-listener"));
        assert_ne!(
            by_type["child-listener"]["backendNodeId"],
            json!(root_backend_id)
        );

        let full_subtree = get_listeners(&mut ctx, 4, &root_id, json!({ "depth": -1 })).await;
        let by_type = listeners_by_type(&full_subtree);
        assert!(by_type.contains_key("grand-listener"));
        assert!(!by_type.contains_key("shadow-listener"));
        assert!(!by_type.contains_key("isolated-listener"));

        let pierced_subtree = get_listeners(
            &mut ctx,
            5,
            &root_id,
            json!({ "depth": -1, "pierce": true }),
        )
        .await;
        let by_type = listeners_by_type(&pierced_subtree);
        assert!(by_type.contains_key("shadow-listener"));
        assert!(by_type.contains_key("isolated-listener"));

        let plain_object_id = evaluate_object(&mut ctx, 6, "({ answer: 42 })", None).await;
        assert!(
            get_listeners(&mut ctx, 7, &plain_object_id, json!({}))
                .await
                .is_empty(),
            "Chromium returns an empty listener array for non-EventTarget objects"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_event_listeners_wraps_handlers_in_the_source_object_group() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><title>object listener</title>").await;
        let target_id = evaluate_object(
            &mut ctx,
            10,
            r#"(() => {
                globalThis.inspectorListenerObject = {
                    handleEvent: function inspectorObjectHandler() {}
                };
                const target = new EventTarget();
                function removedNumericTwo() {}
                target.addEventListener('2', removedNumericTwo);
                target.addEventListener('1', function numericOne() {});
                target.removeEventListener('2', removedNumericTwo);
                target.addEventListener('2', function numericTwo() {});
                target.addEventListener('object-event', inspectorListenerObject);
                return target;
            })()"#,
            Some("listener-group"),
        )
        .await;
        let listeners = get_listeners(&mut ctx, 11, &target_id, json!({})).await;
        assert_eq!(
            listeners
                .iter()
                .map(|listener| listener["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["1", "2", "object-event"],
            "numeric event names and remove/re-add must retain Chromium's event-type insertion order"
        );
        let listener = listeners
            .iter()
            .find(|listener| listener["type"] == json!("object-event"))
            .expect("object listener should be reported");
        assert!(listener.get("backendNodeId").is_none());
        assert_eq!(listener["handler"]["type"], json!("function"));
        assert_eq!(listener["originalHandler"]["type"], json!("object"));
        let handler_id = listener["handler"]["objectId"]
            .as_str()
            .expect("handler should have a RemoteObject id")
            .to_owned();
        let original_handler_id = listener["originalHandler"]["objectId"]
            .as_str()
            .expect("originalHandler should have a RemoteObject id")
            .to_owned();

        for (id, object_id, expression) in [
            (
                12,
                handler_id.as_str(),
                "function() { return this === inspectorListenerObject.handleEvent; }",
            ),
            (
                13,
                original_handler_id.as_str(),
                "function() { return this === inspectorListenerObject; }",
            ),
        ] {
            ctx.process_async(json!({
                "id": id,
                "method": "Runtime.callFunctionOn",
                "sessionId": "SID-1",
                "params": {
                    "objectId": object_id,
                    "functionDeclaration": expression,
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                ctx.take_response_by_id(id)["result"]["result"]["value"],
                json!(true)
            );
        }

        ctx.process_async(json!({
            "id": 14,
            "method": "Runtime.releaseObjectGroup",
            "sessionId": "SID-1",
            "params": { "objectGroup": "listener-group" }
        }))
        .await;
        assert_eq!(ctx.take_response_by_id(14)["result"], json!({}));
        ctx.process_async(json!({
            "id": 15,
            "method": "Runtime.getProperties",
            "sessionId": "SID-1",
            "params": { "objectId": handler_id }
        }))
        .await;
        assert_eq!(ctx.take_response_by_id(15)["error"]["code"], json!(-32000));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_event_listeners_rejects_invalid_params_and_stale_object_ids() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><title>invalid object</title>").await;

        ctx.process_async(json!({
            "id": 20,
            "method": "DOMDebugger.getEventListeners",
            "sessionId": "SID-1",
            "params": {}
        }))
        .await;
        assert_eq!(ctx.take_response_by_id(20)["error"]["code"], json!(-32602));

        let object_id = evaluate_object(
            &mut ctx,
            21,
            "(() => { const target = new EventTarget(); return target; })()",
            None,
        )
        .await;
        // The fixture installs SID-1 directly, so advance the test connection's
        // session-id generator before exercising a second real attachment.
        let _ = ctx.conn.gen_session_id();
        ctx.process_async(json!({
            "id": 22,
            "method": "Runtime.releaseObject",
            "sessionId": "SID-1",
            "params": { "objectId": object_id }
        }))
        .await;
        assert_eq!(ctx.take_response_by_id(22)["result"], json!({}));
        ctx.process_async(json!({
            "id": 23,
            "method": "DOMDebugger.getEventListeners",
            "sessionId": "SID-1",
            "params": { "objectId": object_id }
        }))
        .await;
        let response = ctx.take_response_by_id(23);
        assert_eq!(response["error"]["code"], json!(-32000));
        assert_eq!(
            response["error"]["message"],
            json!("Could not find object with given id")
        );

        let other_session_object_id = evaluate_object(
            &mut ctx,
            24,
            "(() => { const target = new EventTarget(); target.addEventListener('x', () => {}); return target; })()",
            None,
        )
        .await;
        ctx.process_async(json!({
            "id": 25,
            "method": "Target.attachToTarget",
            "params": { "targetId": "TID-1", "flatten": true }
        }))
        .await;
        let attached = ctx.take_response_by_id(25);
        let other_session_id = attached["result"]["sessionId"]
            .as_str()
            .unwrap_or_else(|| {
                panic!("Target.attachToTarget should return a session: {attached:?}")
            })
            .to_owned();
        assert_ne!(other_session_id, "SID-1");
        ctx.process_async(json!({
            "id": 26,
            "method": "DOMDebugger.getEventListeners",
            "sessionId": other_session_id,
            "params": { "objectId": other_session_object_id }
        }))
        .await;
        let response = ctx.take_response_by_id(26);
        assert_eq!(
            response["error"]["code"],
            json!(-32000),
            "another Inspector session must not unwrap the first session's object: {response:?}"
        );
        assert_eq!(
            response["error"]["message"],
            json!("Could not find object with given id")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn event_listener_breakpoint_pauses_matching_callbacks_and_survives_navigation() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><button id=target>run</button>").await;

        let enable = command(
            &mut ctx,
            json!({
                "id": 100,
                "method": "Debugger.enable",
                "sessionId": "SID-1"
            }),
            100,
        )
        .await;
        assert!(enable.get("error").is_none(), "{enable:?}");

        for (id, method) in [
            (101, "DOMDebugger.setEventListenerBreakpoint"),
            (102, "DOMDebugger.removeEventListenerBreakpoint"),
        ] {
            let response = command(
                &mut ctx,
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": "SID-1",
                    "params": { "eventName": "" }
                }),
                id,
            )
            .await;
            assert_eq!(response["error"]["code"], json!(-32000), "{response:?}");
            assert_eq!(
                response["error"]["message"],
                json!("Event name is empty"),
                "{response:?}"
            );
        }

        let set = command(
            &mut ctx,
            json!({
                "id": 103,
                "method": "DOMDebugger.setEventListenerBreakpoint",
                "sessionId": "SID-1",
                "params": { "eventName": "click", "targetName": "button" }
            }),
            103,
        )
        .await;
        assert_eq!(set["result"], json!({}), "{set:?}");

        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 104,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": r#"
                        target.addEventListener('click', function eventBreakpointListener() {
                            globalThis.__eventBreakpointHits =
                                (globalThis.__eventBreakpointHits || 0) + 1;
                        });
                        target.click();
                        true
                    "#,
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 105,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;

        let paused = ctx
            .wait_for_scheduler_message("event listener breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("EventListener")
            })
            .await;
        assert_eq!(
            paused["params"]["data"],
            json!({
                "eventName": "listener:click",
                "targetName": "BUTTON"
            }),
            "{paused:?}"
        );
        let evaluate = ctx.take_response_by_id(104);
        assert_eq!(evaluate["result"]["result"]["value"], json!(true));
        let resume = ctx.take_response_by_id(105);
        assert_eq!(resume["result"], json!({}), "{resume:?}");

        let remove = command(
            &mut ctx,
            json!({
                "id": 106,
                "method": "DOMDebugger.removeEventListenerBreakpoint",
                "sessionId": "SID-1",
                "params": { "eventName": "click", "targetName": "BUTTON" }
            }),
            106,
        )
        .await;
        assert_eq!(remove["result"], json!({}), "{remove:?}");
        let unpaused_click = command(
            &mut ctx,
            json!({
                "id": 107,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "target.click(); globalThis.__eventBreakpointHits",
                    "returnByValue": true
                }
            }),
            107,
        )
        .await;
        assert_eq!(
            unpaused_click["result"]["result"]["value"],
            json!(2),
            "remove must cancel the exact canonical targetName breakpoint: {unpaused_click:?}"
        );

        let restore_set = command(
            &mut ctx,
            json!({
                "id": 108,
                "method": "DOMDebugger.setEventListenerBreakpoint",
                "sessionId": "SID-1",
                "params": { "eventName": "click" }
            }),
            108,
        )
        .await;
        assert_eq!(restore_set["result"], json!({}), "{restore_set:?}");
        let navigate = command(
            &mut ctx,
            json!({
                "id": 109,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": {
                    "url": "data:text/html,<button id=after>after</button>"
                }
            }),
            109,
        )
        .await;
        assert!(navigate["result"]["frameId"].is_string(), "{navigate:?}");
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 110,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "after.addEventListener('click', () => 42); after.click(); true",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 111,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let restored_pause = ctx
            .wait_for_scheduler_message("restored event listener breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("EventListener")
                    && message["params"]["data"]["eventName"] == json!("listener:click")
                    && message["params"]["data"]["targetName"] == json!("BUTTON")
            })
            .await;
        assert_eq!(restored_pause["params"]["reason"], json!("EventListener"));
        let evaluate = ctx.take_response_by_id(110);
        assert_eq!(evaluate["result"]["result"]["value"], json!(true));
        let final_resume = ctx.take_response_by_id(111);
        assert_eq!(final_resume["result"], json!({}), "{final_resume:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn event_listener_breakpoint_is_session_owned_and_covers_simple_event_targets() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><title>simple target breakpoint</title>",
        )
        .await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            assert!(browser_context.assign_auxiliary_session_to_target(
                "TID-1",
                "SID-event-breakpoint-owner".to_owned(),
            ));
        }

        let primary_enable = command(
            &mut ctx,
            json!({
                "id": 120,
                "method": "Debugger.enable",
                "sessionId": "SID-1"
            }),
            120,
        )
        .await;
        assert!(primary_enable.get("error").is_none(), "{primary_enable:?}");
        let setup = command(
            &mut ctx,
            json!({
                "id": 121,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": r#"
                        globalThis.__breakpointTarget = new EventTarget();
                        globalThis.__breakpointCount = 0;
                        __breakpointTarget.addEventListener('custom', () => ++__breakpointCount);
                        true
                    "#
                }
            }),
            121,
        )
        .await;
        assert_eq!(setup["result"]["result"]["value"], json!(true));
        let set = command(
            &mut ctx,
            json!({
                "id": 122,
                "method": "DOMDebugger.setEventListenerBreakpoint",
                "sessionId": "SID-event-breakpoint-owner",
                "params": { "eventName": "custom", "targetName": "EventTargetImpl" }
            }),
            122,
        )
        .await;
        assert_eq!(set["result"], json!({}), "{set:?}");

        let owner_disabled_dispatch = command(
            &mut ctx,
            json!({
                "id": 123,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "__breakpointTarget.dispatchEvent(new Event('custom')); __breakpointCount",
                    "returnByValue": true
                }
            }),
            123,
        )
        .await;
        assert_eq!(
            owner_disabled_dispatch["result"]["result"]["value"],
            json!(1),
            "a peer Debugger agent must not activate a disabled owner's breakpoint"
        );

        let owner_enable = command(
            &mut ctx,
            json!({
                "id": 124,
                "method": "Debugger.enable",
                "sessionId": "SID-event-breakpoint-owner"
            }),
            124,
        )
        .await;
        assert!(owner_enable.get("error").is_none(), "{owner_enable:?}");
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 125,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "__breakpointTarget.dispatchEvent(new Event('custom')); __breakpointCount",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 126,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;

        for session_id in ["SID-1", "SID-event-breakpoint-owner"] {
            let paused = ctx
                .wait_for_scheduler_message("multi-session event listener pause", |message| {
                    message["method"] == json!("Debugger.paused")
                        && message["sessionId"] == json!(session_id)
                })
                .await;
            if session_id == "SID-event-breakpoint-owner" {
                assert_eq!(paused["params"]["reason"], json!("EventListener"));
                assert_eq!(
                    paused["params"]["data"],
                    json!({
                        "eventName": "listener:custom",
                        "targetName": "EventTargetImpl"
                    }),
                    "{paused:?}"
                );
            } else {
                assert_eq!(paused["params"]["reason"], json!("other"), "{paused:?}");
                assert!(paused["params"].get("data").is_none(), "{paused:?}");
            }
        }
        let dispatched = ctx.take_response_by_id(125);
        assert_eq!(dispatched["result"]["result"]["value"], json!(2));
        let resume = ctx.take_response_by_id(126);
        assert_eq!(resume["result"], json!({}), "{resume:?}");

        let remove = command(
            &mut ctx,
            json!({
                "id": 128,
                "method": "DOMDebugger.removeEventListenerBreakpoint",
                "sessionId": "SID-event-breakpoint-owner",
                "params": { "eventName": "custom", "targetName": "eventtargetimpl" }
            }),
            128,
        )
        .await;
        assert_eq!(remove["result"], json!({}), "{remove:?}");
        let unpaused = command(
            &mut ctx,
            json!({
                "id": 129,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "__breakpointTarget.dispatchEvent(new Event('custom')); __breakpointCount",
                    "returnByValue": true
                }
            }),
            129,
        )
        .await;
        assert_eq!(unpaused["result"]["result"]["value"], json!(3));

        let reset = command(
            &mut ctx,
            json!({
                "id": 130,
                "method": "DOMDebugger.setEventListenerBreakpoint",
                "sessionId": "SID-event-breakpoint-owner",
                "params": { "eventName": "custom", "targetName": "EventTargetImpl" }
            }),
            130,
        )
        .await;
        assert_eq!(reset["result"], json!({}), "{reset:?}");
        let detach = command(
            &mut ctx,
            json!({
                "id": 131,
                "method": "Target.detachFromTarget",
                "sessionId": "SID-1",
                "params": {
                    "targetId": "TID-1",
                    "sessionId": "SID-event-breakpoint-owner"
                }
            }),
            131,
        )
        .await;
        assert_eq!(detach["result"], json!({}), "{detach:?}");
        let after_detach = command(
            &mut ctx,
            json!({
                "id": 132,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "__breakpointTarget.dispatchEvent(new Event('custom')); __breakpointCount",
                    "returnByValue": true
                }
            }),
            132,
        )
        .await;
        assert_eq!(
            after_detach["result"]["result"]["value"],
            json!(4),
            "detaching the owner session must remove its renderer breakpoint state"
        );

        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            assert!(browser_context.assign_auxiliary_session_to_target(
                "TID-1",
                "SID-event-breakpoint-owner".to_owned(),
            ));
        }
        let reattached_enable = command(
            &mut ctx,
            json!({
                "id": 133,
                "method": "Debugger.enable",
                "sessionId": "SID-event-breakpoint-owner"
            }),
            133,
        )
        .await;
        assert!(
            reattached_enable.get("error").is_none(),
            "{reattached_enable:?}"
        );
        let after_reattach = command(
            &mut ctx,
            json!({
                "id": 134,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "__breakpointTarget.dispatchEvent(new Event('custom')); __breakpointCount",
                    "returnByValue": true
                }
            }),
            134,
        )
        .await;
        assert_eq!(
            after_reattach["result"]["result"]["value"],
            json!(5),
            "reattaching the same wire session id must not revive detached breakpoints"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn xhr_breakpoint_pauses_fetch_and_xhr_and_survives_navigation() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><title>XHR breakpoint</title>").await;

        let enable = command(
            &mut ctx,
            json!({
                "id": 200,
                "method": "Debugger.enable",
                "sessionId": "SID-1"
            }),
            200,
        )
        .await;
        assert!(enable.get("error").is_none(), "{enable:?}");

        let invalid = command(
            &mut ctx,
            json!({
                "id": 201,
                "method": "DOMDebugger.setXHRBreakpoint",
                "sessionId": "SID-1",
                "params": {}
            }),
            201,
        )
        .await;
        assert_eq!(invalid["error"]["code"], json!(-32602), "{invalid:?}");

        for (id, url) in [(202, "needle-specific"), (203, "needle")] {
            let set = command(
                &mut ctx,
                json!({
                    "id": id,
                    "method": "DOMDebugger.setXHRBreakpoint",
                    "sessionId": "SID-1",
                    "params": { "url": url }
                }),
                id,
            )
            .await;
            assert_eq!(set["result"], json!({}), "{set:?}");
        }

        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 204,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,needle-specific').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 205,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let fetch_pause = ctx
            .wait_for_scheduler_message("fetch XHR breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("XHR")
            })
            .await;
        assert_eq!(
            fetch_pause["params"]["data"],
            json!({
                "breakpointURL": "needle",
                "url": "data:text/plain,needle-specific"
            }),
            "Chromium chooses the lexicographically first matching URL key: {fetch_pause:?}"
        );
        assert_eq!(
            ctx.take_response_by_id(204)["result"]["result"]["value"],
            json!("needle-specific")
        );
        assert_eq!(ctx.take_response_by_id(205)["result"], json!({}));

        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 206,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": r#"
                        new Promise((resolve, reject) => {
                            const xhr = new XMLHttpRequest();
                            xhr.onload = () => resolve(xhr.responseText);
                            xhr.onerror = () => reject(new Error('XHR failed'));
                            xhr.open('GET', 'data:text/plain,xhr-needle');
                            xhr.send();
                        })
                    "#,
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 207,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let xhr_pause = ctx
            .wait_for_scheduler_message("XMLHttpRequest XHR breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("XHR")
            })
            .await;
        assert_eq!(
            xhr_pause["params"]["data"],
            json!({
                "breakpointURL": "needle",
                "url": "data:text/plain,xhr-needle"
            }),
            "{xhr_pause:?}"
        );
        assert_eq!(
            ctx.take_response_by_id(206)["result"]["result"]["value"],
            json!("xhr-needle")
        );
        assert_eq!(ctx.take_response_by_id(207)["result"], json!({}));

        let set_all = command(
            &mut ctx,
            json!({
                "id": 208,
                "method": "DOMDebugger.setXHRBreakpoint",
                "sessionId": "SID-1",
                "params": { "url": "" }
            }),
            208,
        )
        .await;
        assert_eq!(set_all["result"], json!({}), "{set_all:?}");
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 209,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "(() => { const xhr = new XMLHttpRequest(); try { xhr.send(); } catch (error) { return error.name; } })()",
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 210,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let invalid_state_pause = ctx
            .wait_for_scheduler_message("match-all invalid-state XHR pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("XHR")
            })
            .await;
        assert_eq!(
            invalid_state_pause["params"]["data"],
            json!({ "breakpointURL": "", "url": "" }),
            "Chromium pauses before XMLHttpRequest.send validates OPENED state"
        );
        assert_eq!(
            ctx.take_response_by_id(209)["result"]["result"]["value"],
            json!("InvalidStateError")
        );
        assert_eq!(ctx.take_response_by_id(210)["result"], json!({}));

        let navigate = command(
            &mut ctx,
            json!({
                "id": 211,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": "data:text/html,<title>after navigation</title>" }
            }),
            211,
        )
        .await;
        assert!(navigate["result"]["frameId"].is_string(), "{navigate:?}");
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 212,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,after-navigation').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 213,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        let restored_pause = ctx
            .wait_for_scheduler_message("restored match-all XHR breakpoint pause", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["reason"] == json!("XHR")
            })
            .await;
        assert_eq!(
            restored_pause["params"]["data"],
            json!({
                "breakpointURL": "",
                "url": "data:text/plain,after-navigation"
            }),
            "{restored_pause:?}"
        );
        assert_eq!(
            ctx.take_response_by_id(212)["result"]["result"]["value"],
            json!("after-navigation")
        );
        assert_eq!(ctx.take_response_by_id(213)["result"], json!({}));

        for (id, url) in [(214, ""), (215, "needle"), (216, "needle-specific")] {
            let remove = command(
                &mut ctx,
                json!({
                    "id": id,
                    "method": "DOMDebugger.removeXHRBreakpoint",
                    "sessionId": "SID-1",
                    "params": { "url": url }
                }),
                id,
            )
            .await;
            assert_eq!(remove["result"], json!({}), "{remove:?}");
        }
        let unpaused = command(
            &mut ctx,
            json!({
                "id": 217,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,needle').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            217,
        )
        .await;
        assert_eq!(unpaused["result"]["result"]["value"], json!("needle"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn xhr_breakpoint_is_session_owned_and_cleared_on_detach() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><title>XHR owner</title>").await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            assert!(browser_context.assign_auxiliary_session_to_target(
                "TID-1",
                "SID-xhr-breakpoint-owner".to_owned(),
            ));
        }

        let primary_enable = command(
            &mut ctx,
            json!({
                "id": 300,
                "method": "Debugger.enable",
                "sessionId": "SID-1"
            }),
            300,
        )
        .await;
        assert!(primary_enable.get("error").is_none(), "{primary_enable:?}");
        let set = command(
            &mut ctx,
            json!({
                "id": 301,
                "method": "DOMDebugger.setXHRBreakpoint",
                "sessionId": "SID-xhr-breakpoint-owner",
                "params": { "url": "session-owner" }
            }),
            301,
        )
        .await;
        assert_eq!(set["result"], json!({}), "{set:?}");

        let disabled_owner = command(
            &mut ctx,
            json!({
                "id": 302,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,session-owner-disabled').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            302,
        )
        .await;
        assert_eq!(
            disabled_owner["result"]["result"]["value"],
            json!("session-owner-disabled"),
            "a peer Debugger must not activate a disabled owner's XHR breakpoint"
        );

        let owner_enable = command(
            &mut ctx,
            json!({
                "id": 303,
                "method": "Debugger.enable",
                "sessionId": "SID-xhr-breakpoint-owner"
            }),
            303,
        )
        .await;
        assert!(owner_enable.get("error").is_none(), "{owner_enable:?}");
        run_pausing_evaluate_with_observed_resumes(
            &mut ctx,
            json!({
                "id": 304,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,session-owner-enabled').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            vec![json!({
                "id": 305,
                "method": "Debugger.resume",
                "sessionId": "SID-1"
            })],
        )
        .await;
        for session_id in ["SID-1", "SID-xhr-breakpoint-owner"] {
            let paused = ctx
                .wait_for_scheduler_message("session-owned XHR breakpoint pause", |message| {
                    message["method"] == json!("Debugger.paused")
                        && message["sessionId"] == json!(session_id)
                })
                .await;
            if session_id == "SID-xhr-breakpoint-owner" {
                assert_eq!(paused["params"]["reason"], json!("XHR"), "{paused:?}");
                assert_eq!(
                    paused["params"]["data"],
                    json!({
                        "breakpointURL": "session-owner",
                        "url": "data:text/plain,session-owner-enabled"
                    }),
                    "{paused:?}"
                );
            } else {
                assert_eq!(paused["params"]["reason"], json!("other"), "{paused:?}");
                assert!(paused["params"].get("data").is_none(), "{paused:?}");
            }
        }
        assert_eq!(
            ctx.take_response_by_id(304)["result"]["result"]["value"],
            json!("session-owner-enabled")
        );
        assert_eq!(ctx.take_response_by_id(305)["result"], json!({}));

        let detach = command(
            &mut ctx,
            json!({
                "id": 306,
                "method": "Target.detachFromTarget",
                "sessionId": "SID-1",
                "params": {
                    "targetId": "TID-1",
                    "sessionId": "SID-xhr-breakpoint-owner"
                }
            }),
            306,
        )
        .await;
        assert_eq!(detach["result"], json!({}), "{detach:?}");
        let after_detach = command(
            &mut ctx,
            json!({
                "id": 307,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,session-owner-detached').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            307,
        )
        .await;
        assert_eq!(
            after_detach["result"]["result"]["value"],
            json!("session-owner-detached")
        );

        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            assert!(browser_context.assign_auxiliary_session_to_target(
                "TID-1",
                "SID-xhr-breakpoint-owner".to_owned(),
            ));
        }
        let reattached_enable = command(
            &mut ctx,
            json!({
                "id": 308,
                "method": "Debugger.enable",
                "sessionId": "SID-xhr-breakpoint-owner"
            }),
            308,
        )
        .await;
        assert!(
            reattached_enable.get("error").is_none(),
            "{reattached_enable:?}"
        );
        let after_reattach = command(
            &mut ctx,
            json!({
                "id": 309,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "expression": "fetch('data:text/plain,session-owner-reattached').then(response => response.text())",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }),
            309,
        )
        .await;
        assert_eq!(
            after_reattach["result"]["result"]["value"],
            json!("session-owner-reattached"),
            "reattaching the same wire session id must not revive XHR breakpoints"
        );
    }
}

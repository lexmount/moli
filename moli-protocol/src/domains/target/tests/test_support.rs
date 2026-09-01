use super::*;
use moli_core::page::RendererServiceWorkerVersionStatus;
use moli_shared_worker::SharedWorkerInstanceId;

// Helper: quickly set up a browser context with a given id, optional
// target_id, optional session_id, and optional url.
pub(super) fn load_bc(ctx: &mut TestContext, bc_id: &str) {
    ctx.conn.browser_context = Some(BrowserContext::new(bc_id.into()));
}

pub(super) fn load_bc_with_target(ctx: &mut TestContext, bc_id: &str, target_id: &str) {
    let mut bc = BrowserContext::new(bc_id.into());
    bc.set_active_target_id(target_id);
    ctx.conn.browser_context = Some(bc);
}

pub(super) fn tab_id_for_page(ctx: &TestContext, page_target_id: &str) -> String {
    ctx.conn
        .tab_target_id_for_page_target_id(page_target_id)
        .unwrap_or_else(|| panic!("page target {page_target_id} has no tab target"))
        .to_owned()
}

pub(super) fn push_background_target(
    ctx: &mut TestContext,
    target_id: &str,
    url: &str,
    session_id: Option<&str>,
) {
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context must exist before adding background target");
    bc.insert_page_target_host(crate::conn::PageTargetHost::new(
        target_id.to_owned(),
        session_id.map(str::to_owned),
        crate::conn::TargetIdentityState::new(
            url.to_owned(),
            crate::conn::URL_BASE.into(),
            "Secure".into(),
        ),
        crate::conn::TargetPageSlot::empty_for_test_fixture(),
    ));
}

pub(super) fn loaded_page_for_target<'a>(
    browser_context: &'a BrowserContext,
    target_id: &str,
) -> Option<&'a moli_core::page::Page> {
    if browser_context.is_active_target(target_id) {
        browser_context.loaded_page()
    } else {
        browser_context
            .background_target(target_id)
            .and_then(crate::conn::PageTargetHost::loaded_page)
    }
}

pub(super) fn push_shared_worker_target(
    ctx: &mut TestContext,
    renderer_instance_id: SharedWorkerInstanceId,
    target_id: &str,
    url: &str,
    name: &str,
    session_id: Option<&str>,
) {
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context must exist before adding shared worker target");
    let mut target = crate::conn::SharedWorkerTargetState::new(
        moli_core::RendererOwnerLocalHostId::new_for_testing(1),
        renderer_instance_id,
        target_id.to_owned(),
        None,
        url.to_owned(),
        name.to_owned(),
    );
    if let Some(session_id) = session_id {
        target.attach_session(session_id.to_owned());
    }
    bc.insert_shared_worker_target(target);
}

pub(super) fn push_service_worker_target(
    ctx: &mut TestContext,
    renderer_version_id: u64,
    target_id: &str,
    script_url: &str,
    scope_url: &str,
    session_id: Option<&str>,
) {
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context must exist before adding service worker target");
    let target = crate::conn::ServiceWorkerTargetState::new(
        1,
        renderer_version_id,
        target_id.to_owned(),
        script_url.to_owned(),
        scope_url.to_owned(),
        RendererServiceWorkerVersionStatus::Activated,
        None,
    );
    bc.insert_service_worker_target(target);
    if let Some(session_id) = session_id {
        assert!(bc.assign_session_to_service_worker_target(target_id, session_id.to_owned()));
    }
}

pub(super) fn push_dedicated_worker_target(
    ctx: &mut TestContext,
    renderer_instance_id: u64,
    target_id: &str,
    owner_page: crate::conn::TargetPageResidenceIdentity,
) {
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context must exist before adding dedicated worker target");
    bc.insert_dedicated_worker_target(crate::conn::DedicatedWorkerTargetState::new(
        owner_page,
        moli_core::RendererOwnerLocalHostId::new_for_testing(1),
        renderer_instance_id,
        target_id.to_owned(),
        String::new(),
        Vec::new(),
    ));
}

pub(super) async fn load_bc_with_titled_page_async(
    ctx: &mut TestContext,
    bc_id: &str,
    target_id: &str,
    html: &str,
) {
    // Most tests that use this helper assert Target-domain discovery output
    // after manually seeding a loaded target. Real CDP clients only receive
    // Target.targetCreated after Target.setDiscoverTargets(true), so model that
    // client setup explicitly in the harness instead of relying on createTarget
    // to emit discovery events unconditionally.
    enable_root_target_discovery_for_test(ctx);
    let mut bc = BrowserContext::new(bc_id.into());
    bc.set_active_target_id(target_id);
    ctx.conn.insert_browser_context(bc);
    let page = ctx
        .conn
        .load_page_via_runtime_async(&format!("data:text/html,{html}"))
        .await
        .expect("page should load");
    let renderer_page = crate::conn::RendererPageResidenceIdentity::from_page(&page);
    let url = page.final_url().as_str().to_owned();
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("loaded Target fixture must retain its BrowserContext owner");
    bc.set_target_url(url);
    let _ = bc
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));
    // Even this lightweight Target-domain fixture owns a real renderer Page
    // with a concrete output stream. Bind that stream before any later test
    // turn consumes its queued `Opened`/publication records; resolving the
    // owner from the then-current Page would be wrong after navigation has
    // replaced this initial fixture.
    let page_owner = ctx
        .conn
        .target_page_residence_identity_for_session(None)
        .expect("loaded Target fixture must have an exact Page owner");
    ctx.conn
        .bind_renderer_page_output_owner(renderer_page, page_owner);
}

pub(super) fn enable_root_target_discovery_for_test(ctx: &mut TestContext) {
    ctx.conn.set_root_target_discovery_enabled(true);
}

/// Consume one `Target.createTarget` result and its matching discovery event
/// without imposing an ordering between the response and event queues.
///
/// CDP identifies these outputs by command id and target id. Tests that take
/// the queue head accidentally turn unrelated, already-produced protocol
/// output into an ordering requirement.
pub(super) fn take_created_target_id(ctx: &mut TestContext, command_id: u64) -> String {
    let response = take_response_by_id(ctx, command_id);
    let target_id = response["result"]["targetId"]
        .as_str()
        .expect("Target.createTarget response should contain a target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
            }
        })),
    );
    target_id
}

pub(super) fn consume_main_document_navigation_start(ctx: &mut TestContext) {
    let started_navigating = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameStartedNavigating"))
        .unwrap_or_else(|| panic!("missing Page.frameStartedNavigating: {:?}", ctx.sent));
    let started_loading = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameStartedLoading"))
        .unwrap_or_else(|| panic!("missing Page.frameStartedLoading: {:?}", ctx.sent));
    assert!(
        started_navigating < started_loading,
        "Page.frameStartedNavigating must precede Page.frameStartedLoading: {:?}",
        ctx.sent
    );
    ctx.sent.remove(started_loading);
    ctx.sent.remove(started_navigating);
}

pub(super) fn take_main_document_request_pause(ctx: &mut TestContext) -> Value {
    consume_main_document_navigation_start(ctx);
    let pause_position = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Fetch.requestPaused"))
        .unwrap_or_else(|| panic!("missing main-document Fetch.requestPaused: {:?}", ctx.sent));
    let pause = ctx.sent.remove(pause_position);
    ctx.sent.drain(..pause_position);
    pause
}

pub(super) fn take_response_by_id(ctx: &mut TestContext, id: u64) -> Value {
    let position = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing response for id {id}: {:?}", ctx.sent));
    ctx.sent.remove(position)
}

pub(super) fn expect_inspector_detached(ctx: &mut TestContext, session_id: &str) {
    let event = ctx.take_first_matching("Inspector.detached", |message| {
        message["method"] == json!("Inspector.detached")
            && message["sessionId"] == json!(session_id)
    });
    assert_eq!(event["params"]["reason"], "Render process gone.");
}

pub(super) struct AttachedPageSession {
    pub(super) browser_context_id: String,
    pub(super) target_id: String,
    pub(super) session_id: String,
}

pub(super) fn patchright_page_binding_wrapper_source(
    binding_name: &str,
    deliver_name: &str,
    take_handle_name: Option<&str>,
    needs_handle: bool,
) -> String {
    r#"
        (() => {
            function addPageBinding(bindingName, needsHandle, utilityScriptSerializersFactory) {
                const { serializeAsCallArgument } = utilityScriptSerializersFactory;
                const binding = globalThis[bindingName];
                if (!binding || binding.toString().startsWith("(...args) => {"))
                    return;
                globalThis[bindingName] = (...args) => {
                    const me = globalThis[bindingName];
                    if (needsHandle && args.slice(1).some(arg => arg !== undefined))
                        throw new Error(`exposeBindingHandle supports a single argument, ${args.length} received`);
                    let callbacks = me.callbacks;
                    if (!callbacks) {
                        callbacks = new Map();
                        me.callbacks = callbacks;
                    }
                    const seq = (me.lastSeq || 0) + 1;
                    me.lastSeq = seq;
                    const promise = new Promise((resolve, reject) => callbacks.set(seq, { resolve, reject }));
                    let payload;
                    if (needsHandle) {
                        let handles = me.handles;
                        if (!handles) {
                            handles = new Map();
                            me.handles = handles;
                        }
                        handles.set(seq, args[0]);
                        payload = { name: bindingName, seq };
                    } else {
                        const serializedArgs = [];
                        for (let i = 0; i < args.length; i++) {
                            serializedArgs[i] = serializeAsCallArgument(args[i], v => {
                                return { fallThrough: v };
                            });
                        }
                        payload = { name: bindingName, seq, serializedArgs };
                    }
                    binding(JSON.stringify(payload));
                    return promise;
                };
            }
            function takeBindingHandle(arg) {
                const handles = globalThis[arg.name].handles;
                const handle = handles.get(arg.seq);
                handles.delete(arg.seq);
                return handle;
            }
            function deliverBindingResult(arg) {
                const callbacks = globalThis[arg.name].callbacks;
                if ('error' in arg)
                    callbacks.get(arg.seq).reject(arg.error);
                else
                    callbacks.get(arg.seq).resolve(arg.result);
                callbacks.delete(arg.seq);
            }
            const utilityScriptSerializersFactory = () => ({
                serializeAsCallArgument(value, fallback) {
                    const serialized = fallback(value);
                    if (serialized && typeof serialized === 'object' && 'fallThrough' in serialized)
                        return serialized.fallThrough;
                    return serialized;
                }
            });
            addPageBinding('__BINDING_NAME__', __NEEDS_HANDLE__, utilityScriptSerializersFactory());
            globalThis.__DELIVER_NAME__ = deliverBindingResult;
            __TAKE_HANDLE_ASSIGNMENT__
            return typeof globalThis['__BINDING_NAME__'];
        })()
    "#
    .replace("__BINDING_NAME__", binding_name)
    .replace("__DELIVER_NAME__", deliver_name)
    .replace("__NEEDS_HANDLE__", if needs_handle { "true" } else { "false" })
    .replace(
        "__TAKE_HANDLE_ASSIGNMENT__",
        &take_handle_name
            .map(|name| format!("globalThis.{name} = takeBindingHandle;"))
            .unwrap_or_default(),
    )
}

pub(super) async fn install_patchright_crpage_binding_in_existing_worlds_async(
    ctx: &mut TestContext,
    session_id: &str,
    utility_context_id: i64,
    add_main_binding_id: u64,
    add_utility_binding_id: u64,
    install_main_wrapper_id: u64,
    install_utility_wrapper_id: u64,
    binding_name: &str,
    wrapper_source: &str,
) {
    ctx.process_async(json!({
        "id": add_main_binding_id,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": {
            "name": binding_name
        }
    }))
    .await;
    let add_main_binding = take_response_by_id(ctx, add_main_binding_id);
    assert_eq!(add_main_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": add_utility_binding_id,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": {
            "name": binding_name,
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_utility_binding = take_response_by_id(ctx, add_utility_binding_id);
    assert_eq!(add_utility_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": install_main_wrapper_id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    let install_main_wrapper = take_response_by_id(ctx, install_main_wrapper_id);
    assert_eq!(
        install_main_wrapper["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
        "id": install_utility_wrapper_id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    let install_utility_wrapper = take_response_by_id(ctx, install_utility_wrapper_id);
    assert_eq!(
        install_utility_wrapper["result"]["result"]["value"],
        json!("function")
    );
}

pub(super) async fn attach_page_session_in_existing_context_async(
    ctx: &mut TestContext,
    browser_context_id: &str,
    create_target_id: u64,
    attach_id: u64,
    network_enable_id: u64,
    runtime_enable_id: u64,
) -> AttachedPageSession {
    // This helper consumes Target.targetCreated below, so it must represent a
    // client with target discovery enabled.
    enable_root_target_discovery_for_test(ctx);
    ctx.process_async(json!({
        "id": create_target_id,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let create_target_response = take_response_by_id(ctx, create_target_id);
    let target_id = create_target_response["result"]["targetId"]
        .as_str()
        .expect("target id should exist")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );
    assert_eq!(
        create_target_response["result"],
        json!({ "targetId": target_id })
    );

    ctx.process_async(json!({
        "id": attach_id,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    let attach_response = take_response_by_id(ctx, attach_id);
    let session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("session id should exist")
        .to_owned();
    assert_eq!(
        attach_response["result"],
        json!({ "sessionId": session_id })
    );
    ctx.expect_event("Target.attachedToTarget", None);

    let activate_id = attach_id
        .checked_add(9_000_000_000)
        .expect("test activation id should not overflow");
    ctx.process_async(json!({
        "id": activate_id,
        "method": "Target.activateTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    ctx.expect_result(activate_id, json!({}), None);

    ctx.process_async(json!({
        "id": network_enable_id,
        "method": "Network.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(network_enable_id, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": runtime_enable_id,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(runtime_enable_id, json!({}), Some(&session_id));

    AttachedPageSession {
        browser_context_id: browser_context_id.to_owned(),
        target_id,
        session_id,
    }
}

pub(super) async fn attach_page_session_without_runtime_enable_in_existing_context_async(
    ctx: &mut TestContext,
    browser_context_id: &str,
    create_target_id: u64,
    attach_id: u64,
) -> AttachedPageSession {
    // This helper consumes Target.targetCreated below, so it must represent a
    // client with target discovery enabled.
    enable_root_target_discovery_for_test(ctx);
    ctx.process_async(json!({
        "id": create_target_id,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let create_target_response = take_response_by_id(ctx, create_target_id);
    let target_id = create_target_response["result"]["targetId"]
        .as_str()
        .expect("target id should exist")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );
    assert_eq!(
        create_target_response["result"],
        json!({ "targetId": target_id })
    );

    ctx.process_async(json!({
        "id": attach_id,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    let session_id = take_response_by_id(ctx, attach_id)["result"]["sessionId"]
        .as_str()
        .expect("session id should exist")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    AttachedPageSession {
        browser_context_id: browser_context_id.to_owned(),
        target_id,
        session_id,
    }
}

pub(super) async fn create_attached_page_session_async(
    ctx: &mut TestContext,
    create_browser_context_id: u64,
    create_target_id: u64,
    attach_id: u64,
    network_enable_id: u64,
    runtime_enable_id: u64,
) -> AttachedPageSession {
    ctx.process_async(json!({
        "id": create_browser_context_id,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id =
        take_response_by_id(ctx, create_browser_context_id)["result"]["browserContextId"]
            .as_str()
            .expect("browser context id should exist")
            .to_owned();

    attach_page_session_in_existing_context_async(
        ctx,
        &browser_context_id,
        create_target_id,
        attach_id,
        network_enable_id,
        runtime_enable_id,
    )
    .await
}

pub(super) async fn create_attached_page_session_without_runtime_enable_async(
    ctx: &mut TestContext,
    create_browser_context_id: u64,
    create_target_id: u64,
    attach_id: u64,
) -> AttachedPageSession {
    ctx.process_async(json!({
        "id": create_browser_context_id,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id =
        take_response_by_id(ctx, create_browser_context_id)["result"]["browserContextId"]
            .as_str()
            .expect("browser context id should exist")
            .to_owned();

    attach_page_session_without_runtime_enable_in_existing_context_async(
        ctx,
        &browser_context_id,
        create_target_id,
        attach_id,
    )
    .await
}

pub(super) async fn current_permission_state_async(
    ctx: &mut TestContext,
    session_id: &str,
    permission_name: &str,
) -> String {
    ctx.process_async(json!({
        "id": 9_100,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": format!(
                "(() => {{ globalThis.__permissionState = 'pending'; navigator.permissions.query({{ name: '{}' }}).then(status => {{ globalThis.__permissionState = status.state; }}); return 'scheduled'; }})()",
                permission_name
            )
        }
    }))
    .await;
    let response = take_response_by_id(ctx, 9_100);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    ctx.process_async(json!({
        "id": 9_101,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "globalThis.__permissionState" }
    }))
    .await;
    take_response_by_id(ctx, 9_101)["result"]["result"]["value"]
        .as_str()
        .expect("permission state should be a string")
        .to_owned()
}

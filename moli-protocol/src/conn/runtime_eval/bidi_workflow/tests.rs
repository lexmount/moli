use super::*;
use crate::automation::{DevToolsBidiChannelProperties, DevToolsResultOwnership};
use crate::conn::{BrowserContext, ProtocolStateOwner};
use crate::domains::activity::ProtocolSchedulerWork;
use crate::testing::TestContext;

fn inspect<T: 'static>(
    access: &ProtocolStateHandle,
    operation: impl FnOnce(&mut CdpConnection) -> T + 'static,
) -> T {
    let result = Rc::new(std::cell::RefCell::new(None));
    let result_slot = result.clone();
    access.defer_cleanup(Box::new(move |conn| {
        *result_slot.borrow_mut() = Some(operation(conn))
    }));
    result
        .borrow_mut()
        .take()
        .expect("direct cleanup is synchronous outside state access")
}

async fn fixture() -> (
    ProtocolStateOwner,
    ProtocolStateHandle,
    CommandOwnerScope,
    tokio::sync::mpsc::UnboundedReceiver<RuntimeInspectorResponseReady>,
) {
    let mut context = TestContext::new();
    let mut browser = BrowserContext::new("BID-bidi-owned".into());
    browser.set_active_target_id("TID-bidi-owned");
    browser.attach_active_session("SID-bidi-owned");
    context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
    context
        .install_navigation_fixture_for_session_owner("about:blank", Some("SID-bidi-owned"))
        .await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    context.with_fixture(|conn| conn.set_runtime_inspector_response_ready_sender(tx));
    let owner = context.state;
    let access = owner.handle();
    (
        owner,
        access,
        CommandOwnerScope::for_session("SID-bidi-owned"),
        rx,
    )
}

async fn proxy(
    driver: &mut ProtocolWorkflowDriver,
    access: &ProtocolStateHandle,
    command_owner: &CommandOwnerScope,
) -> (BidiChannelObjectGroupLease, PendingBidiChannelListener) {
    let realms = runtime_realm_inventory_using(driver, command_owner.clone())
        .await
        .unwrap();
    let realm = realms
        .into_iter()
        .find(|realm| realm.is_default != Some(false))
        .unwrap();
    let owner = command_owner.clone();
    let page_owner = inspect(access, move |conn| {
        BidiChannelPageOwner::capture_for_owner(conn, owner).unwrap()
    });
    let lease = driver
        .with_state(move |state| state.allocate_bidi_channel_object_group(page_owner))
        .unwrap()
        .unwrap();
    let response = dispatch_runtime_helper_using(driver, command_owner.clone(), "Runtime.evaluate", json!({
        "expression": "globalThis.__ownedChannel = {queue: [], getMessage() { if (this.queue.length) return Promise.resolve(this.queue.shift()); return new Promise(resolve => { this.resolve = resolve; }); }, sendMessage(value) { if (this.resolve) { const resolve = this.resolve; this.resolve = null; resolve(value); } else { this.queue.push(value); } }}; globalThis.__ownedChannel",
        "objectGroup": lease.name(), "returnByValue": false,
    })).await.unwrap();
    let BackgroundCommandResponsePayload::Success { result } = response else {
        panic!("the proxy must exist");
    };
    let handle = result["result"]["objectId"].as_str().unwrap().into();
    let listener = PendingBidiChannelListener::new(
        Some("TID-bidi-owned".into()),
        realm.realm_id,
        handle,
        lease.name().to_owned(),
        DevToolsBidiChannelProperties {
            channel: "owned-channel".into(),
            ownership: DevToolsResultOwnership::None,
            serialization_options: None,
        },
    )
    .unwrap();
    (lease, listener)
}

fn release_work(conn: &mut CdpConnection) -> Vec<ProtocolSchedulerWork> {
    conn.take_scheduler_events().into_iter().filter_map(|event| match event {
        CdpSchedulerEvent::ProtocolWorkPublished { work } if work.bidi_channel_owner_action_kind() == Some(crate::conn::bidi_channel_work::BidiChannelOwnerActionKind::ReleaseObjectGroup) => Some(work),
        _ => None,
    }).collect()
}

#[tokio::test]
async fn listener_hands_off_its_promise_without_blocking_peer_commands() {
    let test = async {
        let (_owner, access, command_owner, mut replies) = fixture().await;
        let mut driver = ProtocolWorkflowDriver::new(access.clone());
        let (lease, listener) = proxy(&mut driver, &access, &command_owner).await;
        let prepared = driver
            .with_state(move |state| state.prepare_bidi_channel_listener(lease, listener))
            .unwrap()
            .unwrap();
        let id = prepared
            .cancellation
            .identity
            .correlation
            .frontend_command_id()
            .get();
        let settled = prepared.cancellation.identity.settled.clone();
        let events = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            prepared.run(&mut driver),
        )
        .await
        .expect("startup does not wait for a message");
        assert!(events.events.is_empty());
        assert!(
            events.predecessor.is_some(),
            "the real Native dispatch retains its exact ordering lease"
        );
        assert!(settled.get());
        assert!(matches!(
            replies.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        let pending_owner = command_owner.clone();
        assert!(inspect(&access, move |conn| conn
            .target_devtools_session_state_for_owner(&pending_owner)
            .unwrap()
            .renderer_call_for_frontend(id)
            .is_some()));
        let version =
            crate::conn::ParsedCdpCommand::parse_str(r#"{"id":50,"method":"Browser.getVersion"}"#)
                .unwrap();
        assert!(matches!(
            driver
                .with_state(
                    move |state| state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut crate::conn::CommandDispatchContext::default()
                    )
                )
                .unwrap(),
            crate::conn::CdpCommandTaskStep::Complete(_)
        ));
        dispatch_runtime_helper_using(
            &mut driver,
            command_owner,
            "Runtime.evaluate",
            json!({"expression":"__ownedChannel.sendMessage('payload')","returnByValue":true}),
        )
        .await
        .unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(10), replies.recv())
            .await
            .unwrap()
            .unwrap();
        inspect(&access, move |conn| {
            let mut responses = Vec::new();
            let mut background = Vec::new();
            conn.route_registered_runtime_inspector_response_into(
                reply,
                &mut responses,
                &mut background,
            );
            assert!(responses.is_empty());
            assert_eq!(background.len(), 1);
            let (_, Some(crate::automation::AutomationEvent::ScriptMessage(message))) =
                background.pop().unwrap().into_parts()
            else {
                panic!("the long listener returns exactly one script.message");
            };
            assert_eq!(message.channel, "owned-channel");
            assert_eq!(message.data.value, json!("payload"));
            let restarts = conn.take_scheduler_events().into_iter().filter(|event| matches!(event, CdpSchedulerEvent::ProtocolWorkPublished { work } if work.bidi_channel_owner_action_kind() == Some(crate::conn::bidi_channel_work::BidiChannelOwnerActionKind::StartListener))).count();
            assert_eq!(restarts, 1);
        });
        assert!(matches!(
            replies.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    };
    tokio::task::LocalSet::new().run_until(test).await;
}

#[tokio::test]
async fn listener_cancellation_before_poll_and_after_native_ready_releases_its_exact_group_once() {
    for native_ready in [false, true] {
        let (_owner, access, command_owner, _replies) = fixture().await;
        let mut driver = ProtocolWorkflowDriver::new(access.clone());
        let (lease, listener) = proxy(&mut driver, &access, &command_owner).await;
        let handle = listener.channel_handle().as_str().to_owned();
        let mut prepared = driver
            .with_state(move |state| state.prepare_bidi_channel_listener(lease, listener))
            .unwrap()
            .unwrap();
        let id = prepared
            .cancellation
            .identity
            .correlation
            .frontend_command_id()
            .get();
        if native_ready {
            // Keep the native result ready while dropping its original owner,
            // before any synchronous state completion/handoff can take it.
            drop(prepared.pending.take().unwrap().wait().await.unwrap());
        }
        drop(prepared);
        let pending_owner = command_owner.clone();
        let work = inspect(&access, move |conn| {
            assert!(
                conn.target_devtools_session_state_for_owner(&pending_owner)
                    .unwrap()
                    .renderer_call_for_frontend(id)
                    .is_none()
            );
            let mut work = release_work(conn);
            assert_eq!(work.len(), 1);
            work.pop().unwrap()
        });
        let task = driver
            .with_state(move |state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap_or_else(|_| panic!("cleanup must use the original owner-work supervisor"))
            .unwrap();
        let completed = task.wait().await;
        driver
            .with_state(move |state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap();
        inspect(&access, move |conn| {
            assert!(
                conn.runtime_remote_object_group_for_session_owner(Some("SID-bidi-owned"), &handle)
                    .is_none()
            );
            assert!(
                release_work(conn).is_empty(),
                "finishing a release cannot publish another release"
            );
        });
    }
}

#[tokio::test]
async fn dropping_a_proxy_group_before_listener_handoff_releases_it_and_rejects_rebound_owners() {
    for rebound in [false, true] {
        let (_owner, access, command_owner, _replies) = fixture().await;
        let mut driver = ProtocolWorkflowDriver::new(access.clone());
        let (lease, listener) = proxy(&mut driver, &access, &command_owner).await;
        let handle = listener.channel_handle().as_str().to_owned();
        let group = lease.name().to_owned();
        if rebound {
            let session = command_owner.session_id().unwrap().to_owned();
            inspect(&access, move |conn| {
                let route = conn.session_route(Some(&session)).unwrap();
                conn.register_session_route_for_test(&session, route);
            });
        }
        drop(lease);
        let work = inspect(&access, release_work);
        assert_eq!(work.len(), usize::from(!rebound));
        if let Some(work) = work.into_iter().next() {
            let task = driver
                .with_state(move |state| state.try_start_owned_protocol_scheduler_work(work))
                .unwrap()
                .unwrap_or_else(|_| panic!("a proxy's group release is supervised"))
                .unwrap();
            let completed = task.wait().await;
            driver
                .with_state(move |state| state.complete_owned_protocol_scheduler_work(completed))
                .unwrap();
        }
        inspect(&access, move |conn| {
            assert_eq!(
                conn.runtime_remote_object_group_for_session_owner(Some("SID-bidi-owned"), &handle),
                rebound.then_some(group)
            )
        });
    }
}

#[tokio::test]
async fn preload_proxy_and_listener_run_through_direct_state_without_a_mailbox() {
    let test = async {
        let (_owner, access, command_owner, mut replies) = fixture().await;
        let mut driver = ProtocolWorkflowDriver::new(access.clone());
        let (_proxy_lease, _listener) = proxy(&mut driver, &access, &command_owner).await;
        dispatch_runtime_helper_using(&mut driver, command_owner.clone(), "Runtime.evaluate", json!({
            "expression":"globalThis.__preloadOwnedHandoff = token => token === 'guard' ? __ownedChannel : undefined",
            "returnByValue":true,
        })).await.unwrap();
        let preload_owner = command_owner.clone();
        inspect(&access, move |conn| {
            conn.with_target_owner_state_for_owner_mut(&preload_owner, |state| {
                state.document_start_scripts.push((
                    "owned-preload".into(),
                    crate::conn::DocumentStartScript {
                        registry_key: None,
                        devtools_session: None,
                        source: String::new(),
                        world_name: None,
                        has_bidi_channel_argument: true,
                        bidi_channel_handoffs: vec![moli_core::page::BidiPreloadChannelHandoff {
                            handoff_id: "__preloadOwnedHandoff".into(),
                            token: "guard".into(),
                            channel: "owned-preload-channel".into(),
                            ownership: None,
                            serialization_options: None,
                        }],
                    },
                ));
            })
            .unwrap();
        });
        let context_id = runtime_realm_inventory_using(&mut driver, command_owner.clone())
            .await
            .unwrap()
            .into_iter()
            .find(|realm| realm.is_default != Some(false))
            .unwrap()
            .context_id
            .unwrap();
        let preload_owner = CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::Browser);
        let prepared = driver
            .with_state(move |state| {
                state.prepare_bidi_preload_channel_listeners(preload_owner, context_id)
            })
            .unwrap()
            .unwrap();
        inspect(&access, |conn| {
            let browser = conn.browser_context.as_mut().unwrap();
            browser.stage_background_target(
                "TID-other-active".into(),
                Some("SID-other-active".into()),
                "about:blank".into(),
                None,
                None,
            );
            browser.set_active_target_id("TID-other-active");
        });
        let mut output = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            prepared.run(&mut driver, &mut output),
        )
        .await
        .unwrap();
        assert!(output.is_empty());
        assert!(matches!(
            replies.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        dispatch_runtime_helper_using(
            &mut driver,
            command_owner,
            "Runtime.evaluate",
            json!({"expression":"__ownedChannel.sendMessage('preload')","returnByValue":true}),
        )
        .await
        .unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(10), replies.recv())
            .await
            .unwrap()
            .unwrap();
        inspect(&access, move |conn| {
            let mut responses = Vec::new();
            let mut background = Vec::new();
            conn.route_registered_runtime_inspector_response_into(
                reply,
                &mut responses,
                &mut background,
            );
            assert!(responses.is_empty());
            assert_eq!(background.len(), 1);
            let (_, Some(crate::automation::AutomationEvent::ScriptMessage(message))) =
                background.pop().unwrap().into_parts()
            else {
                panic!("the ordinary async preload workflow owns a real channel listener");
            };
            assert_eq!(message.channel, "owned-preload-channel");
            assert_eq!(message.data.value, json!("preload"));
            assert_eq!(
                message.target_id.as_ref().map(|id| id.as_str()),
                Some("TID-bidi-owned")
            );
        });
    };
    tokio::task::LocalSet::new().run_until(test).await;
}

#[tokio::test]
async fn closing_rejects_new_channel_listeners_but_preserves_group_cleanup() {
    for close_target in [false, true] {
        let (_owner, access, command_owner, _replies) = fixture().await;
        let mut driver = ProtocolWorkflowDriver::new(access.clone());
        let (lease, listener) = proxy(&mut driver, &access, &command_owner).await;
        let handle = listener.channel_handle().as_str().to_owned();
        let live_owner = lease.owner.clone();
        let closing_owner = command_owner.clone();
        inspect(&access, move |conn| {
            if close_target {
                conn.mark_page_target_closing(&closing_owner);
            } else {
                let session = closing_owner.session_id().unwrap();
                let route = conn.session_route(Some(session)).unwrap();
                let plan =
                    crate::conn::SessionDisposalPlan::for_session_route(session, &route).unwrap();
                conn.mark_session_closing(&plan);
            }
            assert!(
                live_owner.is_current(conn),
                "Closing retains the original Page and cleanup authority"
            );
            assert!(!live_owner.can_start_listener(conn));
        });
        let proxy_owner = lease.owner.clone();
        assert!(
            driver
                .with_state(move |state| state.prepare_bidi_channel_proxy(
                    proxy_owner,
                    json!({"functionDeclaration":"function() { return 1; }"})
                ))
                .unwrap()
                .unwrap()
                .is_none()
        );
        let page_owner = lease.owner.clone();
        assert!(
            driver
                .with_state(move |state| state.allocate_bidi_channel_object_group(page_owner))
                .unwrap()
                .is_none()
        );
        assert!(
            driver
                .with_state(move |state| state.prepare_bidi_channel_listener(lease, listener))
                .unwrap()
                .is_none()
        );
        let work = inspect(&access, |conn| {
            assert!(
                !conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bidi-owned")),
                "no new listener entered the closing session"
            );
            let mut work = release_work(conn);
            assert_eq!(work.len(), 1);
            work.pop().unwrap()
        });
        let pending = driver
            .with_state(move |state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap_or_else(|_| panic!("cleanup remains supervised during Closing"))
            .unwrap();
        let completed = pending.wait().await;
        driver
            .with_state(move |state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap();
        inspect(&access, move |conn| {
            assert!(
                conn.runtime_remote_object_group_for_session_owner(Some("SID-bidi-owned"), &handle)
                    .is_none()
            );
            assert!(release_work(conn).is_empty());
        });
    }
}

#[tokio::test]
async fn deferred_channel_reply_cannot_consume_a_rebound_session_call_with_identical_ids() {
    let test = async {
        let (_owner, access, command_owner, mut replies) = fixture().await;
        let mut driver = ProtocolWorkflowDriver::new(access.clone());
        let (lease, listener) = proxy(&mut driver, &access, &command_owner).await;
        let prepared = driver
            .with_state(move |state| state.prepare_bidi_channel_listener(lease, listener))
            .unwrap()
            .unwrap();
        let correlation = prepared.cancellation.identity.correlation;
        let command_id = correlation.frontend_command_id().get();
        let output = prepared.run(&mut driver).await;
        assert!(output.predecessor.is_some());
        dispatch_runtime_helper_using(
            &mut driver,
            command_owner.clone(),
            "Runtime.evaluate",
            json!({"expression":"__ownedChannel.sendMessage('old binding')","returnByValue":true}),
        )
        .await
        .unwrap();
        let ready = tokio::time::timeout(std::time::Duration::from_secs(10), replies.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.command_id(), command_id);
        inspect(&access, move |conn| {
            let session = command_owner.session_id().unwrap();
            let route = conn.session_route(Some(session)).unwrap();
            conn.register_session_route_for_test(session, route);
            let before = conn
                .target_devtools_session_state_for_owner(&command_owner)
                .unwrap()
                .renderer_call_for_frontend(command_id);
            assert_eq!(
                before,
                Some(correlation),
                "retain every numeric component and route to exercise the lifetime proof"
            );
            let mut responses = Vec::new();
            let mut events = Vec::new();
            conn.route_registered_runtime_inspector_response_into(
                ready,
                &mut responses,
                &mut events,
            );
            assert!(responses.is_empty() && events.is_empty());
            assert_eq!(
                conn.target_devtools_session_state_for_owner(&command_owner)
                    .unwrap()
                    .renderer_call_for_frontend(command_id),
                before,
                "an old deferred reply cannot claim the successor binding's journal"
            );
        });
    };
    tokio::task::LocalSet::new().run_until(test).await;
}

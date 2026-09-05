use super::*;

mod accessibility;
mod css;
mod dom;
mod dom_debugger;
mod runtime;
pub use accessibility::RendererAccessibilityInspection;
pub use css::RendererCssInspection;
pub use dom::RendererDomInspection;
pub use dom_debugger::RendererDomDebuggerInspection;
pub use runtime::RendererRuntimeInspection;

impl RendererInspectionEndpoint {
    /// Seals the frontend's Main/IO ingress synchronously, then lets the
    /// original renderer owner destroy its V8 session. This lifecycle control
    /// does not wait behind paused JavaScript or introduce another command lane.
    pub fn detach_session(&self, inspector_session_id: Option<String>) -> Result<()> {
        self.page_context_cancel_tx.with_inspector_admission(|| {
            let session = DevToolsSessionKey::from_wire_session_id(
                inspector_session_id.as_deref().filter(|id| !id.is_empty()),
            );
            let pause_guard = RendererRuntimeInspectorSessionDetachGuard::new(
                self.devtools_target.pause(),
                self.devtools_target.clone(),
                self.devtools_agent_token,
                session,
            );
            self.render_runtime.dispatch_detached(
                RendererOwnerCommand::FinalizeRuntimeInspectorSessionDetach {
                    token: self.token,
                    inspector_session_id,
                    pause_guard,
                },
            )
        })?
    }

    // Only the finite typed agent facades may enter this path. Keep the native
    // PageAgent versus V8 OwnerOnly dispatch boundary of the original command.
    fn enqueue_typed_inspection_command(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
        command: RendererPageCommand,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.page_context_cancel_tx.with_inspector_admission(|| {
            self.devtools_target
                .main_ref()
                .enqueue_bound_protocol_page_command(
                    self.token,
                    self.devtools_agent_token,
                    command,
                    inspector_session_id,
                    attachment,
                )
        })
    }

    pub fn agent_token(&self) -> RendererDevToolsAgentToken {
        self.devtools_agent_token
    }

    pub fn routes_output_stream(&self, stream: RendererOutputStreamIdentity) -> bool {
        self.page_context_cancel_tx
            .with_inspector_admission(|| {
                stream.renderer_agent() == self.devtools_agent_token
                    && stream.residence()
                        == RendererOutputResidenceIdentity::Page {
                            owner_local_host_id: self.token.local_host_id,
                            page_id: self.token.page_id,
                        }
            })
            .unwrap_or(false)
    }

    pub fn enqueue_main_command(
        &self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.page_context_cancel_tx.with_inspector_admission(|| {
            self.devtools_target.main_ref().enqueue_command(
                self.token,
                self.devtools_agent_token,
                envelope,
            )
        })
    }

    pub fn enqueue_io_command(
        &self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> Result<RendererRuntimeInspectorIoCommandRoute> {
        self.enqueue_io_agent_command(RendererDevToolsIoCommandEnvelope::inspector(envelope))
    }

    pub fn enqueue_performance_get_metrics(
        &self,
        ticket: RendererInspectorIngressTicket,
        result: serde_json::Value,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Result<RendererRuntimeInspectorIoCommandRoute> {
        let envelope = match response {
            Some(response) => {
                debug_assert_eq!(
                    response.renderer_agent_attachment_id(),
                    ticket.attachment(),
                    "Performance response must belong to the command attachment"
                );
                RendererDevToolsIoCommandEnvelope::performance_get_metrics_with_response(
                    ticket, result, response,
                )
            }
            None => RendererDevToolsIoCommandEnvelope::performance_get_metrics(ticket),
        };
        self.enqueue_io_agent_command(envelope)
    }

    pub fn enqueue_set_script_execution_disabled(
        &self,
        ticket: RendererInspectorIngressTicket,
        disabled: bool,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Result<RendererRuntimeInspectorIoCommandRoute> {
        let control = self.script_execution_control.clone();
        let envelope = match response {
            Some(response) => {
                debug_assert_eq!(
                    response.renderer_agent_attachment_id(),
                    ticket.attachment(),
                    "Emulation response must belong to the command attachment"
                );
                RendererDevToolsIoCommandEnvelope::set_script_execution_disabled_with_response(
                    ticket, control, disabled, response,
                )
            }
            None => RendererDevToolsIoCommandEnvelope::set_script_execution_disabled(
                ticket, control, disabled,
            ),
        };
        self.enqueue_io_agent_command(envelope)
    }

    fn enqueue_io_agent_command(
        &self,
        envelope: RendererDevToolsIoCommandEnvelope,
    ) -> Result<RendererRuntimeInspectorIoCommandRoute> {
        self.page_context_cancel_tx.with_inspector_admission(|| {
            self.devtools_target
                .io_ref()
                .enqueue_command(self.devtools_agent_token, envelope)
        })
    }

    pub fn pause_active(&self) -> bool {
        self.page_context_cancel_tx
            .with_inspector_admission(|| self.devtools_target.pause_ref().is_pause_active())
            .unwrap_or(false)
    }

    // Only the physical Page owner can revoke this capability. Clone/drop of
    // an endpoint does not retire the Page or any frontend session.
    pub(super) fn retire_page(&self) {
        self.page_context_cancel_tx
            .cancel(RendererPageContextCancelReason::PageClosed);
        self.devtools_target.detach_page(
            self.token.page_id,
            self.devtools_agent_token,
            "Inspector Page handle was dropped",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devtools::{
        ingress::{io::RendererInspectorIoIngress, main::RendererInspectorMainIngress},
        pause::RendererInspectorPauseBridge,
        route::RendererInspectorSessionExecutorRouteId,
        target::{RendererDevToolsTargetHandle, RendererDevToolsTargetShutdownRegistry},
    };
    use futures_util::FutureExt;

    fn endpoint() -> RendererInspectionEndpoint {
        let pause = RendererInspectorPauseBridge::default();
        let main = RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(1),
            pause.pause_loop_wake(),
        );
        let io = RendererInspectorIoIngress::new(pause.pause_loop_wake(), None);
        let (page_context_cancel_tx, _) = renderer_page_context_cancel_channel();
        RendererInspectionEndpoint {
            render_runtime: RenderRuntimeHandle::disconnected(),
            token: RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
            devtools_agent_token: RendererDevToolsAgentToken::allocate(),
            page_context_cancel_tx,
            devtools_target: RendererDevToolsTargetHandle::new(pause, main, io),
            script_execution_control: Default::default(),
        }
    }

    fn main_command() -> RendererInspectorCommandEnvelope {
        let (response_tx, _) = oneshot::channel();
        RendererInspectorCommandEnvelope::new_main_protocol(
            RendererInspectorIngressTicket::new(
                None,
                None,
                RendererInspectorCommandRoute::MainThread,
            ),
            None,
            r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"42"}}"#.into(),
            RendererRuntimeInspectorResponseSender::new(1, response_tx),
        )
    }

    fn io_command() -> RendererInspectorCommandEnvelope {
        RendererInspectorCommandEnvelope::new_io(
            RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io),
            r#"{"id":2,"method":"Debugger.pause"}"#.into(),
            None,
        )
    }

    fn dedicated_io_commands(
        endpoint: &RendererInspectionEndpoint,
    ) -> [Result<RendererRuntimeInspectorIoCommandRoute>; 2] {
        let ticket =
            || RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io);
        [
            endpoint.enqueue_performance_get_metrics(ticket(), serde_json::Value::Null, None),
            endpoint.enqueue_set_script_execution_disabled(ticket(), true, None),
        ]
    }

    #[test]
    fn document_agent_bindings_preserve_main_fifo_and_native_vs_v8_nested_boundaries() {
        use crate::devtools::command::RendererDevToolsMainNestedDispatch;

        for agent in ["DOM", "CSS", "AX", "DOMSnapshot", "Runtime", "DOMDebugger"] {
            let endpoint = endpoint();
            let attachment = RendererAgentAttachmentId::allocate();
            let dom = endpoint.dom_inspection(attachment, Some("dom-session".to_owned()));
            let css = endpoint.css_inspection(attachment, Some("dom-session".to_owned()));
            let ax = endpoint.accessibility_inspection(attachment, Some("dom-session".to_owned()));
            let runtime = endpoint.runtime_inspection(attachment, Some("dom-session".to_owned()));
            let debugger =
                endpoint.dom_debugger_inspection(attachment, Some("dom-session".to_owned()));
            let (native, v8) = match agent {
                "DOM" => (
                    dom.start_document_node_snapshot_for_document(true, 1, false),
                    dom.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                        1, None, None,
                    ),
                ),
                "CSS" => (
                    css.start_computed_style_for_backend_node(1),
                    css.start_computed_style_for_object("object"),
                ),
                "AX" => (
                    ax.start_accessibility_tree_payloads_for_document(None),
                    ax.start_accessibility_tree_payloads_for_object_id("object"),
                ),
                "DOMSnapshot" => (
                    dom.start_dom_snapshot_capture("frame".into(), Default::default()),
                    dom.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                        1, None, None,
                    ),
                ),
                "Runtime" => (
                    runtime.start_install_runtime_binding("binding", None, None),
                    debugger.start_dom_debugger_get_event_listeners("object".into(), 1, false),
                ),
                "DOMDebugger" => (
                    debugger.start_dom_debugger_configure_xhr_breakpoint(
                        RendererDomDebuggerXhrBreakpoint::new("/break".into()),
                        true,
                    ),
                    debugger.start_dom_debugger_get_event_listeners("object".into(), 1, false),
                ),
                _ => unreachable!(),
            };
            let native = native.unwrap();
            let v8 = v8.unwrap();
            for route in [&native, &v8] {
                assert_eq!(route.ticket().attachment(), Some(attachment));
                assert_eq!(
                    route.ticket().session().wire_session_id(),
                    Some("dom-session")
                );
            }

            let main = endpoint.devtools_target.main_ref();
            let mut native = main
                .claim_for_pause()
                .expect("native inspection retains its Page-agent boundary");
            assert_eq!(
                native.nested_dispatch(),
                RendererDevToolsMainNestedDispatch::PageAgent
            );
            let handoff = main.first_dispatch_guard(&mut native);
            assert!(
                main.claim_for_owner().is_none(),
                "one session keeps its first-dispatch FIFO"
            );
            drop(handoff);
            assert!(
                main.claim_for_pause().is_none(),
                "typed V8 resolution remains owner-only"
            );
            let v8 = main.claim_for_owner().unwrap();
            assert_eq!(
                v8.nested_dispatch(),
                RendererDevToolsMainNestedDispatch::OwnerOnly
            );
            assert_eq!(v8.ticket().attachment(), Some(attachment));
        }
    }

    #[test]
    fn runtime_bootstrap_preserves_owner_only_main_fifo() {
        use crate::devtools::command::RendererDevToolsMainNestedDispatch;

        let endpoint = endpoint();
        let attachment = RendererAgentAttachmentId::allocate();
        let runtime = endpoint.runtime_inspection(attachment, Some("bootstrap-session".to_owned()));
        let _bootstrap = runtime
            .start_apply_runtime_protocol_state(&[], &[], &[], &[])
            .unwrap();
        let _following = runtime
            .start_install_runtime_binding("afterBootstrap", None, None)
            .unwrap();
        let main = endpoint.devtools_target.main_ref();
        assert!(
            main.claim_for_pause().is_none(),
            "bootstrap enters V8 only on its owner"
        );
        let mut bootstrap = main.claim_for_owner().unwrap();
        assert_eq!(
            bootstrap.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::OwnerOnly
        );
        assert_eq!(bootstrap.ticket().attachment(), Some(attachment));
        assert_eq!(
            bootstrap.ticket().session().wire_session_id(),
            Some("bootstrap-session")
        );
        let handoff = main.first_dispatch_guard(&mut bootstrap);
        assert!(
            main.claim_for_pause().is_none(),
            "native follow-up cannot overtake bootstrap handoff"
        );
        drop(handoff);
        let following = main.claim_for_pause().unwrap();
        assert_eq!(
            following.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::PageAgent
        );
        assert_eq!(following.ticket().attachment(), Some(attachment));
    }

    fn document_agent_commands(
        endpoint: &RendererInspectionEndpoint,
    ) -> [Result<RendererRuntimeInspectorMainCommandRoute>; 16] {
        let attachment = RendererAgentAttachmentId::allocate();
        let dom = endpoint.dom_inspection(attachment, None);
        let css = endpoint.css_inspection(attachment, None);
        let ax = endpoint.accessibility_inspection(attachment, None);
        let runtime = endpoint.runtime_inspection(attachment, None);
        let debugger = endpoint.dom_debugger_inspection(attachment, None);
        [
            dom.start_document_node_snapshot_for_document(true, 1, false),
            dom.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                1, None, None,
            ),
            css.start_computed_style_for_backend_node(1),
            css.start_computed_style_for_object("object"),
            ax.start_accessibility_tree_payloads_for_document(None),
            ax.start_accessibility_tree_payloads_for_object_id("object"),
            dom.start_dom_snapshot_capture("frame".into(), Default::default()),
            runtime.start_install_runtime_binding("binding", None, None),
            runtime.start_remove_runtime_binding("binding"),
            runtime.start_set_runtime_binding_state(&[], &[]),
            runtime.start_apply_runtime_protocol_state(&[], &[], &[], &[]),
            runtime.start_create_isolated_world_runtime_activity(None, "world", false),
            runtime.start_runtime_realm_inventory(),
            runtime.start_remove_document_start_script_by_registry_key("preload"),
            debugger.start_dom_debugger_get_event_listeners("object".into(), 1, false),
            debugger.start_dom_debugger_configure_xhr_breakpoint(
                RendererDomDebuggerXhrBreakpoint::new("/break".into()),
                true,
            ),
        ]
    }

    #[test]
    fn document_agent_binding_retirement_and_context_shutdown_settle_queued_and_late_commands() {
        for context_shutdown in [false, true] {
            let endpoint = endpoint();
            let registry = RendererDevToolsTargetShutdownRegistry::default();
            let _registration = registry.register(endpoint.devtools_target.clone()).unwrap();
            let queued = document_agent_commands(&endpoint).map(Result::unwrap);
            if context_shutdown {
                registry.terminate_all();
            } else {
                endpoint.retire_page();
            }
            for route in queued {
                assert_main_canceled(route);
            }
            for route in document_agent_commands(&endpoint).into_iter().flatten() {
                assert_main_canceled(route);
            }
            assert!(
                endpoint
                    .devtools_target
                    .main_ref()
                    .claim_for_owner()
                    .is_none()
            );
        }
    }

    #[test]
    fn retired_page_rejects_late_main_inspection_without_executor() {
        let endpoint = endpoint();
        endpoint
            .page_context_cancel_tx
            .cancel(RendererPageContextCancelReason::PageClosed);
        assert!(endpoint.enqueue_main_command(main_command()).is_err());
        assert!(
            endpoint
                .devtools_target
                .main_ref()
                .claim_for_owner()
                .is_none()
        );
    }

    #[test]
    fn retired_page_rejects_late_io_inspection_without_executor() {
        let endpoint = endpoint();
        endpoint
            .page_context_cancel_tx
            .cancel(RendererPageContextCancelReason::PageClosed);
        assert!(endpoint.enqueue_io_command(io_command()).is_err());
        assert!(
            endpoint
                .devtools_target
                .io_ref()
                .claim_for_owner()
                .is_none()
        );
    }

    #[test]
    fn inspection_admission_racing_page_retirement_settles_without_executor() {
        let endpoint = endpoint();
        let start = std::sync::Barrier::new(2);
        let (main, io, dedicated, document_agents) = std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                endpoint.retire_page();
            });
            start.wait();
            (
                endpoint.enqueue_main_command(main_command()),
                endpoint.enqueue_io_command(io_command()),
                dedicated_io_commands(&endpoint),
                document_agent_commands(&endpoint),
            )
        });

        if let Ok(main) = main {
            assert_main_canceled(main);
        }
        if let Ok(io) = io {
            assert_io_canceled(io);
        }
        for route in dedicated.into_iter().flatten() {
            assert_io_canceled(route);
        }
        for route in document_agents.into_iter().flatten() {
            assert_main_canceled(route);
        }
        assert_retired(&endpoint);
    }

    #[test]
    fn inspection_endpoint_shutdown_cancels_queued_and_late_work_without_executor() {
        let endpoint = endpoint();
        let registry = RendererDevToolsTargetShutdownRegistry::default();
        let _registration = registry.register(endpoint.devtools_target.clone()).unwrap();
        let main = endpoint.enqueue_main_command(main_command()).unwrap();
        let io = endpoint.enqueue_io_command(io_command()).unwrap();
        let dedicated = dedicated_io_commands(&endpoint).map(Result::unwrap);

        registry.terminate_all();

        assert_main_canceled(main);
        assert_io_canceled(io);
        for route in dedicated {
            assert_io_canceled(route);
        }
        // The target can become terminal before the Context broadcasts Page
        // cancellation. Its existing receivers must also seal late admission.
        assert_main_canceled(endpoint.enqueue_main_command(main_command()).unwrap());
        assert_io_canceled(endpoint.enqueue_io_command(io_command()).unwrap());
        for route in dedicated_io_commands(&endpoint) {
            assert_io_canceled(route.unwrap());
        }
    }

    #[test]
    fn retired_inspection_endpoint_cannot_enter_replacement_agent_lanes() {
        let old = endpoint();
        let (page_context_cancel_tx, _) = renderer_page_context_cancel_channel();
        let replacement = RendererInspectionEndpoint {
            render_runtime: RenderRuntimeHandle::disconnected(),
            token: RendererPageToken::new_for_testing(PageId::new_for_testing(2)),
            devtools_agent_token: RendererDevToolsAgentToken::allocate(),
            page_context_cancel_tx,
            devtools_target: old.devtools_target.clone(),
            script_execution_control: Default::default(),
        };
        let old_stream = RendererOutputStreamIdentity::new_page(
            old.token.local_host_id,
            old.token.page_id,
            old.devtools_agent_token,
        );
        let replacement_stream = RendererOutputStreamIdentity::new_page(
            replacement.token.local_host_id,
            replacement.token.page_id,
            replacement.devtools_agent_token,
        );
        assert!(old.routes_output_stream(old_stream));
        assert!(!old.routes_output_stream(replacement_stream));
        assert!(!replacement.routes_output_stream(old_stream));
        replacement
            .devtools_target
            .pause_ref()
            .configure_page_route(RendererTurnOutputJournal::new(
                RendererOutputStreamIdentity::new_page(
                    replacement.token.local_host_id,
                    replacement.token.page_id,
                    replacement.devtools_agent_token,
                ),
            ));
        let old_main = old.enqueue_main_command(main_command()).unwrap();
        let old_io = old.enqueue_io_command(io_command()).unwrap();
        let old_dedicated = dedicated_io_commands(&old).map(Result::unwrap);
        let _new_main = replacement.enqueue_main_command(main_command()).unwrap();
        let new_io = replacement.enqueue_io_command(io_command()).unwrap();
        let new_dedicated = dedicated_io_commands(&replacement).map(Result::unwrap);

        old.retire_page();

        assert!(
            old.detach_session(None).is_err(),
            "retired detach must not close replacement ingress"
        );
        assert_retired(&old);
        assert!(!old.routes_output_stream(old_stream));
        assert!(replacement.routes_output_stream(replacement_stream));
        assert_main_canceled(old_main);
        assert_io_canceled(old_io);
        for route in old_dedicated {
            assert_io_canceled(route);
        }
        let main = replacement.devtools_target.main_ref();
        let io = replacement.devtools_target.io_ref();
        let mut main_command = main.claim_for_owner().unwrap();
        let mut io_command = io.claim_for_owner().unwrap();
        assert_eq!(main_command.agent_token, replacement.devtools_agent_token);
        assert_eq!(io_command.agent_token, replacement.devtools_agent_token);
        drop(main.first_dispatch_guard(&mut main_command));
        io.first_dispatch_guard(&mut io_command).release();
        assert_eq!(
            new_io
                .wait_for_first_dispatch()
                .now_or_never()
                .unwrap()
                .unwrap(),
            RendererRuntimeInspectorIoCommandClaim::Dispatched
        );
        assert!(main.claim_for_owner().is_none());
        for route in new_dedicated {
            let mut command = io
                .claim_for_owner()
                .expect("replacement IO agent must remain queued");
            assert_eq!(command.agent_token, replacement.devtools_agent_token);
            assert!(
                io.claim_for_owner().is_none(),
                "dedicated agents share the same first-dispatch FIFO"
            );
            io.first_dispatch_guard(&mut command).release();
            assert_eq!(
                route
                    .wait_for_first_dispatch()
                    .now_or_never()
                    .unwrap()
                    .unwrap(),
                RendererRuntimeInspectorIoCommandClaim::Dispatched
            );
        }
        assert!(io.claim_for_owner().is_none());
    }

    #[test]
    fn failed_session_finalization_settles_pending_main_and_io_commands() {
        let endpoint = endpoint();
        let main = endpoint.enqueue_main_command(main_command()).unwrap();
        let io = endpoint.enqueue_io_command(io_command()).unwrap();
        let error = endpoint.detach_session(None).unwrap_err();
        assert!(error.to_string().contains("shut down"));
        assert_main_canceled(main);
        assert_io_canceled(io);
        assert_main_canceled(endpoint.enqueue_main_command(main_command()).unwrap());
        assert_io_canceled(endpoint.enqueue_io_command(io_command()).unwrap());
    }

    async fn real_page() -> (JsRuntimeOwner, RendererPageHandle) {
        let runtime = JsRuntime::initialize();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let page = crate::runtime::tests::create_test_html_page(
            &runtime,
            &loader,
            url::Url::parse("https://example.test/inspection-endpoint").unwrap(),
            "<!doctype html><title>endpoint</title>",
        )
        .await;
        (runtime, page)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inspection_endpoint_dispatches_without_page_borrow_and_is_revoked_on_close() {
        let (_runtime, mut page) = real_page().await;
        let endpoint = page.inspection_endpoint();
        let testing = RendererPageTestingHandle::new_for_testing(&page);
        let (response_tx, _response_rx) = oneshot::channel();
        let route = endpoint
            .enqueue_main_command(RendererInspectorCommandEnvelope::new_main_protocol(
                RendererInspectorIngressTicket::new(
                    None,
                    None,
                    RendererInspectorCommandRoute::MainThread,
                ),
                None,
                r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"42"}}"#.into(),
                RendererRuntimeInspectorResponseSender::new(1, response_tx),
            ))
            .unwrap();
        let RendererRuntimeInspectorMainCommandCompletion::Owner(output) =
            route.wait_for_completion().await.unwrap()
        else {
            panic!("standalone Main dispatch must retain its committed owner output");
        };
        assert_eq!(
            output
                .runtime_inspector_output()
                .unwrap()
                .protocol_response(1)
                .unwrap()["result"]["result"]["value"],
            serde_json::json!(42)
        );
        drop(endpoint);
        assert!(testing.owner_slot_async().await.is_ok());

        let endpoint = page.inspection_endpoint();
        page.close_async().await.unwrap();
        assert_retired(&endpoint);
        assert!(testing.owner_slot_async().await.is_err());
        page.close_async().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inspection_endpoint_does_not_keep_dropped_page_alive() {
        let (_runtime, page) = real_page().await;
        let endpoint = page.inspection_endpoint();
        let testing = RendererPageTestingHandle::new_for_testing(&page);
        assert!(testing.owner_slot_async().await.is_ok());

        drop(page);

        assert_retired(&endpoint);
        assert!(testing.owner_slot_async().await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inspection_endpoint_is_revoked_when_context_owner_drops() {
        let (root, page) = real_page().await;
        let retained_runtime = root.handle();
        let endpoint = page.inspection_endpoint();
        let testing = RendererPageTestingHandle::new_for_testing(&page);
        assert!(testing.owner_slot_async().await.is_ok());

        drop(root);

        assert_retired(&endpoint);
        drop(page);
        drop(retained_runtime);
    }

    fn assert_retired(endpoint: &RendererInspectionEndpoint) {
        assert!(endpoint.enqueue_main_command(main_command()).is_err());
        assert!(endpoint.enqueue_io_command(io_command()).is_err());
        assert!(
            dedicated_io_commands(endpoint)
                .into_iter()
                .all(|route| route.is_err())
        );
        assert!(!endpoint.pause_active());
    }

    fn assert_main_canceled(route: RendererRuntimeInspectorMainCommandRoute) {
        assert!(matches!(
            route.wait_for_completion().now_or_never(),
            Some(Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(
                _
            )))
        ));
    }

    fn assert_io_canceled(route: RendererRuntimeInspectorIoCommandRoute) {
        assert!(matches!(
            route.wait_for_first_dispatch().now_or_never(),
            Some(Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_)))
        ));
    }
}

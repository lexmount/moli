use super::*;

impl RendererInspectionEndpoint {
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
        self.page_context_cancel_tx.with_inspector_admission(|| {
            self.devtools_target.io_ref().enqueue_command(
                self.devtools_agent_token,
                RendererDevToolsIoCommandEnvelope::inspector(envelope),
            )
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
            token: RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
            devtools_agent_token: RendererDevToolsAgentToken::allocate(),
            page_context_cancel_tx,
            devtools_target: RendererDevToolsTargetHandle::new(pause, main, io),
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
        let (main, io) = std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                endpoint.retire_page();
            });
            start.wait();
            (
                endpoint.enqueue_main_command(main_command()),
                endpoint.enqueue_io_command(io_command()),
            )
        });

        if let Ok(main) = main {
            assert_main_canceled(main);
        }
        if let Ok(io) = io {
            assert_io_canceled(io);
        }
        assert!(endpoint.enqueue_main_command(main_command()).is_err());
        assert!(endpoint.enqueue_io_command(io_command()).is_err());
    }

    #[test]
    fn inspection_endpoint_shutdown_cancels_queued_and_late_work_without_executor() {
        let endpoint = endpoint();
        let registry = RendererDevToolsTargetShutdownRegistry::default();
        let _registration = registry.register(endpoint.devtools_target.clone()).unwrap();
        let main = endpoint.enqueue_main_command(main_command()).unwrap();
        let io = endpoint.enqueue_io_command(io_command()).unwrap();

        registry.terminate_all();

        assert_main_canceled(main);
        assert_io_canceled(io);
        // The target can become terminal before the Context broadcasts Page
        // cancellation. Its existing receivers must also seal late admission.
        assert_main_canceled(endpoint.enqueue_main_command(main_command()).unwrap());
        assert_io_canceled(endpoint.enqueue_io_command(io_command()).unwrap());
    }

    #[test]
    fn retired_inspection_endpoint_cannot_enter_replacement_agent_lanes() {
        let old = endpoint();
        let (page_context_cancel_tx, _) = renderer_page_context_cancel_channel();
        let replacement = RendererInspectionEndpoint {
            token: RendererPageToken::new_for_testing(PageId::new_for_testing(2)),
            devtools_agent_token: RendererDevToolsAgentToken::allocate(),
            page_context_cancel_tx,
            devtools_target: old.devtools_target.clone(),
        };
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
        let _new_main = replacement.enqueue_main_command(main_command()).unwrap();
        let new_io = replacement.enqueue_io_command(io_command()).unwrap();

        old.retire_page();

        assert_retired(&old);
        assert_main_canceled(old_main);
        assert_io_canceled(old_io);
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
        assert!(io.claim_for_owner().is_none());
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

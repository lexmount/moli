use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
    TargetPageResidenceIdentity,
};
use crate::domains::{command_output::CommandOutputBuffer, page};
use moli_core::browser::DocumentHandle;

impl CdpConnection {
    /// Observe an exact Browser commit. This never starts or completes a
    /// navigation, and never substitutes the Target's current Document for a
    /// stale event. Command completions and snapshot recovery share the rebind.
    pub async fn project_browser_document_commit(
        &mut self,
        document: DocumentHandle,
    ) -> Vec<BackgroundProtocolEvent> {
        let Ok(snapshot) = self.browser.document_commit_snapshot(document) else {
            return Vec::new();
        };
        if let Some(sender) = self.scheduler_hooks.renderer_publication_sender()
            && let Err(error) = snapshot.inspection_endpoint.bind_output_transport(sender)
        {
            tracing::warn!(%error, "native Document output transport binding failed");
            return Vec::new();
        }
        let metadata = snapshot.metadata.clone();
        let mut allocator = std::mem::take(&mut self.network_request_id_allocator);
        let projected = (|| {
            let context =
                self.browser_context_by_browser_id_mut(document.web_contents().context())?;
            let target_id = context
                .page_targets
                .get_for_web_contents(document.web_contents().id())?
                .target_id()
                .to_owned();
            let projection = context.project_document_commit_snapshot(&target_id, snapshot)?;
            let target = context.page_targets.get_mut(&target_id)?;
            if projection.fence.is_ok()
                && let Err(error) = target
                    .runtime_slot
                    .restore_native_document_sessions(&target.devtools_sessions)
            {
                tracing::warn!(%error, "native Document inspection restore admission failed");
            }
            let loader_id = context.project_committed_document_loader_for_target(
                &target_id,
                metadata.navigation,
                &mut allocator,
            )?;
            let owner = CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
                context.id.clone(),
                Some(target_id.clone()),
                document.id(),
            ));
            Some((owner, target_id, loader_id, projection))
        })();
        self.network_request_id_allocator = allocator;
        let Some((owner, frame_id, loader_id, projection)) = projected else {
            return Vec::new();
        };
        let (binding, lifecycle_events) = self.project_committed_document_lifecycle_for_owner(
            &owner,
            metadata.lifecycle.clone(),
            metadata.navigation,
            frame_id.clone(),
            loader_id.clone(),
        );
        // Core can advance independently, including between synchronous reads.
        // An obsolete occurrence must not publish a fence for a newer document.
        let Some(binding) = binding.filter(|binding| {
            binding.document_id == document.id()
                && binding.browser_sequence == metadata.lifecycle.browser_sequence
        }) else {
            return Vec::new();
        };
        if self
            .page_event_session_ids_for_owner(&owner)
            .iter()
            .any(|session| {
                let session_owner = owner.for_target_event_session(self, session.as_deref());
                self.target_runtime_session_state_for_owner(&session_owner)
                    .is_some_and(|state| state.runtime_frontend_enabled)
            })
        {
            let _ = self
                .set_renderer_runtime_agent_owns_page_console_api_events_for_owner(&owner, true);
        }
        let mut events = Vec::new();
        crate::domains::target::emit_target_info_changed_for_owner_background_event(
            self,
            &mut events,
            &owner,
        );
        if let Some(info) = metadata.info.as_ref() {
            for session_id in self.page_event_session_ids_for_owner(&owner) {
                let event_owner = owner.for_target_event_session(self, session_id.as_deref());
                page::emit_navigation_frame_commit_background_events(
                    &mut events,
                    session_id.as_deref(),
                    crate::domains::dom::dom_agent_enabled_for_owner(self, &event_owner),
                    &frame_id,
                    &loader_id,
                    info.url.as_str(),
                    None,
                    &info.security_origin,
                    &info.secure_context_type,
                );
            }
        }
        page::emit_bound_renderer_document_lifecycle_background_events(
            self,
            &mut events,
            &owner,
            &binding,
            &lifecycle_events,
        );
        let mut out = CommandOutputBuffer::default();
        let mut command_context = CommandDispatchContext::default();
        out.extend_background_events_after_messages(events);
        match projection.fence {
            Ok(Some(fence)) => {
                let release =
                    self.publish_document_projection_fence_for_owner(&owner, &binding, fence);
                page::release_document_projection_output_async(
                    self,
                    &mut out,
                    &mut command_context,
                    &owner,
                    release,
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "inspection projection failed after native Browser commit");
                if let Ok(slot) = self.runtime_session_owner_slot_mut_for_owner(&owner) {
                    slot.install_pending_renderer_call_replacements(Default::default());
                }
                let sessions = self.page_event_session_ids_for_owner(&owner);
                page::fail_navigation_inspection_sessions(
                    self,
                    &mut out,
                    &mut command_context,
                    &owner,
                    sessions,
                    "Inspector rebind failed after navigation",
                );
            }
        }
        if let Some(previous) = projection.replaced_page_owner {
            out.extend_background_events_after_messages(
                crate::domains::target::retire_dedicated_worker_targets_for_replaced_page_async(
                    self, &previous,
                )
                .await,
            );
        }
        out.extend_background_events_after_messages(command_context.take_protocol_events());
        out.into_plan().into_background_events(None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestContext;
    use moli_core::browser::web_contents::DocumentNavigationDestination;
    use moli_core::browser::{
        BrowserContextHandle, NavigationRequestLoadPolicy, WebContentsHandle,
    };
    use moli_core::runtime::{
        CommittedDocumentResourceSource, ExternalRawDocumentBodyStream, PageVmInitStage,
        RendererReplyBoundary,
    };
    use serde_json::json;
    use url::Url;

    // Deliberately no DevTools navigation admission, commit, loader, or frame
    // projection: the native Browser completes while its observer is idle.
    async fn navigate_native(
        context: &BrowserContextHandle,
        contents: WebContentsHandle,
        url: &str,
    ) -> DocumentHandle {
        let navigation = context.start_document_navigation(contents).unwrap();
        let inherited = context.inherited_document_policy(Default::default(), &[], None, None);
        let mut load = context
            .start_navigation_load(
                contents,
                navigation,
                NavigationRequestLoadPolicy::BrowserInitiated,
                inherited,
            )
            .unwrap();
        let fetched = load
            .fetch_navigation("GET", url, None, Vec::new())
            .await
            .unwrap();
        let response = fetched
            .fetch_result
            .into_parts_with_observation_journal()
            .0
            .into_materialized_raw_response()
            .await
            .unwrap();
        let destination = DocumentNavigationDestination {
            url: response.final_url.clone(),
            security_origin: response.final_url.origin().ascii_serialization(),
            secure_context_type: "SecureLocalhost".to_owned(),
        };
        let prepared = load
            .prepare_document_response_async(
                Url::parse(url).unwrap(),
                response.final_url.clone(),
                response.redirected,
                response.redirect_chain.len(),
                response.status,
                response.headers.clone(),
                ExternalRawDocumentBodyStream::from_bytes(response.clone_body_bytes()),
                PageVmInitStage::DomContentLoaded,
                RendererReplyBoundary::DocumentCommit,
                CommittedDocumentResourceSource::Navigation(Box::new(
                    fetched.document_fetch_context_seed,
                )),
                fetched.reserved_service_worker_client,
            )
            .await
            .unwrap();
        let built = context
            .start_document_materialization(
                contents,
                navigation,
                prepared,
                destination,
                context.inherited_document_policy(Default::default(), &[], None, None),
            )
            .unwrap()
            .materialize()
            .await
            .unwrap();
        let commit = context.commit_document_navigation(built.page).unwrap();
        commit.post_response_continuation.unwrap().release();
        commit.retirement.close().await;
        commit.snapshot.document
    }

    async fn fixture() -> (
        TestContext,
        BrowserContextHandle,
        DocumentHandle,
        CommandOwnerScope,
    ) {
        let mut ctx = TestContext::new();
        let mut context = ctx
            .conn
            .new_browser_context_fixture_for_test("BID-native-commit");
        context.set_active_target_id("TID-native-commit");
        ctx.conn.install_browser_context_fixture_for_test(context);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<title>old</title><p>old</p>",
            None,
        )
        .await;
        for (id, method) in [(2, "Page.enable"), (3, "Runtime.enable"), (4, "DOM.enable")] {
            ctx.process_and_wait_for_response_async(json!({"id": id, "method": method}))
                .await;
            assert!(ctx.take_response_by_id(id).get("error").is_none());
        }
        let owner = CommandOwnerScope::capture(&ctx.conn, None);
        let (context_id, target_id) = ctx
            .conn
            .resolved_page_owner_identity_for_owner(&owner)
            .unwrap();
        let (context, document) = ctx
            .conn
            .browser_context_by_id(&context_id)
            .unwrap()
            .inspection_document_handle_for_test(&target_id)
            .unwrap();
        ctx.take_all();
        (ctx, context, document, owner)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn command_document_commit_events_and_snapshot_recovery_do_not_rebind_twice() {
        let (mut ctx, _, document, owner) = fixture().await;
        let attachment = ctx
            .conn
            .current_renderer_agent_attachment_for_owner(&owner)
            .unwrap();
        assert!(
            ctx.conn
                .project_browser_document_commit(document)
                .await
                .is_empty()
        );
        let snapshot = ctx.conn.subscribe_browser_events().unwrap().0;
        assert_eq!(snapshot.documents, [document]);
        assert!(
            ctx.conn
                .project_browser_document_commit(snapshot.documents[0])
                .await
                .is_empty()
        );
        assert_eq!(
            ctx.conn.current_renderer_agent_attachment_for_owner(&owner),
            Some(attachment)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn native_document_commit_rebinds_runtime_and_invalidates_old_objects_once() {
        let (mut ctx, context, old, owner) = fixture().await;
        ctx.process_and_wait_for_response_async(
            json!({"id": 5, "method": "Runtime.evaluate", "params": {
                "expression": "({value:'old'})",
            }}),
        )
        .await;
        let old_object = ctx.take_response_by_id(5)["result"]["result"]["objectId"]
            .as_str()
            .unwrap()
            .to_owned();
        ctx.take_all();
        let new = navigate_native(
            &context,
            old.web_contents(),
            "data:text/html,<title>native</title><p>new</p>",
        )
        .await;
        assert_ne!(new, old);
        assert_eq!(
            ctx.conn
                .current_renderer_agent_attachment_for_owner(&owner)
                .unwrap()
                .document(),
            old.id()
        );
        let projected = ctx.conn.project_browser_document_commit(new).await;
        ctx.sent.extend(
            projected
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message),
        );
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|event| event["method"] == "Page.frameNavigated")
                .count(),
            1
        );
        assert!(
            ctx.sent
                .iter()
                .any(|event| event["method"] == "DOM.documentUpdated")
        );
        assert_eq!(
            ctx.conn
                .current_renderer_agent_attachment_for_owner(&owner)
                .unwrap()
                .document(),
            new.id()
        );
        assert!(
            ctx.conn
                .project_browser_document_commit(new)
                .await
                .is_empty()
        );
        assert!(
            ctx.conn
                .project_browser_document_commit(old)
                .await
                .is_empty()
        );
        ctx.process_and_wait_for_response_async(
            json!({"id": 6, "method": "Runtime.evaluate", "params": {
                "expression": "console.log('native-after-restore'); document.title",
            }}),
        )
        .await;
        assert_eq!(
            ctx.take_response_by_id(6)["result"]["result"]["value"],
            "native"
        );
        ctx.process_and_wait_for_response_async(
            json!({"id": 7, "method": "Runtime.getProperties", "params": {"objectId": old_object}}),
        )
        .await;
        assert!(
            ctx.take_response_by_id(7).get("error").is_some(),
            "old object must not resolve in the new Document"
        );
        let frame = ctx
            .sent
            .iter()
            .position(|event| event["method"] == "Page.frameNavigated")
            .unwrap();
        let created = ctx
            .sent
            .iter()
            .filter(|event| event["method"] == "Runtime.executionContextCreated")
            .count();
        assert!(
            created > 0,
            "native navigation must restore enabled Runtime sessions: {:?}",
            ctx.sent
        );
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|event| event["method"] == "Runtime.consoleAPICalled"
                    && event["params"]["args"][0]["value"] == "native-after-restore")
                .count(),
            1
        );
        for (index, event) in ctx.sent.iter().enumerate() {
            if event["method"] == "Runtime.executionContextCreated" {
                assert!(
                    index > frame,
                    "renderer contexts must follow the exact frame projection"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn native_document_recovery_projects_only_latest_and_rejects_closed_document() {
        let (mut ctx, context, old, owner) = fixture().await;
        let skipped = navigate_native(
            &context,
            old.web_contents(),
            "data:text/html,<title>skipped</title>",
        )
        .await;
        let current = navigate_native(
            &context,
            old.web_contents(),
            "data:text/html,<title>current</title>",
        )
        .await;
        assert!(
            ctx.conn
                .project_browser_document_commit(skipped)
                .await
                .is_empty()
        );
        assert_eq!(
            ctx.conn
                .current_renderer_agent_attachment_for_owner(&owner)
                .unwrap()
                .document(),
            old.id()
        );
        let snapshot = ctx.conn.subscribe_browser_events().unwrap().0;
        assert_eq!(snapshot.documents, [current]);
        let events = ctx
            .conn
            .project_browser_document_commit(snapshot.documents[0])
            .await;
        let events = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["method"] == "Page.frameNavigated")
                .count(),
            1
        );
        assert_eq!(
            ctx.conn
                .current_renderer_agent_attachment_for_owner(&owner)
                .unwrap()
                .document(),
            current.id()
        );
        assert!(
            ctx.conn
                .project_browser_document_commit(current)
                .await
                .is_empty()
        );
        context
            .close_web_contents(current.web_contents())
            .unwrap()
            .close_async()
            .await;
        assert!(
            ctx.conn
                .project_browser_document_commit(current)
                .await
                .is_empty()
        );
        assert!(
            ctx.conn
                .project_browser_document_commit(skipped)
                .await
                .is_empty()
        );
    }
}

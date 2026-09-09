use moli_core::page::{
    DevToolsSessionKey, RendererAgentAttachmentId, RendererRuntimeInspectorMessageBatch,
};

use super::state::{
    CommittedRendererDocumentBinding, DocumentProjectionFence, DocumentProjectionOutputRelease,
    RendererAgentAttachment,
};
use super::{CdpConnection, CommandOwnerScope, NavigationId};

impl CdpConnection {
    pub(crate) fn renderer_agent_attachment_is_current_for_session_owner(
        &self,
        session_id: Option<&str>,
        attachment_id: RendererAgentAttachmentId,
    ) -> bool {
        self.runtime_session_owner_slot(session_id)
            .ok()
            .and_then(|slot| slot.current_renderer_attachment())
            .is_some_and(|attachment| attachment.id() == attachment_id)
    }

    pub(crate) fn current_renderer_agent_attachment_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<RendererAgentAttachmentId> {
        self.current_renderer_agent_attachment_for_owner(owner)
            .map(RendererAgentAttachment::id)
    }

    pub(crate) fn current_renderer_agent_attachment_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<RendererAgentAttachment> {
        self.runtime_session_owner_slot_for_owner(owner)
            .ok()
            .and_then(|slot| slot.current_renderer_attachment())
    }

    pub(crate) fn route_current_renderer_inspector_output_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) -> Vec<RendererRuntimeInspectorMessageBatch> {
        let session_id = owner.session_id();
        let mut batches = self.filter_renderer_inspector_batches_for_target_owner(owner, batches);
        if batches.is_empty() {
            return Vec::new();
        }
        let current_attachment = self
            .runtime_session_owner_slot_for_owner(owner)
            .ok()
            .and_then(|slot| slot.current_renderer_attachment());
        if let Some(current_attachment) = current_attachment {
            // Page-creation facts are frozen before protocol installs the new
            // Page attachment. When their exact Page stream reaches ingress,
            // bind only batches from that same DevTools agent to the now
            // committed attachment. This is a one-time route completion, not
            // a projection-time fallback to whichever Page happens to exist.
            for batch in &mut batches {
                if batch.renderer_agent_attachment_id().is_none()
                    && batch.agent_token == current_attachment.agent_token()
                {
                    batch.bind_renderer_agent_attachment(current_attachment.id());
                }
            }
        }
        let Some(attachment_id) = batches
            .first()
            .and_then(RendererRuntimeInspectorMessageBatch::renderer_agent_attachment_id)
        else {
            tracing::debug!(
                session_id,
                "dropping renderer Inspector output without a source attachment"
            );
            return Vec::new();
        };
        if batches
            .iter()
            .any(|batch| batch.renderer_agent_attachment_id() != Some(attachment_id))
        {
            tracing::debug!(
                session_id,
                "dropping renderer Inspector output spanning multiple attachment leases"
            );
            return Vec::new();
        }
        let state_updates = batches
            .iter()
            .filter_map(|batch| {
                batch
                    .v8_state_update
                    .clone()
                    .map(|state| (batch.session.clone(), state))
            })
            .collect::<Vec<_>>();
        match self
            .runtime_session_owner_slot_mut_for_owner(owner)
            .and_then(|slot| {
                slot.route_current_renderer_inspector_output(attachment_id, batches)
                    .map_err(|error| error.to_string())
            }) {
            Ok(batches) => {
                let primary_session_id =
                    self.runtime_session_owner_primary_session_id_for_owner(owner);
                for (session, state) in state_updates {
                    let state_session_id = match &session {
                        DevToolsSessionKey::Primary => primary_session_id.as_deref(),
                        DevToolsSessionKey::Attached(session_id) => Some(session_id.as_str()),
                    };
                    let state_owner = state_session_id
                        .map(CommandOwnerScope::for_session)
                        .unwrap_or_else(|| owner.clone());
                    let _ = self.merge_v8_inspector_session_state_for_owner(&state_owner, state);
                }
                batches
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    session_id,
                    "dropping renderer Inspector output rejected by the target channel"
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn finish_navigation_without_document_projection_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        token: &NavigationId,
    ) -> Option<DocumentProjectionOutputRelease> {
        if self
            .validate_navigation_target_owner_for_scope(owner, token)
            .is_err()
        {
            return None;
        }
        match self
            .runtime_session_owner_slot_mut_for_owner(owner)
            .and_then(|slot| {
                slot.finish_navigation_without_document_projection(token)
                    .map_err(|error| error.to_string())
            }) {
            Ok(finish) => Some(finish),
            Err(error) => {
                tracing::debug!(
                    %error,
                    session_id = owner.session_id(),
                    navigation_id = token.get(),
                    "renderer channel rejected navigation completion"
                );
                None
            }
        }
    }

    pub(crate) fn publish_document_projection_fence_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        projected: &CommittedRendererDocumentBinding,
        fence: DocumentProjectionFence,
    ) -> DocumentProjectionOutputRelease {
        let current_attachment = self
            .current_renderer_agent_attachment_for_owner(owner)
            .expect("a rebound Document projection must retain its renderer attachment");
        assert_eq!(
            (
                projected.document_id,
                projected.browser_sequence,
                current_attachment.id(),
                current_attachment.document(),
                current_attachment.browser_sequence(),
            ),
            (
                fence.document(),
                fence.browser_sequence(),
                fence.renderer_attachment(),
                fence.document(),
                fence.browser_sequence(),
            ),
            "a Document projection fence requires its exact frame and renderer binding"
        );
        self.runtime_session_owner_slot_mut_for_owner(owner)
            .and_then(|slot| {
                slot.publish_document_projection_fence(fence)
                    .map_err(|error| error.to_string())
            })
            .expect("an exact Document projection fence must publish once")
    }

    fn filter_renderer_inspector_batches_for_target_owner(
        &self,
        owner: &CommandOwnerScope,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) -> Vec<RendererRuntimeInspectorMessageBatch> {
        let owner_identity = self.target_owner_identity_for_owner(owner);
        batches
            .into_iter()
            .filter(|batch| match &batch.session {
                DevToolsSessionKey::Primary => owner_identity.is_some(),
                DevToolsSessionKey::Attached(attached_session_id) => {
                    owner_identity.is_some()
                        && self.target_owner_identity_for_session(Some(attached_session_id))
                            == owner_identity
                }
            })
            .collect()
    }

    fn validate_navigation_target_owner_for_scope(
        &self,
        owner: &CommandOwnerScope,
        token: &NavigationId,
    ) -> Result<(), String> {
        if !self
            .runtime_session_owner_slot_for_owner(owner)?
            .has_renderer_navigation(token)
        {
            return Err("renderer channel navigation target owner mismatch".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        RendererDevToolsAgentToken, RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage,
        V8InspectorSessionState,
    };
    use serde_json::json;

    use super::*;
    use crate::conn::BrowserContext;
    use crate::testing::TestContext;

    #[test]
    fn cancellation_retires_navigation_initial_document_and_resource_state() {
        let mut context = BrowserContext::new("BID-rollback".to_owned());
        context.set_active_target_id("TID-rollback");
        context.begin_active_target_initial_empty_document("about:blank".to_owned());
        let navigation = context
            .start_document_navigation_for_target("TID-rollback", "LOADER-rollback".to_owned())
            .unwrap();
        let cancellation = context
            .document_navigation_cancellation_handle(&navigation)
            .unwrap();
        context
            .active_page_target_mut()
            .owner_state
            .page_resource_store
            .record_main_document_body(
                "TID-rollback".to_owned(),
                "LOADER-rollback".to_owned(),
                "https://example.test/".parse().unwrap(),
                Vec::new(),
                false,
                crate::conn::CapturedBody::from_string("candidate".to_owned()),
            );
        let mut conn = crate::test_support::connection();
        conn.install_browser_context_fixture_for_test(context);
        let owner = CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id: "BID-rollback".to_owned(),
            target_id: "TID-rollback".to_owned(),
            session_key: DevToolsSessionKey::Primary,
        });
        conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &navigation);

        assert!(cancellation.is_cancelled());
        assert!(!conn.accepts_pending_document_navigation_for_owner(&owner, &navigation));
        let target = conn.browser_context.as_ref().unwrap().active_page_target();
        assert!(
            !conn
                .browser_context
                .as_ref()
                .unwrap()
                .target_initial_empty_document_has_pending_cross_document_navigation(
                    target.target_id()
                ),
            "cancellation must retire the initial document's pending state with its navigation"
        );
        assert!(target.owner_state.page_resource_store.is_empty());
    }

    #[test]
    fn navigation_cleanup_uses_exact_identity_for_requests_and_resources() {
        let mut context = BrowserContext::new("BID-cleanup".to_owned());
        context.set_active_target_id("TID-cleanup");
        context.begin_active_target_initial_empty_document("about:blank".to_owned());
        let mut conn = crate::test_support::connection();
        conn.install_browser_context_fixture_for_test(context);
        let owner = CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id: "BID-cleanup".to_owned(),
            target_id: "TID-cleanup".to_owned(),
            session_key: DevToolsSessionKey::Primary,
        });
        let record_candidate_body = |conn: &mut CdpConnection| {
            assert!(
                conn.with_target_owner_state_for_owner_mut(&owner, |state| {
                    state.page_resource_store.record_main_document_body(
                        "TID-cleanup".to_owned(),
                        "LOADER-reused".to_owned(),
                        "https://example.test/".parse().unwrap(),
                        Vec::new(),
                        false,
                        crate::conn::CapturedBody::from_string("candidate".to_owned()),
                    );
                })
                .is_some()
            );
        };
        let first = conn
            .start_document_navigation_for_owner(&owner, "LOADER-reused".to_owned())
            .unwrap();
        record_candidate_body(&mut conn);
        let second = conn
            .start_document_navigation_for_owner(&owner, "LOADER-reused".to_owned())
            .unwrap();
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .active_page_target()
                .owner_state
                .page_resource_store
                .is_empty(),
            "superseding a request must retire its candidate body"
        );
        record_candidate_body(&mut conn);
        let cancellation = conn
            .document_navigation_cancellation_handle(&second)
            .unwrap();

        assert!(!conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &first));
        assert!(!cancellation.is_cancelled());
        assert!(conn.accepts_pending_document_navigation_for_owner(&owner, &second));
        let target = conn.browser_context.as_ref().unwrap().active_page_target();
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .target_initial_empty_document_has_pending_cross_document_navigation(
                    target.target_id()
                )
        );
        assert_eq!(
            target.owner_state.page_resource_store.retained_body_bytes(),
            9
        );

        assert!(conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &second));
        assert!(cancellation.is_cancelled());
        assert!(!conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &second));
        let target = conn.browser_context.as_ref().unwrap().active_page_target();
        assert!(
            !conn
                .browser_context
                .as_ref()
                .unwrap()
                .target_initial_empty_document_has_pending_cross_document_navigation(
                    target.target_id()
                )
        );
        assert!(target.owner_state.page_resource_store.is_empty());

        let committed = conn
            .start_document_navigation_for_owner(&owner, "LOADER-reused".to_owned())
            .unwrap();
        let cancellation = conn
            .document_navigation_cancellation_handle(&committed)
            .unwrap();
        record_candidate_body(&mut conn);
        conn.commit_document_navigation_for_owner_if_matches(&owner, &committed);
        assert!(!conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &committed));
        assert!(!cancellation.is_cancelled());
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .active_page_target()
                .owner_state
                .page_resource_store
                .is_empty()
        );

        record_candidate_body(&mut conn);
        assert!(conn.commit_main_document_resource_for_owner(
            &owner,
            "TID-cleanup".to_owned(),
            "LOADER-reused".to_owned(),
            "https://example.test/".parse().unwrap(),
            Vec::new(),
            false,
            None,
        ));
        assert!(!conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &committed));
        let target = conn.browser_context.as_ref().unwrap().active_page_target();
        assert_eq!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .current_document_loader_id_for_target(target.target_id()),
            Some("LOADER-reused")
        );
        assert_eq!(
            target.owner_state.page_resource_store.retained_body_bytes(),
            9
        );
    }

    #[test]
    fn navigation_identity_cannot_authorize_a_different_target_with_the_same_loader() {
        let mut context = BrowserContext::new("BID-navigation-route".to_owned());
        context.set_active_target_id("TID-first");
        context.register_page_target_url_fixture(
            "TID-second".to_owned(),
            None,
            "about:blank".to_owned(),
        );
        let first = context
            .start_document_navigation_for_target("TID-first", "LOADER-shared".to_owned())
            .unwrap();
        let second = context
            .start_document_navigation_for_target("TID-second", "LOADER-shared".to_owned())
            .unwrap();
        let mut conn = crate::test_support::connection();
        conn.install_browser_context_fixture_for_test(context);
        let owner = CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id: "BID-navigation-route".to_owned(),
            target_id: "TID-second".to_owned(),
            session_key: DevToolsSessionKey::Primary,
        });
        assert!(!conn.accepts_pending_document_navigation_for_owner(&owner, &first));
        conn.commit_document_navigation_for_owner_if_matches(&owner, &first);
        assert!(!conn.clear_pending_document_navigation_for_owner_if_matches(&owner, &first));
        assert!(conn.accepts_pending_document_navigation_for_owner(&owner, &second));
        assert!(
            conn.finish_navigation_without_document_projection_for_owner(&owner, &first)
                .is_none()
        );
        assert!(
            conn.finish_navigation_without_document_projection_for_owner(&owner, &second)
                .is_some()
        );
    }

    fn batch(session: DevToolsSessionKey) -> RendererRuntimeInspectorMessageBatch {
        RendererRuntimeInspectorMessageBatch::new(
            RendererDevToolsAgentToken::allocate(),
            session,
            vec![RendererRuntimeInspectorMessage::protocol(json!({
                "method": "Runtime.consoleAPICalled",
                "params": {},
            }))],
        )
    }

    #[test]
    fn renderer_output_session_filter_rejects_other_target_sessions() {
        let mut browser_context = BrowserContext::new("BID-route".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active-primary".to_owned());
        browser_context.register_page_target_url_fixture(
            "TID-background".to_owned(),
            Some("SID-background-primary".to_owned()),
            "about:blank#background".to_owned(),
        );
        assert!(
            browser_context
                .assign_attached_session_to_target("TID-active", "SID-active-attached".to_owned(),)
        );
        assert!(browser_context.assign_attached_session_to_target(
            "TID-background",
            "SID-background-attached".to_owned(),
        ));
        let mut conn = crate::test_support::connection();
        conn.install_browser_context_fixture_for_test(browser_context);

        let filtered = conn.filter_renderer_inspector_batches_for_target_owner(
            &CommandOwnerScope::for_session("SID-active-primary"),
            vec![
                batch(DevToolsSessionKey::Primary),
                batch(DevToolsSessionKey::Attached(
                    "SID-active-attached".to_owned(),
                )),
                batch(DevToolsSessionKey::Attached(
                    "SID-background-attached".to_owned(),
                )),
                batch(DevToolsSessionKey::Attached("SID-unknown".to_owned())),
            ],
        );

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].session, DevToolsSessionKey::Primary);
        assert_eq!(
            filtered[1].session,
            DevToolsSessionKey::Attached("SID-active-attached".to_owned())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inspector_state_updates_require_the_current_attachment_and_agent() {
        let mut ctx = TestContext::new();
        let mut browser_context = ctx
            .conn
            .new_browser_context_fixture_for_test("BID-state-route");
        browser_context.set_active_target_id("TID-state-route".to_owned());
        browser_context.attach_active_session("SID-state-primary".to_owned());
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-state-route",
                "SID-state-attached".to_owned(),
            )
        );
        ctx.conn
            .install_browser_context_fixture_for_test(browser_context);
        let _ = ctx
            .conn
            .install_navigation_fixture_for_session_owner_for_test(
                "data:text/html,<title>inspector-state-source-validation</title>",
                None,
            )
            .await;
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        let current = browser_context
            .active_page_target()
            .runtime_slot
            .current_renderer_attachment()
            .expect("installed page should have a renderer attachment");

        let accepted_state = V8InspectorSessionState::from_bytes(vec![1, 2, 3]);
        let mut accepted = batch(DevToolsSessionKey::Primary);
        accepted.agent_token = current.agent_token();
        accepted.v8_state_update = Some(accepted_state.clone());
        accepted.bind_renderer_agent_attachment(current.id());
        assert_eq!(
            ctx.conn
                .route_current_renderer_inspector_output_for_owner(
                    &CommandOwnerScope::capture(&ctx.conn, None),
                    vec![accepted],
                )
                .len(),
            1
        );
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .inspector_session_state
                .v8_state,
            Some(accepted_state.clone())
        );

        let attached_state = V8InspectorSessionState::from_bytes(vec![7, 8]);
        let mut attached = batch(DevToolsSessionKey::Attached(
            "SID-state-attached".to_owned(),
        ));
        attached.agent_token = current.agent_token();
        attached.v8_state_update = Some(attached_state.clone());
        attached.bind_renderer_agent_attachment(current.id());
        assert_eq!(
            ctx.conn
                .route_current_renderer_inspector_output_for_owner(
                    &CommandOwnerScope::for_session("SID-state-primary"),
                    vec![attached],
                )
                .len(),
            1
        );
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions
                .attached("SID-state-attached")
                .expect("attached session state")
                .inspector_session_state
                .v8_state,
            Some(attached_state),
            "attached session cookies must remain isolated from the primary session"
        );

        let rejected_state = V8InspectorSessionState::from_bytes(vec![9, 9, 9]);
        let mut stale_attachment = batch(DevToolsSessionKey::Primary);
        stale_attachment.agent_token = current.agent_token();
        stale_attachment.v8_state_update = Some(rejected_state.clone());
        stale_attachment.bind_renderer_agent_attachment(RendererAgentAttachmentId::allocate());
        assert!(
            ctx.conn
                .route_current_renderer_inspector_output_for_owner(
                    &CommandOwnerScope::capture(&ctx.conn, None),
                    vec![stale_attachment],
                )
                .is_empty()
        );

        let mut stale_agent = batch(DevToolsSessionKey::Primary);
        stale_agent.v8_state_update = Some(rejected_state.clone());
        stale_agent.bind_renderer_agent_attachment(current.id());
        assert!(
            ctx.conn
                .route_current_renderer_inspector_output_for_owner(
                    &CommandOwnerScope::capture(&ctx.conn, None),
                    vec![stale_agent],
                )
                .is_empty()
        );

        let route_completed_state = V8InspectorSessionState::from_bytes(vec![4, 4, 4]);
        let mut page_creation_batch = batch(DevToolsSessionKey::Primary);
        page_creation_batch.agent_token = current.agent_token();
        page_creation_batch.v8_state_update = Some(route_completed_state.clone());
        assert_eq!(
            ctx.conn
                .route_current_renderer_inspector_output_for_owner(
                    &CommandOwnerScope::capture(&ctx.conn, None),
                    vec![page_creation_batch],
                )
                .len(),
            1,
            "a page-creation batch may bind its matching agent to the attachment committed after it was frozen"
        );
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .inspector_session_state
                .v8_state,
            Some(route_completed_state)
        );

        let response_state = V8InspectorSessionState::from_bytes(vec![4, 5, 6]);
        let output = RendererRuntimeCommandOutput::from_parts(
            Some(current.id()),
            Some(response_state.clone()),
            Vec::new(),
        );
        let mut ordered_events = Vec::new();
        let owner = CommandOwnerScope::capture(&ctx.conn, None);
        assert!(
            !ctx.conn
                .route_renderer_runtime_command_output_for_owner_into(
                    output,
                    Some(77),
                    &owner,
                    &mut ordered_events,
                ),
            "a validated state-only response must not invent a frontend completion"
        );
        assert!(ordered_events.is_empty());
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .inspector_session_state
                .v8_state,
            Some(response_state),
            "current attachment state must merge even when no pending call matches"
        );
    }
}

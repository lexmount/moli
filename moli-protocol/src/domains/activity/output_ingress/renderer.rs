use crate::conn::{CdpConnection, CommandOwnerScope};

pub(super) fn renderer_owner_action_owner(
    conn: &CdpConnection,
    publication_owner: &CommandOwnerScope,
    renderer_cause: Option<&moli_core::RendererRuntimeCommandCausalIdentity>,
) -> CommandOwnerScope {
    if let Some(cause) = renderer_cause
        && let Some(attachment) = conn
            .target_page_protocol_attachment_identity_for_renderer_inspector_owner(
                publication_owner,
                cause.inspector_session_id(),
            )
    {
        return CommandOwnerScope::for_page_attachment(&attachment);
    }
    if publication_owner.session_id().is_some() {
        return publication_owner.clone();
    }
    let Some((browser_context_id, target_id)) =
        conn.target_owner_identity_for_owner(publication_owner)
    else {
        return publication_owner.clone();
    };
    let Some(target_id) = target_id else {
        return publication_owner.clone();
    };
    conn.target_page_protocol_attachment_identity_for_target(&browser_context_id, &target_id)
        .as_ref()
        .map(CommandOwnerScope::for_page_attachment)
        .unwrap_or_else(|| publication_owner.clone())
}

#[cfg(test)]
mod tests {
    use moli_core::{
        PageId, RendererOutputCursor, RendererOutputItem, RendererOutputRecord,
        RendererOutputStreamIdentity, RendererProtocolObservation,
        RendererRuntimeCommandCausalIdentity,
        page::{
            DevToolsSessionKey, RendererAgentAttachmentId, RendererRuntimeInspectorMessage,
            RendererRuntimeInspectorMessageBatch,
        },
    };
    use moli_page_types::RendererInspectorResponseDelivery;
    use serde_json::json;

    use crate::conn::{
        BrowserContext, CdpConnection, CommandDispatchContext, CommandOwnerScope, ParsedCdpCommand,
        RendererCommandDescriptor,
    };

    use super::renderer_owner_action_owner;
    use crate::conn::{CommandWorkflowContext, ProtocolWorkflowDriver};
    use crate::domains::activity::RendererOutputProjection;
    use crate::domains::activity::publication_route::RendererPublicationProjection;

    async fn project_renderer_output_records_for_owner(
        handle: &crate::conn::ProtocolStateHandle,
        owner: &CommandOwnerScope,
        records: Vec<RendererOutputRecord>,
        cursor: RendererOutputCursor,
        projection: RendererPublicationProjection,
        command_context: &mut CommandDispatchContext,
    ) {
        let (mut context, _scope) = CommandWorkflowContext::take_from(command_context, None, None);
        for record in records {
            let prepared = handle
                .with_state(|state| {
                    state.fixture(|conn| {
                        RendererOutputProjection::prepare_record_for_route(
                            conn,
                            Some(owner.clone()),
                            projection,
                            cursor,
                            record,
                            &mut context,
                        )
                    })
                })
                .unwrap();
            if let Some(prepared) = prepared {
                prepared
                    .run_using(
                        &mut ProtocolWorkflowDriver::new(handle.clone()),
                        &mut context,
                    )
                    .await
                    .expect("retired output projection retains its owner");
            }
        }
        handle
            .with_state(|state| {
                state.fixture(|conn| context.finish_projection(conn, command_context))
            })
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn retiring_page_ingress_preserves_only_correlated_inspector_responses() {
        let mut conn = CdpConnection::new();
        let page = conn
            .load_page_via_runtime_async("data:text/html,<title>replacement</title>")
            .await
            .expect("replacement page should load");
        let mut browser_context = BrowserContext::new("BID-retiring-output".to_owned());
        browser_context.set_active_target_id("TID-retiring-output");
        browser_context.attach_active_session("SID-retiring-output");
        let _ = browser_context
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        conn.install_browser_context_fixture_for_test(browser_context);

        let retired_attachment = RendererAgentAttachmentId::allocate();
        let frontend = ParsedCdpCommand::parse_str(
            r#"{"id":901002,"method":"Runtime.evaluate","sessionId":"SID-retiring-output","params":{"expression":"({ oldDocument: true })"}}"#,
        )
        .expect("frontend command");
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-retiring-output"),
                901_002,
                Some(retired_attachment),
                RendererCommandDescriptor::from_frontend_policy(
                    frontend.json().to_owned(),
                    frontend.renderer_policy(),
                    RendererInspectorResponseDelivery::SessionSink,
                ),
            )
            .expect("pending command on the retired attachment");
        let renderer_call_id = prepared.correlation().renderer_call_id().get();
        drop(prepared);

        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(91));
        let response = |attachment, call_id| {
            let mut batch = RendererRuntimeInspectorMessageBatch::new(
                stream.renderer_agent(),
                DevToolsSessionKey::Primary,
                vec![
                    RendererRuntimeInspectorMessage::protocol(json!({
                        "method": "Runtime.consoleAPICalled",
                        "params": {"type": "log", "args": [], "executionContextId": 1},
                    })),
                    RendererRuntimeInspectorMessage::protocol(json!({
                        "id": call_id,
                        "result": {"result": {"type": "object", "objectId": "retired-object"}},
                    })),
                ],
            );
            batch.bind_renderer_agent_attachment(attachment);
            RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
                RendererProtocolObservation::RuntimeInspector(batch),
            ))
        };
        let owner = CommandOwnerScope::for_session("SID-retiring-output");
        let mut command_context = CommandDispatchContext::default();

        let state_owner = crate::conn::ProtocolStateOwner::new(conn);
        let handle = state_owner.handle();

        project_renderer_output_records_for_owner(
            &handle,
            &owner,
            vec![
                response(RendererAgentAttachmentId::allocate(), renderer_call_id),
                response(retired_attachment, renderer_call_id + 1),
            ],
            RendererOutputCursor::new_for_test(stream, 1),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut command_context,
        )
        .await;
        assert!(command_context.take_protocol_events().is_empty());
        assert!(
            handle
                .with_state(|state| state.fixture(|conn| conn
                    .renderer_runtime_command_cause_for_frontend(
                        Some("SID-retiring-output"),
                        901_002
                    )))
                .unwrap()
                .is_some(),
            "a mismatched attachment or call id must not consume the pending response"
        );

        let terminal = response(retired_attachment, renderer_call_id);
        project_renderer_output_records_for_owner(
            &handle,
            &owner,
            vec![
                RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
                    RendererProtocolObservation::RuntimeLifecycleError {
                        text: "retired document error".to_owned(),
                        execution_context_id: None,
                    },
                )),
                terminal.clone(),
            ],
            RendererOutputCursor::new_for_test(stream, 2),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut command_context,
        )
        .await;
        let messages = command_context
            .take_protocol_events()
            .into_iter()
            .filter_map(crate::conn::BackgroundProtocolEvent::into_wire_message)
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![json!({
                "id": 901_002,
                "sessionId": "SID-retiring-output",
                "result": {"result": {"type": "object", "objectId": "retired-object"}},
            })],
            "the terminal response must survive retirement without stale notifications"
        );
        assert!(
            handle
                .with_state(|state| state.fixture(|conn| conn
                    .renderer_runtime_command_cause_for_frontend(
                        Some("SID-retiring-output"),
                        901_002
                    )))
                .unwrap()
                .is_none(),
            "the exact response must consume the correlation"
        );
        assert_eq!(
            handle
                .with_state(|state| state.fixture(|conn| conn
                    .runtime_remote_object_group_for_session_owner(
                        Some("SID-retiring-output"),
                        "retired-object",
                    )))
                .unwrap(),
            None,
            "retired objects must not be registered on the replacement document"
        );

        project_renderer_output_records_for_owner(
            &handle,
            &owner,
            vec![terminal],
            RendererOutputCursor::new_for_test(stream, 3),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut command_context,
        )
        .await;
        assert!(
            command_context.take_protocol_events().is_empty(),
            "a duplicate terminal response must not be delivered twice"
        );
        // Reuse a frontend id, then cancel the new pending call as session
        // disposal does. Neither the old success nor the cancelled call's
        // late success may deliver a second terminal or consume a future call.
        let attachment = RendererAgentAttachmentId::allocate();
        let prepared = handle
            .with_state(|state| {
                state.fixture(|conn| {
                    conn.try_register_renderer_call_for_session_owner(
                        Some("SID-retiring-output"),
                        901_002,
                        Some(attachment),
                        RendererCommandDescriptor::from_frontend_policy(
                            frontend.json().to_owned(),
                            frontend.renderer_policy(),
                            RendererInspectorResponseDelivery::SessionSink,
                        ),
                    )
                })
            })
            .unwrap()
            .expect("frontend id can be reused after its terminal");
        let cancelled_call_id = prepared.correlation().renderer_call_id().get();
        drop(prepared);
        project_renderer_output_records_for_owner(
            &handle,
            &owner,
            vec![response(retired_attachment, renderer_call_id)],
            RendererOutputCursor::new_for_test(stream, 4),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut command_context,
        )
        .await;
        assert!(command_context.take_protocol_events().is_empty());
        assert!(
            handle
                .with_state(|state| state.fixture(|conn| conn
                    .renderer_runtime_command_cause_for_frontend(
                        Some("SID-retiring-output"),
                        901_002
                    )))
                .unwrap()
                .is_some(),
            "the old terminal must not consume a new call reusing the frontend id"
        );
        let mut cancellations = Vec::new();
        let mut claimed = Vec::new();
        handle
            .with_state(|state| {
                state.fixture(|conn| {
                    conn.fail_pending_inspector_awaits_for_owner_background_events_into(
                        &mut cancellations,
                        &mut claimed,
                        &owner,
                        "Inspector detached",
                    )
                })
            })
            .unwrap();
        let cancellations = cancellations
            .into_iter()
            .chain(claimed)
            .filter_map(crate::conn::BackgroundProtocolEvent::into_wire_message)
            .collect::<Vec<_>>();
        assert_eq!(
            cancellations.len(),
            1,
            "cancellation owes exactly one terminal"
        );
        assert_eq!(cancellations[0]["id"], 901_002);
        assert!(cancellations[0].get("error").is_some());
        project_renderer_output_records_for_owner(
            &handle,
            &owner,
            vec![
                response(retired_attachment, renderer_call_id),
                response(attachment, cancelled_call_id),
            ],
            RendererOutputCursor::new_for_test(stream, 5),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut command_context,
        )
        .await;
        assert!(
            command_context.take_protocol_events().is_empty(),
            "late results after cancellation must be discarded"
        );
        assert!(
            handle
                .with_state(|state| state.fixture(|conn| conn
                    .renderer_runtime_command_cause_for_frontend(
                        Some("SID-retiring-output"),
                        901_002
                    )))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unbound_owner_actions_choose_a_stable_attachment_without_overriding_exact_root_cause() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner-action".to_owned());
        browser_context.set_active_target_id("TID-owner-action".to_owned());
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-owner-action",
                "SID-owner-action".to_owned(),
            )
        );
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);
        let owner = CommandOwnerScope::capture(&conn, None);

        assert_eq!(
            renderer_owner_action_owner(&conn, &owner, None).session_id(),
            Some("SID-owner-action"),
            "an asynchronous target action should use its concrete attachment"
        );
        assert_eq!(
            renderer_owner_action_owner(
                &conn,
                &owner,
                Some(&RendererRuntimeCommandCausalIdentity::new(
                    Some("SID-owner-action".to_owned()),
                    1,
                )),
            )
            .session_id(),
            Some("SID-owner-action"),
        );
        let implicit = renderer_owner_action_owner(
            &conn,
            &owner,
            Some(&RendererRuntimeCommandCausalIdentity::new(None, 2)),
        );
        assert_eq!(
            conn.target_owner_identity_for_owner(&implicit,),
            Some((
                "BID-owner-action".to_owned(),
                Some("TID-owner-action".to_owned()),
            )),
            "an exact implicit-primary command must not be reassigned to a peer session"
        );
    }
}

use super::*;
use moli_core::page::RendererJavaScriptDialogCompletion;

fn dialog_for_test(
    frame_id: Option<&str>,
    dialog_type: &str,
    message: &str,
) -> RendererPendingJavaScriptDialog {
    renderer_dialog_for_test(frame_id, dialog_type, message, "", None)
}

fn enable_page_events_for_fixture(ctx: &TestContext, session_id: &str) {
    // These fixtures inspect nonblocking renderer output. Subscribe to Page
    // events without enabling the renderer's modal wait.
    ctx.with_fixture(|conn| {
        conn.with_target_devtools_session_state_for_session_mut(Some(session_id), |state| {
            state.page_session_state.enable_page_domain(1);
        })
        .expect("Page observer fixture");
    });
}

fn page_owner_for_test(
    ctx: &mut TestContext,
    session_id: Option<&str>,
) -> crate::conn::TargetPageResidenceIdentity {
    ctx.with_fixture(|conn| {
        let runtime_slot = {
            conn.runtime_session_owner_slot_mut(session_id)
                .expect("target session should expose a runtime owner slot")
        };
        if runtime_slot.page_attachment_id().is_none() {
            runtime_slot.replace_page_attachment_id_for_test();
        }
        {
            conn.target_page_residence_identity_for_session(session_id)
                .expect("target session should expose a Page residence")
        }
    })
}

fn push_dialog_for_session(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    dialog: RendererPendingJavaScriptDialog,
) {
    let page_owner = page_owner_for_test(ctx, session_id);
    let source_frame_id = match dialog.source() {
        moli_core::page::RendererJavaScriptDialogSource::ChildFrame { frame_id, .. } => {
            frame_id.clone()
        }
        moli_core::page::RendererJavaScriptDialogSource::RootFrame
        | moli_core::page::RendererJavaScriptDialogSource::LightweightPopup { .. } => ctx
            .with_fixture(|conn| {
                conn.target_session_owner_frame_tree_identity(session_id)
                    .map(|(frame_id, _, _, _)| frame_id)
                    .or_else(|| page_owner.target_id().map(str::to_owned))
                    .expect("test target should expose a root frame")
            }),
    };
    let mut dialog = crate::conn::TargetJavaScriptDialog::new(page_owner, source_frame_id, dialog);
    ctx.with_fixture(|conn| {
        let owner = crate::conn::CommandOwnerScope::capture(conn, session_id);
        dialog.set_page_subscriptions(
            conn.subscribed_page_event_session_ids_for_owner(&owner)
                .iter()
                .filter_map(|session_id| {
                    conn.page_domain_subscription_generation_for_session_owner(
                        session_id.as_deref(),
                    )
                })
                .collect(),
        );
        conn.with_target_javascript_dialog_state_for_owner_mut(&owner, |state| {
            state.push(dialog);
        })
        .expect("target session state should exist")
    });
}

fn shared_dialog_context() -> TestContext {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-primary", "about:blank");
    ctx.with_fixture(|conn| {
        let context = conn.browser_context.as_mut().unwrap();
        assert!(context.assign_attached_session_to_target("TID-1", "SID-attached".to_owned()));
        assert!(context.assign_attached_session_to_target("TID-1", "SID-disabled".to_owned()));
        conn.commit_declared_session_fixtures_for_test();
        for session_id in ["SID-primary", "SID-attached"] {
            assert!(conn.set_page_domain_enabled_for_session_owner(Some(session_id), true));
        }
    });
    ctx
}

async fn assert_last_dialog_subscriber_detach_dismisses(ending: &str, other: &str) {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| assert!(conn.disable_page_domain_for_session_owner(Some(other))));
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some(ending),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "last original subscriber",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.process_async(json!({"id": 201, "method": "Page.enable", "sessionId": "SID-disabled"}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 201)["result"], json!({}));
    ctx.process_async(json!({
        "id": 202, "method": "Target.detachFromTarget", "params": {"sessionId": ending},
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 202)["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_pending_javascript_dialog(),
            "a late Page subscriber cannot handle the abandoned dialog; detach must dismiss it"
        )
    });
    assert!(
        !completion.finish(true, String::new()),
        "detach must settle the renderer completion once"
    );
    assert!(!completion.wait().accepted);
    let closed = ctx
        .take_all()
        .into_iter()
        .filter(|message| message["method"] == "Page.javascriptDialogClosed")
        .collect::<Vec<_>>();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["sessionId"], "SID-disabled");
    assert_eq!(closed[0]["params"]["result"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn last_primary_dialog_subscriber_detach_dismisses_with_late_observer() {
    assert_last_dialog_subscriber_detach_dismisses("SID-primary", "SID-attached").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn last_attached_dialog_subscriber_detach_dismisses_with_late_observer() {
    assert_last_dialog_subscriber_detach_dismisses("SID-attached", "SID-primary").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn page_disable_dismisses_every_dialog_observed_by_that_subscription() {
    let mut ctx = shared_dialog_context();
    let completions = [
        RendererJavaScriptDialogCompletion::pending(),
        RendererJavaScriptDialogCompletion::pending(),
    ];
    for (index, completion) in completions.iter().enumerate() {
        push_dialog_for_session(
            &mut ctx,
            Some("SID-primary"),
            renderer_dialog_for_test(
                Some("TID-1"),
                "confirm",
                &format!("queued dialog {index}"),
                "",
                Some(completion.clone()),
            ),
        );
    }
    ctx.process_async(json!({"id": 203, "method": "Page.disable", "sessionId": "SID-attached"}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 203)["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_pending_javascript_dialog(),
            "Page.disable must cancel the entire observed queue"
        )
    });
    for completion in completions {
        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }
    assert_eq!(
        ctx.take_all()
            .iter()
            .filter(|message| message["method"] == "Page.javascriptDialogClosed")
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_disable_dismisses_observed_tail_without_consuming_unobserved_head() {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(conn.disable_page_domain_for_session_owner(Some("SID-attached")))
    });
    let head = RendererJavaScriptDialogCompletion::pending();
    let tail = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "unobserved head",
            "",
            Some(head.clone()),
        ),
    );
    ctx.process_async(json!({"id": 204, "method": "Page.enable", "sessionId": "SID-attached"}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 204)["result"], json!({}));
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "observed tail",
            "",
            Some(tail.clone()),
        ),
    );
    ctx.process_async(json!({"id": 205, "method": "Page.disable", "sessionId": "SID-attached"}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 205)["result"], json!({}));
    assert!(
        !tail.finish(true, String::new()),
        "disable must cancel its queued callback even when it is not the head"
    );
    assert!(!tail.wait().accepted);
    ctx.process_async(json!({"id": 206, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-primary", "params": {"accept": true}})).await;
    assert_eq!(take_response_by_id(&mut ctx, 206)["result"], json!({}));
    assert!(!head.finish(false, String::new()));
    assert!(head.wait().accepted);
    ctx.with_fixture(|conn| assert!(!conn.has_pending_javascript_dialog()));
}

#[tokio::test(flavor = "multi_thread")]
async fn last_cdp_subscriber_detach_preserves_native_dialog_handler() {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(conn.disable_page_domain_for_session_owner(Some("SID-primary")));
        assert!(conn.register_automation_javascript_dialog_handler(
            FrontendProtocol::WebDriverClassic,
            true,
        ));
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-attached"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "prompt",
            "native handler survives",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.process_async(json!({"id": 207, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-attached"}})).await;
    assert_eq!(take_response_by_id(&mut ctx, 207)["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(conn.has_pending_javascript_dialog());
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .javascript_dialog_handler_enabled()
        );
    });
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::HandleJavaScriptDialog(
            crate::automation::DevToolsHandleJavaScriptDialogCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverClassic,
                    session_id: None,
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                accept: true,
                prompt_text: "native answer".to_owned(),
            },
        ),
    )
    .await
    .expect("native handler retains access to the shared dialog");
    assert!(!completion.finish(false, String::new()));
    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "native answer");
}

#[tokio::test(flavor = "multi_thread")]
async fn bidi_dialog_handler_survives_last_cdp_detach_without_enabling_modal_waits() {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(conn.disable_page_domain_for_session_owner(Some("SID-primary")));
        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false,
            )
        );
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-attached"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "BiDi handler",
            "",
            Some(completion.clone()),
        ),
    );
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: None,
        target_id: Some(DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::GetJavaScriptDialog(
            crate::automation::DevToolsGetJavaScriptDialogCommand {
                context: context.clone(),
            },
        ),
    )
    .await
    .expect("BiDi can observe the prompt without a CDP Page callback");
    ctx.process_async(json!({"id": 210, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-attached"}})).await;
    assert_eq!(take_response_by_id(&mut ctx, 210)["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(conn.has_pending_javascript_dialog());
        assert!(
            !conn
                .browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .javascript_dialog_handler_enabled(),
            "recording a native handler must preserve its existing non-modal mode"
        );
    });
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::HandleJavaScriptDialog(
            crate::automation::DevToolsHandleJavaScriptDialogCommand {
                context,
                accept: true,
                prompt_text: String::new(),
            },
        ),
    )
    .await
    .expect("BiDi retains access after the last CDP subscriber detaches");
    assert!(!completion.finish(false, String::new()));
    assert!(completion.wait().accepted);
}

async fn assert_page_disable_preserves_native_dialog_handler(
    disabling: &str,
    other: &str,
    protocol: FrontendProtocol,
) {
    let mut ctx = shared_dialog_context();
    let native_modal_wait = protocol == FrontendProtocol::WebDriverClassic;
    ctx.with_fixture(|conn| {
        assert!(conn.disable_page_domain_for_session_owner(Some(other)));
        assert!(conn.register_automation_javascript_dialog_handler(protocol, native_modal_wait));
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some(disabling),
        renderer_dialog_for_test(
            Some("TID-1"),
            "prompt",
            "native handler survives Page.disable",
            "",
            Some(completion.clone()),
        ),
    );
    let context = AutomationContext {
        protocol,
        session_id: None,
        target_id: Some(DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::GetJavaScriptDialog(
            crate::automation::DevToolsGetJavaScriptDialogCommand {
                context: context.clone(),
            },
        ),
    )
    .await
    .expect("native automation can observe the shared dialog");

    ctx.process_async(json!({"id": 211, "method": "Page.disable", "sessionId": disabling}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 211)["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(
            conn.has_pending_javascript_dialog(),
            "Page.disable must preserve a dialog with an available native handler"
        );
        assert_eq!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .javascript_dialog_handler_enabled(),
            native_modal_wait,
            "Page.disable must preserve the native frontend's modal waiting mode"
        );
    });
    assert!(
        ctx.take_all()
            .iter()
            .all(|message| message["method"] != "Page.javascriptDialogClosed")
    );
    ctx.process_async(json!({"id": 212, "method": "Page.handleJavaScriptDialog", "sessionId": disabling, "params": {"accept": true}})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 212)["error"]["message"],
        "No dialog is showing",
        "disabling Page must still revoke the CDP session's pending callback"
    );
    ctx.process_async(json!({"id": 213, "method": "Page.enable", "sessionId": disabling}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 213)["result"], json!({}));
    ctx.process_async(json!({"id": 214, "method": "Page.handleJavaScriptDialog", "sessionId": disabling, "params": {"accept": true}})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 214)["error"]["message"],
        "No dialog is showing",
        "re-enabling Page must not restore the old pending callback"
    );

    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::HandleJavaScriptDialog(
            crate::automation::DevToolsHandleJavaScriptDialogCommand {
                context,
                accept: true,
                prompt_text: "native answer".to_owned(),
            },
        ),
    )
    .await
    .expect("native automation retains the dialog after Page.disable");
    assert!(!completion.finish(false, String::new()));
    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "native answer");
    ctx.with_fixture(|conn| assert!(!conn.has_pending_javascript_dialog()));
}

#[tokio::test(flavor = "multi_thread")]
async fn primary_page_disable_preserves_native_classic_dialog_handler() {
    assert_page_disable_preserves_native_dialog_handler(
        "SID-primary",
        "SID-attached",
        FrontendProtocol::WebDriverClassic,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_page_disable_preserves_native_classic_dialog_handler() {
    assert_page_disable_preserves_native_dialog_handler(
        "SID-attached",
        "SID-primary",
        FrontendProtocol::WebDriverClassic,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn primary_page_disable_preserves_native_bidi_dialog_handler() {
    assert_page_disable_preserves_native_dialog_handler(
        "SID-primary",
        "SID-attached",
        FrontendProtocol::WebDriverBidi,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_page_disable_preserves_native_bidi_dialog_handler() {
    assert_page_disable_preserves_native_dialog_handler(
        "SID-attached",
        "SID-primary",
        FrontendProtocol::WebDriverBidi,
    )
    .await;
}

async fn assert_native_dialog_handler_release_preserves_peer(
    releasing: FrontendProtocol,
    remaining: FrontendProtocol,
) {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(conn.register_automation_javascript_dialog_handler(
            FrontendProtocol::WebDriverClassic,
            true
        ));
        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false
            )
        );
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            None,
            "prompt",
            "retained native prompt",
            "",
            Some(completion.clone()),
        ),
    );
    for (id, session) in [(220, "SID-primary"), (221, "SID-attached")] {
        ctx.process_async(json!({"id": id, "method": "Page.disable", "sessionId": session}))
            .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    ctx.with_fixture(|conn| {
        assert!(
            conn.unregister_automation_javascript_dialog_handler(releasing)
                .is_empty()
        );
        assert!(conn.has_pending_javascript_dialog());
        assert!(conn.has_automation_javascript_dialog_handler());
        assert_eq!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .javascript_dialog_handler_enabled(),
            remaining == FrontendProtocol::WebDriverClassic,
        );
    });
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::HandleJavaScriptDialog(
            crate::automation::DevToolsHandleJavaScriptDialogCommand {
                context: AutomationContext {
                    protocol: remaining,
                    session_id: None,
                    target_id: Some("TID-1".into()),
                    browser_context_id: None,
                },
                accept: true,
                prompt_text: "peer answer".to_owned(),
            },
        ),
    )
    .await
    .expect("the remaining native adapter handles the prompt");
    assert!(!completion.finish(false, String::new()));
    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "peer answer");
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_bidi_handler_preserves_classic_handler_and_modal_waits() {
    assert_native_dialog_handler_release_preserves_peer(
        FrontendProtocol::WebDriverBidi,
        FrontendProtocol::WebDriverClassic,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_classic_handler_preserves_non_modal_bidi_handler() {
    assert_native_dialog_handler_release_preserves_peer(
        FrontendProtocol::WebDriverClassic,
        FrontendProtocol::WebDriverBidi,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_last_native_handler_settles_every_abandoned_dialog() {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false
            )
        );
    });
    let completions = [
        RendererJavaScriptDialogCompletion::pending(),
        RendererJavaScriptDialogCompletion::pending(),
    ];
    for completion in &completions {
        push_dialog_for_session(
            &mut ctx,
            Some("SID-primary"),
            renderer_dialog_for_test(
                None,
                "confirm",
                "abandoned native prompt",
                "",
                Some(completion.clone()),
            ),
        );
    }
    for (id, session) in [(222, "SID-primary"), (223, "SID-attached")] {
        ctx.process_async(json!({"id": id, "method": "Page.disable", "sessionId": session}))
            .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    let events = ctx.with_fixture(|conn| {
        let events =
            conn.unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi);
        assert!(!conn.has_automation_javascript_dialog_handler());
        assert!(!conn.has_pending_javascript_dialog());
        assert!(
            conn.unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi)
                .is_empty()
        );
        events
    });
    assert_eq!(events.len(), 2);
    for event in events {
        assert!(!event.has_protocol_wire_message());
        assert!(
            matches!(event.into_parts().1, Some(AutomationEvent::UserPromptClosed(closed)) if !closed.accepted)
        );
    }
    for completion in completions {
        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_last_native_handler_preserves_original_page_subscribers() {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false
            )
        );
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            None,
            "confirm",
            "original CDP observer",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.with_fixture(|conn| {
        assert!(
            conn.unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi)
                .is_empty()
        );
        assert!(!conn.has_automation_javascript_dialog_handler());
        assert!(conn.has_pending_javascript_dialog());
    });
    ctx.process_async(json!({"id": 224, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-attached", "params": {"accept": true}})).await;
    assert_eq!(take_response_by_id(&mut ctx, 224)["result"], json!({}));
    assert!(!completion.finish(false, String::new()));
    assert!(completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn dialog_broadcast_emits_one_native_event_per_physical_dialog() {
    let mut ctx = shared_dialog_context();
    page_owner_for_test(&mut ctx, Some("SID-primary"));
    let dialog = ctx.with_fixture(|conn| {
        crate::conn::TargetPreparedJavaScriptDialog::capture(
            conn.target_page_protocol_attachment_identity_for_session(Some("SID-primary"))
                .unwrap(),
            conn.runtime_session_owner_slot(Some("SID-primary"))
                .unwrap()
                .javascript_dialog_scope_observer(),
            "TID-1",
            dialog_for_test(None, "confirm", "one physical dialog"),
        )
    });
    let mut opening = Vec::new();
    ctx.with_fixture(|conn| {
        crate::domains::page::javascript_dialog::emit_prepared(conn, &mut opening, vec![dialog])
    });
    let native_openings = opening
        .iter()
        .filter(|event| {
            matches!(
                (*event).clone().into_parts().1,
                Some(AutomationEvent::PageJavaScriptDialogOpening(_))
            )
        })
        .count();
    assert_eq!(
        native_openings, 1,
        "CDP fan-out must not duplicate native opening events"
    );
    assert_eq!(
        opening
            .iter()
            .filter(|event| (*event).clone().into_wire_message().is_some())
            .count(),
        2
    );
    let (result, _, closing) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::HandleJavaScriptDialog(
            crate::automation::DevToolsHandleJavaScriptDialogCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverClassic,
                    session_id: None,
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                accept: true,
                prompt_text: String::new(),
            },
        ))
        .await
        .into_parts_with_protocol_events();
    result.expect("shared dialog should close");
    assert_eq!(
        closing
            .iter()
            .filter(|event| matches!(
                (*event).clone().into_parts().1,
                Some(AutomationEvent::UserPromptClosed(_))
            ))
            .count(),
        1,
        "CDP fan-out must not duplicate native closing events"
    );
    assert_eq!(
        closing
            .iter()
            .filter(|event| (*event).clone().into_wire_message().is_some())
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_dialog_arriving_after_last_page_disable_is_dismissed() {
    let mut ctx = shared_dialog_context();
    page_owner_for_test(&mut ctx, Some("SID-primary"));
    let completion = RendererJavaScriptDialogCompletion::pending();
    let dialog = ctx.with_fixture(|conn| {
        crate::conn::TargetPreparedJavaScriptDialog::capture(
            conn.target_page_protocol_attachment_identity_for_session(Some("SID-primary"))
                .unwrap(),
            conn.runtime_session_owner_slot(Some("SID-primary"))
                .unwrap()
                .javascript_dialog_scope_observer(),
            "TID-1",
            renderer_dialog_for_test(
                None,
                "confirm",
                "delayed renderer prefix",
                "",
                Some(completion.clone()),
            ),
        )
    });
    for (id, session) in [(208, "SID-primary"), (209, "SID-attached")] {
        ctx.process_async(json!({"id": id, "method": "Page.disable", "sessionId": session}))
            .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    let mut events = Vec::new();
    ctx.with_fixture(|conn| {
        crate::domains::page::javascript_dialog::emit_prepared(conn, &mut events, vec![dialog]);
        assert!(!conn.has_pending_javascript_dialog());
    });
    assert!(
        !completion.finish(true, String::new()),
        "a delayed prefix must not install an orphaned renderer wait"
    );
    assert!(!completion.wait().accepted);
    assert!(
        events
            .iter()
            .all(|event| !event.has_protocol_wire_message())
    );
    let native = events
        .into_iter()
        .filter_map(|event| event.into_parts().1)
        .collect::<Vec<_>>();
    assert!(
        matches!(native.as_slice(), [AutomationEvent::PageJavaScriptDialogOpening(_), AutomationEvent::UserPromptClosed(closed)] if !closed.accepted)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn primary_detach_preserves_shared_javascript_dialog_and_completion() {
    let mut ctx = shared_dialog_context();
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-attached"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "survive detach",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context
                .as_mut()
                .unwrap()
                .remove_page_session_binding(
                    "TID-1",
                    "SID-primary",
                    &moli_page_types::DevToolsSessionKey::Primary,
                )
        );
        assert!(
            conn.has_pending_javascript_dialog(),
            "detaching Primary must retain the Page-owned dialog"
        );
    });
    ctx.process_async(json!({
        "id": 101, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-attached",
        "params": {"accept": true}
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 101)["result"], json!({}));
    assert!(
        !completion.finish(false, String::new()),
        "the surviving session must settle the completion exactly once"
    );
    assert!(completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_cannot_be_handled_without_page_enable() {
    let mut ctx = shared_dialog_context();
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "enabled sessions only",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.process_async(json!({
        "id": 102, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-disabled",
        "params": {"accept": true}
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 102);
    assert_eq!(response["error"]["message"], json!("No dialog is showing"));
    ctx.with_fixture(|conn| assert!(conn.has_pending_javascript_dialog()));
    ctx.process_async(json!({
        "id": 103, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-attached",
        "params": {"accept": true}
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 103)["result"], json!({}));
    assert!(!completion.finish(false, String::new()));
    assert!(completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn late_page_enable_cannot_handle_or_cancel_an_existing_javascript_dialog() {
    let mut ctx = shared_dialog_context();
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "original subscribers only",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.process_async(json!({
        "id": 106, "method": "Page.enable", "sessionId": "SID-disabled",
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 106)["result"], json!({}));
    ctx.process_async(json!({
        "id": 107, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-disabled",
        "params": {"accept": true}
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 107)["error"]["message"],
        json!("No dialog is showing")
    );
    ctx.process_async(json!({
        "id": 108, "method": "Page.disable", "sessionId": "SID-disabled",
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 108)["result"], json!({}));
    ctx.with_fixture(|conn| assert!(conn.has_pending_javascript_dialog()));
    ctx.process_async(json!({
        "id": 109, "method": "Page.handleJavaScriptDialog", "sessionId": "SID-attached",
        "params": {"accept": true}
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 109)["result"], json!({}));
    assert!(!completion.finish(false, String::new()));
    assert!(completion.wait().accepted);
}

async fn assert_page_disable_dismisses_shared_dialog(disabling: &str, observing: &str) {
    let mut ctx = shared_dialog_context();
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "cancel on disable",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.process_async(json!({
        "id": 104, "method": "Page.disable", "sessionId": disabling,
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 104)["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_pending_javascript_dialog(),
            "Page.disable must dismiss the shared dialog from either session"
        )
    });
    assert!(
        !completion.finish(true, String::new()),
        "Page.disable must settle the renderer completion"
    );
    assert!(!completion.wait().accepted);
    let closed = ctx
        .take_all()
        .into_iter()
        .filter(|message| message["method"] == "Page.javascriptDialogClosed")
        .collect::<Vec<_>>();
    assert_eq!(
        closed.len(),
        1,
        "only the remaining Page subscriber should receive the close event: {closed:?}"
    );
    assert_eq!(closed[0]["sessionId"], json!(observing));
    assert_eq!(closed[0]["params"]["result"], json!(false));
    ctx.process_async(json!({
        "id": 105, "method": "Page.handleJavaScriptDialog", "sessionId": observing,
        "params": {"accept": true}
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 105)["error"]["message"],
        json!("No dialog is showing")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn primary_page_disable_dismisses_shared_javascript_dialog() {
    assert_page_disable_dismisses_shared_dialog("SID-primary", "SID-attached").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_page_disable_dismisses_shared_javascript_dialog() {
    assert_page_disable_dismisses_shared_dialog("SID-attached", "SID-primary").await;
}

#[test]
fn dropping_page_target_dismisses_installed_javascript_dialog() {
    let mut ctx = shared_dialog_context();
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-attached"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "prompt",
            "target closes",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.with_fixture(|conn| {
        let target = conn
            .browser_context
            .as_mut()
            .unwrap()
            .take_page_target_for_close("TID-1")
            .expect("live Page target");
        drop(target);
    });
    assert!(
        !completion.finish(true, String::new()),
        "dropping a Page target must settle its installed completion"
    );
    assert!(!completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_last_page_subscriber_dismisses_without_cdp_close_event() {
    let mut ctx = shared_dialog_context();
    ctx.with_fixture(|conn| {
        assert!(conn.disable_page_domain_for_session_owner(Some("SID-attached")))
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-primary"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "confirm",
            "last observer",
            "",
            Some(completion.clone()),
        ),
    );
    ctx.process_async(json!({"id": 110, "method": "Page.disable", "sessionId": "SID-primary"}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 110)["result"], json!({}));
    assert!(!completion.finish(true, String::new()));
    assert!(!completion.wait().accepted);
    assert!(
        ctx.take_all()
            .iter()
            .all(|message| message["method"] != "Page.javascriptDialogClosed")
    );
}

#[test]
fn retiring_page_scope_and_clearing_dialog_state_dismisses_installed_dialog() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-dialog-clear",
        "TID-dialog-clear",
        "SID-dialog-clear",
        "about:blank",
    );
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-dialog-clear"),
        renderer_dialog_for_test(
            Some("FRAME-dialog-clear"),
            "prompt",
            "retire me",
            "default",
            Some(completion.clone()),
        ),
    );
    let retired_scope = ctx.with_fixture(|conn| {
        conn.runtime_session_owner_slot(Some("SID-dialog-clear"))
            .expect("target Page runtime slot")
            .javascript_dialog_scope_observer()
    });

    ctx.with_fixture(|conn| {
        conn.runtime_session_owner_slot_mut(Some("SID-dialog-clear"))
            .expect("target Page runtime slot")
            .retire_javascript_dialog_scope()
    });

    ctx.with_fixture(|conn| {
        conn.with_target_javascript_dialog_state_for_owner_mut(
            &crate::conn::CommandOwnerScope::capture(conn, Some("SID-dialog-clear")),
            |state| state.clear(),
        )
        .expect("target page session state should clear")
    });

    ctx.with_fixture(|conn| {
        let state = &conn
            .target_javascript_dialog_state_for_owner(&crate::conn::CommandOwnerScope::capture(
                conn,
                Some("SID-dialog-clear"),
            ))
            .expect("target page session state");
        assert!(state.is_empty());
    });
    ctx.with_fixture(|conn| {
        assert!(!{
            conn.runtime_session_owner_slot(Some("SID-dialog-clear"))
                .expect("target Page runtime slot")
                .observes_javascript_dialog_scope(&retired_scope)
        })
    });
    assert!(
        !completion.finish(true, "late input".to_owned()),
        "clearing the exact dialog residence must settle its completion once"
    );
    let result = completion.wait();
    assert!(!result.accepted);
    assert!(result.user_input.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_javascript_dialogs_are_preserved_per_background_target() {
    let context = |target_id: &str| AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: None,
        target_id: Some(DevToolsTargetId::from(target_id)),
        browser_context_id: None,
    };

    fn dialog(target_id: &str, message: &str) -> crate::conn::TargetJavaScriptDialog {
        target_dialog_for_test(
            crate::conn::TargetPageResidenceIdentity::new_for_test(
                "BID-1".to_owned(),
                Some(target_id.to_owned()),
                1,
            ),
            target_id,
            "alert",
            message,
            "",
            None,
        )
    }

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-A", "SID-A", "about:blank");

    {
        ctx.with_fixture(|conn| {
            let browser_context = conn.browser_context.as_mut().unwrap();
            browser_context
                .active_page_target_mut()
                .javascript_dialog_state
                .push(dialog("TID-A", "a"));
            browser_context.insert_page_target_host(PageTargetHost::new(
                "TID-B".to_owned(),
                Some("SID-B".to_owned()),
                crate::conn::TargetIdentityState::new(
                    "about:blank".to_owned(),
                    URL_BASE.to_owned(),
                    "Secure".to_owned(),
                ),
                crate::conn::TargetPageSlot::empty_for_test_fixture(),
            ));
            assert!(
                browser_context
                    .start_select_page_target("TID-B")
                    .expect("fixture target should exist")
                    .is_empty()
            );
            assert!(
                browser_context
                    .active_page_target()
                    .javascript_dialog_state
                    .is_empty()
            );
        });
    }
    ctx.with_fixture(|conn| assert!(conn.has_pending_javascript_dialog()));
    ctx.with_fixture(|conn| {
        assert!({ conn.has_pending_javascript_dialog_for_devtools_context(&context("TID-A")) })
    });
    ctx.with_fixture(|conn| {
        assert!(!{ conn.has_pending_javascript_dialog_for_devtools_context(&context("TID-B")) })
    });
    ctx.with_fixture(|conn| {
        assert!(!{
            conn.has_pending_javascript_dialog_for_devtools_context(&context("TID-missing"))
        })
    });
    {
        ctx.with_fixture(|conn| {
            let browser_context = conn.browser_context.as_mut().unwrap();
            browser_context
                .active_page_target_mut()
                .javascript_dialog_state
                .push(dialog("TID-B", "b"));
        });
    }

    {
        ctx.with_fixture(|conn| {
            let browser_context = conn.browser_context.as_mut().unwrap();
            assert!(
                browser_context
                    .start_select_page_target("TID-A")
                    .expect("fixture target should exist")
                    .is_empty()
            );
            assert_eq!(
                browser_context
                    .active_page_target()
                    .javascript_dialog_state
                    .pending_dialogs(),
                &[dialog("TID-A", "a")]
            );
        });
    }

    {
        ctx.with_fixture(|conn| {
            let browser_context = conn.browser_context.as_mut().unwrap();
            assert!(
                browser_context
                    .start_select_page_target("TID-B")
                    .expect("fixture target should exist")
                    .is_empty()
            );
            assert_eq!(
                browser_context
                    .active_page_target()
                    .javascript_dialog_state
                    .pending_dialogs(),
                &[dialog("TID-B", "b")]
            );
        });
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_round_trip_through_page_domain() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    enable_page_events_for_fixture(&ctx, "SID-1");
    let page = ctx
        .build_fixture_page("data:text/html,<button id='b'>alert</button>")
        .await
        .expect("page should load");
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .active_page_target_mut()
            .runtime_slot
            .set_loaded_page_for_test(page)
    });

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "alert('smoke alert'); 'done'",
            "returnByValue": true
        }
    }))
    .await;

    let opening = ctx
        .take_all()
        .into_iter()
        .find(|message| message["method"] == json!("Page.javascriptDialogOpening"))
        .expect("alert should emit Page.javascriptDialogOpening");
    assert_eq!(opening["sessionId"], json!("SID-1"));
    assert_eq!(opening["params"]["type"], json!("alert"));
    assert_eq!(opening["params"]["message"], json!("smoke alert"));
    assert_eq!(opening["params"]["hasBrowserHandler"], json!(false));
    assert_eq!(opening["params"]["frameId"], json!("TID-1"));

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-1",
        "params": { "accept": true }
    }))
    .await;
    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
                && message["params"]["frameId"] == json!("TID-1")
        }),
        "handleJavaScriptDialog should emit a closed event: {sent:?}"
    );
    assert!(
        sent.iter()
            .any(|message| message["id"] == json!(3) && message["result"] == json!({})),
        "handleJavaScriptDialog should resolve: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_preserves_dialog_order_and_retires_replaced_document_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-dialog-open",
        "TID-dialog-open",
        "SID-dialog-open",
        "about:blank",
    );
    enable_page_events_for_fixture(&ctx, "SID-dialog-open");
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://dialog-replacement.example/").unwrap(),
        "<!doctype html><body>dialog replacement</body>".into(),
        Some("SID-dialog-open"),
    )
    .await;

    ctx.process_async(json!({
        "id": 39,
        "method": "Runtime.evaluate",
        "sessionId": "SID-dialog-open",
        "params": {
            "expression": "alert('retired dialog'); document.open(); document.write('<main>replacement</main>'); document.close(); 'done'",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    let opening_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("retired dialog")
        })
        .expect("the dialog produced before document.open() must remain observable");
    let response_index = sent
        .iter()
        .position(|message| message["id"] == json!(39))
        .expect("Runtime.evaluate response");
    assert!(
        opening_index < response_index,
        "concrete dialog output must retain its source order before the command response: {sent:?}"
    );
    ctx.with_fixture(|conn| {
        assert!(
            conn.target_javascript_dialog_state_for_owner(
                &crate::conn::CommandOwnerScope::capture(conn, Some("SID-dialog-open")),
            )
            .expect("replacement target page session state")
            .is_empty(),
            "the replacement Document must retire the already-emitted dialog state"
        )
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn get_javascript_dialog_text_peeks_without_closing_dialog() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page_owner = page_owner_for_test(&mut ctx, Some("SID-1"));
    ctx.with_fixture(|conn| {
        conn.with_target_javascript_dialog_state_for_owner_mut(
            &crate::conn::CommandOwnerScope::capture(conn, Some("SID-1")),
            |state| {
                state.push(target_dialog_for_test(
                    page_owner,
                    "TID-1",
                    "alert",
                    "classic alert",
                    "",
                    None,
                ));
            },
        )
        .expect("target session state should exist")
    });

    let context = crate::automation::AutomationContext {
        protocol: crate::automation::FrontendProtocol::WebDriverClassic,
        session_id: Some(crate::automation::DevToolsSessionId::from("SID-1")),
        target_id: Some(crate::automation::DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            crate::automation::AutomationCommand::GetJavaScriptDialog(
                crate::automation::DevToolsGetJavaScriptDialogCommand {
                    context: context.clone(),
                },
            ),
        )
        .await;
    let crate::automation::AutomationResult::JavaScriptDialog(result) =
        result.expect("get dialog text should resolve")
    else {
        panic!("get dialog text should return dialog result");
    };
    assert_eq!(result.message, "classic alert");
    assert_eq!(result.dialog_type, "alert");

    let (result, _, events) = ctx
        .execute_owned_automation_command_for_test(
            crate::automation::AutomationCommand::HandleJavaScriptDialog(
                crate::automation::DevToolsHandleJavaScriptDialogCommand {
                    context,
                    accept: true,
                    prompt_text: String::new(),
                },
            ),
        )
        .await
        .into_parts_with_protocol_events();
    assert!(matches!(
        result.expect("handle dialog should resolve"),
        crate::automation::AutomationResult::Empty
    ));
    let closed_event = events
        .into_iter()
        .find(|event| {
            matches!(
                event.clone().into_parts().1,
                Some(AutomationEvent::UserPromptClosed(_))
            )
        })
        .expect("native automation must observe the dialog closing without Page.enable");
    assert!(
        closed_event.protocol_message().is_none(),
        "closed dialog event should stay typed until wire projection"
    );
    assert!(
        closed_event.clone().into_wire_message().is_none(),
        "a disabled Page session must not receive CDP dialog events"
    );
    let (_, Some(AutomationEvent::UserPromptClosed(closed))) = closed_event.into_parts() else {
        panic!("typed native close event");
    };
    assert!(closed.accepted);
    assert_eq!(closed.frame_id.as_str(), "TID-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_javascript_dialog_prompt_text_is_shared_across_attached_sessions() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .assign_attached_session_to_target("TID-1", "SID-2".to_owned())
        );
        conn.commit_declared_session_fixtures_for_test();
    });
    let page_owner = page_owner_for_test(&mut ctx, Some("SID-2"));
    ctx.with_fixture(|conn| {
        conn.with_target_javascript_dialog_state_for_owner_mut(
            &crate::conn::CommandOwnerScope::capture(conn, Some("SID-1")),
            |state| {
                state.push(target_dialog_for_test(
                    page_owner, "TID-1", "prompt", "prompt?", "", None,
                ));
            },
        )
        .expect("target session state should exist")
    });

    let context = crate::automation::AutomationContext {
        protocol: crate::automation::FrontendProtocol::WebDriverClassic,
        session_id: Some(crate::automation::DevToolsSessionId::from("SID-1")),
        target_id: Some(crate::automation::DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    let get_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            crate::automation::AutomationCommand::GetJavaScriptDialog(
                crate::automation::DevToolsGetJavaScriptDialogCommand {
                    context: context.clone(),
                },
            ),
        )
        .await;
    let crate::automation::AutomationResult::JavaScriptDialog(dialog) =
        get_result.expect("other attached session should read shared prompt")
    else {
        panic!("get dialog should return shared prompt details");
    };
    assert_eq!(dialog.dialog_type, "prompt");
    assert_eq!(dialog.message, "prompt?");

    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            crate::automation::AutomationCommand::SetJavaScriptDialogPromptText(
                crate::automation::DevToolsSetJavaScriptDialogPromptTextCommand {
                    context: context.clone(),
                    prompt_text: "cheese".to_owned(),
                },
            ),
        )
        .await;
    assert!(matches!(
        result.expect("set prompt text should resolve"),
        crate::automation::AutomationResult::Empty
    ));

    let attached_context = crate::automation::AutomationContext {
        protocol: crate::automation::FrontendProtocol::WebDriverClassic,
        session_id: Some(crate::automation::DevToolsSessionId::from("SID-2")),
        target_id: Some(crate::automation::DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    let (result, _, events) = ctx
        .execute_owned_automation_command_for_test(
            crate::automation::AutomationCommand::HandleJavaScriptDialog(
                crate::automation::DevToolsHandleJavaScriptDialogCommand {
                    context: attached_context,
                    accept: true,
                    prompt_text: String::new(),
                },
            ),
        )
        .await
        .into_parts_with_protocol_events();
    assert!(matches!(
        result.expect("accept prompt should resolve"),
        crate::automation::AutomationResult::Empty
    ));
    let events = events
        .into_iter()
        .filter_map(|event| event.into_parts().1)
        .collect::<Vec<_>>();
    assert!(
        events.iter().any(
            |event| matches!(event, AutomationEvent::UserPromptClosed(closed)
            if closed.accepted && closed.user_text == "cheese")
        ),
        "stored prompt text should be consumed by native accept: {events:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn handle_javascript_dialog_finishes_renderer_completion() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.with_fixture(|conn| {
        assert!(conn.set_page_domain_enabled_for_session_owner(Some("SID-1"), true))
    });
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-1"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "prompt",
            "prompt?",
            "default",
            Some(completion.clone()),
        ),
    );

    ctx.process_async(json!({
        "id": 33,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-1",
        "params": { "accept": true, "promptText": "typed" }
    }))
    .await;

    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "typed");
    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
                && message["params"]["userInput"] == json!("typed")
        }),
        "handle should close protocol dialog while resuming renderer completion: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn handle_javascript_dialog_rejects_when_no_dialog_is_showing() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 34,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-1",
        "params": { "accept": true }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["id"] == json!(34)
                && message["error"]["code"] == json!(-32602)
                && message["error"]["message"] == json!("No dialog is showing")
        }),
        "handleJavaScriptDialog without a pending dialog should reject like Chromium: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Page.javascriptDialogClosed")),
        "no-dialog handling must not emit a closed event: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_pending_state_is_shared_across_active_attached_sessions() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-dialog-attached",
        "TID-dialog-attached",
        "SID-primary",
        "about:blank",
    );
    ctx.with_fixture(|conn| {
        let browser_context = conn.browser_context.as_mut().unwrap();
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-dialog-attached",
                "SID-attached".to_owned()
            )
        );
    });
    ctx.with_fixture(|conn| conn.commit_declared_session_fixtures_for_test());
    ctx.with_fixture(|conn| {
        for session_id in ["SID-primary", "SID-attached"] {
            assert!(conn.set_page_domain_enabled_for_session_owner(Some(session_id), true));
        }
    });
    push_dialog_for_session(
        &mut ctx,
        Some("SID-attached"),
        dialog_for_test(Some("TID-dialog-attached"), "alert", "aux dialog"),
    );
    // A dialog opened from one attachment is stored once for the Page
    // residence, so an unrelated attachment with the Page domain enabled sees
    // and can resolve the same dialog (Chromium behavior; see #2063).
    ctx.with_fixture(|conn| {
        assert!(
            !conn
                .target_javascript_dialog_state_for_owner(&crate::conn::CommandOwnerScope::capture(
                    conn,
                    Some("SID-primary")
                ),)
                .expect("primary page session state")
                .is_empty(),
            "every attached session observes the Page's canonical pending dialog"
        )
    });
    let context = AutomationContext {
        protocol: FrontendProtocol::Cdp,
        session_id: Some(crate::automation::DevToolsSessionId::from("SID-primary")),
        target_id: Some(DevToolsTargetId::from("TID-dialog-attached")),
        browser_context_id: None,
    };
    ctx.with_fixture(|conn| {
        assert!(
            conn.has_pending_javascript_dialog_for_devtools_context(&context),
            "a dialog owned by another DevTools attachment still blocks the same Page"
        )
    });

    // Any attached session may resolve the dialog. Handling it from the session
    // that did *not* open it must succeed.
    ctx.process_async(json!({
        "id": 36,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-primary",
        "params": { "accept": true }
    }))
    .await;
    let primary = take_response_by_id(&mut ctx, 36);
    assert_eq!(primary["sessionId"], json!("SID-primary"));
    assert_eq!(primary["result"], json!({}));
    let closed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Page.javascriptDialogClosed"))
        .expect("a cross-session handle should emit a closed event");
    assert_eq!(closed["sessionId"], json!("SID-primary"));
    assert_eq!(closed["params"]["frameId"], json!("TID-dialog-attached"));
    assert_eq!(closed["params"]["result"], json!(true));

    // The one-shot renderer completion is consumed exactly once: a second
    // handle from the other session finds no dialog.
    ctx.process_async(json!({
        "id": 37,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-attached",
        "params": { "accept": true }
    }))
    .await;
    let attached = take_response_by_id(&mut ctx, 37);
    assert_eq!(attached["sessionId"], json!("SID-attached"));
    assert_eq!(attached["error"]["code"], json!(-32602));
    assert_eq!(attached["error"]["message"], json!("No dialog is showing"));
    ctx.with_fixture(|conn| {
        assert!(
            conn.target_javascript_dialog_state_for_owner(
                &crate::conn::CommandOwnerScope::capture(conn, Some("SID-attached")),
            )
            .expect("canonical page session state")
            .is_empty(),
            "resolving the shared dialog clears it exactly once"
        )
    });
    ctx.with_fixture(|conn| {
        assert!(!conn.has_pending_javascript_dialog_for_devtools_context(&context))
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn handle_javascript_dialog_rejects_dialog_without_current_page_residence() {
    let mut ctx = TestContext::new();
    {
        let fixture_value = Some(BrowserContext::new_with_page_for_test(
            "BID-dialog-missing-frame",
            "TID-dialog-missing-frame",
        ));
        ctx.with_fixture(|conn| {
            conn.browser_context = fixture_value;
        });
    }
    ctx.with_fixture(|conn| {
        assert!(conn.set_page_domain_enabled_for_session_owner(None, true));
        let generation = conn
            .page_domain_subscription_generation_for_session_owner(None)
            .unwrap();
        let mut dialog = target_dialog_for_test(
            crate::conn::TargetPageResidenceIdentity::new_for_test(
                "BID-dialog-missing-frame".to_owned(),
                Some("retired-target".to_owned()),
                1,
            ),
            "retired-target",
            "alert",
            "stale dialog",
            "",
            None,
        );
        dialog.set_page_subscriptions(vec![generation]);
        conn.with_target_javascript_dialog_state_for_owner_mut(
            &crate::conn::CommandOwnerScope::capture(conn, None),
            |state| {
                state.push(dialog);
            },
        )
        .expect("target session state should exist")
    });

    ctx.process_async(json!({
        "id": 35,
        "method": "Page.handleJavaScriptDialog",
        "params": { "accept": true }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["id"] == json!(35)
                && message["error"]["code"] == json!(-32602)
                && message["error"]["message"] == json!("No dialog is showing")
        }),
        "dialog without its exact Page residence should reject: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Page.javascriptDialogClosed")),
        "stale dialog must not emit a closed event for a replacement target: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_are_emitted_after_runtime_call_function_on() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    enable_page_events_for_fixture(&ctx, "SID-1");
    let page = ctx
        .build_fixture_page("data:text/html,<body>callFunctionOn</body>")
        .await
        .expect("page should load");
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .active_page_target_mut()
            .runtime_slot
            .set_loaded_page_for_test(page)
    });

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ run: function() { alert('callFunctionOn alert'); } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 5)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return an object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { this.run(); }",
            "awaitPromise": true,
            "returnByValue": true,
            "userGesture": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("callFunctionOn alert")
        }),
        "Runtime.callFunctionOn should flush pending JavaScript dialogs: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_are_emitted_from_playwright_utility_world_call_function_on() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    enable_page_events_for_fixture(&ctx, "SID-1");
    let page = ctx
        .build_fixture_page("data:text/html,<body>utility dialog</body>")
        .await
        .expect("page should load");
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .active_page_target_mut()
            .runtime_slot
            .set_loaded_page_for_test(page)
    });

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 8,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "__playwright_utility_world__"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 8)["result"]["executionContextId"]
        .as_i64()
        .expect("isolated context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": isolated_context_id,
            "expression": "({ global: globalThis, evaluate(isFunction, returnByValue, expression, argCount, ...args) { let result = this.global.eval(expression); if (isFunction === true) { result = result(...args.slice(0, argCount)); } else if (isFunction !== false && typeof result === 'function') { result = result(...args.slice(0, argCount)); } return returnByValue ? result : result; } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 9)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a utility object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                { "value": true },
                { "value": true },
                { "value": "() => alert('utility alert')" },
                { "value": 0 }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("utility alert")
        }),
        "Playwright-style Runtime.callFunctionOn should flush pending JavaScript dialogs: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_are_emitted_from_playwright_serialized_utility_call_function_on()
{
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    enable_page_events_for_fixture(&ctx, "SID-1");
    let page = ctx
        .build_fixture_page("data:text/html,<body>serialized utility dialog</body>")
        .await
        .expect("page should load");
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .active_page_target_mut()
            .runtime_slot
            .set_loaded_page_for_test(page)
    });

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": 1,
            "expression": "({ global: globalThis, evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount).map(value => value && value.v === 'null' ? null : value); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? undefined : result; } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a utility object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                {},
                { "value": true },
                { "value": "() => alert('serialized utility alert')" },
                { "value": 1 },
                { "value": { "v": "null" } }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("serialized utility alert")
        }),
        "Playwright serialized Runtime.callFunctionOn should flush pending JavaScript dialogs: {sent:?}"
    );
}

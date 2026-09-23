use moli_core::RendererDocumentTitleChanged;
use moli_core::page::{
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentNetworkSnapshot,
    ChildFrameNavigationSnapshot, RENDERER_BACKEND_NODE_ID_START,
    RendererDocumentLifecycleIdentity, RendererDocumentLifecycleSnapshot,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererDocumentToken, RendererFrameToken,
    RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId, RendererJavaScriptDialogSource,
    RendererLifecycleEpoch, RendererLifecycleEventStamp, RendererPageCreationArtifacts,
    RendererPendingDownloadActivation, RendererPendingFileChooserActivation,
    RendererPendingJavaScriptDialog, RendererPendingPopupActivation,
    RendererPendingSameDocumentNavigation, RendererPopupDisposition, RendererWindowDocumentSource,
    SubresourceResponseBody,
};
use serde_json::{Value, json};

use crate::automation::{AutomationEvent, NavigationFrameEventKind};
use crate::conn::{
    BackgroundProtocolEvent, BrowserContext, CdpConnection, CdpTargetFilter, CommandOwnerScope,
};
use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSlot,
};
use crate::domains::input::{InputPreparedOutputSlot, InputPreparedOutputs};
use crate::testing::TestContext;

fn renderer_document_identity_for_test(
    lifecycle_document_id: u64,
    epoch: u64,
) -> RendererDocumentLifecycleIdentity {
    let page_id = moli_core::PageId::new_for_testing(1);
    RendererDocumentLifecycleIdentity {
        frame: RendererFrameToken { page_id },
        document: RendererDocumentToken::new_for_testing(page_id, lifecycle_document_id),
        epoch: RendererLifecycleEpoch(epoch),
    }
}

fn bind_renderer_document_for_test(
    conn: &mut CdpConnection,
    session_id: &str,
    frame_id: &str,
    identity: RendererDocumentLifecycleIdentity,
) {
    let runtime_slot = conn
        .runtime_session_owner_slot_mut(Some(session_id))
        .expect("test target should expose a runtime owner slot");
    if runtime_slot.page_attachment_id().is_none() {
        runtime_slot.set_page_attachment_id_for_test(identity.document.page_id.as_u64());
    }
    let lifecycle_snapshot = RendererDocumentLifecycleSnapshot {
        frame: identity.frame,
        document: identity.document,
        epoch: identity.epoch,
        started: RendererLifecycleEventStamp {
            sequence: 1,
            timestamp_micros: 1,
        },
        dom_content_loaded: None,
        load: None,
        terminated: None,
    };
    let (binding, initial_events) = conn.bind_renderer_document_lifecycle_for_owner(
        &crate::conn::CommandOwnerScope::for_session(session_id),
        RendererPageCreationArtifacts {
            active_document: identity.document,
            active_epoch: identity.epoch,
            lifecycle_snapshot,
            initial_lifecycle_events: Vec::new(),
        },
        None,
        frame_id.to_owned(),
        super::LOADER_ID.to_owned(),
    );
    assert!(binding.is_some(), "test renderer Document should bind");
    assert!(initial_events.is_empty());
}

fn page_residence_identity_for_test(
    conn: &mut CdpConnection,
    session_id: &str,
) -> crate::conn::TargetPageResidenceIdentity {
    let runtime_slot = conn
        .runtime_session_owner_slot_mut(Some(session_id))
        .expect("test target should expose a runtime owner slot");
    if runtime_slot.page_attachment_id().is_none() {
        runtime_slot.replace_page_attachment_id_for_test();
    }
    conn.target_page_residence_identity_for_session(Some(session_id))
        .expect("test target should expose a Page residence identity")
}

fn take_top_level_location_navigation_work_for_test(
    conn: &mut CdpConnection,
) -> crate::domains::activity::ProtocolSchedulerWork {
    let [event]: [crate::conn::CdpSchedulerEvent; 1] = conn
        .take_scheduler_events()
        .try_into()
        .expect("prepared navigation should publish one concrete scheduler action");
    let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
        panic!("prepared navigation must not publish a source-shaped scheduler event");
    };
    assert!(work.is_top_level_location_navigation_owner_action());
    work
}

fn root_document_attachment_for_test(
    conn: &CdpConnection,
    session_id: &str,
    source_document: RendererDocumentLifecycleIdentity,
) -> crate::conn::TargetRootDocumentProtocolAttachmentIdentity {
    conn.target_root_document_protocol_attachment_identity_for_session(
        Some(session_id),
        source_document,
    )
    .expect("test target should expose the exact root Document attachment")
}

fn prepared_child_frame_activity_for_test(
    conn: &CdpConnection,
    session_id: &str,
    source_document: RendererDocumentLifecycleIdentity,
    document: super::PagePreparedChildFrameDocumentActivity,
) -> super::PagePreparedChildFrameActivity {
    let binding = root_document_attachment_for_test(conn, session_id, source_document);
    super::PagePreparedChildFrameActivity::from_document(binding, document)
}

fn default_prepared_child_frame_activity_for_test(
    conn: &CdpConnection,
    session_id: &str,
    source_document: RendererDocumentLifecycleIdentity,
) -> super::PagePreparedChildFrameActivity {
    super::PagePreparedOutputs::from_child_frame_activity_for_test(
        root_document_attachment_for_test(conn, session_id, source_document),
    )
    .child_frame_activities
    .pop()
    .expect("default child-frame fixture should contain one activity batch")
}

fn javascript_dialog_scope_for_test(
    conn: &CdpConnection,
    session_id: &str,
) -> crate::conn::TargetJavaScriptDialogScopeObserver {
    conn.runtime_session_owner_slot(Some(session_id))
        .map(|slot| slot.javascript_dialog_scope_observer())
        .unwrap_or_else(|_| {
            crate::conn::TargetJavaScriptDialogScopeObserver::stale_for_absent_owner_test()
        })
}

fn document_sourced_same_document_navigation_for_test(
    source_document: RendererDocumentLifecycleIdentity,
    url: &str,
) -> RendererDocumentSourcedSameDocumentNavigation {
    RendererDocumentSourcedSameDocumentNavigation::new(
        source_document,
        RendererPendingSameDocumentNavigation {
            url: url.to_owned(),
            navigation_type: "fragment".to_owned(),
        },
    )
}

fn renderer_javascript_dialog_for_test(
    source_document: RendererDocumentLifecycleIdentity,
    frame_id: &str,
    message: &str,
    completion: Option<RendererJavaScriptDialogCompletion>,
) -> RendererPendingJavaScriptDialog {
    RendererPendingJavaScriptDialog::new(
        RendererJavaScriptDialogId::new(1),
        source_document,
        RendererJavaScriptDialogSource::ChildFrame {
            frame_id: frame_id.to_owned(),
            local_window_id: 1,
            document_id: 1,
        },
        "https://example.test/dialog-source".to_owned(),
        "alert".to_owned(),
        message.to_owned(),
        String::new(),
        completion,
    )
}

fn renderer_popup_javascript_dialog_for_test(
    source_document: RendererDocumentLifecycleIdentity,
    popup_id: u64,
    popup_document_id: u64,
    message: &str,
    completion: Option<RendererJavaScriptDialogCompletion>,
) -> RendererPendingJavaScriptDialog {
    RendererPendingJavaScriptDialog::new(
        RendererJavaScriptDialogId::new(2),
        source_document,
        RendererJavaScriptDialogSource::LightweightPopup {
            popup_id,
            popup_document_id,
        },
        "https://popup.example/dialog-source".to_owned(),
        "alert".to_owned(),
        message.to_owned(),
        String::new(),
        completion,
    )
}

fn protocol_messages_from_background_events(events: Vec<BackgroundProtocolEvent>) -> Vec<Value> {
    events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect()
}

#[test]
fn stale_document_title_cannot_overwrite_replacement_target_metadata() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-title-source".into());
    bc.set_active_target_id("TID-title-source");
    bc.attach_active_session("SID-title-source");
    conn.install_browser_context_fixture_for_test(bc);

    let predecessor = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-title-source",
        "TID-title-source",
        predecessor,
    );
    assert_eq!(
        conn.apply_renderer_document_title_for_owner(
            &CommandOwnerScope::for_session("SID-title-source"),
            &RendererDocumentTitleChanged {
                source_document: predecessor,
                title: "predecessor".to_owned(),
            },
        ),
        Some(true)
    );

    let replacement = renderer_document_identity_for_test(2, 2);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-title-source",
        "TID-title-source",
        replacement,
    );
    assert_eq!(
        conn.apply_renderer_document_title_for_owner(
            &CommandOwnerScope::for_session("SID-title-source"),
            &RendererDocumentTitleChanged {
                source_document: replacement,
                title: "replacement".to_owned(),
            },
        ),
        Some(true)
    );

    assert_eq!(
        conn.apply_renderer_document_title_for_owner(
            &CommandOwnerScope::for_session("SID-title-source"),
            &RendererDocumentTitleChanged {
                source_document: predecessor,
                title: "late predecessor".to_owned(),
            },
        ),
        None,
        "an old renderer Document must lose authority at replacement commit"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .and_then(|context| context.target_info("TID-title-source"))
            .and_then(|target| target["title"].as_str().map(str::to_owned)),
        Some("replacement".to_owned())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_drain_consumes_prepared_dialogs_without_page_readback() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-1");
    let source_document = renderer_document_identity_for_test(1, 1);
    let mut out: Vec<BackgroundProtocolEvent> = Vec::new();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner.clone(),
                Some("SID-1"),
                javascript_dialog_scope_for_test(&conn, "SID-1"),
                "TID-active",
                vec![renderer_javascript_dialog_for_test(
                    source_document,
                    "FRAME-1",
                    "prepared dialog",
                    None,
                )],
            ),
        ));

    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;

    assert_eq!(out.len(), 1);
    let (message, automation_event) = out.remove(0).into_parts();
    assert_eq!(message["method"], json!("Page.javascriptDialogOpening"));
    assert_eq!(message["params"]["frameId"], json!("FRAME-1"));
    assert_eq!(message["params"]["type"], json!("alert"));
    assert_eq!(message["params"]["message"], json!("prepared dialog"));
    assert_eq!(message["sessionId"], json!("SID-1"));
    let Some(AutomationEvent::PageJavaScriptDialogOpening(event)) = automation_event else {
        panic!("expected typed Page.javascriptDialogOpening sidecar");
    };
    assert_eq!(
        event.frame_id.as_ref().map(|frame_id| frame_id.as_str()),
        Some("FRAME-1")
    );
    assert_eq!(event.dialog_type, "alert");
    assert_eq!(event.message, "prepared dialog");
    assert!(!event.has_browser_handler);
    let installed = conn
        .target_page_session_state_for_session(Some("SID-1"))
        .expect("target page session state should exist")
        .javascript_dialog_state
        .pending_dialogs();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].page_owner(), &page_owner);
    assert_eq!(installed[0].source_frame_id(), "FRAME-1");
    assert_eq!(installed[0].message(), "prepared dialog");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_dialog_output_stays_with_its_exact_protocol_attachment() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-dialog-attachment".into());
    browser_context.set_active_target_id("TID-dialog-attachment");
    browser_context.attach_active_session("SID-primary");
    assert!(
        browser_context
            .assign_attached_session_to_target("TID-dialog-attachment", "SID-attached".to_owned(),)
    );
    conn.install_browser_context_fixture_for_test(browser_context);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-attached");
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner,
                Some("SID-attached"),
                javascript_dialog_scope_for_test(&conn, "SID-attached"),
                "TID-dialog-attachment",
                vec![renderer_javascript_dialog_for_test(
                    renderer_document_identity_for_test(1, 1),
                    "FRAME-child",
                    "attached-session dialog",
                    None,
                )],
            ),
        ));

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;

    assert_eq!(out.len(), 1);
    let message = out.remove(0).into_protocol_message();
    assert_eq!(message["sessionId"], json!("SID-attached"));
    assert_eq!(message["params"]["frameId"], json!("FRAME-child"));
    assert!(
        conn.target_page_session_state_for_session(Some("SID-primary"))
            .expect("primary session state")
            .javascript_dialog_state
            .is_empty(),
        "drain-time session must not acquire another attachment's dialog"
    );
    assert_eq!(
        conn.target_page_session_state_for_session(Some("SID-attached"))
            .expect("attached session state")
            .javascript_dialog_state
            .pending_dialogs()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detached_source_attachment_dismisses_prepared_child_dialog() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-dialog-detached".into());
    browser_context.set_active_target_id("TID-dialog-detached");
    browser_context.attach_active_session("SID-primary");
    assert!(
        browser_context
            .assign_attached_session_to_target("TID-dialog-detached", "SID-detached".to_owned(),)
    );
    conn.install_browser_context_fixture_for_test(browser_context);
    let completion = RendererJavaScriptDialogCompletion::pending();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_residence_identity_for_test(&mut conn, "SID-detached"),
                Some("SID-detached"),
                javascript_dialog_scope_for_test(&conn, "SID-detached"),
                "TID-dialog-detached",
                vec![renderer_javascript_dialog_for_test(
                    renderer_document_identity_for_test(1, 1),
                    "FRAME-detached",
                    "detached attachment dialog",
                    Some(completion.clone()),
                )],
            ),
        ));
    assert!(
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .remove_page_session_binding(
                "TID-dialog-detached",
                "SID-detached",
                &moli_page_types::DevToolsSessionKey::Attached("SID-detached".to_owned()),
            )
    );
    conn.rollback_attached_session_without_event("SID-detached");

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;

    assert!(out.is_empty());
    assert!(!completion.finish(true, String::new()));
    assert!(!completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_popup_dialog_rejects_a_retired_source_attachment() {
    const POPUP_ID: u64 = 76;

    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-popup-stale-source".into());
    browser_context.set_active_target_id("TID-popup-stale-source");
    browser_context.attach_active_session("SID-primary");
    assert!(
        browser_context
            .assign_attached_session_to_target("TID-popup-stale-source", "SID-source".to_owned(),)
    );
    conn.install_browser_context_fixture_for_test(browser_context);
    conn.set_auto_attach_owner(None, true, false, CdpTargetFilter::default_auto_attach());
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-source");
    let source_document = renderer_document_identity_for_test(1, 1);
    let completion = RendererJavaScriptDialogCompletion::pending();
    let mut dialog_output =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner.clone(),
                Some("SID-source"),
                javascript_dialog_scope_for_test(&conn, "SID-source"),
                "TID-popup-stale-source",
                vec![renderer_popup_javascript_dialog_for_test(
                    source_document,
                    POPUP_ID,
                    8,
                    "stale source popup dialog",
                    Some(completion.clone()),
                )],
            ),
        ));
    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut dialog_output,
    )
    .await;
    assert!(
        out.is_empty(),
        "dialog should remain pending before popup creation"
    );

    assert!(
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .remove_page_session_binding(
                "TID-popup-stale-source",
                "SID-source",
                &moli_page_types::DevToolsSessionKey::Attached("SID-source".to_owned()),
            )
    );
    conn.rollback_attached_session_without_event("SID-source");
    let mut popup_output =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_popup_activations_for_test(
                page_owner,
                vec![RendererPendingPopupActivation::window(
                    source_document,
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    Some(POPUP_ID),
                    "about:blank".to_owned(),
                    "_blank".to_owned(),
                    RendererPopupDisposition::Background,
                )],
            ),
        ));
    super::emit_popup_activity_background_events_async(&mut conn, &mut out, &mut popup_output)
        .await;

    let messages = protocol_messages_from_background_events(out);
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == json!("Target.attachedToTarget")),
        "the popup must have a real attachment so source retirement is the rejection reason"
    );
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("Page.javascriptDialogOpening"))
    );
    assert!(!completion.finish(true, String::new()));
    assert!(!completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn lightweight_popup_dialog_waits_for_and_uses_popup_attachment() {
    const POPUP_ID: u64 = 77;

    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-popup-dialog".into());
    browser_context.set_active_target_id("TID-opener");
    browser_context.attach_active_session("SID-opener");
    conn.install_browser_context_fixture_for_test(browser_context);
    conn.set_auto_attach_owner(None, true, false, CdpTargetFilter::default_auto_attach());
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-opener");
    let source_dialog_scope = javascript_dialog_scope_for_test(&conn, "SID-opener");
    let source_document = renderer_document_identity_for_test(1, 1);
    let completion = RendererJavaScriptDialogCompletion::pending();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner.clone(),
                Some("SID-opener"),
                source_dialog_scope.clone(),
                "TID-opener",
                vec![renderer_popup_javascript_dialog_for_test(
                    source_document,
                    POPUP_ID,
                    9,
                    "popup-owned dialog",
                    Some(completion.clone()),
                )],
            ),
        ));
    prepared.extend_payload(
        super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_popup_activations_for_test(
                page_owner.clone(),
                vec![RendererPendingPopupActivation::window(
                    source_document,
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    Some(POPUP_ID),
                    "about:blank".to_owned(),
                    "_blank".to_owned(),
                    RendererPopupDisposition::Background,
                )],
            ),
        )
        .into(),
    );

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;
    assert!(
        out.is_empty(),
        "dialog must wait for its popup target instead of falling back to the opener"
    );

    super::emit_popup_activity_background_events_async(&mut conn, &mut out, &mut prepared).await;

    let browser_context = conn
        .browser_context_by_id("BID-popup-dialog")
        .expect("popup browser context");
    let popup_target_id = browser_context
        .target_id_for_popup_id(POPUP_ID)
        .expect("popup id should resolve to its created target")
        .to_owned();
    let popup_session_id = browser_context
        .background_target(&popup_target_id)
        .and_then(|target| target.session_id())
        .expect("auto-attached popup session")
        .to_owned();
    let observed_messages = protocol_messages_from_background_events(out);
    let dialog_event = observed_messages
        .iter()
        .find(|message| message["method"] == json!("Page.javascriptDialogOpening"))
        .unwrap_or_else(|| {
            panic!(
                "popup dialog opening event; popup_session_id={popup_session_id}; \
                     observed={observed_messages:?}"
            )
        });
    assert_eq!(dialog_event["sessionId"], json!(popup_session_id));
    assert_eq!(
        dialog_event["params"]["frameId"],
        json!(popup_target_id.clone())
    );
    assert!(
        conn.target_page_session_state_for_session(Some("SID-opener"))
            .expect("opener session state")
            .javascript_dialog_state
            .is_empty(),
        "opener session must not own the popup's modal dialog"
    );
    assert_eq!(
        conn.target_page_session_state_for_session(Some(&popup_session_id))
            .expect("popup session state")
            .javascript_dialog_state
            .pending_dialogs()
            .len(),
        1
    );

    let mut later_dialog =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner,
                Some("SID-opener"),
                source_dialog_scope,
                "TID-opener",
                vec![renderer_popup_javascript_dialog_for_test(
                    source_document,
                    POPUP_ID,
                    9,
                    "later popup-owned dialog",
                    None,
                )],
            ),
        ));
    let mut later_out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut later_out,
        &mut later_dialog,
    )
    .await;
    let later_messages = protocol_messages_from_background_events(later_out);
    assert!(later_messages.iter().any(|message| {
        message["method"] == json!("Page.javascriptDialogOpening")
            && message["sessionId"] == json!(popup_session_id)
            && message["params"]["message"] == json!("later popup-owned dialog")
    }));
    assert_eq!(
        conn.target_page_session_state_for_session(Some(&popup_session_id))
            .expect("popup session state")
            .javascript_dialog_state
            .pending_dialogs()
            .len(),
        2,
        "a later output batch should resolve through the existing popup attachment"
    );
    conn.with_target_devtools_session_state_for_session_mut(Some(&popup_session_id), |state| {
        state.page_session_state.javascript_dialog_state.clear()
    })
    .expect("popup session state should clear");
    assert!(!completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn unattached_popup_dialog_is_dismissed_without_opener_fallback() {
    const POPUP_ID: u64 = 78;

    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-popup-no-session".into());
    browser_context.set_active_target_id("TID-opener-no-session");
    browser_context.attach_active_session("SID-opener-no-session");
    conn.install_browser_context_fixture_for_test(browser_context);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-opener-no-session");
    let source_document = renderer_document_identity_for_test(1, 1);
    let completion = RendererJavaScriptDialogCompletion::pending();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner.clone(),
                Some("SID-opener-no-session"),
                javascript_dialog_scope_for_test(&conn, "SID-opener-no-session"),
                "TID-opener-no-session",
                vec![renderer_popup_javascript_dialog_for_test(
                    source_document,
                    POPUP_ID,
                    10,
                    "unattached popup dialog",
                    Some(completion.clone()),
                )],
            ),
        ));
    prepared.extend_payload(
        super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_popup_activations_for_test(
                page_owner,
                vec![RendererPendingPopupActivation::window(
                    source_document,
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    Some(POPUP_ID),
                    "about:blank".to_owned(),
                    "_blank".to_owned(),
                    RendererPopupDisposition::Background,
                )],
            ),
        )
        .into(),
    );

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;
    super::emit_popup_activity_background_events_async(&mut conn, &mut out, &mut prepared).await;

    let messages = protocol_messages_from_background_events(out);
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("Page.javascriptDialogOpening"))
    );
    assert!(
        conn.target_page_session_state_for_session(Some("SID-opener-no-session"))
            .expect("opener session state")
            .javascript_dialog_state
            .is_empty()
    );
    assert!(!completion.finish(true, String::new()));
    assert!(!completion.wait().accepted);
}

#[test]
fn renderer_document_epoch_change_retires_page_dialog_scope_once() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-dialog-epoch".into());
    bc.set_active_target_id("TID-dialog-epoch");
    bc.attach_active_session("SID-dialog-epoch");
    conn.install_browser_context_fixture_for_test(bc);
    let first_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-dialog-epoch",
        "TID-dialog-epoch",
        first_document,
    );
    let observer = conn
        .runtime_session_owner_slot(Some("SID-dialog-epoch"))
        .expect("target Page runtime slot")
        .javascript_dialog_scope_observer();

    bind_renderer_document_for_test(
        &mut conn,
        "SID-dialog-epoch",
        "TID-dialog-epoch",
        first_document,
    );
    assert!(
        conn.runtime_session_owner_slot(Some("SID-dialog-epoch"))
            .expect("target Page runtime slot")
            .observes_javascript_dialog_scope(&observer),
        "rebinding the same exact renderer Document must preserve prepared dialog output"
    );

    bind_renderer_document_for_test(
        &mut conn,
        "SID-dialog-epoch",
        "TID-dialog-epoch",
        renderer_document_identity_for_test(1, 2),
    );
    assert!(
        !conn
            .runtime_session_owner_slot(Some("SID-dialog-epoch"))
            .expect("target Page runtime slot")
            .observes_javascript_dialog_scope(&observer),
        "a new lifecycle epoch for the same Document token must retire old prepared dialogs"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_prepared_action_dismisses_replacement_page_output() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-dialog-stale-page".into());
    bc.set_active_target_id("TID-dialog-stale-page");
    bc.attach_active_session("SID-dialog-stale-page");
    conn.install_browser_context_fixture_for_test(bc);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-dialog-stale-page");
    let completion = moli_core::page::RendererJavaScriptDialogCompletion::pending();
    let dialog = renderer_javascript_dialog_for_test(
        renderer_document_identity_for_test(1, 1),
        "FRAME-stale-page",
        "stale page dialog",
        Some(completion.clone()),
    );
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner.clone(),
                Some("SID-dialog-stale-page"),
                javascript_dialog_scope_for_test(&conn, "SID-dialog-stale-page"),
                "TID-dialog-stale-page",
                vec![dialog],
            ),
        ));
    conn.runtime_session_owner_slot_mut(Some("SID-dialog-stale-page"))
        .expect("test target runtime slot")
        .replace_page_attachment_id_for_test();

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;

    assert!(out.is_empty());
    assert!(
        conn.target_page_session_state_for_session(Some("SID-dialog-stale-page"))
            .expect("target page session state")
            .javascript_dialog_state
            .is_empty()
    );
    assert!(
        !completion.finish(true, "late accept".to_owned()),
        "stale apply must already dismiss the renderer completion"
    );
    let result = completion.wait();
    assert!(!result.accepted);
    assert!(result.user_input.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_prepared_action_dismisses_retired_dialog_scope() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-dialog-generation".into());
    bc.set_active_target_id("TID-dialog-generation");
    bc.attach_active_session("SID-dialog-generation");
    conn.install_browser_context_fixture_for_test(bc);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-dialog-generation");
    let completion = moli_core::page::RendererJavaScriptDialogCompletion::pending();
    let dialog = renderer_javascript_dialog_for_test(
        renderer_document_identity_for_test(1, 1),
        "FRAME-dialog-generation",
        "retired dialog",
        Some(completion.clone()),
    );
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner,
                Some("SID-dialog-generation"),
                javascript_dialog_scope_for_test(&conn, "SID-dialog-generation"),
                "TID-dialog-generation",
                vec![dialog],
            ),
        ));
    conn.runtime_session_owner_slot_mut(Some("SID-dialog-generation"))
        .expect("target Page runtime slot")
        .retire_javascript_dialog_scope();
    conn.with_target_devtools_session_state_for_session_mut(
        Some("SID-dialog-generation"),
        |state| state.page_session_state.javascript_dialog_state.clear(),
    )
    .expect("target session state should retire its dialog scope");

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;

    assert!(out.is_empty());
    assert!(!completion.finish(true, String::new()));
    assert!(!completion.wait().accepted);
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_projection_uses_captured_url_and_frame() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-dialog-source".into());
    bc.set_active_target_id("TID-dialog-source");
    bc.set_target_url("https://example.test/current-before-capture".to_owned());
    bc.attach_active_session("SID-dialog-source");
    conn.install_browser_context_fixture_for_test(bc);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-dialog-source");
    let dialog = RendererPendingJavaScriptDialog::new(
        RendererJavaScriptDialogId::new(9),
        renderer_document_identity_for_test(2, 3),
        RendererJavaScriptDialogSource::ChildFrame {
            frame_id: "FRAME-source".to_owned(),
            local_window_id: 4,
            document_id: 5,
        },
        "https://source.example/dialog".to_owned(),
        "alert".to_owned(),
        "source identity".to_owned(),
        String::new(),
        None,
    );
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner,
                Some("SID-dialog-source"),
                javascript_dialog_scope_for_test(&conn, "SID-dialog-source"),
                "TID-dialog-source",
                vec![dialog],
            ),
        ));
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .set_target_url("https://replacement.example/current".to_owned());

    let mut out = Vec::new();
    super::emit_javascript_dialog_activity_background_events_async(
        &mut conn,
        &mut out,
        &mut prepared,
    )
    .await;

    assert_eq!(out.len(), 1);
    let message = out.remove(0).into_protocol_message();
    assert_eq!(message["params"]["frameId"], json!("FRAME-source"));
    assert_eq!(
        message["params"]["url"],
        json!("https://source.example/dialog")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn canonical_activity_drain_order_survives_ordered_typed_event_stream() {
    let mut conn = CdpConnection::default();
    conn.set_root_target_discovery_enabled(true);
    conn.download_behavior
        .set_global("deny".to_owned(), None, true);
    let mut bc = BrowserContext::new("BID-activity-order".into());
    bc.set_active_target_id("TID-activity-order");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-activity-order");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_file_chooser_opened_event_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-activity-order",
        "TID-activity-order",
        source_document,
    );
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-activity-order");

    let mut prepared = ProtocolOutputPayloads::from_slot(InputPreparedOutputSlot::from_outputs(
        InputPreparedOutputs::from_file_chooser_activations_for_test(
            page_owner.clone(),
            "TID-activity-order",
            vec![RendererPendingFileChooserActivation::new(
                source_document,
                Some("TID-activity-order".to_owned()),
                RENDERER_BACKEND_NODE_ID_START + 42,
                false,
            )],
        ),
    ));
    prepared.extend_payload(
        InputPreparedOutputSlot::from_outputs(
            InputPreparedOutputs::from_download_activations_for_test(vec![
                RendererPendingDownloadActivation {
                    url: "https://example.test/report.txt".to_owned(),
                    suggested_filename: Some("report.txt".to_owned()),
                    response: None,
                },
            ]),
        )
        .into(),
    );
    prepared.extend_payload(
        super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                page_owner.clone(),
                Some("SID-activity-order"),
                javascript_dialog_scope_for_test(&conn, "SID-activity-order"),
                "TID-activity-order",
                vec![renderer_javascript_dialog_for_test(
                    source_document,
                    "TID-activity-order",
                    "ordered dialog",
                    None,
                )],
            ),
        )
        .into(),
    );
    prepared.extend_payload(
        super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_popup_activations_for_test(
                page_owner.clone(),
                vec![RendererPendingPopupActivation::window(
                    source_document,
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    None,
                    "data:text/html,%3Cmain%3Eordered-popup%3C/main%3E".to_owned(),
                    "_blank".to_owned(),
                    RendererPopupDisposition::Background,
                )],
            ),
        )
        .into(),
    );

    let owner = crate::conn::CommandOwnerScope::for_session("SID-activity-order");
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let mut context = ProtocolOutputProjectionContext::new(&owner, &mut command_context);

    for step in [
        ProtocolOutputSlot::FileChooser,
        ProtocolOutputSlot::Download,
        ProtocolOutputSlot::JavascriptDialog,
        ProtocolOutputSlot::Popup,
    ] {
        super::output::project_page_output_async(step, &mut conn, &mut context, &mut prepared)
            .await;
    }

    let events = context.command.take_protocol_events();
    let parts = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    let ordered_methods = parts
        .iter()
        .filter_map(|(message, automation_event)| match automation_event {
            Some(AutomationEvent::BrowserDownloadWillBegin(_)) => Some("Browser.downloadWillBegin"),
            Some(AutomationEvent::BrowserDownloadProgress(_)) => Some("Browser.downloadProgress"),
            _ => message["method"].as_str(),
        })
        .filter(|method| {
            matches!(
                *method,
                "Page.fileChooserOpened"
                    | "Browser.downloadWillBegin"
                    | "Browser.downloadProgress"
                    | "Page.javascriptDialogOpening"
                    | "Target.targetCreated"
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        ordered_methods,
        vec![
            "Page.fileChooserOpened",
            "Browser.downloadWillBegin",
            "Browser.downloadProgress",
            "Page.javascriptDialogOpening",
            "Target.targetCreated",
        ],
        "typed activity sidecars must stay in canonical activity order"
    );
    assert!(matches!(
        parts[0].1.as_ref(),
        Some(AutomationEvent::PageFileChooserOpened(event))
            if event.backend_node_id == RENDERER_BACKEND_NODE_ID_START + 42
    ));
    assert!(
        parts.iter().any(|(_, event)| matches!(
            event,
            Some(AutomationEvent::BrowserDownloadWillBegin(download))
                if download.suggested_filename == "report.txt"
        )),
        "download willBegin should retain its typed sidecar in the ordered stream"
    );
    assert!(
        parts.iter().any(|(_, event)| matches!(
            event,
            Some(AutomationEvent::PageJavaScriptDialogOpening(dialog))
                if dialog.message == "ordered dialog"
        )),
        "javascript dialog should retain its typed sidecar in the ordered stream"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn later_navigation_drain_order_survives_ordered_typed_event_stream() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-later-activity-order".into());
    bc.set_active_target_id("TID-later-activity-order");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-later-activity-order");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-later-activity-order",
        "TID-later-activity-order",
        source_document,
    );

    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_child_frame_activity_for_test(
                root_document_attachment_for_test(
                    &conn,
                    "SID-later-activity-order",
                    source_document,
                ),
            ),
        ));
    prepared.extend_payload(
        super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_same_document_navigations_for_test(
                page_residence_identity_for_test(&mut conn, "SID-later-activity-order"),
                vec![document_sourced_same_document_navigation_for_test(
                    source_document,
                    "https://example.test/page#ordered",
                )],
            ),
        )
        .into(),
    );
    prepared.extend_payload(
        super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                page_residence_identity_for_test(&mut conn, "SID-later-activity-order"),
                Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                    source_document,
                    "data:text/html,%3Cmain%3Eordered-location%3C/main%3E".to_owned(),
                )),
            ),
        )
        .into(),
    );

    let owner = crate::conn::CommandOwnerScope::for_session("SID-later-activity-order");
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let mut context = ProtocolOutputProjectionContext::new(&owner, &mut command_context);

    for step in [
        ProtocolOutputSlot::ChildFrameActivity,
        ProtocolOutputSlot::SameDocumentNavigation,
        ProtocolOutputSlot::TopLevelLocationNavigation,
    ] {
        super::output::project_page_output_async(step, &mut conn, &mut context, &mut prepared)
            .await;
    }

    let work = take_top_level_location_navigation_work_for_test(&mut conn);
    let (navigation_events, nested_scheduler_events) = conn
        .complete_ready_protocol_scheduler_work_turn(work)
        .await
        .into_protocol_event_parts();
    assert!(
        !nested_scheduler_events.iter().any(|event| {
            matches!(
                event,
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.is_top_level_location_navigation_owner_action()
            )
        }),
        "executing the concrete navigation must not republish its own owner action"
    );
    context
        .command
        .protocol_events_mut()
        .extend(navigation_events);

    let events = context.command.take_protocol_events();
    let parts = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    let ordered_markers = parts
        .iter()
        .filter_map(|(message, _)| {
            let method = message["method"].as_str()?;
            let params = &message["params"];
            match method {
                "Page.frameAttached"
                    if params["frameId"] == json!("CHILD-FRAME-1")
                        && params["parentFrameId"] == json!("TID-1") =>
                {
                    Some("child-frame-completion")
                }
                "Page.navigatedWithinDocument"
                    if params["url"] == json!("https://example.test/page#ordered") =>
                {
                    Some("same-document-navigation")
                }
                "Page.frameStartedNavigating"
                    if params["url"]
                        == json!("data:text/html,%3Cmain%3Eordered-location%3C/main%3E") =>
                {
                    Some("top-level-location-navigation")
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    let child_frame_completion_index = ordered_markers
        .iter()
        .position(|marker| *marker == "child-frame-completion")
        .expect("child-frame completion marker");
    let same_document_navigation_index = ordered_markers
        .iter()
        .position(|marker| *marker == "same-document-navigation")
        .expect("same-document navigation marker");
    let top_level_location_navigation_index = ordered_markers
        .iter()
        .position(|marker| *marker == "top-level-location-navigation")
        .expect("top-level location navigation marker");

    assert!(
        child_frame_completion_index < same_document_navigation_index
            && same_document_navigation_index < top_level_location_navigation_index,
        "typed navigation activity sidecars must not be appended after later raw activity output: {ordered_markers:?}"
    );
    assert!(
        parts.iter().any(|(_, event)| matches!(
            event,
            Some(AutomationEvent::NavigationFrame(navigation))
                if navigation.kind == NavigationFrameEventKind::Navigated
                    && navigation.frame_id.as_str() == "CHILD-FRAME-1"
                    && navigation.loader_id.as_ref().is_some_and(|loader_id| {
                        loader_id.as_str() == "LOADER-CHILD-FRAME-1"
                    })
                    && navigation.url == "https://example.test/child"
        )),
        "child-frame completion should regain its typed navigation sidecar after out projection"
    );
    assert!(
        parts.iter().any(|(_, event)| matches!(
            event,
            Some(AutomationEvent::SameDocumentNavigation(navigation))
                if navigation.url == "https://example.test/page#ordered"
        )),
        "same-document navigation should regain its typed sidecar after out projection"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_initiated_child_frame_completion_omits_renderer_request_events() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let mut background_events = Vec::new();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_child_frame_activity_for_test(
                root_document_attachment_for_test(&conn, "SID-1", source_document),
            ),
        ));
    let activity = prepared
        .page_mut()
        .and_then(super::PagePreparedOutputSlot::take_child_frame_activity)
        .and_then(|mut activities| activities.pop())
        .expect("test prepared output should carry one child frame activity");

    super::emit_prepared_child_frame_activity(
        &mut conn,
        &mut background_events,
        activity,
        Some("CHILD-FRAME-1"),
    )
    .await;
    let out = protocol_messages_from_background_events(background_events);

    assert!(
        conn.runtime_session_owner_slot(Some("SID-1"))
            .expect("runtime owner slot should exist")
            .loaded_page()
            .is_none(),
        "prepared child-frame completion emission must not require a loaded page"
    );
    assert!(out.iter().any(|message| {
        message["method"] == json!("Page.frameAttached")
            && message["params"]["frameId"] == json!("CHILD-FRAME-1")
            && message["params"]["parentFrameId"] == json!("TID-1")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(out.iter().any(|message| {
        message["method"] == json!("Page.frameNavigated")
            && message["params"]["frame"]["id"] == json!("CHILD-FRAME-1")
            && message["params"]["frame"]["url"] == json!("https://example.test/child")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(out.iter().any(|message| {
        message["method"] == json!("Page.frameStoppedLoading")
            && message["params"]["frameId"] == json!("CHILD-FRAME-1")
    }));
    assert!(out.iter().any(|message| {
        message["method"] == json!("Page.frameStartedNavigating")
            && message["params"]["frameId"] == json!("CHILD-FRAME-1")
    }));
    assert!(
        !out.iter().any(|message| matches!(
            message["method"].as_str(),
            Some(
                "Page.frameScheduledNavigation"
                    | "Page.frameRequestedNavigation"
                    | "Page.frameClearedScheduledNavigation"
            )
        )),
        "Page.navigate(frameId=child) must not fabricate renderer navigation probes: {out:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_activity_emits_navigation_before_init_before_lifecycle_terminal() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let mut background_events = Vec::new();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_child_frame_activity_for_test(
                root_document_attachment_for_test(&conn, "SID-1", source_document),
            ),
        ));
    let activity = prepared
        .page_mut()
        .and_then(super::PagePreparedOutputSlot::take_child_frame_activity)
        .and_then(|mut activities| activities.pop())
        .expect("test prepared output should carry one child frame activity");

    super::emit_prepared_child_frame_activity(&mut conn, &mut background_events, activity, None)
        .await;
    let out = protocol_messages_from_background_events(background_events);

    let navigated_index = out
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!("CHILD-FRAME-1")
        })
        .expect("child frameNavigated should be emitted");
    let init_index = out
        .iter()
        .position(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("CHILD-FRAME-1")
                && message["params"]["name"] == json!("init")
        })
        .expect("child init lifecycle should be emitted");
    let stopped_index = out
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!("CHILD-FRAME-1")
        })
        .expect("child frameStoppedLoading should be emitted");

    assert!(
        navigated_index < init_index,
        "frameNavigated must precede child init lifecycle output"
    );
    assert!(
        init_index < stopped_index,
        "child lifecycle terminal markers must precede stoppedLoading"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_activity_fans_out_page_events_to_enabled_attached_session() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-child-page-fanout".into());
    bc.set_active_target_id("TID-child-page-fanout");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-primary");
    assert!(
        bc.assign_attached_session_to_target("TID-child-page-fanout", "SID-attached".to_owned(),)
    );
    conn.install_browser_context_fixture_for_test(bc);
    conn.with_target_devtools_session_state_for_session_mut(Some("SID-attached"), |state| {
        state.page_session_state.page_domain_enabled = true;
        state.page_session_state.page_lifecycle_events = true;
    })
    .expect("attached session should expose Page state");

    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-primary",
        "TID-child-page-fanout",
        source_document,
    );
    let activity =
        default_prepared_child_frame_activity_for_test(&conn, "SID-primary", source_document);
    let mut background_events = Vec::new();

    super::emit_prepared_child_frame_activity(&mut conn, &mut background_events, activity, None)
        .await;
    let out = protocol_messages_from_background_events(background_events);

    for method in [
        "Page.frameAttached",
        "Page.frameNavigated",
        "Page.frameStoppedLoading",
    ] {
        assert!(
            out.iter().any(|message| {
                message["sessionId"] == json!("SID-attached") && message["method"] == json!(method)
            }),
            "Page-enabled attached session should receive {method}: {out:?}"
        );
        assert!(
            !out.iter().any(|message| {
                message["sessionId"] == json!("SID-primary") && message["method"] == json!(method)
            }),
            "Page-disabled primary session must not receive {method}: {out:?}"
        );
    }
    assert!(out.iter().any(|message| {
        message["sessionId"] == json!("SID-attached")
            && message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == json!("CHILD-FRAME-1")
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_activity_projects_sandboxed_about_blank_from_document_url() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://top.example/page".to_owned());
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let mut background_events = Vec::new();
    let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
        12.5,
        vec![super::PagePreparedChildFrameTreeEvent::Attached {
            frame_id: "CHILD-FRAME-1".to_owned(),
            parent_frame_id: "TID-1".to_owned(),
        }],
        Vec::new(),
        Vec::new(),
        vec![ChildFrameNavigationSnapshot {
            frame_id: "CHILD-FRAME-1".to_owned(),
            parent_frame_id: Some("TID-1".to_owned()),
            loader_id: Some("LID-CHILD-1".to_owned()),
            name: Some("sandboxed-blank".to_owned()),
            url: "about:blank".to_owned(),
            document_open_replacement: false,
            security_origin_inherited: true,
            security_origin_opaque: true,
            document_network: None,
        }],
        "https://top.example".to_owned(),
        "Secure".to_owned(),
    );
    let activity =
        prepared_child_frame_activity_for_test(&conn, "SID-1", source_document, document);

    super::emit_prepared_child_frame_activity(&mut conn, &mut background_events, activity, None)
        .await;
    let out = protocol_messages_from_background_events(background_events);

    let navigated = out
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!("CHILD-FRAME-1")
        })
        .expect("child frame navigation event");
    // Page.Frame projects securityOrigin from DocumentLoader::Url while
    // secureContextType still reflects the live inherited security state.
    assert_eq!(navigated["params"]["frame"]["securityOrigin"], json!("://"));
    assert_eq!(
        navigated["params"]["frame"]["secureContextType"],
        json!("Secure")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_activity_emits_document_network_events_from_prepared_load() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    assert!(bc.assign_attached_session_to_target("TID-1", "SID-ATTACHED".to_owned(),));
    conn.install_browser_context_fixture_for_test(bc);
    assert!(conn.enable_network_listener_for_session_owner(Some("SID-1")));
    assert!(conn.enable_network_listener_for_session_owner(Some("SID-ATTACHED")));
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let mut background_events = Vec::new();
    let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
        12.5,
        vec![super::PagePreparedChildFrameTreeEvent::Attached {
            frame_id: "CHILD-FRAME-1".to_owned(),
            parent_frame_id: "TID-1".to_owned(),
        }],
        Vec::new(),
        Vec::new(),
        vec![ChildFrameNavigationSnapshot {
            frame_id: "CHILD-FRAME-1".to_owned(),
            parent_frame_id: Some("TID-1".to_owned()),
            loader_id: Some("LID-CHILD-1".to_owned()),
            name: Some("child-frame".to_owned()),
            url: "https://example.test/child".to_owned(),
            document_open_replacement: false,
            security_origin_inherited: false,
            security_origin_opaque: false,
            document_network: Some(ChildFrameDocumentNetworkSnapshot {
                request_url: "https://example.test/child".to_owned(),
                request_method: "GET".to_owned(),
                request_headers: vec![("Accept".to_owned(), "text/html".to_owned())],
                final_url: "https://example.test/child".to_owned(),
                status: 200,
                response_headers: vec![("Content-Type".to_owned(), b"text/html".to_vec())],
                encoded_data_length: 3,
                response_body: Some(SubresourceResponseBody::from_bytes(vec![0x00, 0xff, b'a'])),
                from_cache: true,
                cache_state: moli_fetch::ResponseCacheState::Local,
                preload_state: Default::default(),
            }),
        }],
        "https://example.test".to_owned(),
        "Secure".to_owned(),
    );
    let activity =
        prepared_child_frame_activity_for_test(&conn, "SID-1", source_document, document);

    super::emit_prepared_child_frame_activity(&mut conn, &mut background_events, activity, None)
        .await;
    let out = protocol_messages_from_background_events(background_events);

    let request = out
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .expect("child document request event");
    assert_eq!(request["sessionId"], json!("SID-1"));
    assert_eq!(request["params"]["frameId"], json!("CHILD-FRAME-1"));
    assert_eq!(request["params"]["loaderId"], json!("LID-CHILD-1"));
    assert_eq!(
        request["params"]["request"]["url"],
        json!("https://example.test/child")
    );
    assert_eq!(request["params"]["type"], json!("Document"));

    let request_index = out
        .iter()
        .position(|message| message["method"] == json!("Network.requestWillBeSent"))
        .expect("child document request event index");
    let cached_index = out
        .iter()
        .position(|message| message["method"] == json!("Network.requestServedFromCache"))
        .expect("child document cache event");
    let response = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .expect("child document response event");
    let response_index = out
        .iter()
        .position(|message| message["method"] == json!("Network.responseReceived"))
        .expect("child document response event index");
    assert!(request_index < cached_index && cached_index < response_index);
    assert_eq!(
        out[cached_index]["params"]["requestId"],
        json!("LID-CHILD-1")
    );
    assert_eq!(response["params"]["frameId"], json!("CHILD-FRAME-1"));
    assert_eq!(response["params"]["loaderId"], json!("LID-CHILD-1"));
    assert_eq!(
        response["params"]["response"]["url"],
        json!("https://example.test/child")
    );
    assert_eq!(response["params"]["response"]["status"], json!(200));
    assert_eq!(response["params"]["response"]["fromDiskCache"], json!(true));

    let finished = out
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFinished"))
        .expect("child document loadingFinished event");
    assert_eq!(finished["params"]["requestId"], json!("LID-CHILD-1"));
    assert_eq!(finished["params"]["encodedDataLength"], json!(3));
    assert!(out.iter().any(|message| {
        message["sessionId"] == json!("SID-ATTACHED")
            && message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!("LID-CHILD-1")
    }));

    let mut ctx = TestContext::from_conn(conn);
    ctx.process_async(json!({
        "id": 7_501,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": "LID-CHILD-1" }
    }))
    .await;
    ctx.expect_result(
        7_501,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );
    ctx.process_async(json!({
        "id": 7_504,
        "method": "Network.getResponseBody",
        "sessionId": "SID-ATTACHED",
        "params": { "requestId": "LID-CHILD-1" }
    }))
    .await;
    ctx.expect_result(
        7_504,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-ATTACHED"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_child_document_response_emits_network_without_navigation_or_lifecycle() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    assert!(conn.enable_network_listener_for_session_owner(Some("SID-1")));
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
        19.25,
        Vec::new(),
        Vec::new(),
        vec![ChildFrameDocumentNetworkActivitySnapshot {
            frame_id: "RETIRED-CHILD-FRAME".to_owned(),
            parent_frame_id: Some("TID-1".to_owned()),
            loader_id: "LID-RETIRED-CHILD".to_owned(),
            snapshot: ChildFrameDocumentNetworkSnapshot {
                request_url: "https://example.test/retired-child".to_owned(),
                request_method: "GET".to_owned(),
                request_headers: Vec::new(),
                final_url: "https://example.test/retired-child".to_owned(),
                status: 200,
                response_headers: vec![("Content-Type".to_owned(), b"text/html".to_vec())],
                encoded_data_length: 21,
                response_body: Some(SubresourceResponseBody::from_bytes(
                    b"historical child body".to_vec(),
                )),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
            },
        }],
        Vec::new(),
        "https://example.test".to_owned(),
        "Secure".to_owned(),
    );
    let activity =
        prepared_child_frame_activity_for_test(&conn, "SID-1", source_document, document);
    let mut background_events = Vec::new();

    super::emit_prepared_child_frame_activity(&mut conn, &mut background_events, activity, None)
        .await;
    let out = protocol_messages_from_background_events(background_events);

    assert_eq!(
        out.iter()
            .filter(|message| {
                matches!(
                    message["method"].as_str(),
                    Some(
                        "Network.requestWillBeSent"
                            | "Network.responseReceived"
                            | "Network.dataReceived"
                            | "Network.loadingFinished"
                    )
                )
            })
            .count(),
        4,
        "historical response should retain its complete Network event family: {out:?}"
    );
    assert_eq!(
        out.iter()
            .filter_map(|message| message["method"].as_str())
            .filter(|method| method.starts_with("Network."))
            .collect::<Vec<_>>(),
        vec![
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.dataReceived",
            "Network.loadingFinished",
        ],
        "historical response must preserve the request/response/data/finish protocol order"
    );
    assert!(
        out.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some(
                    "Page.frameNavigated"
                        | "Page.frameStartedLoading"
                        | "Page.frameStoppedLoading"
                        | "Page.frameStartedNavigating"
                        | "Page.frameRequestedNavigation"
                        | "Page.frameScheduledNavigation"
                        | "Page.lifecycleEvent"
                )
            )
        }),
        "historical Network-only output must not imply a commit or lifecycle transition: {out:?}"
    );
    let request = out
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .expect("historical request event");
    assert_eq!(request["params"]["frameId"], json!("RETIRED-CHILD-FRAME"));
    assert_eq!(request["params"]["loaderId"], json!("LID-RETIRED-CHILD"));

    let mut ctx = TestContext::from_conn(conn);
    ctx.process_async(json!({
        "id": 7_502,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": "LID-RETIRED-CHILD" }
    }))
    .await;
    ctx.expect_result(
        7_502,
        json!({ "body": "historical child body", "base64Encoded": false }),
        Some("SID-1"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_network_without_body_records_known_no_data() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);
    assert!(conn.enable_network_listener_for_session_owner(Some("SID-1")));
    let snapshot = ChildFrameDocumentNetworkSnapshot {
        request_url: "https://example.test/legacy-child".to_owned(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        final_url: "https://example.test/legacy-child".to_owned(),
        status: 200,
        response_headers: vec![("Content-Type".to_owned(), b"text/html".to_vec())],
        encoded_data_length: 0,
        response_body: None,
        from_cache: false,
        cache_state: Default::default(),
        preload_state: Default::default(),
    };
    let mut background_events = Vec::new();

    crate::domains::network::emit_child_document_navigation_network_background_events(
        &mut conn,
        &mut background_events,
        Some("SID-1"),
        "CHILD-FRAME-LEGACY",
        "LID-CHILD-LEGACY",
        "LID-CHILD-LEGACY",
        12.5,
        &snapshot,
    );

    let messages = protocol_messages_from_background_events(background_events);
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == json!("Network.loadingFinished"))
    );
    let mut ctx = TestContext::from_conn(conn);
    ctx.process_async(json!({
        "id": 7_503,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": "LID-CHILD-LEGACY" }
    }))
    .await;
    ctx.expect_error(
        7_503,
        -32000,
        "No data found for resource with given identifier",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_activity_drain_preserves_prepared_attachment_only_token() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
        12.5,
        vec![super::PagePreparedChildFrameTreeEvent::Attached {
            frame_id: "CHILD-FRAME-ATTACH-ONLY".to_owned(),
            parent_frame_id: "TID-1".to_owned(),
        }],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        "https://example.test".to_owned(),
        "Secure".to_owned(),
    );
    let outputs = super::PagePreparedOutputs {
        javascript_dialogs: Vec::new(),
        window_open_events: Vec::new(),
        popup_activations: Vec::new(),
        document_title_changes: Vec::new(),
        document_lifecycle_events: Vec::new(),
        child_frame_activities: vec![super::PagePreparedChildFrameActivity::from_document(
            root_document_attachment_for_test(&conn, "SID-1", source_document),
            document,
        )],
        same_document_navigations: Vec::new(),
        session_history_updates: Vec::new(),
        top_level_location_navigation: None,
        top_level_history_traversal: None,
    };
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(outputs));
    let owner = crate::conn::CommandOwnerScope::for_session("SID-1");
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let mut context = ProtocolOutputProjectionContext::new(&owner, &mut command_context);

    super::output::project_page_output_async(
        ProtocolOutputSlot::ChildFrameActivity,
        &mut conn,
        &mut context,
        &mut prepared,
    )
    .await;

    let events = context
        .command
        .take_protocol_events()
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.frameAttached"))
            .count(),
        1,
        "prepared child-frame completion must not drop attachment-only tokens"
    );
    assert!(events.iter().any(|message| {
        message["method"] == json!("Page.frameAttached")
            && message["params"]["frameId"] == json!("CHILD-FRAME-ATTACH-ONLY")
            && message["params"]["parentFrameId"] == json!("TID-1")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .all(|message| message["method"] != json!("Page.frameNavigated")),
        "attachment-only child-frame token should not synthesize navigation events"
    );
    assert!(
        conn.runtime_session_owner_slot(Some("SID-1"))
            .expect("runtime owner slot should exist")
            .loaded_page()
            .is_none(),
        "prepared attachment-only emission must not require live page readback"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_child_frame_activity_does_not_follow_replacement_page_residence() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-child-page-owner".into());
    browser_context.set_active_target_id("TID-child-page-owner");
    browser_context.set_target_url("https://example.test/page".to_owned());
    browser_context.attach_active_session("SID-child-page-owner");
    conn.install_browser_context_fixture_for_test(browser_context);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-child-page-owner",
        "TID-child-page-owner",
        source_document,
    );
    let activity = default_prepared_child_frame_activity_for_test(
        &conn,
        "SID-child-page-owner",
        source_document,
    );
    conn.runtime_session_owner_slot_mut(Some("SID-child-page-owner"))
        .expect("test runtime owner")
        .replace_page_attachment_id_for_test();

    let mut events = Vec::new();
    super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

    assert!(
        events.is_empty(),
        "retired Page output must not be projected through its replacement attachment"
    );
    assert!(
        !conn.has_attached_child_frame_id("CHILD-FRAME-1"),
        "retired Page output must not mutate the replacement attached-frame registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_child_frame_activity_does_not_follow_root_document_open_replacement() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-child-root-document".into());
    browser_context.set_active_target_id("TID-child-root-document");
    browser_context.set_target_url("https://example.test/page".to_owned());
    browser_context.attach_active_session("SID-child-root-document");
    conn.install_browser_context_fixture_for_test(browser_context);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-child-root-document",
        "TID-child-root-document",
        source_document,
    );
    let activity = default_prepared_child_frame_activity_for_test(
        &conn,
        "SID-child-root-document",
        source_document,
    );
    bind_renderer_document_for_test(
        &mut conn,
        "SID-child-root-document",
        "TID-child-root-document",
        renderer_document_identity_for_test(2, 2),
    );

    let mut events = Vec::new();
    super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

    assert!(
        events.is_empty(),
        "old root Document output must not appear in the document.open replacement"
    );
    assert!(
        !conn.has_attached_child_frame_id("CHILD-FRAME-1"),
        "old root Document output must not mutate the replacement child-frame registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_child_frame_activity_keeps_root_document_route_until_delivery() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-child-delivery-route".into());
    browser_context.set_active_target_id("TID-child-delivery-route");
    browser_context.set_target_url("https://example.test/page".to_owned());
    browser_context.attach_active_session("SID-child-delivery-route");
    browser_context.active_page_target_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(browser_context);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-child-delivery-route",
        "TID-child-delivery-route",
        source_document,
    );
    let activity = default_prepared_child_frame_activity_for_test(
        &conn,
        "SID-child-delivery-route",
        source_document,
    );

    let mut events = Vec::new();
    super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

    assert!(
        !events.is_empty(),
        "live child activity should produce output"
    );
    assert!(events.iter().all(|event| event.route_is_current(&conn)));

    // Projection and scheduler delivery are separate steps. A replacement
    // root Document may commit between them, so the concrete events must
    // retain their exact Document route instead of inheriting the new
    // target's still-live session.
    bind_renderer_document_for_test(
        &mut conn,
        "SID-child-delivery-route",
        "TID-child-delivery-route",
        renderer_document_identity_for_test(2, 2),
    );

    assert!(
        events.iter().all(|event| !event.route_is_current(&conn)),
        "already-projected child output must not enter the replacement Document's FIFO"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_child_frame_activity_does_not_follow_detached_protocol_session() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-child-session".into());
    browser_context.set_active_target_id("TID-child-session");
    browser_context.set_target_url("https://example.test/page".to_owned());
    browser_context.attach_active_session("SID-child-session");
    conn.install_browser_context_fixture_for_test(browser_context);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-child-session",
        "TID-child-session",
        source_document,
    );
    let activity =
        default_prepared_child_frame_activity_for_test(&conn, "SID-child-session", source_document);
    assert_eq!(
        conn.browser_context
            .as_mut()
            .expect("test browser context")
            .detach_active_session()
            .as_deref(),
        Some("SID-child-session")
    );
    conn.rollback_attached_session_without_event("SID-child-session");

    let mut events = Vec::new();
    super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

    assert!(
        events.is_empty(),
        "held output must not be routed after its exact protocol attachment detaches"
    );
    assert!(
        !conn.has_attached_child_frame_id("CHILD-FRAME-1"),
        "detached-session output must not mutate target-wide child-frame state"
    );
}

#[test]
fn child_frame_tree_emission_deduplicates_attach_and_removes_owner_state_on_detach() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("about:blank".to_owned());
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_domain_enabled = true;
    conn.install_browser_context_fixture_for_test(bc);
    let child_frame_id = "CHILD-FRAME-1".to_owned();
    let mut emitted = Vec::new();
    super::emit_prepared_child_frame_tree_background_events(
        &mut conn,
        &mut emitted,
        Some("SID-1"),
        vec![super::PagePreparedChildFrameTreeEvent::Attached {
            frame_id: child_frame_id.clone(),
            parent_frame_id: "TID-1".to_owned(),
        }],
    );
    assert_eq!(emitted.len(), 1);
    assert!(
        conn.target_owner_state_for_session(Some("SID-1"))
            .expect("owner state should exist")
            .has_attached_child_frame_id(&child_frame_id),
        "initial child-frame attachment should be recorded before the next prepare"
    );
    super::emit_prepared_child_frame_tree_background_events(
        &mut conn,
        &mut emitted,
        Some("SID-1"),
        vec![
            super::PagePreparedChildFrameTreeEvent::Attached {
                frame_id: child_frame_id.clone(),
                parent_frame_id: "TID-1".to_owned(),
            },
            super::PagePreparedChildFrameTreeEvent::Detached {
                frame_id: child_frame_id.clone(),
            },
        ],
    );

    assert_eq!(
        emitted
            .iter()
            .map(BackgroundProtocolEvent::protocol_method)
            .collect::<Vec<_>>(),
        vec![Some("Page.frameAttached"), Some("Page.frameDetached")],
        "duplicate attach should be suppressed without suppressing the following detach"
    );
    assert!(
        !conn
            .target_owner_state_for_session(Some("SID-1"))
            .expect("owner state should exist")
            .has_attached_child_frame_id(&child_frame_id),
        "detach must remove the child frame from the owner state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_activity_drain_requires_prepared_output() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);
    let owner = crate::conn::CommandOwnerScope::for_session("SID-1");
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let mut context = ProtocolOutputProjectionContext::new(&owner, &mut command_context);

    super::output::project_page_output_async(
        ProtocolOutputSlot::ChildFrameActivity,
        &mut conn,
        &mut context,
        &mut ProtocolOutputPayloads::default(),
    )
    .await;

    assert!(
        context.command.take_protocol_events().is_empty(),
        "child-frame completion drain should not emit from live page state without prepared output"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_activation_creates_target_and_schedules_navigation_without_page_readback() {
    let mut conn = CdpConnection::default();
    conn.set_root_target_discovery_enabled(true);
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-1");
    let source_document = renderer_document_identity_for_test(1, 1);
    let mut out = Vec::new();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_popup_activations_for_test(
                page_owner,
                vec![RendererPendingPopupActivation::window(
                    source_document,
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    None,
                    "data:text/html,%3Cmain%3Eprepared-popup%3C/main%3E".to_owned(),
                    "_blank".to_owned(),
                    RendererPopupDisposition::Background,
                )],
            ),
        ));

    super::emit_popup_activity_background_events_async(&mut conn, &mut out, &mut prepared).await;

    let events = out
        .into_iter()
        .map(BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    let (target_created, target_created_sidecar) = events
        .iter()
        .find(|(message, _)| message["method"] == json!("Target.targetCreated"))
        .unwrap_or_else(|| panic!("missing Target.targetCreated event: {events:?}"));
    assert_eq!(
        target_created["params"]["targetInfo"]["url"],
        json!("data:text/html,%3Cmain%3Eprepared-popup%3C/main%3E")
    );
    assert!(matches!(
        target_created_sidecar,
        Some(AutomationEvent::TargetCreated(event))
            if event.url == "data:text/html,%3Cmain%3Eprepared-popup%3C/main%3E"
    ));
    assert!(
        events
            .iter()
            .all(|(message, _)| message["method"] != json!("Page.frameNavigated")),
        "the opener action must not join the popup Page stream; its concrete commit is published by that stream: {events:?}"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .unwrap()
            .background_target_count(),
        1,
        "prepared popup should create the owner popup target without reading a loaded page"
    );
    assert!(
        conn.browser_context
            .as_ref()
            .unwrap()
            .background_targets()
            .next()
            .and_then(|target| target.loaded_page())
            .is_some_and(|page| moli_url::is_about_blank(page.final_url())),
        "target creation should install only the initial empty Document"
    );
    let scheduler_events = conn.take_scheduler_events();
    assert!(matches!(
        scheduler_events.as_slice(),
        [crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }]
            if work.kind()
                == crate::domains::activity::ProtocolSchedulerWorkKind::PopupTargetNavigationOwnerAction
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_activation_publishes_automation_lifecycle_without_cdp_discovery() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-automation".into());
    bc.set_active_target_id("TID-opener");
    bc.attach_active_session("SID-opener");
    conn.install_browser_context_fixture_for_test(bc);
    let page_owner = page_residence_identity_for_test(&mut conn, "SID-opener");
    let source_document = renderer_document_identity_for_test(1, 1);
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_popup_activations_for_test(
                page_owner,
                vec![RendererPendingPopupActivation::window(
                    source_document,
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    None,
                    "data:text/html,%3Cmain%3Eautomation-popup%3C/main%3E".to_owned(),
                    "_blank".to_owned(),
                    RendererPopupDisposition::Background,
                )],
            ),
        ));
    let mut out = Vec::new();

    super::emit_popup_activity_background_events_async(&mut conn, &mut out, &mut prepared).await;

    let events = out
        .into_iter()
        .map(BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .all(|(message, _)| message["method"] != json!("Target.targetCreated")),
        "CDP discovery must remain the gate for Target.targetCreated: {events:?}"
    );
    assert!(
        events.iter().any(|(_, event)| {
            matches!(
                event,
                Some(AutomationEvent::TargetCreated(event))
                    if event.url
                        == "data:text/html,%3Cmain%3Eautomation-popup%3C/main%3E"
            )
        }),
        "popup creation must publish its internal browsing-context lifecycle fact even without CDP discovery: {events:?}"
    );
}

#[test]
#[should_panic(expected = "popup activation must not carry an existing-context special target")]
fn popup_carrier_rejects_existing_context_special_targets() {
    let _ = RendererPendingPopupActivation::window(
        renderer_document_identity_for_test(1, 1),
        RendererWindowDocumentSource::RootFrame,
        true,
        None,
        "https://example.test/self".to_owned(),
        "_self".to_owned(),
        RendererPopupDisposition::Background,
    );
}

async fn emit_committed_history_and_navigation_for_test(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    prepared: &mut ProtocolOutputPayloads,
) {
    // Production sends a separate committed-history action before the
    // observer notification. Exercise both projections and their authority.
    if let Some(slot) = prepared.page_mut() {
        slot.outputs.session_history_updates = slot
            .outputs
            .same_document_navigations
            .iter()
            .map(|navigation| {
                (
                    navigation.owner().clone(),
                    navigation.source_document(),
                    moli_page_types::SessionHistoryUpdate {
                        position: moli_session_history::SessionHistoryPosition::INITIAL,
                        update: moli_page_types::SessionHistoryUpdateKind::Push,
                        root_url: navigation.clone().into_navigation().url,
                        root_entry_steps: vec![0],
                    },
                )
            })
            .collect();
    }
    let mut command = crate::conn::CommandDispatchContext::default();
    let mut context = ProtocolOutputProjectionContext::new(owner, &mut command);
    super::output::project_page_output_async(
        ProtocolOutputSlot::SessionHistoryUpdate,
        conn,
        &mut context,
        prepared,
    )
    .await;
    super::emit_same_document_navigation_activity_background_events_async(
        conn, out, owner, prepared,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_drain_consumes_prepared_navigations_without_page_readback() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("https://example.test/page".to_owned());
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
    let mut out = Vec::new();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_same_document_navigations_for_test(
                page_residence_identity_for_test(&mut conn, "SID-1"),
                vec![document_sourced_same_document_navigation_for_test(
                    source_document,
                    "https://example.test/page#prepared",
                )],
            ),
        ));

    emit_committed_history_and_navigation_for_test(
        &mut conn,
        &mut out,
        &CommandOwnerScope::for_session("SID-1"),
        &mut prepared,
    )
    .await;

    assert!(
        conn.runtime_session_owner_slot(Some("SID-1"))
            .expect("runtime owner slot should exist")
            .loaded_page()
            .is_none(),
        "prepared same-document navigation emission must not require a loaded page"
    );
    assert_eq!(out.len(), 1);
    assert!(
        out[0].protocol_message().is_none(),
        "same-document navigation should stay typed until wire projection"
    );
    assert_eq!(
        out[0].protocol_method(),
        Some("Page.navigatedWithinDocument")
    );
    assert!(out[0].has_protocol_wire_message());
    let events = out
        .into_iter()
        .map(BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0["method"], json!("Page.navigatedWithinDocument"));
    assert_eq!(events[0].0["params"]["frameId"], json!("TID-1"));
    assert_eq!(
        events[0].0["params"]["url"],
        json!("https://example.test/page#prepared")
    );
    assert_eq!(events[0].0["params"]["navigationType"], json!("fragment"));
    assert_eq!(events[0].0["sessionId"], json!("SID-1"));
    assert!(matches!(
        events[0].1.as_ref(),
        Some(AutomationEvent::SameDocumentNavigation(event))
            if event.frame_id.as_str() == "TID-1"
                && event.url == "https://example.test/page#prepared"
                && event.navigation_type == "fragment"
    ));
    assert_eq!(
        conn.browser_context.as_ref().unwrap().target_url(),
        "https://example.test/page#prepared",
        "prepared same-document navigation should still update owner URL state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_replacement_keeps_same_document_navigation_handoff() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-document-open-same-document".into());
    bc.set_active_target_id("TID-document-open-same-document");
    bc.set_target_url("https://example.test/source".to_owned());
    bc.attach_active_session("SID-document-open-same-document");
    conn.install_browser_context_fixture_for_test(bc);

    let source_document = renderer_document_identity_for_test(1, 1);
    let replacement_document = renderer_document_identity_for_test(2, 2);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-document-open-same-document",
        "TID-document-open-same-document",
        source_document,
    );
    let owner = page_residence_identity_for_test(&mut conn, "SID-document-open-same-document");
    bind_renderer_document_for_test(
        &mut conn,
        "SID-document-open-same-document",
        "TID-document-open-same-document",
        replacement_document,
    );
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_same_document_navigations_for_test(
                owner,
                vec![document_sourced_same_document_navigation_for_test(
                    source_document,
                    "https://example.test/source#preserved",
                )],
            ),
        ));
    let mut out = Vec::new();

    emit_committed_history_and_navigation_for_test(
        &mut conn,
        &mut out,
        &CommandOwnerScope::for_session("SID-document-open-same-document"),
        &mut prepared,
    )
    .await;

    assert_eq!(
        out.len(),
        1,
        "document.open must not erase prior history output"
    );
    assert_eq!(
        conn.browser_context.as_ref().unwrap().target_url(),
        "https://example.test/source#preserved",
        "same-Document history mutation survives replacement of only the Document shell"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_page_residence_same_document_navigation_cannot_mutate_replacement() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-stale-page-same-document".into());
    bc.set_active_target_id("TID-stale-page-same-document");
    bc.set_target_url("https://example.test/replacement".to_owned());
    bc.attach_active_session("SID-stale-page-same-document");
    conn.install_browser_context_fixture_for_test(bc);

    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-stale-page-same-document",
        "TID-stale-page-same-document",
        source_document,
    );
    let owner = page_residence_identity_for_test(&mut conn, "SID-stale-page-same-document");
    conn.runtime_session_owner_slot_mut(Some("SID-stale-page-same-document"))
        .expect("test runtime slot should exist")
        .replace_page_attachment_id_for_test();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_same_document_navigations_for_test(
                owner,
                vec![document_sourced_same_document_navigation_for_test(
                    source_document,
                    "https://example.test/replacement#stale",
                )],
            ),
        ));
    let mut out = Vec::new();

    emit_committed_history_and_navigation_for_test(
        &mut conn,
        &mut out,
        &CommandOwnerScope::for_session("SID-stale-page-same-document"),
        &mut prepared,
    )
    .await;

    assert!(
        out.is_empty(),
        "a retired Page residence must emit no event"
    );
    assert_eq!(
        conn.browser_context.as_ref().unwrap().target_url(),
        "https://example.test/replacement",
        "a retired Page's output must not update replacement target state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_top_level_location_navigation_waits_for_its_scheduler_turn() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-location".into());
    bc.set_active_target_id("TID-location");
    bc.set_target_url("about:blank".to_owned());
    bc.attach_active_session("SID-location");
    conn.install_browser_context_fixture_for_test(bc);
    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(&mut conn, "SID-location", "TID-location", source_document);

    let target_url = "data:text/html,%3Cmain%3Eprepared-location%3C/main%3E".to_owned();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                page_residence_identity_for_test(&mut conn, "SID-location"),
                Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                    source_document,
                    target_url.clone(),
                )),
            ),
        ));

    super::publish_prepared_top_level_location_navigation_owner_action(
        &mut conn,
        &CommandOwnerScope::for_session("SID-location"),
        &mut prepared,
    );

    assert_eq!(
        conn.browser_context.as_ref().unwrap().target_url(),
        "about:blank",
        "capturing prepared output must not execute its owner action"
    );
    assert!(
        !conn.has_pending_document_navigation_for_session_owner(Some("SID-location")),
        "capturing prepared output must not start navigation"
    );

    let work = take_top_level_location_navigation_work_for_test(&mut conn);
    let (events, scheduler_events) = conn
        .complete_ready_protocol_scheduler_work_turn(work)
        .await
        .into_protocol_event_parts();
    assert!(!scheduler_events.iter().any(|event| {
        matches!(
            event,
            crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                if work.is_top_level_location_navigation_owner_action()
        )
    }));
    let events = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    let (message, automation_event) = events
        .iter()
        .find(|(message, _)| message["method"] == json!("Page.frameStartedNavigating"))
        .expect("prepared top-level location navigation should emit frameStartedNavigating");

    assert_eq!(message["sessionId"], json!("SID-location"));
    assert_eq!(message["params"]["frameId"], json!("TID-location"));
    assert_eq!(message["params"]["loaderId"], json!(super::LOADER_ID));
    assert_eq!(message["params"]["url"], json!(target_url));
    assert!(matches!(
        automation_event,
        Some(AutomationEvent::NavigationFrame(event))
            if event.kind == NavigationFrameEventKind::StartedNavigating
                && event.frame_id.as_str() == "TID-location"
                && event.loader_id.as_ref().map(|id| id.as_str()) == Some(super::LOADER_ID)
                && event.url == target_url
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_replacement_keeps_requested_top_level_navigation() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-document-open-location".into());
    bc.set_active_target_id("TID-document-open-location");
    bc.set_target_url("https://example.test/source".to_owned());
    bc.attach_active_session("SID-document-open-location");
    conn.install_browser_context_fixture_for_test(bc);

    let source_document = renderer_document_identity_for_test(1, 1);
    let replacement_document = renderer_document_identity_for_test(2, 2);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-document-open-location",
        "TID-document-open-location",
        source_document,
    );
    let owner = page_residence_identity_for_test(&mut conn, "SID-document-open-location");
    bind_renderer_document_for_test(
        &mut conn,
        "SID-document-open-location",
        "TID-document-open-location",
        replacement_document,
    );
    let target_url = "data:text/html,%3Cmain%3Epreserved%3C/main%3E".to_owned();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                owner,
                Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                    source_document,
                    target_url.clone(),
                )),
            ),
        ));
    super::publish_prepared_top_level_location_navigation_owner_action(
        &mut conn,
        &CommandOwnerScope::for_session("SID-document-open-location"),
        &mut prepared,
    );
    let work = take_top_level_location_navigation_work_for_test(&mut conn);
    let (out, scheduler_events) = conn
        .complete_ready_protocol_scheduler_work_turn(work)
        .await
        .into_protocol_event_parts();
    assert!(!scheduler_events.iter().any(|event| {
        matches!(
            event,
            crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                if work.is_top_level_location_navigation_owner_action()
        )
    }));

    assert!(
        out.iter().any(|event| {
            event.protocol_method() == Some("Page.frameStartedNavigating")
                && event.protocol_message().is_none()
        }),
        "document.open must not cancel a navigation already requested by the same Page"
    );
    // A data: navigation may complete and clear its pending token before
    // this helper returns; the typed frame-start event proves the action
    // was admitted rather than discarded as a stale Document.
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_page_residence_top_level_navigation_cannot_replace_current_page() {
    let mut conn = CdpConnection::default();
    let mut bc = BrowserContext::new("BID-stale-page-location".into());
    bc.set_active_target_id("TID-stale-page-location");
    bc.set_target_url("https://example.test/replacement".to_owned());
    bc.attach_active_session("SID-stale-page-location");
    conn.install_browser_context_fixture_for_test(bc);

    let source_document = renderer_document_identity_for_test(1, 1);
    bind_renderer_document_for_test(
        &mut conn,
        "SID-stale-page-location",
        "TID-stale-page-location",
        source_document,
    );
    let owner = page_residence_identity_for_test(&mut conn, "SID-stale-page-location");
    conn.runtime_session_owner_slot_mut(Some("SID-stale-page-location"))
        .expect("test runtime slot should exist")
        .replace_page_attachment_id_for_test();
    let mut prepared =
        ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
            super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                owner,
                Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                    source_document,
                    "data:text/html,%3Cmain%3Estale%3C/main%3E".to_owned(),
                )),
            ),
        ));
    super::publish_prepared_top_level_location_navigation_owner_action(
        &mut conn,
        &CommandOwnerScope::for_session("SID-stale-page-location"),
        &mut prepared,
    );
    let work = take_top_level_location_navigation_work_for_test(&mut conn);
    let (out, scheduler_events) = conn
        .complete_ready_protocol_scheduler_work_turn(work)
        .await
        .into_protocol_event_parts();
    assert!(scheduler_events.is_empty());

    assert!(out.is_empty(), "a retired Page must start no navigation");
    assert_eq!(
        conn.browser_context.as_ref().unwrap().target_url(),
        "https://example.test/replacement",
        "a retired Page's action must not navigate its replacement"
    );
    assert!(
        !conn.has_pending_document_navigation_for_session_owner(Some("SID-stale-page-location")),
        "discarding the retired Page action must install no navigation token"
    );
}

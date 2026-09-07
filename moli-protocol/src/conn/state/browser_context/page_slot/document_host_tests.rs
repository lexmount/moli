use super::*;
use moli_core::runtime::{Browser, BrowserConfig};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

const TARGET: &str = "TID-dialog-owner";

async fn prepare_navigation(
    owner: &BrowserContext,
    navigation: NavigationId,
    page: Page,
    url: url::Url,
    artifacts: &RendererPageCreationArtifacts,
) -> crate::conn::PreparedDocumentNavigation {
    owner
        .start_loaded_document_navigation_for_target(
            TARGET,
            navigation,
            page,
            crate::conn::DocumentNavigationDestination {
                url,
                security_origin: "null".into(),
                secure_context_type: "InsecureScheme".into(),
            },
            artifacts,
            &Default::default(),
        )
        .unwrap()
        .await
        .unwrap()
}

fn empty_document_context() -> BrowserContext {
    let mut owner = BrowserContext::new("BID-dialog-owner".into());
    owner.set_active_target_id(TARGET);
    owner
}

fn context_with_document(page: Page) -> BrowserContext {
    let mut owner = empty_document_context();
    assert!(
        owner
            .replace_target_page_for_test(TARGET, Some(page))
            .is_none()
    );
    owner
}

#[tokio::test]
async fn loaded_navigation_commit_settles_document_history_and_navigation_together() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let mut owner = context_with_document(first);
    let old_document = owner.target_document_id(TARGET).unwrap();
    let token = owner.begin_target_document_navigation(TARGET, "LOADER-atomic".into());
    let expected_document = owner
        .web_contents_for_target(TARGET)
        .unwrap()
        .navigation()
        .pending_document()
        .unwrap()
        .1;
    let mut page = browser
        .fetch("data:text/html,<title>second</title>")
        .await
        .unwrap();
    let url = page.final_url().clone();
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let prepared = prepare_navigation(&owner, token, page, url.clone(), &artifacts).await;
    let committed = owner.commit_loaded_navigation(prepared).unwrap();
    assert!(committed.inspection_projection.is_ok());
    assert_eq!(
        owner.target_document_id(TARGET),
        Some(expected_document),
        "commit must install the Document allocated by this Browser navigation"
    );
    assert_ne!(expected_document, old_document);
    assert!(
        !owner.has_pending_document_navigation_for_target(TARGET),
        "returning a committed Page must not leave its Browser navigation pending"
    );
    assert_eq!(
        owner.committed_document_loader_id_for_target(TARGET),
        Some("LOADER-atomic")
    );
    let (_, entries) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(entries.last().unwrap().url, url.as_str());
    assert_eq!(entries.last().unwrap().title, "second");
    assert_eq!(
        owner.page_targets.get(TARGET).unwrap().target_url(),
        url.as_str()
    );
    assert!(
        owner
            .document_lifecycle_for_target(TARGET)
            .unwrap()
            .snapshot()
            .is_some()
    );
    committed.previous_document_retirement.close().await;
}

#[tokio::test]
async fn disappearing_agent_host_cannot_cancel_an_admitted_browser_commit() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut owner = empty_document_context();
    let navigation = owner.begin_target_document_navigation(TARGET, "LOADER-native".into());
    let contents_id = owner.page_targets.get(TARGET).unwrap().web_contents_id();
    let expected_document = owner.physical.web_contents[&contents_id]
        .navigation()
        .pending_document()
        .unwrap()
        .1;
    let mut page = browser
        .fetch("data:text/html,<title>native</title>")
        .await
        .unwrap();
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let url = page.final_url().clone();
    let prepared = prepare_navigation(&owner, navigation, page, url.clone(), &artifacts).await;

    drop(owner.page_targets.remove(TARGET).unwrap());
    let committed = owner.commit_loaded_navigation(prepared).unwrap();
    assert!(committed.inspection_projection.is_err());
    let contents = owner.physical.web_contents.get_mut(&contents_id).unwrap();
    let document = contents.main_frame.current_document.as_ref().unwrap();
    assert_eq!(
        document.lifecycle.snapshot(),
        Some(artifacts.lifecycle_snapshot)
    );
    assert!(document.lifecycle.snapshot().unwrap().load.is_some());
    let renderer_page = RendererPageResidenceIdentity::from_page(&document.page);
    assert_eq!(
        contents.main_frame.current_document.as_ref().unwrap().id,
        expected_document
    );
    assert_eq!(contents.navigation().pending_document(), None);
    assert_eq!(
        contents.navigation().committed_document_navigation(),
        Some(navigation)
    );
    let (_, history) = contents.navigation().navigation_history_snapshot();
    assert_eq!(history.last().unwrap().url, url.as_str());
    assert_eq!(history.last().unwrap().title, "native");
    assert_eq!(
        contents
            .main_frame
            .current_document
            .as_mut()
            .unwrap()
            .page
            .evaluate_runtime_expression_async("40 + 2")
            .await
            .unwrap()["value"],
        42
    );
    let snapshot = artifacts.lifecycle_snapshot;
    let event = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: u64::MAX,
        timestamp_micros: 100,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(RendererDocumentLifecycleMilestone::Load),
            reason: moli_core::page::RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    assert!(
        owner
            .apply_renderer_document_lifecycle(
                renderer_page,
                RendererDocumentLifecycleEvent {
                    document: event.document.successor_for_testing(),
                    ..event
                }
            )
            .is_none()
    );
    let occurrence = owner
        .apply_renderer_document_lifecycle(renderer_page, event)
        .unwrap();
    assert_eq!(occurrence.document(), expected_document);
    assert_eq!(occurrence.event(), event);
    assert!(
        owner
            .apply_renderer_document_lifecycle(renderer_page, event)
            .is_none()
    );
    assert_eq!(
        owner.physical.web_contents[&contents_id]
            .main_frame
            .current_document
            .as_ref()
            .unwrap()
            .lifecycle
            .snapshot()
            .unwrap()
            .terminated
            .unwrap()
            .sequence,
        event.sequence
    );
    committed.previous_document_retirement.close().await;
}

#[tokio::test]
async fn creation_projection_cannot_rewind_native_progress_or_retarget_a_replacement() {
    use crate::conn::CommittedDocumentLifecycle;
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut owner = empty_document_context();
    let navigation = owner.begin_target_document_navigation(TARGET, "LOADER-native".into());
    let mut page = browser
        .fetch("data:text/html,<title>native</title>")
        .await
        .unwrap();
    let renderer_page = RendererPageResidenceIdentity::from_page(&page);
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let snapshot = artifacts.lifecycle_snapshot;
    assert!(snapshot.load.is_some());
    let url = page.final_url().clone();
    let candidate = prepare_navigation(&owner, navigation, page, url, &artifacts).await;
    let commit = owner.commit_loaded_navigation(candidate).unwrap();
    let document = commit.lifecycle.document;
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );

    let terminated = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: u64::MAX - 1,
        timestamp_micros: 100,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(RendererDocumentLifecycleMilestone::Load),
            reason: moli_core::page::RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    let occurrence = owner
        .apply_renderer_document_lifecycle(renderer_page, terminated)
        .unwrap();
    let native = owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET);
    // Projection is delayed until after the Browser has accepted more progress.
    // Replaying/rebinding the creation occurrence may only affect visibility.
    let browser_sequence = moli_core::browser::BrowserSequence::allocate();
    for _ in 0..2 {
        let projected = owner.project_committed_document_lifecycle_for_target(
            TARGET,
            CommittedDocumentLifecycle {
                document,
                browser_sequence,
                artifacts: artifacts.clone(),
            },
            Some(navigation),
            TARGET.into(),
            "LOADER-native".into(),
        );
        assert_eq!(projected, artifacts.initial_lifecycle_events);
        assert_eq!(
            owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
            native
        );
        assert_eq!(
            owner
                .page_slot_for_target(TARGET)
                .unwrap()
                .renderer_document_lifecycle_visible_snapshot(),
            Some(snapshot)
        );
    }
    assert_eq!(
        owner.project_renderer_document_lifecycle_events_for_target(
            TARGET,
            vec![occurrence.event()]
        ),
        vec![terminated]
    );
    assert_eq!(
        owner
            .page_slot_for_target(TARGET)
            .unwrap()
            .renderer_document_lifecycle_visible_snapshot(),
        native
    );

    let next = owner.begin_target_document_navigation(TARGET, "LOADER-next".into());
    let mut page = browser.fetch("data:text/html,next").await.unwrap();
    let url = page.final_url().clone();
    let next_artifacts = page.take_page_creation_artifacts().unwrap();
    let candidate = prepare_navigation(&owner, next, page, url, &next_artifacts).await;
    let replacement = owner.commit_loaded_navigation(candidate).unwrap();
    assert_ne!(
        owner.target_document_id(TARGET),
        Some(occurrence.document())
    );
    assert!(
        owner
            .apply_renderer_document_lifecycle(renderer_page, terminated)
            .is_none()
    );
    assert!(
        owner
            .project_committed_document_lifecycle_for_target(
                TARGET,
                commit.lifecycle,
                Some(navigation),
                TARGET.into(),
                "LOADER-native".into(),
            )
            .is_empty()
    );
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );
    assert_eq!(
        owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        Some(next_artifacts.lifecycle_snapshot)
    );
    commit.previous_document_retirement.close().await;
    replacement.previous_document_retirement.close().await;
}

#[tokio::test]
async fn failed_inspection_projection_cannot_veto_browser_document_commit() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let mut owner = context_with_document(first);
    let previous_document = owner.target_document_id(TARGET).unwrap();
    let navigation = owner.begin_target_document_navigation(TARGET, "LOADER-projection".into());
    let expected_document = owner
        .web_contents_for_target(TARGET)
        .unwrap()
        .navigation()
        .pending_document()
        .unwrap()
        .1;
    // Fail only the DevTools projection. The Browser WebContents and its pending
    // navigation remain live and must not require that projection's approval.
    owner
        .page_targets
        .get_mut(TARGET)
        .unwrap()
        .runtime_slot
        .retire_for_target_close();
    let mut page = browser
        .fetch("data:text/html,<title>committed</title>")
        .await
        .unwrap();
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let url = page.final_url().clone();
    let prepared = prepare_navigation(&owner, navigation, page, url.clone(), &artifacts).await;
    let result = owner.commit_loaded_navigation(prepared);

    assert_eq!(
        owner.target_document_id(TARGET),
        Some(expected_document),
        "a closed DevTools channel must not veto the Browser commit"
    );
    assert_ne!(expected_document, previous_document);
    assert!(!owner.has_pending_document_navigation_for_target(TARGET));
    let (_, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(history.last().unwrap().url, url.as_str());
    assert_eq!(history.last().unwrap().title, "committed");
    assert_eq!(
        owner
            .loaded_page_for_target_mut(TARGET)
            .unwrap()
            .evaluate_runtime_expression_async("40 + 2")
            .await
            .unwrap()["value"],
        42
    );
    let commit = result.expect("Browser commit is independent of its projection");
    assert!(commit.inspection_projection.is_err());
    commit.previous_document_retirement.close().await;
}

#[tokio::test]
async fn committed_occurrence_retains_the_previous_document_output_projection() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let artifacts = first.take_page_creation_artifacts().unwrap();
    let previous_renderer = moli_core::browser::RendererPageResidenceIdentity::from_page(&first);
    let mut owner = context_with_document(first);
    let previous_document = owner.target_document_id(TARGET).unwrap();
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        TARGET.into(),
        "LOADER-first".into(),
    );
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_some()
    );
    let navigation = owner.begin_target_document_navigation(TARGET, "LOADER-current".into());
    let mut page = browser
        .fetch("data:text/html,<title>current</title>")
        .await
        .unwrap();
    let current_renderer = moli_core::browser::RendererPageResidenceIdentity::from_page(&page);
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let url = page.final_url().clone();
    let prepared = prepare_navigation(&owner, navigation, page, url, &artifacts).await;
    let committed = owner.commit_loaded_navigation(prepared).unwrap();
    let current_document = owner.target_document_id(TARGET).unwrap();
    assert_ne!(current_document, previous_document);
    let projection = &owner.page_targets.get(TARGET).unwrap().runtime_slot;
    assert!(
        projection.routes_retiring_renderer_page_owner(previous_renderer, previous_document),
        "post-commit retirement must match the occurrence's old Document, not the new current identity"
    );
    assert!(!projection.routes_retiring_renderer_page_owner(current_renderer, current_document));
    committed.previous_document_retirement.close().await;
}

#[tokio::test]
async fn inspection_configuration_failure_cannot_roll_back_a_committed_browser_document() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let mut owner = context_with_document(first);
    let navigation = owner.begin_target_document_navigation(TARGET, "LOADER-restore".into());
    let mut page = browser.fetch("data:text/html,<title>committed</title><script>Object.defineProperty(globalThis,'protectedBinding',{value:1,configurable:false})</script>").await.unwrap();
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let url = page.final_url().clone();
    let prepared = prepare_navigation(&owner, navigation, page, url.clone(), &artifacts).await;
    let committed = owner.commit_loaded_navigation(prepared).unwrap();
    assert!(committed.inspection_projection.is_ok());
    let document = owner.target_document_id(TARGET);
    let registration = moli_core::page::RuntimeBindingRegistration {
        devtools_session: None,
        name: "protectedBinding".into(),
        execution_context_name: None,
    };
    let pending = owner
        .page_targets
        .get(TARGET)
        .unwrap()
        .runtime_slot
        .current_renderer_inspection_binding()
        .unwrap()
        .runtime_inspection(None)
        .start_apply_runtime_protocol_state(
            &[],
            &[],
            std::slice::from_ref(&registration),
            std::slice::from_ref(&registration),
        )
        .unwrap();
    let error = moli_core::page::PendingPageCommand::from_inspector_main_route(pending)
        .wait()
        .await
        .and_then(|completion| completion.into_unit_page_command_turn())
        .map(|_| ())
        .expect_err("a non-configurable global must reject binding installation");
    assert!(error.to_string().contains("runtime binding"), "{error:#}");
    assert_eq!(owner.target_document_id(TARGET), document);
    assert!(!owner.has_pending_document_navigation_for_target(TARGET));
    let (_, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(history.last().unwrap().url, url.as_str());
    assert_eq!(history.last().unwrap().title, "committed");
    assert_eq!(
        owner
            .loaded_page_for_target_mut(TARGET)
            .unwrap()
            .evaluate_runtime_expression_async("protectedBinding + 41")
            .await
            .unwrap()["value"],
        42
    );
    committed.previous_document_retirement.close().await;
}

#[tokio::test]
async fn rejected_browser_candidate_cannot_rotate_inspection_or_document_projection() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let mut owner = context_with_document(first);
    let document = owner.target_document_id(TARGET);
    let attachment = owner
        .page_targets
        .get(TARGET)
        .unwrap()
        .runtime_slot
        .current_renderer_attachment();
    let stale = owner.begin_target_document_navigation(TARGET, "LOADER-stale".into());
    let mut page = browser
        .fetch("data:text/html,<title>stale</title>")
        .await
        .unwrap();
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let url = page.final_url().clone();
    let prepared = prepare_navigation(&owner, stale, page, url, &artifacts).await;
    let current = owner.begin_target_document_navigation(TARGET, "LOADER-current".into());
    let history = owner.target_navigation_history_snapshot(TARGET).unwrap();
    let target_url = owner
        .page_targets
        .get(TARGET)
        .unwrap()
        .target_url()
        .to_owned();
    assert!(owner.commit_loaded_navigation(prepared).is_err());
    assert_eq!(owner.target_document_id(TARGET), document);
    assert_eq!(
        owner
            .web_contents_for_target(TARGET)
            .unwrap()
            .navigation()
            .pending_document()
            .unwrap()
            .0,
        current
    );
    let projection = owner.page_targets.get(TARGET).unwrap();
    assert_eq!(
        projection.runtime_slot.current_renderer_attachment(),
        attachment
    );
    assert!(projection.runtime_slot.has_renderer_navigation(&current));
    assert_eq!(projection.target_url(), target_url);
    assert_eq!(
        owner.target_navigation_history_snapshot(TARGET).unwrap(),
        history
    );
}

#[tokio::test]
async fn document_replacement_updates_inspection_binding_with_physical_page() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let second = browser
        .fetch("data:text/html,<title>second</title>")
        .await
        .unwrap();
    let agent = second.renderer_devtools_agent_token();
    let mut owner = context_with_document(first);
    let old_attachment = owner
        .active_page_target()
        .runtime_slot
        .current_renderer_attachment()
        .unwrap();
    let previous = owner
        .replace_loaded_page_for_target(TARGET, Some(second))
        .unwrap();
    let attachment = owner
        .active_page_target()
        .runtime_slot
        .current_renderer_attachment()
        .unwrap();
    assert_eq!(attachment.agent_token(), agent);
    assert_ne!(attachment.id(), old_attachment.id());
    assert_eq!(
        owner
            .loaded_page_for_target(TARGET)
            .unwrap()
            .renderer_devtools_agent_token(),
        attachment.agent_token()
    );
    assert!(
        owner
            .active_page_target()
            .runtime_slot
            .current_renderer_inspection_binding()
            .is_some()
    );
    drop(previous);
}

async fn page_with_installed_dialog_for_test(
    browser: &Browser,
) -> (
    BrowserContext,
    moli_core::page::RendererJavaScriptDialogCompletion,
) {
    use moli_core::page::{
        RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
    };

    let mut page = browser
        .fetch("data:text/html,<p>dialog owner</p>")
        .await
        .unwrap();
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let source = artifacts.lifecycle_snapshot;
    let mut owner = context_with_document(page);
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        "FRAME-dialog-owner".into(),
        "loader".into(),
    );
    owner.attach_active_session("SID-dialog-owner");
    let completion = RendererJavaScriptDialogCompletion::pending();
    assert!(owner.install_javascript_dialog_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        crate::conn::TargetPageResidenceIdentity::new(
            "BID-dialog-owner".into(),
            Some("TID-dialog-owner".into()),
            owner.target_document_id(TARGET).unwrap(),
        ),
        "FRAME-dialog-owner".into(),
        RendererPendingJavaScriptDialog::new(
            RendererJavaScriptDialogId::new(1),
            RendererDocumentLifecycleIdentity {
                frame: source.frame,
                document: source.document,
                epoch: source.epoch,
            },
            RendererJavaScriptDialogSource::RootFrame,
            "about:blank".into(),
            "prompt".into(),
            "owned dialog".into(),
            "default".into(),
            Some(completion.clone()),
        ),
    ));
    (owner, completion)
}

#[tokio::test]
async fn document_replacement_dismisses_dialog_without_protocol_session_cleanup() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let (mut owner, completion) = page_with_installed_dialog_for_test(&browser).await;
    let page = browser
        .fetch("data:text/html,<p>replacement</p>")
        .await
        .unwrap();
    let previous = owner
        .replace_loaded_page_for_target(TARGET, Some(page))
        .unwrap();

    assert!(
        !completion.finish(true, "late reply".into()),
        "Browser Document replacement must dismiss its dialog before Protocol cleanup"
    );
    assert!(!completion.wait().accepted);
    previous.close_async().await.unwrap();
}

#[tokio::test]
async fn browser_drop_dismisses_dialog_even_when_session_snapshot_survives() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let (mut owner, completion) = page_with_installed_dialog_for_test(&browser).await;
    let dialog_projection_snapshot = owner.active_page_target().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .javascript_dialog_state
        .clone();
    let id = owner.selected_web_contents_id().unwrap();
    drop(owner.page_targets.remove(TARGET).unwrap());
    let contents = owner.physical.web_contents.get(&id).unwrap();
    assert!(contents.main_frame.current_document.is_some());
    assert!(!contents.javascript_dialogs.is_empty());
    drop(owner);

    assert!(
        !completion.finish(true, "late reply".into()),
        "Browser drop must dismiss the dialog even if a dialog projection snapshot survives"
    );
    assert!(!completion.wait().accepted);
    drop(dialog_projection_snapshot);
}

#[tokio::test]
async fn browser_dialog_can_be_handled_after_protocol_projection_is_dropped() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let (mut owner, completion) = page_with_installed_dialog_for_test(&browser).await;
    let key = owner.active_page_target().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .javascript_dialog_state
        .pending_dialogs()[0]
        .key;
    let id = owner.selected_web_contents_id().unwrap();
    drop(owner.page_targets.remove(TARGET).unwrap());
    let contents = owner.physical.web_contents.get_mut(&id).unwrap();

    assert_eq!(
        contents.javascript_dialogs.snapshot(key).unwrap().message,
        "owned dialog"
    );
    contents
        .javascript_dialogs
        .set_prompt_text(key, "Browser input".into())
        .unwrap();
    let closed = contents.javascript_dialogs.finish(key, true, None).unwrap();
    assert_eq!(closed.dialog_type, "prompt");
    assert_eq!(closed.user_input, "Browser input");
    assert!(contents.javascript_dialogs.snapshot(key).is_none());
    assert!(
        contents
            .javascript_dialogs
            .finish(key, false, None)
            .is_none()
    );
    assert!(!completion.finish(false, "late reply".into()));
    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "Browser input");
}

#[tokio::test]
async fn browser_dialog_retirement_follows_admitted_document_lifecycle_without_projection() {
    use moli_core::page::RendererDocumentTerminationReason;
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let (mut owner, completion) = page_with_installed_dialog_for_test(&browser).await;
    let contents_id = owner.selected_web_contents_id().unwrap();
    drop(owner.page_targets.remove(TARGET).unwrap());
    let contents = owner.physical.web_contents.get_mut(&contents_id).unwrap();
    let document = contents.main_frame.current_document.as_ref().unwrap();
    let id = document.id;
    let snapshot = document.lifecycle.snapshot().unwrap();
    contents.begin_initial_empty_document("about:blank".into(), None, None);
    contents.mark_initial_empty_document_materialized();
    assert!(!contents.javascript_dialogs.is_empty());
    let terminated = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: u64::MAX - 2,
        timestamp_micros: 10,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: None,
            reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    assert!(
        contents
            .apply_document_lifecycle(RendererDocumentLifecycleEvent {
                document: snapshot.document.successor_for_testing(),
                ..terminated
            })
            .is_none()
    );
    assert!(
        !contents.javascript_dialogs.is_empty(),
        "foreign lifecycle must not dismiss current dialog"
    );
    assert!(contents.apply_document_lifecycle(terminated).is_some());
    assert!(contents.javascript_dialogs.is_empty());
    assert!(!completion.finish(true, "late reply".into()));
    assert!(!completion.wait().accepted);
    assert!(
        contents
            .apply_document_lifecycle(RendererDocumentLifecycleEvent {
                epoch: RendererLifecycleEpoch(snapshot.epoch.0 + 1),
                sequence: u64::MAX - 1,
                kind: RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::ExplicitDocumentOpen
                },
                ..terminated
            })
            .is_some()
    );
    assert_eq!(
        contents.main_frame.current_document.as_ref().unwrap().id,
        id
    );
    assert_eq!(
        contents.navigation().is_on_initial_empty_document(),
        Some(false)
    );
}

#[tokio::test]
async fn dialog_disable_and_exact_detach_dismiss_only_their_browser_dialogs() {
    use moli_core::page::{
        RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
    };
    use moli_page_types::DevToolsSessionKey;
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let (mut owner, primary_completion) = page_with_installed_dialog_for_test(&browser).await;
    let peer = DevToolsSessionKey::Attached("SID-dialog-peer".into());
    let peer_completion = RendererJavaScriptDialogCompletion::pending();
    let document = owner.target_document_id(TARGET).unwrap();
    let snapshot = owner
        .web_contents_for_target(TARGET)
        .unwrap()
        .main_frame
        .current_document
        .as_ref()
        .unwrap()
        .lifecycle
        .snapshot()
        .unwrap();
    assert!(owner.install_javascript_dialog_for_target(
        TARGET,
        &peer,
        crate::conn::TargetPageResidenceIdentity::new(
            "BID-dialog-owner".into(),
            Some("TID-dialog-owner".into()),
            document
        ),
        "FRAME-dialog-owner".into(),
        RendererPendingJavaScriptDialog::new(
            RendererJavaScriptDialogId::new(2),
            RendererDocumentLifecycleIdentity {
                frame: snapshot.frame,
                document: snapshot.document,
                epoch: snapshot.epoch
            },
            RendererJavaScriptDialogSource::RootFrame,
            "about:blank".into(),
            "alert".into(),
            "peer".into(),
            String::new(),
            Some(peer_completion.clone())
        )
    ));
    owner.disable_devtools_page_domain_for_target(TARGET, &DevToolsSessionKey::Primary);
    assert!(!primary_completion.finish(true, "late primary".into()));
    assert!(!primary_completion.wait().accepted);
    assert!(owner.has_pending_javascript_dialog_for_target(TARGET));
    assert_eq!(
        owner
            .javascript_dialog_snapshot_for_target(TARGET, &peer)
            .unwrap()
            .message,
        "peer"
    );
    assert!(!owner.dispose_devtools_session_for_target(TARGET, "SID-wrong", &peer));
    assert!(
        owner
            .javascript_dialog_snapshot_for_target(TARGET, &peer)
            .is_some()
    );
    assert!(owner.dispose_devtools_session_for_target(TARGET, "SID-dialog-peer", &peer));
    assert!(!peer_completion.finish(true, "late peer".into()));
    assert!(!peer_completion.wait().accepted);
    assert!(!owner.has_pending_javascript_dialog_for_target(TARGET));
    assert_eq!(owner.target_document_id(TARGET), Some(document));
}

#[tokio::test]
async fn document_replacement_preserves_stable_page_engine_history_and_storage() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let mut owner = context_with_document(first);
    let mut config = moli_fetch::FetchConfig::default();
    config.set_user_agent("stable-engine");
    owner
        .web_contents_for_target_mut(TARGET)
        .unwrap()
        .install_navigation_engine(moli_core::runtime::NavigationEngine::new_with_fetch_config(
            config,
        ));
    let stable_ids = (
        owner.selected_web_contents_id().unwrap(),
        owner.active_page_target().main_frame_slot_id(),
    );
    owner.set_target_window_surface_state(TARGET, crate::conn::WindowSurfaceState::Fullscreen);
    owner.set_target_window_surface_geometry(TARGET, Some(800), Some(600), Some(10), Some(20));
    let window = owner.target_window_surface(TARGET).unwrap();
    owner.apply_target_emulation_policy_change(
        TARGET,
        crate::conn::EmulationPolicyChange::CpuThrottlingRate(4.0),
    );
    owner.mutate_devtools_network_session_state_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        |raw| {
            raw.network_enabled = true;
            raw.cache_disabled = true;
            raw.bypass_service_worker = true;
            raw.blocked_url_patterns = vec!["blocked/*".into()];
            raw.extra_headers = vec![("X-Stable".into(), "contents".into())];
        },
    );
    owner.set_devtools_browser_identity_override_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        crate::conn::DevToolsBrowserIdentityOverride::from_command(
            &moli_browser_profile::BrowserIdentityProfile::default(),
            "Moli/Stable-Identity".into(),
            Some("fr-FR".into()),
            None,
            None,
        ),
    );
    owner
        .set_devtools_locale_override_for_target(
            TARGET,
            &moli_page_types::DevToolsSessionKey::Primary,
            Some("de-DE".into()),
        )
        .unwrap();
    owner
        .set_devtools_timezone_override_for_target(
            TARGET,
            &moli_page_types::DevToolsSessionKey::Primary,
            Some("Europe/Berlin".into()),
        )
        .unwrap();
    let policy = owner.effective_policy_for_target(TARGET);
    owner.set_network_offline_for_target(TARGET, true);
    owner.set_tls_verify_host_override_for_target(TARGET, Some(false));
    owner.set_devtools_bypass_csp_enabled_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        true,
    );
    let first_document = owner.target_document_id(TARGET).unwrap();
    let storage = owner
        .web_contents_for_target(TARGET)
        .unwrap()
        .session_storage
        .store()
        .clone();
    assert!(
        storage
            .lock()
            .set_item("https://example.test", "key", "value")
    );
    let observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();

    let navigation = owner.begin_target_document_navigation(TARGET, "second-loader".into());
    let second = browser.fetch("data:text/html,<p>second</p>").await.unwrap();
    let reserved = owner.reserve_renderer_document_for_target(
        TARGET,
        RendererPageResidenceIdentity::from_page(&second),
    );
    let first = owner
        .replace_loaded_page_for_target(TARGET, Some(second))
        .unwrap();
    assert!(owner.commit_pending_document_navigation_if_matches_for_target(TARGET, &navigation));
    owner
        .web_contents_for_target_mut(TARGET)
        .unwrap()
        .record_navigation_history_for_test((
            "https://example.test/second".into(),
            "second".into(),
        ));

    assert_eq!(
        (
            owner.selected_web_contents_id().unwrap(),
            owner.active_page_target().main_frame_slot_id()
        ),
        stable_ids
    );
    assert_eq!(owner.target_document_id(TARGET), Some(reserved));
    assert_eq!(owner.target_window_surface(TARGET).unwrap(), window);
    assert_eq!(
        owner
            .target_emulation_policy(TARGET)
            .unwrap()
            .cpu_throttling_rate,
        4.0
    );
    assert_eq!(owner.effective_policy_for_target(TARGET), policy);
    assert!(owner.network_offline_for_target(TARGET));
    assert_eq!(
        owner.tls_verify_host_override_for_target(TARGET),
        Some(false)
    );
    assert!(owner.bypass_content_security_policy_for_target(TARGET));
    assert_ne!(first_document, reserved);
    assert_eq!(
        owner
            .page_navigation_engine(TARGET)
            .unwrap()
            .fetch_config()
            .user_agent(),
        "stable-engine"
    );
    assert!(std::sync::Arc::ptr_eq(
        &storage,
        owner
            .web_contents_for_target(TARGET)
            .unwrap()
            .session_storage
            .store()
    ));
    assert_eq!(
        storage.lock().get_item("https://example.test", "key"),
        Some("value".into())
    );
    let (index, history) = owner
        .web_contents_for_target_mut(TARGET)
        .unwrap()
        .navigation()
        .navigation_history_snapshot();
    assert_eq!(index, 1);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].title, "first");
    assert_eq!(history[1].title, "second");
    assert_eq!(
        observer.wait().await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    first.close_async().await.unwrap();
}

#[tokio::test]
async fn web_contents_owns_live_document_and_navigation_after_protocol_residence_is_dropped() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut page = browser.fetch("data:text/html,<p>owned</p>").await.unwrap();
    let renderer = RendererPageResidenceIdentity::from_page(&page);
    let artifacts = page.take_page_creation_artifacts().unwrap();
    let mut owner = context_with_document(page);
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        "frame".into(),
        "loader".into(),
    );
    let document = owner.target_document_id(TARGET).unwrap();
    let observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();
    let stable_id = owner.web_contents_for_target_mut(TARGET).unwrap().id();
    let frame_id = owner
        .web_contents_for_target_mut(TARGET)
        .unwrap()
        .main_frame
        .id();
    let navigation = owner.begin_target_document_navigation(TARGET, "pending-loader".into());
    let cancellation = owner
        .document_navigation_cancellation_handle_for_target(TARGET, &navigation)
        .unwrap();
    let snapshot = owner
        .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
        .unwrap();

    // Drop only DevTools; the registered Browser subtree stays in its Context.
    drop(owner.page_targets.remove(TARGET).unwrap());
    assert_eq!(owner.selected_web_contents_id(), Some(stable_id));
    let contents = owner.physical.web_contents.get(&stable_id).unwrap();
    assert_eq!(contents.id(), stable_id);
    assert_eq!(contents.main_frame.id(), frame_id);
    let current = contents.main_frame.current_document.as_ref().unwrap();
    assert_eq!(current.id, document);
    assert_eq!(
        RendererPageResidenceIdentity::from_page(&current.page),
        renderer
    );
    assert_eq!(current.lifecycle.snapshot(), Some(snapshot));
    assert!(
        contents
            .navigation()
            .accepts_pending_document_navigation_event(&navigation)
    );
    assert!(!cancellation.is_cancelled());
    let mut wait = Box::pin(observer.wait());
    assert_eq!(
        wait.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );

    drop(owner);
    assert!(cancellation.is_cancelled());
    assert_eq!(
        wait.await,
        moli_core::browser::DocumentRetirement::Unavailable
    );
}

#[tokio::test]
async fn replacement_retires_document_identity_lifecycle_and_lifetime_together() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut first = browser.fetch("data:text/html,<p>first</p>").await.unwrap();
    let first_renderer = RendererPageResidenceIdentity::from_page(&first);
    let first_artifacts = first.take_page_creation_artifacts().unwrap();
    let mut owner = empty_document_context();
    let first_id = owner.reserve_renderer_document_for_target(TARGET, first_renderer);
    assert!(
        owner
            .replace_loaded_page_for_target(TARGET, Some(first))
            .is_none()
    );
    assert_eq!(owner.target_document_id(TARGET), Some(first_id));
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        first_artifacts.clone(),
        None,
        "frame".into(),
        "first-loader".into(),
    );
    assert!(
        owner
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .is_some()
    );
    let first_observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();
    let another_first_observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();

    // Moving the whole Context or failing a pending navigation must not retire
    // the current Document. Its Page/lifecycle/identity move as one object.
    let mut moved = owner;
    let failed_navigation = moved.begin_target_document_navigation(TARGET, "failed-loader".into());
    let before = moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET);
    assert!(
        moved.clear_pending_document_navigation_if_matches_for_target(TARGET, &failed_navigation)
    );
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        before
    );
    let mut first_wait = Box::pin(first_observer.wait());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(first_wait.as_mut().poll(&mut context), Poll::Pending);

    let mut second = browser.fetch("data:text/html,<p>second</p>").await.unwrap();
    let second_renderer = RendererPageResidenceIdentity::from_page(&second);
    let second_artifacts = second.take_page_creation_artifacts().unwrap();
    let navigation = moved.begin_target_document_navigation(TARGET, "second-loader".into());
    let reserved_id = moved.target_pending_document_id(TARGET).unwrap();
    assert!(
        moved.bind_pending_document_navigation_renderer_page_for_target(
            TARGET,
            &navigation,
            second_renderer
        )
    );
    let previous_page = moved
        .replace_loaded_page_for_target(TARGET, Some(second))
        .unwrap();
    assert_eq!(moved.target_document_id(TARGET), Some(reserved_id));
    assert_ne!(first_id, reserved_id);
    assert_eq!(
        RendererPageResidenceIdentity::from_page(&previous_page),
        first_renderer
    );
    assert!(!moved.routes_renderer_page_for_target(TARGET, first_renderer));
    assert!(moved.routes_renderer_page_for_target(TARGET, second_renderer));
    assert_eq!(
        first_wait.await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    assert_eq!(
        another_first_observer.wait().await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    assert!(
        moved
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .is_none(),
        "the replacement must not retain the previous Document's lifecycle"
    );
    assert!(
        moved
            .page_slot_for_target(TARGET)
            .unwrap()
            .renderer_document_lifecycle_visible_snapshot()
            .is_none()
    );
    assert!(
        moved
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );

    assert!(moved.commit_pending_document_navigation_if_matches_for_target(TARGET, &navigation));
    moved.bind_renderer_document_lifecycle_for_target(
        TARGET,
        second_artifacts,
        Some(navigation),
        "frame".into(),
        "second-loader".into(),
    );
    let second_snapshot =
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET);
    assert!(second_snapshot.is_some());
    assert!(
        moved
            .ingest_renderer_document_lifecycle_events_for_target(
                TARGET,
                first_artifacts.initial_lifecycle_events
            )
            .is_empty()
    );
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        second_snapshot
    );
    let second_observer = moved.document_lifetime_observer_for_target(TARGET).unwrap();
    let second_page = moved
        .replace_loaded_page_with_reason_for_target(
            TARGET,
            None,
            TargetPageAbsenceReason::TargetClosed,
        )
        .unwrap();
    assert!(!moved.target_has_loaded_page(TARGET));
    assert_eq!(moved.target_document_id(TARGET), None);
    assert!(
        moved
            .document_lifetime_observer_for_target(TARGET)
            .is_none()
    );
    assert!(
        moved
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .is_none()
    );
    assert_eq!(
        second_observer.wait().await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    previous_page.close_async().await.unwrap();
    second_page.close_async().await.unwrap();
}

#[tokio::test]
async fn rejected_reservation_preserves_current_document_until_owner_loss() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser.fetch("data:text/html,<p>first</p>").await.unwrap();
    let first_renderer = RendererPageResidenceIdentity::from_page(&first);
    let mut owner = context_with_document(first);
    let first_id = owner.target_document_id(TARGET);
    let observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();
    let candidate = browser
        .fetch("data:text/html,<p>candidate</p>")
        .await
        .unwrap();
    let navigation = owner.begin_target_document_navigation(TARGET, "candidate-loader".into());
    assert!(
        owner.bind_pending_document_navigation_renderer_page_for_target(
            TARGET,
            &navigation,
            first_renderer
        )
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owner.replace_loaded_page_for_target(TARGET, Some(candidate));
        }))
        .is_err()
    );
    assert_eq!(owner.target_document_id(TARGET), first_id);
    assert_eq!(
        RendererPageResidenceIdentity::from_page(owner.loaded_page_for_target(TARGET).unwrap()),
        first_renderer
    );
    assert!(owner.accepts_pending_document_navigation_event_for_target(TARGET, &navigation));
    let mut wait = Box::pin(observer.wait());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(wait.as_mut().poll(&mut context), Poll::Pending);
    drop(owner);
    assert_eq!(
        wait.await,
        moli_core::browser::DocumentRetirement::Unavailable
    );
}

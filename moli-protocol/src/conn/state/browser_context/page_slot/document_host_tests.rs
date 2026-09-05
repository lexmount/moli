use super::*;
use moli_core::runtime::{Browser, BrowserConfig};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

const TARGET: &str = "TID-dialog-owner";

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
    let snapshot = owner.active_page_target().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .clone();
    let id = owner.selected_web_contents_id().unwrap();
    drop(owner.page_targets.remove(TARGET).unwrap());
    let contents = owner.physical.web_contents.get(&id).unwrap();
    assert!(contents.main_frame.current_document.is_some());
    assert!(!contents.javascript_dialogs.is_empty());
    drop(owner);

    assert!(
        !completion.finish(true, "late reply".into()),
        "Browser drop must dismiss the dialog even if a cloned session projection survives"
    );
    assert!(!completion.wait().accepted);
    drop(snapshot);
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
    assert!(contents.bind_document_lifecycle(snapshot));
    assert!(
        !contents.javascript_dialogs.is_empty(),
        "same-source rebind must preserve its dialog"
    );
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
        !contents.observe_document_lifecycle(RendererDocumentLifecycleEvent {
            document: snapshot.document.successor_for_testing(),
            ..terminated
        })
    );
    assert!(
        !contents.javascript_dialogs.is_empty(),
        "foreign lifecycle must not dismiss current dialog"
    );
    assert!(contents.observe_document_lifecycle(terminated));
    assert!(contents.javascript_dialogs.is_empty());
    assert!(!completion.finish(true, "late reply".into()));
    assert!(!completion.wait().accepted);
    assert!(
        contents.observe_document_lifecycle(RendererDocumentLifecycleEvent {
            epoch: RendererLifecycleEpoch(snapshot.epoch.0 + 1),
            sequence: u64::MAX - 1,
            kind: RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen
            },
            ..terminated
        })
    );
    assert_eq!(
        contents.main_frame.current_document.as_ref().unwrap().id,
        id
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
    let first = browser.fetch("data:text/html,<p>first</p>").await.unwrap();
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
    owner
        .web_contents_for_target_mut(TARGET)
        .unwrap()
        .navigation
        .record_loaded_page_navigation_history((
            "https://example.test/first".into(),
            "first".into(),
        ));
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
        .navigation
        .record_loaded_page_navigation_history((
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
        .navigation
        .navigation_history_snapshot(None);
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
            .navigation
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

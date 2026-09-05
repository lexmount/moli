use super::*;
use moli_core::runtime::{Browser, BrowserConfig};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

#[tokio::test]
async fn document_replacement_preserves_stable_page_engine_history_and_storage() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let first = browser.fetch("data:text/html,<p>first</p>").await.unwrap();
    let mut target = crate::conn::PageTargetHost::new(
        "TID-stable-contents".into(),
        None,
        crate::conn::TargetIdentityState::about_blank(),
        TargetPageSlot::with_loaded_page_for_test(first),
    );
    let mut config = moli_fetch::FetchConfig::default();
    config.set_user_agent("stable-engine");
    target.install_navigation_engine(moli_core::runtime::NavigationEngine::new_with_fetch_config(
        config,
    ));
    let stable_ids = (target.web_contents_id(), target.main_frame_slot_id());
    target.set_window_surface_state(crate::conn::WindowSurfaceState::Fullscreen);
    target.set_window_surface_geometry(Some(800), Some(600), Some(10), Some(20));
    let window = target.window_surface();
    target
        .apply_emulation_policy_change(crate::conn::EmulationPolicyChange::CpuThrottlingRate(4.0));
    target.mutate_devtools_network_session_state(
        &moli_page_types::DevToolsSessionKey::Primary,
        |raw| {
            raw.network_enabled = true;
            raw.cache_disabled = true;
            raw.bypass_service_worker = true;
            raw.blocked_url_patterns = vec!["blocked/*".into()];
            raw.extra_headers = vec![("X-Stable".into(), "contents".into())];
        },
    );
    target.set_devtools_browser_identity_override(
        &moli_page_types::DevToolsSessionKey::Primary,
        crate::conn::DevToolsBrowserIdentityOverride::from_command(
            &moli_browser_profile::BrowserIdentityProfile::default(),
            "Moli/Stable-Identity".into(),
            Some("fr-FR".into()),
            None,
            None,
        ),
    );
    target
        .set_devtools_locale_override(
            &moli_page_types::DevToolsSessionKey::Primary,
            Some("de-DE".into()),
        )
        .unwrap();
    target
        .set_devtools_timezone_override(
            &moli_page_types::DevToolsSessionKey::Primary,
            Some("Europe/Berlin".into()),
        )
        .unwrap();
    let policy = target.effective_policy();
    let first_document = target.current_document_id().unwrap();
    let storage = target.session_storage_store().clone();
    assert!(
        storage
            .lock()
            .set_item("https://example.test", "key", "value")
    );
    target
        .runtime_slot
        .page_slot_mut()
        .contents
        .navigation
        .record_loaded_page_navigation_history((
            "https://example.test/first".into(),
            "first".into(),
        ));
    let observer = target
        .runtime_slot
        .page_slot_mut()
        .document_lifetime_observer()
        .unwrap();

    let navigation = target
        .runtime_slot
        .start_document_navigation("second-loader".into());
    let second = browser.fetch("data:text/html,<p>second</p>").await.unwrap();
    let reserved = target
        .runtime_slot
        .page_slot_mut()
        .reserve_renderer_document(RendererPageResidenceIdentity::from_page(&second));
    let first = target
        .runtime_slot
        .replace_loaded_page(Some(second))
        .unwrap();
    assert!(
        target
            .runtime_slot
            .commit_pending_document_navigation_if_matches(&navigation)
    );
    target
        .runtime_slot
        .page_slot_mut()
        .contents
        .navigation
        .record_loaded_page_navigation_history((
            "https://example.test/second".into(),
            "second".into(),
        ));

    assert_eq!(
        (target.web_contents_id(), target.main_frame_slot_id()),
        stable_ids
    );
    assert_eq!(target.current_document_id(), Some(reserved));
    assert_eq!(target.window_surface(), window);
    assert_eq!(target.emulation_policy().cpu_throttling_rate, 4.0);
    assert_eq!(target.effective_policy(), policy);
    assert_ne!(first_document, reserved);
    assert_eq!(
        target
            .navigation_engine()
            .unwrap()
            .fetch_config()
            .user_agent(),
        "stable-engine"
    );
    assert!(std::sync::Arc::ptr_eq(
        &storage,
        target.session_storage_store()
    ));
    assert_eq!(
        storage.lock().get_item("https://example.test", "key"),
        Some("value".into())
    );
    let (index, history) = target
        .runtime_slot
        .page_slot_mut()
        .contents
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
    let mut slot = TargetPageSlot::with_loaded_page_for_test(page);
    slot.bind_renderer_document_lifecycle(artifacts, None, "frame".into(), "loader".into());
    let document = slot.document_id().unwrap();
    let observer = slot.document_lifetime_observer().unwrap();
    let stable_id = slot.contents.id();
    let frame_id = slot.contents.main_frame.id();
    let navigation = slot.start_document_navigation("pending-loader".into());
    let cancellation = slot
        .document_navigation_cancellation_handle(&navigation)
        .unwrap();
    let snapshot = slot
        .renderer_document_lifecycle_authoritative_snapshot()
        .unwrap();

    // Move only the Browser subtree. All loader/binding/output state is dropped.
    let contents = {
        let protocol_residence = slot;
        protocol_residence.contents
    };
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

    drop(contents);
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
    let mut slot = TargetPageSlot::default();
    let first_id = slot.reserve_renderer_document(first_renderer);
    assert!(slot.replace_loaded_page(Some(first)).is_none());
    assert_eq!(slot.document_id(), Some(first_id));
    slot.bind_renderer_document_lifecycle(
        first_artifacts.clone(),
        None,
        "frame".into(),
        "first-loader".into(),
    );
    assert!(
        slot.renderer_document_lifecycle_authoritative_snapshot()
            .is_some()
    );
    let first_observer = slot.document_lifetime_observer().unwrap();
    let another_first_observer = slot.document_lifetime_observer().unwrap();

    // Moving the whole slot or failing a pending navigation must not retire
    // the current Document. Its Page/lifecycle/identity move as one object.
    let mut moved = slot;
    let failed_navigation = moved.start_document_navigation("failed-loader".into());
    let before = moved.renderer_document_lifecycle_authoritative_snapshot();
    assert!(moved.clear_pending_document_navigation_if_matches(&failed_navigation));
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot(),
        before
    );
    let mut first_wait = Box::pin(first_observer.wait());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(first_wait.as_mut().poll(&mut context), Poll::Pending);

    let mut second = browser.fetch("data:text/html,<p>second</p>").await.unwrap();
    let second_renderer = RendererPageResidenceIdentity::from_page(&second);
    let second_artifacts = second.take_page_creation_artifacts().unwrap();
    let navigation = moved.start_document_navigation("second-loader".into());
    let reserved_id = moved.pending_document_id().unwrap();
    assert!(moved.bind_pending_document_navigation_renderer_page(&navigation, second_renderer));
    let previous_page = moved.replace_loaded_page(Some(second)).unwrap();
    assert_eq!(moved.document_id(), Some(reserved_id));
    assert_ne!(first_id, reserved_id);
    assert_eq!(
        RendererPageResidenceIdentity::from_page(&previous_page),
        first_renderer
    );
    assert!(!moved.routes_renderer_page(first_renderer));
    assert!(moved.routes_renderer_page(second_renderer));
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
            .renderer_document_lifecycle_authoritative_snapshot()
            .is_none(),
        "the replacement must not retain the previous Document's lifecycle"
    );
    assert!(
        moved
            .renderer_document_lifecycle_visible_snapshot()
            .is_none()
    );
    assert!(moved.renderer_document_lifecycle_binding().is_none());

    assert!(moved.commit_pending_document_navigation_if_matches(&navigation));
    moved.bind_renderer_document_lifecycle(
        second_artifacts,
        Some(navigation),
        "frame".into(),
        "second-loader".into(),
    );
    let second_snapshot = moved.renderer_document_lifecycle_authoritative_snapshot();
    assert!(second_snapshot.is_some());
    assert!(
        moved
            .ingest_renderer_document_lifecycle_events(first_artifacts.initial_lifecycle_events)
            .is_empty()
    );
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot(),
        second_snapshot
    );
    let second_observer = moved.document_lifetime_observer().unwrap();
    let second_page = moved
        .replace_loaded_page_with_reason(None, TargetPageAbsenceReason::TargetClosed)
        .unwrap();
    assert!(!moved.has_loaded_page());
    assert_eq!(moved.document_id(), None);
    assert!(moved.document_lifetime_observer().is_none());
    assert!(
        moved
            .renderer_document_lifecycle_authoritative_snapshot()
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
    let mut slot = TargetPageSlot::with_loaded_page_for_test(first);
    let first_id = slot.document_id();
    let observer = slot.document_lifetime_observer().unwrap();
    let candidate = browser
        .fetch("data:text/html,<p>candidate</p>")
        .await
        .unwrap();
    let navigation = slot.start_document_navigation("candidate-loader".into());
    assert!(slot.bind_pending_document_navigation_renderer_page(&navigation, first_renderer));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            slot.replace_loaded_page(Some(candidate));
        }))
        .is_err()
    );
    assert_eq!(slot.document_id(), first_id);
    assert_eq!(
        RendererPageResidenceIdentity::from_page(slot.loaded_page().unwrap()),
        first_renderer
    );
    assert!(slot.accepts_pending_document_navigation_event(&navigation));
    let mut wait = Box::pin(observer.wait());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(wait.as_mut().poll(&mut context), Poll::Pending);
    drop(slot);
    assert_eq!(
        wait.await,
        moli_core::browser::DocumentRetirement::Unavailable
    );
}

use super::*;
use moli_core::runtime::{Browser, BrowserConfig};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

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

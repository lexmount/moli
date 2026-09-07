use super::*;
use crate::conn::state::DevToolsRendererChannelError;
use moli_core::browser::{DocumentRetirement, NavigationId};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

#[test]
fn window_and_crash_state_outlive_the_devtools_projection() {
    let mut context = BrowserContext::new("BID-window".into());
    context.set_active_target_id("TID-window");
    context.attach_active_session("SID-window");
    let handle = context
        .web_contents_handle_for_target("TID-window")
        .unwrap();
    context
        .update_web_contents_window_surface(
            handle,
            Some(WindowSurfaceState::Minimized),
            Some(800),
            Some(600),
            Some(-10),
            Some(20),
        )
        .unwrap();
    context.set_target_crash_state("TID-window", true);
    let id = context.selected_web_contents_id().unwrap();
    let surface = context.web_contents_window_surface(handle).unwrap();
    let opener = WebContentsId::allocate();
    context
        .set_web_contents_window_name(handle, Some("report".into()))
        .unwrap();
    context
        .browser_context
        .set_web_contents_opener_for_test(handle, opener, true)
        .unwrap();
    let target = context.page_targets.get_mut("TID-window").unwrap();
    target.opener_frame_id = Some("FRAME-opener".into());
    assert_eq!(target.detach_session().as_deref(), Some("SID-window"));
    assert!(context.target_is_crashed("TID-window"));
    assert_eq!(context.web_contents_window_surface(handle), Ok(surface));

    drop(context.page_targets.remove("TID-window").unwrap());
    assert_eq!(context.selected_web_contents_id(), Some(id));
    assert!(
        context
            .browser_context
            .web_contents_is_crashed(handle)
            .unwrap()
    );
    assert_eq!(
        context.browser_context.web_contents_window_surface(handle),
        Ok(surface)
    );
    assert_eq!(
        context.browser_context.web_contents_window_name(handle),
        Ok(Some("report".to_owned()))
    );
    assert_eq!(
        context.browser_context.web_contents_opener(handle),
        Ok(Some((opener, true)))
    );

    context.set_active_target_id("TID-window");
    let replacement_id = context.selected_web_contents_id().unwrap();
    assert_ne!(replacement_id, id);
    let replacement = context.selected_web_contents_handle().unwrap();
    assert!(
        !context
            .browser_context
            .web_contents_is_crashed(replacement)
            .unwrap()
    );
    assert_eq!(
        context
            .browser_context
            .web_contents_window_surface(replacement),
        Ok(WindowSurface::default())
    );
    assert_eq!(
        context
            .browser_context
            .web_contents_window_name(replacement),
        Ok(None)
    );
    assert_eq!(
        context.browser_context.web_contents_opener(replacement),
        Ok(None)
    );
    assert!(context.browser_context.contains_web_contents(handle));
}

#[tokio::test]
async fn projection_drop_preserves_the_contexts_page_engine_selection_and_document_lifetime() {
    let mut conn = crate::test_support::connection();
    let mut context = conn.new_browser_context_fixture_for_test("BID-physical-owner");
    context.set_active_target_id("TID-physical-owner");
    context.bind_page_navigation_engines(Default::default(), None);
    conn.install_browser_context_fixture_for_test(context);
    conn.install_navigation_fixture_for_session_owner_for_test(
        "data:text/html,<title>Browser owned</title>",
        None,
    )
    .await;
    let mut context = conn.browser_context.take().unwrap();
    let residence = context
        .target_renderer_page_residence_identity("TID-physical-owner")
        .unwrap();
    let id = context.selected_web_contents_id().unwrap();
    let handle = context.selected_web_contents_handle().unwrap();
    let document_id = context.target_document_id("TID-physical-owner").unwrap();
    let engine = context
        .page_navigation_renderer_owner_id("TID-physical-owner")
        .unwrap();
    let observer = context
        .document_lifetime_observer_for_target("TID-physical-owner")
        .unwrap();

    let history = context
        .target_navigation_history_snapshot("TID-physical-owner")
        .unwrap();
    assert_eq!(history.1.len(), 1);
    assert_eq!(history.1[0].title, "Browser owned");
    let target = context.page_targets.get_mut("TID-physical-owner").unwrap();
    target.set_target_url("https://projection.invalid/wrong".into());
    target.owner_state.committed_document_title = Some("stale projection".into());
    assert_eq!(
        context.target_navigation_history_snapshot("TID-physical-owner"),
        Some(history.clone())
    );

    drop(context.page_targets.remove("TID-physical-owner").unwrap());
    assert_eq!(context.selected_web_contents_id(), Some(id));
    assert_eq!(
        context.browser_context.navigation_history_snapshot(handle),
        Ok(history)
    );
    assert_eq!(
        context
            .browser_context
            .web_contents_navigation_renderer_owner_id(handle)
            .unwrap(),
        engine
    );
    let document = moli_core::browser::DocumentHandle::new(handle, document_id);
    assert_eq!(
        context
            .browser_context
            .document_renderer_residence(document)
            .unwrap(),
        residence
    );
    assert_eq!(
        context.browser_context.document_title(document).unwrap(),
        "Browser owned"
    );
    let mut retired = Box::pin(observer.wait());
    assert_eq!(
        retired
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );

    context.close_all_pages_async().await;
    assert_eq!(retired.await, DocumentRetirement::Superseded);
    assert_eq!(context.browser_context.web_contents_count(), 0);
    assert!(context.page_targets.is_empty());
    assert_eq!(context.selected_web_contents_id(), None);
}

#[test]
fn emulation_policy_survives_projection_drop_and_updates_without_sessions() {
    let mut context = BrowserContext::new_with_page_for_test("CTX-policy", "TID-policy");
    context.attach_active_session("SID-primary");
    assert!(context.assign_attached_session_to_target("TID-policy", "SID-attached".into()));
    let id = context.active_page_target().web_contents_id();
    let mut conn = crate::test_support::connection();
    conn.install_browser_context_fixture_for_test(context);
    for (session, change) in [
        ("SID-primary", EmulationPolicyChange::CpuThrottlingRate(4.0)),
        (
            "SID-attached",
            EmulationPolicyChange::CpuThrottlingRate(2.0),
        ),
        ("SID-primary", EmulationPolicyChange::FocusEnabled(true)),
    ] {
        assert!(conn.apply_emulation_override_for_session_owner(Some(session), change));
    }
    let primary = conn
        .emulation_session_state_for_session_owner(Some("SID-primary"))
        .unwrap();
    assert_eq!(primary.overrides.as_ref().unwrap().cpu_throttling_rate, 4.0);
    drop(primary);

    let mut context = conn.browser_context.take().unwrap();
    drop(context.page_targets.remove("TID-policy").unwrap());
    drop(conn);
    let handle = WebContentsHandle::new(context.browser_context_id(), id);
    let policy = context
        .browser_context
        .web_contents_emulation_policy(handle)
        .unwrap();
    assert_eq!(policy.cpu_throttling_rate, 2.0);
    assert!(policy.focus_emulation_enabled);
    let snapshot = policy;
    context
        .browser_context
        .apply_web_contents_emulation_policy_change(
            handle,
            EmulationPolicyChange::ScriptExecutionDisabled(true),
        )
        .unwrap();
    assert!(!snapshot.script_execution_disabled);
    assert!(
        context
            .browser_context
            .web_contents_emulation_policy(handle)
            .unwrap()
            .script_execution_disabled
    );
}

#[tokio::test]
async fn close_retires_projection_waiters_and_channel_before_the_owned_page_teardown() {
    use crate::conn::state::InitialDocumentAdmission;
    let mut context = BrowserContext::new("BID-close".into());
    context.bind_page_navigation_engines(Default::default(), None);
    context.set_active_target_id("TID-close");
    context.attach_active_session("SID-close");
    context.target_popup_ids.insert("TID-close".into(), 7);
    let InitialDocumentAdmission::Build(build) = context
        .start_initial_document_for_target("TID-close", Default::default(), &Default::default())
        .unwrap()
    else {
        panic!("expected build");
    };
    let InitialDocumentAdmission::Join(waiter) = context
        .start_initial_document_for_target("TID-close", Default::default(), &Default::default())
        .unwrap()
    else {
        panic!("expected join");
    };
    let slot = &context.page_targets.get("TID-close").unwrap().runtime_slot;
    let dialog_scope = slot.javascript_dialog_scope_observer();
    let handle = context.web_contents_handle_for_target("TID-close").unwrap();

    let (mut projection, closing) = context.begin_web_contents_close(handle).unwrap();
    assert!(!context.browser_context.contains_web_contents(handle));
    assert!(context.page_targets.is_empty());
    assert_eq!(context.selected_web_contents_id(), None);
    assert!(context.target_popup_ids.is_empty());
    assert_eq!(projection.session_id(), Some("SID-close"));
    assert!(
        !projection
            .runtime_slot
            .observes_javascript_dialog_scope(&dialog_scope)
    );
    assert!(matches!(
        projection
            .runtime_slot
            .finish_navigation_without_document_projection(&NavigationId::allocate()),
        Err(DevToolsRendererChannelError::Closed)
    ));
    assert_eq!(
        waiter.wait().await,
        Err("InitialDocumentPageBuildCancelled".into())
    );
    // Cancellation of the teardown future must not resurrect either authority.
    drop(closing);
    assert!(build.materialize().await.is_err());
    assert!(context.begin_web_contents_close(handle).is_err());
    drop(projection);
}

#[tokio::test]
async fn close_all_retires_background_builds_and_removes_every_projection() {
    use crate::conn::state::InitialDocumentAdmission;
    let mut context = BrowserContext::new("BID-close-all".into());
    context.bind_page_navigation_engines(Default::default(), None);
    let mut waiters = Vec::new();
    let mut builds = Vec::new();
    for id in ["TID-first", "TID-background"] {
        assert!(context.register_page_target_fixture(
            id.into(),
            None,
            TargetIdentityState::about_blank(),
            TargetPageSlot::empty_for_initial_document_page_build(),
        ));
        let InitialDocumentAdmission::Build(build) = context
            .start_initial_document_for_target(id, Default::default(), &Default::default())
            .unwrap()
        else {
            panic!("expected build");
        };
        builds.push(build);
        let InitialDocumentAdmission::Join(waiter) = context
            .start_initial_document_for_target(id, Default::default(), &Default::default())
            .unwrap()
        else {
            panic!("expected join");
        };
        waiters.push(waiter);
    }
    context.set_active_target_id("TID-first");
    context.close_all_pages_async().await;
    assert_eq!(context.browser_context.web_contents_count(), 0);
    assert!(context.page_targets.is_empty());
    assert_eq!(context.selected_web_contents_id(), None);
    for waiter in waiters {
        assert_eq!(
            waiter.wait().await,
            Err("InitialDocumentPageBuildCancelled".into())
        );
    }
    context.close_all_pages_async().await;
    assert_eq!(context.selected_web_contents_id(), None);
    for build in builds {
        assert!(build.materialize().await.is_err());
    }
}

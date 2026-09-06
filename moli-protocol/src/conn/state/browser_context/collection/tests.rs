use super::*;
use crate::conn::state::{DevToolsRendererChannelError, WindowOpener};
use moli_core::{
    browser::{DocumentRetirement, NavigationId, RendererPageResidenceIdentity},
    runtime::{Browser, BrowserConfig, NavigationEngine},
};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

#[test]
fn window_and_crash_state_outlive_the_devtools_projection() {
    let mut context = BrowserContext::new("BID-window".into());
    context.set_active_target_id("TID-window");
    context.attach_active_session("SID-window");
    context.set_target_window_surface_state("TID-window", WindowSurfaceState::Minimized);
    context.set_target_window_surface_geometry(
        "TID-window",
        Some(800),
        Some(600),
        Some(-10),
        Some(20),
    );
    context.set_target_crash_state("TID-window", true);
    let id = context.selected_web_contents_id().unwrap();
    let surface = context.target_window_surface("TID-window").unwrap();
    let opener = WebContentsId::allocate();
    let window = &mut context.physical.web_contents.get_mut(&id).unwrap().window;
    window.name = Some("report".into());
    window.opener = Some(WindowOpener {
        web_contents_id: opener,
        can_access: true,
    });
    let target = context.page_targets.get_mut("TID-window").unwrap();
    target.opener_frame_id = Some("FRAME-opener".into());
    assert_eq!(target.detach_session().as_deref(), Some("SID-window"));
    assert!(context.target_is_crashed("TID-window"));
    assert_eq!(context.target_window_surface("TID-window"), Some(surface));

    drop(context.page_targets.remove("TID-window").unwrap());
    assert_eq!(context.selected_web_contents_id(), Some(id));
    let contents = context.physical.web_contents.get(&id).unwrap();
    assert_eq!(contents.id(), id);
    assert!(contents.crashed);
    assert_eq!(contents.window.surface, surface);
    assert_eq!(contents.window.name.as_deref(), Some("report"));
    let relationship = contents.window.opener.unwrap();
    assert_eq!(relationship.web_contents_id, opener);
    assert!(relationship.can_access);

    context.set_active_target_id("TID-window");
    let replacement_id = context.selected_web_contents_id().unwrap();
    assert_ne!(replacement_id, id);
    let replacement = context.physical.web_contents.get(&replacement_id).unwrap();
    assert!(!replacement.crashed);
    assert_eq!(replacement.window.surface, WindowSurface::default());
    assert!(replacement.window.name.is_none());
    assert!(replacement.window.opener.is_none());
    assert!(context.physical.web_contents.contains_key(&id));
}

#[tokio::test]
async fn projection_drop_preserves_the_contexts_page_engine_selection_and_document_lifetime() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let page = browser
        .fetch("data:text/html,<title>Browser owned</title>")
        .await
        .unwrap();
    let residence = RendererPageResidenceIdentity::from_page(&page);
    let mut context = BrowserContext::new("BID-physical-owner".into());
    context.set_active_target_id("TID-physical-owner");
    context.bind_page_navigation_engines(Default::default(), None);
    assert!(context.replace_loaded_page(Some(page)).is_none());
    let id = context.selected_web_contents_id().unwrap();
    let document_id = context.target_document_id("TID-physical-owner").unwrap();
    let engine = context
        .page_navigation_engine("TID-physical-owner")
        .unwrap() as *const NavigationEngine;
    let observer = context
        .document_lifetime_observer_for_target("TID-physical-owner")
        .unwrap();

    drop(context.page_targets.remove("TID-physical-owner").unwrap());
    assert_eq!(context.selected_web_contents_id(), Some(id));
    let contents = context.physical.web_contents.get(&id).unwrap();
    assert_eq!(
        contents.navigation_engine_for_test().unwrap() as *const NavigationEngine,
        engine
    );
    let document = contents.main_frame.current_document.as_ref().unwrap();
    assert_eq!(document.id, document_id);
    assert_eq!(
        RendererPageResidenceIdentity::from_page(&document.page),
        residence
    );
    assert_eq!(document.page.document_title(), "Browser owned");
    let mut retired = Box::pin(observer.wait());
    assert_eq!(
        retired
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );

    context.close_all_pages_async().await;
    assert_eq!(retired.await, DocumentRetirement::Superseded);
    assert!(context.physical.web_contents.is_empty());
    assert!(context.page_targets.is_empty());
    assert_eq!(context.selected_web_contents_id(), None);
}

#[test]
fn emulation_policy_survives_projection_drop_and_updates_without_sessions() {
    let mut context = BrowserContext::new_with_page_for_test("CTX-policy", "TID-policy");
    context.attach_active_session("SID-primary");
    assert!(context.assign_attached_session_to_target("TID-policy", "SID-attached".into()));
    let id = context.active_page_target().web_contents_id();
    let mut conn = crate::conn::CdpConnection::default();
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
    let contents = context.physical.web_contents.get_mut(&id).unwrap();
    assert_eq!(contents.id(), id);
    assert_eq!(contents.emulation_policy.cpu_throttling_rate, 2.0);
    assert!(contents.emulation_policy.focus_emulation_enabled);
    let snapshot = contents.emulation_policy.clone();
    contents
        .emulation_policy
        .apply(EmulationPolicyChange::ScriptExecutionDisabled(true));
    assert!(!snapshot.script_execution_disabled);
    assert!(contents.emulation_policy.script_execution_disabled);
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
    let id = context.selected_web_contents_id().unwrap();

    let (mut projection, closing) = context.take_page_target_for_close("TID-close").unwrap();
    assert!(!context.physical.web_contents.contains_key(&id));
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
            .finish_renderer_document_navigation(&NavigationId::allocate()),
        Err(DevToolsRendererChannelError::Closed)
    ));
    assert_eq!(
        waiter.wait().await,
        Err("InitialDocumentPageBuildCancelled".into())
    );
    // Cancellation of the teardown future must not resurrect either authority.
    drop(closing);
    assert!(build.materialize().await.is_err());
    assert!(context.take_page_target_for_close("TID-close").is_none());
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
    assert!(context.physical.web_contents.is_empty());
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

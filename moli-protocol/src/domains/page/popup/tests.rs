use super::*;
use crate::conn::{CommandOwnerScope, PageCloseNotifications, TargetPageResidenceIdentity};
use moli_core::page::RendererWindowDocumentSource;

async fn source() -> (CdpConnection, TargetPageResidenceIdentity) {
    let mut conn = crate::test_support::connection();
    let mut context = conn.new_browser_context_fixture_for_test("BID-source");
    context.set_active_target_id("TID-source");
    context.attach_active_session("SID-source");
    conn.install_browser_context_fixture_for_test(context);
    conn.install_navigation_fixture_for_session_owner_for_test("about:blank", Some("SID-source"))
        .await
        .unwrap();
    let owner = conn
        .target_page_residence_identity_for_session(Some("SID-source"))
        .unwrap();
    (conn, owner)
}

fn install_peer(conn: &mut CdpConnection) {
    let mut context = conn.new_browser_context_fixture_for_test("BID-current");
    context.set_active_target_id("TID-current");
    context.attach_active_session("SID-current");
    conn.insert_browser_context(context);
    assert!(conn.activate_browser_context_by_id("BID-current"));
}

async fn project(
    conn: &mut CdpConnection,
    openings: Vec<Arc<RendererPopupOpening>>,
) -> Vec<String> {
    let mut handles = Vec::new();
    for opening in &openings {
        handles.push(
            conn.wait_for_renderer_popup(opening.clone())
                .await
                .unwrap()
                .web_contents,
        );
    }
    emit_prepared(
        conn,
        &mut Vec::new(),
        openings
            .into_iter()
            .map(PagePreparedPopupOpening::new)
            .collect(),
    )
    .await;
    handles
        .into_iter()
        .filter_map(|handle| {
            conn.browser_context_by_browser_id(handle.context())
                .and_then(|context| context.target_id_for_web_contents(handle.id()))
                .map(str::to_owned)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn reused_popup_native_request_waits_for_its_source_fifo_observation() {
    let (mut conn, owner) = source().await;
    let (_, mut events) = conn.subscribe_browser_events().unwrap();
    // Capture both real inputs on one immutable renderer transport, then
    // project only the creation prefix while holding the reuse observation.
    let mut openings = capture_popup_script_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank','native-fifo');window.open('data:text/html,native-fifo','native-fifo')",
        2,
    )
    .await;
    let target = project(&mut conn, vec![openings.remove(0)]).await.remove(0);
    let opening = openings.remove(0);
    let admission = conn.wait_for_renderer_popup(opening.clone()).await.unwrap();
    assert!(!admission.created);
    let (contents, paused) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some((contents, paused)) = conn.native_navigation_decision_for_target(&target)
                && matches!(
                    paused.stage,
                    moli_core::browser::NavigationDecisionStage::Request { .. }
                )
            {
                break (contents, paused);
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("native named reuse request boundary");
    assert_eq!(contents, admission.web_contents);
    assert_eq!(Some(paused.permit.navigation()), admission.navigation);
    assert!(
        conn.project_browser_navigation_decision(contents)
            .await
            .is_empty(),
        "unobserved input cannot publish request events ahead of its source FIFO"
    );
    let (_, still_paused) = conn.native_navigation_decision_for_target(&target).unwrap();
    assert_eq!(
        still_paused.permit, paused.permit,
        "source FIFO must retain the exact unclaimed decision"
    );
    assert_eq!(
        project(&mut conn, vec![opening]).await,
        std::slice::from_ref(&target)
    );
    conn.project_browser_navigation_decision(contents).await;
    assert!(
        conn.native_navigation_decision_for_target(&target)
            .is_none_or(|(_, next)| next.permit != paused.permit),
        "the observed source must release its own request, not wait for another popup"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_uses_captured_context_and_opener_after_another_context_becomes_active() {
    let (mut conn, owner) = source().await;
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#captured', '_blank')",
    )
    .await;
    install_peer(&mut conn);
    let targets = project(&mut conn, openings).await;
    let info = conn
        .browser_context_by_id("BID-source")
        .unwrap()
        .devtools_target_info(&targets[0])
        .unwrap();
    assert_eq!(
        info.opener_id.as_ref().map(|id| id.as_str()),
        Some("TID-source")
    );
    assert_eq!(
        info.opener_frame_id.as_ref().map(|id| id.as_str()),
        Some("TID-source")
    );
    assert_eq!(conn.browser_context.as_ref().unwrap().id, "BID-current");
    assert!(
        conn.browser_context_by_id("BID-current")
            .unwrap()
            .has_no_background_targets()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn noopener_popup_retains_devtools_creator_without_dom_opener_access() {
    let (mut conn, owner) = source().await;
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#noopener', '_blank', 'noopener')",
    )
    .await;
    let targets = project(&mut conn, openings).await;
    let context = conn.browser_context_by_id("BID-source").unwrap();
    let info = context.devtools_target_info(&targets[0]).unwrap();
    assert_eq!(
        info.opener_id.as_ref().map(|id| id.as_str()),
        Some("TID-source")
    );
    assert_eq!(
        info.opener_frame_id.as_ref().map(|id| id.as_str()),
        Some("TID-source")
    );
    assert!(!info.can_access_opener);
    assert_eq!(context.active_target_id(), Some("TID-source"));
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_opener_downgrades_access_without_rebinding_to_current_target() {
    let (mut conn, owner) = source().await;
    let document = conn
        .resolve_browser_document_for_owner(&CommandOwnerScope::capture(&conn, Some("SID-source")))
        .unwrap();
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#removed-opener', '_blank')",
    )
    .await;
    install_peer(&mut conn);
    conn.close_browser_web_contents_async(
        document.web_contents(),
        PageCloseNotifications::BrowserEvent,
    )
    .await;
    let targets = project(&mut conn, openings).await;
    let info = conn
        .browser_context_by_id("BID-source")
        .unwrap()
        .devtools_target_info(&targets[0])
        .unwrap();
    assert!(info.opener_id.is_none());
    assert!(info.opener_frame_id.is_none());
    assert!(!info.can_access_opener);
    assert_eq!(
        conn.browser_context.as_ref().unwrap().active_target_id(),
        Some("TID-current")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_window_popup_preserves_its_exact_opener_frame() {
    let (mut conn, owner) = source().await;
    let openings = capture_openings_for_test(&mut conn, &owner,
        r#"const child=document.createElement('iframe');child.srcdoc="<script>window.open('about:blank#child','child-popup')</script>";document.body.append(child)"#).await;
    let moli_core::page::RendererPopupActivationSource::Window {
        window: RendererWindowDocumentSource::ChildFrame { frame_id, .. },
        ..
    } = openings[0].source()
    else {
        panic!("exact child Window source: {:?}", openings[0].source());
    };
    let frame = frame_id.clone();
    assert_ne!(frame, "TID-source");
    let targets = project(&mut conn, openings).await;
    let info = conn
        .browser_context_by_id("BID-source")
        .unwrap()
        .devtools_target_info(&targets[0])
        .unwrap();
    assert_eq!(
        info.opener_id.as_ref().map(|id| id.as_str()),
        Some("TID-source")
    );
    assert_eq!(
        info.opener_frame_id.as_ref().map(|id| id.as_str()),
        Some(frame.as_str())
    );
    assert!(info.can_access_opener);
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_source_uses_entry_window_when_parent_calls_child_functions() {
    let (mut conn, owner) = source().await;
    let openings = capture_popup_script_for_test(
        &mut conn,
        &owner,
        r#"const child=document.createElement('iframe');
        child.srcdoc="<script>window.openFromChild=()=>window.open('about:blank#function','borrowed-function')</script>";
        child.onload=()=>{
            child.contentWindow.open('about:blank#method','borrowed-method');
            child.contentWindow.openFromChild();
        };
        document.body.append(child);"#,
        2,
    ).await;
    for opening in &openings {
        assert!(
            matches!(
                opening.source(),
                moli_core::page::RendererPopupActivationSource::Window {
                    window: RendererWindowDocumentSource::RootFrame,
                    ..
                }
            ),
            "entry is the parent, not the child function's realm: {:?}",
            opening.source()
        );
    }
    let targets = project(&mut conn, openings).await;
    assert_eq!(targets.len(), 2);
    for target in targets {
        let info = conn
            .browser_context_by_id("BID-source")
            .unwrap()
            .devtools_target_info(&target)
            .unwrap();
        assert_eq!(
            info.opener_frame_id.as_ref().map(|id| id.as_str()),
            Some("TID-source")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fifo_popup_batch_resolves_a_lightweight_popup_as_the_next_opener() {
    let (mut conn, owner) = source().await;
    let openings = capture_popup_script_for_test(
        &mut conn,
        &owner,
        r#"window.open("javascript:window.open('about:blank#second','second')",'first')"#,
        2,
    )
    .await;
    assert_eq!(openings.len(), 2);
    let targets = project(&mut conn, openings).await;
    let info = conn
        .browser_context_by_id("BID-source")
        .unwrap()
        .devtools_target_info(&targets[1])
        .unwrap();
    assert_eq!(
        info.opener_id.as_ref().map(|id| id.as_str()),
        Some(targets[0].as_str())
    );
    assert_eq!(
        info.opener_frame_id.as_ref().map(|id| id.as_str()),
        Some(targets[0].as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_captured_context_does_not_fall_back_to_the_active_context() {
    let (mut conn, owner) = source().await;
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#removed-context','_blank')",
    )
    .await;
    install_peer(&mut conn);
    let raw = serde_json::json!({"id": 900002, "method": "Target.disposeBrowserContext",
        "params": {"browserContextId": "BID-source"}})
    .to_string();
    let mut step = conn.start_command_dispatch(&raw);
    while let crate::conn::CdpCommandTaskStep::Pending(pending) = step {
        step = conn
            .complete_pending_command_dispatch(pending.wait().await)
            .await;
    }
    assert!(!conn.has_browser_context_id("BID-source"));
    assert!(project(&mut conn, openings).await.is_empty());
    assert!(
        conn.browser_context
            .as_ref()
            .unwrap()
            .has_no_background_targets()
    );
    assert_eq!(
        conn.browser_context.as_ref().unwrap().active_target_id(),
        Some("TID-current")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_snapshot_respects_live_renderer_fifo_and_emits_only_once() {
    let (mut conn, owner) = source().await;
    conn.set_root_target_discovery_enabled(true);
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#snapshot','snapshot-popup')",
    )
    .await;
    let popup = conn
        .wait_for_renderer_popup(openings[0].clone())
        .await
        .unwrap()
        .web_contents;
    let (snapshot, _) = conn.subscribe_browser_events().unwrap();
    assert!(snapshot.web_contents.contains(&popup));
    let events = conn.project_browser_snapshot(snapshot).await;
    assert!(
        events
            .into_iter()
            .all(|event| event.into_parts().0["method"] != "Target.targetCreated")
    );
    assert!(
        conn.browser_context_by_id("BID-source")
            .unwrap()
            .target_id_for_web_contents(popup.id())
            .is_none()
    );
    let mut events = Vec::new();
    emit_prepared(
        &mut conn,
        &mut events,
        openings
            .into_iter()
            .map(PagePreparedPopupOpening::new)
            .collect(),
    )
    .await;
    assert_eq!(
        events
            .into_iter()
            .filter(|event| event.clone().into_parts().0["method"] == "Target.targetCreated")
            .count(),
        1
    );
    let (snapshot, _) = conn.subscribe_browser_events().unwrap();
    assert!(
        conn.project_browser_snapshot(snapshot)
            .await
            .into_iter()
            .all(|event| event.into_parts().0["method"] != "Target.targetCreated")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn abandoned_popup_observation_is_recovered_from_native_state() {
    let (mut conn, owner) = source().await;
    conn.set_root_target_discovery_enabled(true);
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#orphan','orphan-popup')",
    )
    .await;
    let popup = conn
        .wait_for_renderer_popup(openings[0].clone())
        .await
        .unwrap()
        .web_contents;
    assert!(conn.project_created_web_contents(popup).await.is_empty());
    drop(openings);
    let events = conn.project_unobserved_popups().await;
    assert_eq!(
        events
            .into_iter()
            .filter(|event| event.clone().into_parts().0["method"] == "Target.targetCreated")
            .count(),
        1
    );
    let context = conn.browser_context_by_id("BID-source").unwrap();
    let target = context.target_id_for_web_contents(popup.id()).unwrap();
    let info = context.devtools_target_info(target).unwrap();
    assert_eq!(
        info.opener_id.as_ref().map(|id| id.as_str()),
        Some("TID-source")
    );
    assert!(conn.project_unobserved_popups().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn closed_native_popup_cannot_be_resurrected_by_a_late_observation() {
    let (mut conn, owner) = source().await;
    let openings = capture_openings_for_test(
        &mut conn,
        &owner,
        "window.open('about:blank#closed','closed-popup')",
    )
    .await;
    let popup = conn
        .wait_for_renderer_popup(openings[0].clone())
        .await
        .unwrap()
        .web_contents;
    conn.close_browser_web_contents_async(popup, PageCloseNotifications::BrowserEvent)
        .await;
    assert!(project(&mut conn, openings).await.is_empty());
    assert!(
        conn.browser_context_by_id("BID-source")
            .unwrap()
            .has_no_background_targets()
    );
    let (snapshot, _) = conn.subscribe_browser_events().unwrap();
    assert!(!snapshot.web_contents.contains(&popup));
}

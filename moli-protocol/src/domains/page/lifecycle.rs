use crate::devtools_runtime::{
    DevToolsFrameId, DevToolsLoaderId, DevToolsTargetId, NavigationFrameEvent,
    NavigationFrameEventKind, NavigationLifecycleEvent, PageLifecycleEvent,
};

use crate::conn::{BackgroundProtocolEvent, CdpConnection, CommittedRendererDocumentBinding};
use moli_core::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleMilestone, RendererLifecycleStartReason,
};

enum CdpPageAutomationEvent {
    NavigationFrame(NavigationFrameEvent),
    DomContentLoaded(NavigationLifecycleEvent),
    PageLifecycle(PageLifecycleEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigationStartInitiator {
    Browser,
    Renderer,
    RendererChildFrame,
}

pub(crate) fn emit_navigation_started_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
    url: &str,
    initiator: NavigationStartInitiator,
) {
    for event in navigation_started_automation_events(frame_id, loader_id, url, initiator) {
        emit_cdp_page_background_automation_event(out, event, session_id);
    }
}

fn navigation_started_automation_events(
    frame_id: &str,
    loader_id: &str,
    url: &str,
    initiator: NavigationStartInitiator,
) -> Vec<CdpPageAutomationEvent> {
    let mut events = Vec::new();
    if initiator != NavigationStartInitiator::Browser {
        events.extend([
            navigation_frame_event(
                NavigationFrameEventKind::Scheduled,
                frame_id,
                Some(loader_id),
                url,
            ),
            navigation_frame_event(
                NavigationFrameEventKind::Requested,
                frame_id,
                Some(loader_id),
                url,
            ),
        ]);
        if initiator == NavigationStartInitiator::Renderer {
            events.push(navigation_frame_event(
                NavigationFrameEventKind::ClearedScheduled,
                frame_id,
                Some(loader_id),
                url,
            ));
        }
    }
    events.extend([
        navigation_frame_event(
            NavigationFrameEventKind::StartedNavigating,
            frame_id,
            Some(loader_id),
            url,
        ),
        navigation_frame_event(
            NavigationFrameEventKind::StartedLoading,
            frame_id,
            Some(loader_id),
            url,
        ),
    ]);
    events
}

pub(crate) fn emit_navigation_lifecycle_init_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    if lifecycle_enabled {
        emit_cdp_page_lifecycle_marker_background_event(
            out, session_id, "init", frame_id, loader_id, timestamp,
        );
    }
}

pub(crate) fn emit_navigation_frame_commit_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    dom_enabled: bool,
    frame_id: &str,
    loader_id: &str,
    url: &str,
    unreachable_url: Option<&str>,
    security_origin: &str,
    secure_context_type: &str,
) {
    let event = NavigationFrameEvent {
        target_id: DevToolsTargetId::from(frame_id),
        frame_id: DevToolsFrameId::from(frame_id),
        parent_frame_id: None,
        loader_id: Some(DevToolsLoaderId::from(loader_id)),
        url: url.to_owned(),
        kind: NavigationFrameEventKind::Navigated,
        frame_name: None,
        security_origin: Some(security_origin.to_owned()),
        secure_context_type: Some(secure_context_type.to_owned()),
    };
    out.push(match unreachable_url {
        Some(unreachable_url) => {
            BackgroundProtocolEvent::page_navigation_frame_with_unreachable_url(
                session_id,
                event,
                unreachable_url,
            )
        }
        None => BackgroundProtocolEvent::page_navigation_frame(session_id, event),
    });
    if dom_enabled {
        emit_cdp_page_background_automation_event(
            out,
            navigation_frame_event(
                NavigationFrameEventKind::DocumentUpdated,
                frame_id,
                Some(loader_id),
                url,
            ),
            session_id,
        );
    }
}

fn emit_renderer_navigation_domcontentloaded_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    emit_cdp_page_background_automation_event(
        out,
        CdpPageAutomationEvent::DomContentLoaded(NavigationLifecycleEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: DevToolsFrameId::from(frame_id),
            navigation_id: None,
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: String::new(),
            timestamp,
        }),
        session_id,
    );
    if lifecycle_enabled {
        emit_cdp_page_lifecycle_marker_background_event(
            out,
            session_id,
            "DOMContentLoaded",
            frame_id,
            loader_id,
            timestamp,
        );
    }
}

pub(crate) fn emit_bound_renderer_document_lifecycle_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner_session_id: Option<&str>,
    binding: &CommittedRendererDocumentBinding,
    events: &[RendererDocumentLifecycleEvent],
) {
    let session_ids = conn.page_event_session_ids_for_session_owner(owner_session_id);
    for event in events {
        if event.frame != binding.renderer_frame || event.document != binding.renderer_document {
            continue;
        }
        let timestamp = event.timestamp_micros as f64 / 1_000_000.0;
        match event.kind {
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ) => {
                for session_id in &session_ids {
                    if crate::domains::dom::dom_agent_enabled_for_session(
                        conn,
                        session_id.as_deref(),
                    ) {
                        emit_cdp_page_background_automation_event(
                            out,
                            navigation_frame_event(
                                NavigationFrameEventKind::DocumentUpdated,
                                &binding.frame_id,
                                Some(&binding.loader_id),
                                "",
                            ),
                            session_id.as_deref(),
                        );
                    }
                    let lifecycle_enabled =
                        page_lifecycle_events_enabled_for_session(conn, session_id.as_deref());
                    emit_renderer_navigation_domcontentloaded_background_events(
                        out,
                        session_id.as_deref(),
                        lifecycle_enabled,
                        &binding.frame_id,
                        &binding.loader_id,
                        timestamp,
                    );
                }
            }
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::Load,
            ) => {
                for session_id in &session_ids {
                    let lifecycle_enabled =
                        page_lifecycle_events_enabled_for_session(conn, session_id.as_deref());
                    emit_renderer_navigation_load_background_events(
                        out,
                        session_id.as_deref(),
                        lifecycle_enabled,
                        event.document,
                        event.epoch,
                        &binding.frame_id,
                        &binding.loader_id,
                        timestamp,
                    );
                }
            }
            RendererDocumentLifecycleEventKind::Started {
                reason:
                    RendererLifecycleStartReason::ExplicitDocumentOpen
                    | RendererLifecycleStartReason::JavascriptDocumentReplacement,
            } => {
                let frame_identity = conn
                    .target_session_owner_frame_tree_identity(owner_session_id)
                    .map(|(_, url, security_origin, secure_context_type)| {
                        (url, security_origin, secure_context_type)
                    });
                for session_id in &session_ids {
                    if let Some((url, security_origin, secure_context_type)) =
                        frame_identity.as_ref()
                    {
                        out.push(BackgroundProtocolEvent::page_document_opened(
                            session_id.as_deref(),
                            binding.frame_id.clone(),
                            None,
                            binding.loader_id.clone(),
                            url.clone(),
                            None,
                            security_origin.clone(),
                            secure_context_type.clone(),
                        ));
                    }
                    let lifecycle_enabled =
                        page_lifecycle_events_enabled_for_session(conn, session_id.as_deref());
                    emit_navigation_lifecycle_init_background_events(
                        out,
                        session_id.as_deref(),
                        lifecycle_enabled,
                        &binding.frame_id,
                        &binding.loader_id,
                        timestamp,
                    );
                }
            }
            RendererDocumentLifecycleEventKind::Started { .. }
            | RendererDocumentLifecycleEventKind::Terminated { .. } => {}
        }
    }
}

fn page_lifecycle_events_enabled_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    conn.target_page_session_state_for_session(session_id)
        .is_some_and(|state| state.page_lifecycle_events)
}

pub(crate) fn emit_navigation_frame_stop_after_download_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
) {
    emit_cdp_page_background_automation_event(
        out,
        navigation_frame_event(
            NavigationFrameEventKind::ClearedScheduled,
            frame_id,
            Some(loader_id),
            "",
        ),
        session_id,
    );
    emit_cdp_page_background_automation_event(
        out,
        navigation_frame_event(
            NavigationFrameEventKind::StoppedLoading,
            frame_id,
            Some(loader_id),
            "",
        ),
        session_id,
    );
}

pub(crate) fn emit_child_frame_navigation_commit(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    frame_id: &str,
    parent_frame_id: Option<&str>,
    frame_name: Option<&str>,
    loader_id: &str,
    url: &str,
    security_origin: &str,
    secure_context_type: &str,
) {
    emit_cdp_page_background_automation_event(
        out,
        CdpPageAutomationEvent::NavigationFrame(NavigationFrameEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: DevToolsFrameId::from(frame_id),
            parent_frame_id: parent_frame_id.map(DevToolsFrameId::from),
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: url.to_owned(),
            kind: NavigationFrameEventKind::Navigated,
            frame_name: frame_name.map(str::to_owned),
            security_origin: Some(security_origin.to_owned()),
            secure_context_type: Some(secure_context_type.to_owned()),
        }),
        session_id,
    );
}

pub(crate) fn emit_child_frame_lifecycle_terminal(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    if lifecycle_enabled {
        emit_cdp_page_lifecycle_marker_background_event(
            out, session_id, "init", frame_id, loader_id, timestamp,
        );
        emit_cdp_page_lifecycle_marker_background_event(
            out,
            session_id,
            "DOMContentLoaded",
            frame_id,
            loader_id,
            timestamp,
        );
        emit_cdp_page_lifecycle_marker_background_event(
            out, session_id, "load", frame_id, loader_id, timestamp,
        );
        emit_cdp_page_lifecycle_marker_background_event(
            out,
            session_id,
            "networkAlmostIdle",
            frame_id,
            loader_id,
            timestamp,
        );
        emit_cdp_page_lifecycle_marker_background_event(
            out,
            session_id,
            "networkIdle",
            frame_id,
            loader_id,
            timestamp,
        );
    }
    emit_cdp_page_background_automation_event(
        out,
        navigation_frame_event(
            NavigationFrameEventKind::StoppedLoading,
            frame_id,
            Some(loader_id),
            "",
        ),
        session_id,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_child_frame_document_opened_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    frame_id: &str,
    parent_frame_id: Option<&str>,
    frame_name: Option<&str>,
    loader_id: &str,
    url: &str,
    security_origin: &str,
    secure_context_type: &str,
    timestamp: f64,
) {
    out.push(BackgroundProtocolEvent::page_document_opened(
        session_id,
        frame_id,
        parent_frame_id.map(str::to_owned),
        loader_id,
        url,
        frame_name.map(str::to_owned),
        security_origin,
        secure_context_type,
    ));
    emit_navigation_lifecycle_init_background_events(
        out,
        session_id,
        lifecycle_enabled,
        frame_id,
        loader_id,
        timestamp,
    );
}

pub(crate) fn emit_child_frame_document_open_completed_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    if lifecycle_enabled {
        emit_cdp_page_lifecycle_marker_background_event(
            out,
            session_id,
            "DOMContentLoaded",
            frame_id,
            loader_id,
            timestamp,
        );
        emit_cdp_page_lifecycle_marker_background_event(
            out, session_id, "load", frame_id, loader_id, timestamp,
        );
    }
}

fn emit_cdp_page_background_automation_event(
    out: &mut Vec<BackgroundProtocolEvent>,
    event: CdpPageAutomationEvent,
    session_id: Option<&str>,
) {
    match event {
        CdpPageAutomationEvent::NavigationFrame(event) => {
            out.push(BackgroundProtocolEvent::page_navigation_frame(
                session_id, event,
            ));
        }
        CdpPageAutomationEvent::DomContentLoaded(event) => {
            out.push(BackgroundProtocolEvent::page_dom_content_loaded(
                session_id, event,
            ));
        }
        CdpPageAutomationEvent::PageLifecycle(event) => {
            out.push(BackgroundProtocolEvent::page_lifecycle(session_id, event));
        }
    }
}

fn emit_cdp_page_lifecycle_marker_background_event(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    name: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    emit_cdp_page_background_automation_event(
        out,
        CdpPageAutomationEvent::PageLifecycle(PageLifecycleEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: DevToolsFrameId::from(frame_id),
            loader_id: DevToolsLoaderId::from(loader_id),
            name: name.to_owned(),
            timestamp,
        }),
        session_id,
    );
}

pub(crate) fn emit_navigation_network_idle_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    if !lifecycle_enabled {
        return;
    }
    emit_cdp_page_lifecycle_marker_background_event(
        out,
        session_id,
        "networkAlmostIdle",
        frame_id,
        loader_id,
        timestamp,
    );
    emit_cdp_page_lifecycle_marker_background_event(
        out,
        session_id,
        "networkIdle",
        frame_id,
        loader_id,
        timestamp,
    );
}

pub(crate) fn emit_navigation_frame_stopped_loading_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
) {
    emit_cdp_page_background_automation_event(
        out,
        navigation_frame_event(
            NavigationFrameEventKind::StoppedLoading,
            frame_id,
            Some(loader_id),
            "",
        ),
        session_id,
    );
}

fn emit_renderer_navigation_load_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    lifecycle_enabled: bool,
    renderer_document: moli_core::page::RendererDocumentToken,
    renderer_epoch: moli_core::page::RendererLifecycleEpoch,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
) {
    out.push(BackgroundProtocolEvent::page_load_for_renderer_document(
        session_id,
        NavigationLifecycleEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: DevToolsFrameId::from(frame_id),
            navigation_id: None,
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: String::new(),
            timestamp,
        },
        renderer_document,
        renderer_epoch,
    ));
    if lifecycle_enabled {
        emit_cdp_page_lifecycle_marker_background_event(
            out, session_id, "load", frame_id, loader_id, timestamp,
        );
    }
}

fn navigation_frame_event(
    kind: NavigationFrameEventKind,
    frame_id: &str,
    loader_id: Option<&str>,
    url: &str,
) -> CdpPageAutomationEvent {
    CdpPageAutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from(frame_id),
        frame_id: DevToolsFrameId::from(frame_id),
        parent_frame_id: None,
        loader_id: loader_id.map(DevToolsLoaderId::from),
        url: url.to_owned(),
        kind,
        frame_name: None,
        security_origin: None,
        secure_context_type: None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn protocol_messages_from_background_events(
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Vec<serde_json::Value> {
        events
            .into_iter()
            .map(|event| {
                assert!(
                    event.protocol_message().is_none(),
                    "Page lifecycle events should stay typed until wire projection: {event:?}"
                );
                assert!(event.protocol_method().is_some());
                assert!(event.has_protocol_wire_message());
                event.into_protocol_message()
            })
            .collect()
    }

    #[test]
    fn renderer_navigation_started_serializes_requested_events_before_load_start() {
        let mut events = Vec::new();

        super::emit_navigation_started_background_events(
            &mut events,
            Some("SID-page"),
            "FRAME-page",
            "LOADER-page",
            "https://example.test/start",
            super::NavigationStartInitiator::Renderer,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "Page.frameScheduledNavigation",
                "Page.frameRequestedNavigation",
                "Page.frameClearedScheduledNavigation",
                "Page.frameStartedNavigating",
                "Page.frameStartedLoading",
            ]
        );
        assert_eq!(out[0]["sessionId"], json!("SID-page"));
        assert_eq!(
            out[0]["params"],
            json!({
                "frameId": "FRAME-page",
                "delay": 0,
                "reason": "scriptInitiated",
                "url": "https://example.test/start",
            })
        );
        assert_eq!(
            out[1]["params"],
            json!({
                "frameId": "FRAME-page",
                "reason": "scriptInitiated",
                "url": "https://example.test/start",
                "disposition": "currentTab",
            })
        );
        assert_eq!(
            out[3]["params"],
            json!({
                "frameId": "FRAME-page",
                "url": "https://example.test/start",
                "loaderId": "LOADER-page",
                "navigationType": "differentDocument",
            })
        );
        assert_eq!(out[2]["params"], json!({ "frameId": "FRAME-page" }));
        assert_eq!(out[4]["params"], json!({ "frameId": "FRAME-page" }));
    }

    #[test]
    fn browser_navigation_started_does_not_fabricate_renderer_request_events() {
        let mut events = Vec::new();

        super::emit_navigation_started_background_events(
            &mut events,
            Some("SID-page"),
            "FRAME-page",
            "LOADER-page",
            "https://example.test/start",
            super::NavigationStartInitiator::Browser,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Page.frameStartedNavigating", "Page.frameStartedLoading",]
        );
    }

    #[test]
    fn child_renderer_navigation_does_not_invent_top_level_clear_timing() {
        let mut events = Vec::new();

        super::emit_navigation_started_background_events(
            &mut events,
            Some("SID-page"),
            "FRAME-child",
            "LOADER-child",
            "https://example.test/child",
            super::NavigationStartInitiator::RendererChildFrame,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "Page.frameScheduledNavigation",
                "Page.frameRequestedNavigation",
                "Page.frameStartedNavigating",
                "Page.frameStartedLoading",
            ]
        );
    }

    #[test]
    fn navigation_frame_commit_serializes_from_automation_event_shape() {
        let mut events = Vec::new();

        super::emit_navigation_frame_commit_background_events(
            &mut events,
            Some("SID-page"),
            true,
            "FRAME-page",
            "LOADER-page",
            "https://example.test/commit",
            None,
            "https://example.test",
            "Secure",
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Page.frameNavigated", "DOM.documentUpdated"]
        );
        assert_eq!(out[0]["sessionId"], json!("SID-page"));
        assert_eq!(
            out[0]["params"],
            json!({
                "type": "Navigation",
                "frame": {
                    "id": "FRAME-page",
                    "loaderId": "LOADER-page",
                    "url": "https://example.test/commit",
                    "domainAndRegistry": "",
                    "securityOrigin": "https://example.test",
                    "mimeType": "text/html",
                    "adFrameStatus": { "adFrameType": "none" },
                    "secureContextType": "Secure",
                    "crossOriginIsolatedContextType": "NotIsolated",
                    "gatedAPIFeatures": [],
                }
            })
        );
        assert_eq!(out[1]["params"], json!({}));
    }

    #[test]
    fn navigation_frame_commit_omits_document_updated_when_dom_is_disabled() {
        let mut events = Vec::new();

        super::emit_navigation_frame_commit_background_events(
            &mut events,
            Some("SID-page"),
            false,
            "FRAME-page",
            "LOADER-page",
            "https://example.test/",
            None,
            "https://example.test",
            "Secure",
        );

        let out = protocol_messages_from_background_events(events);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], "Page.frameNavigated");
    }

    #[test]
    fn child_frame_navigation_serializes_from_automation_event_shape() {
        let mut events = Vec::new();

        super::emit_child_frame_navigation_commit(
            &mut events,
            Some("SID-page"),
            "FRAME-child",
            Some("FRAME-parent"),
            Some("child-name"),
            "LOADER-child",
            "https://child.example.test/",
            "https://child.example.test",
            "Secure",
        );
        super::emit_child_frame_lifecycle_terminal(
            &mut events,
            Some("SID-page"),
            false,
            "FRAME-child",
            "LOADER-child",
            14.5,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Page.frameNavigated", "Page.frameStoppedLoading"]
        );
        assert_eq!(
            out[0]["params"],
            json!({
                "type": "Navigation",
                "frame": {
                    "id": "FRAME-child",
                    "parentId": "FRAME-parent",
                    "loaderId": "LOADER-child",
                    "url": "https://child.example.test/",
                    "domainAndRegistry": "",
                    "securityOrigin": "https://child.example.test",
                    "mimeType": "text/html",
                    "adFrameStatus": { "adFrameType": "none" },
                    "secureContextType": "Secure",
                    "crossOriginIsolatedContextType": "NotIsolated",
                    "gatedAPIFeatures": [],
                    "name": "child-name",
                }
            })
        );
        assert_eq!(out[1]["params"], json!({ "frameId": "FRAME-child" }));
    }

    #[test]
    fn dom_content_loaded_serializes_from_automation_event_shape() {
        let mut events = Vec::new();

        super::emit_renderer_navigation_domcontentloaded_background_events(
            &mut events,
            Some("SID-page"),
            true,
            "FRAME-page",
            "LOADER-page",
            12.5,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["method"], json!("Page.domContentEventFired"));
        assert_eq!(out[0]["sessionId"], json!("SID-page"));
        assert_eq!(out[0]["params"], json!({ "timestamp": 12.5 }));
        assert_eq!(out[1]["method"], json!("Page.lifecycleEvent"));
        assert_eq!(out[1]["sessionId"], json!("SID-page"));
        assert_eq!(
            out[1]["params"],
            json!({
                "frameId": "FRAME-page",
                "loaderId": "LOADER-page",
                "name": "DOMContentLoaded",
                "timestamp": 12.5,
            })
        );
    }

    #[test]
    fn renderer_load_network_idle_and_frame_stop_are_independent_outputs() {
        let mut events = Vec::new();
        let page_id = moli_core::PageId::new_for_testing(903);

        super::emit_renderer_navigation_load_background_events(
            &mut events,
            Some("SID-page"),
            true,
            moli_core::page::RendererDocumentToken::new_for_testing(page_id, 1),
            moli_core::page::RendererLifecycleEpoch(1),
            "FRAME-page",
            "LOADER-page",
            13.5,
        );
        super::emit_navigation_network_idle_background_events(
            &mut events,
            Some("SID-page"),
            true,
            "FRAME-page",
            "LOADER-page",
            14.5,
        );
        super::emit_navigation_frame_stopped_loading_background_events(
            &mut events,
            Some("SID-page"),
            "FRAME-page",
            "LOADER-page",
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "Page.loadEventFired",
                "Page.lifecycleEvent",
                "Page.lifecycleEvent",
                "Page.lifecycleEvent",
                "Page.frameStoppedLoading",
            ]
        );
        assert_eq!(out[0]["sessionId"], json!("SID-page"));
        assert_eq!(out[0]["params"], json!({ "timestamp": 13.5 }));
        assert_eq!(
            out[1]["params"],
            json!({
                "frameId": "FRAME-page",
                "loaderId": "LOADER-page",
                "name": "load",
                "timestamp": 13.5,
            })
        );
        assert_eq!(out[2]["params"]["name"], json!("networkAlmostIdle"));
        assert_eq!(out[2]["params"]["timestamp"], json!(14.5));
        assert_eq!(out[3]["params"]["name"], json!("networkIdle"));
        assert_eq!(
            out[4]["params"],
            json!({
                "frameId": "FRAME-page",
            })
        );
    }

    #[test]
    fn download_frame_stop_emits_background_automation_events() {
        let mut out = Vec::new();

        super::emit_navigation_frame_stop_after_download_background_events(
            &mut out,
            Some("SID-page"),
            "FRAME-page",
            "LOADER-page",
        );

        let parts = out
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        assert!(
            parts
                .iter()
                .all(|(_, automation_event)| automation_event.is_some())
        );
        let out = parts
            .into_iter()
            .map(|(message, _)| message)
            .collect::<Vec<_>>();

        assert_eq!(
            out.iter()
                .map(|message| message["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "Page.frameClearedScheduledNavigation",
                "Page.frameStoppedLoading"
            ]
        );
        assert_eq!(out[0]["sessionId"], json!("SID-page"));
        assert_eq!(out[0]["params"], json!({ "frameId": "FRAME-page" }));
        assert_eq!(out[1]["sessionId"], json!("SID-page"));
        assert_eq!(out[1]["params"], json!({ "frameId": "FRAME-page" }));
    }
}

use std::collections::{BTreeMap, BTreeSet, HashMap};

use moli_page_types::{
    DevToolsSessionKey, RendererDomDebuggerDomBreakpointType,
    RendererDomDebuggerEventListenerBreakpoint, RendererDomDebuggerXhrBreakpoint,
};
use serde_json::{Value, json};

use super::JsContextHost;
use crate::{
    document_runtime::{DevToolsDomPrepublishedRemoval, DomHandle, EventTargetHandle},
    frame_owner_model::DocumentId,
    runtime::{
        RendererRuntimeInspectorMessage,
        page_dom::{inspector_whitespace_text_node, live_inspector_document_node_snapshot},
    },
    script_vm::{RendererDomDebuggerPauseScheduler, RendererDomDebuggerScheduledPause},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct DomBreakpointNodeKey {
    document_id: DocumentId,
    handle: DomHandle,
}

#[derive(Clone, Debug)]
struct DomBreakpointPause {
    session_key: DevToolsSessionKey,
    owner: DomHandle,
    target: DomHandle,
    breakpoint_type: RendererDomDebuggerDomBreakpointType,
    insertion: bool,
}

pub(super) struct DomDebuggerState {
    event_listener_breakpoints:
        BTreeMap<DevToolsSessionKey, BTreeSet<RendererDomDebuggerEventListenerBreakpoint>>,
    xhr_breakpoints: BTreeMap<DevToolsSessionKey, BTreeSet<RendererDomDebuggerXhrBreakpoint>>,
    dom_breakpoints: BTreeMap<
        DevToolsSessionKey,
        HashMap<DomBreakpointNodeKey, BTreeSet<RendererDomDebuggerDomBreakpointType>>,
    >,
    pause_scheduler: RendererDomDebuggerPauseScheduler,
}

impl DomDebuggerState {
    pub(super) fn new(pause_scheduler: RendererDomDebuggerPauseScheduler) -> Self {
        Self {
            event_listener_breakpoints: BTreeMap::new(),
            xhr_breakpoints: BTreeMap::new(),
            dom_breakpoints: BTreeMap::new(),
            pause_scheduler,
        }
    }

    fn configure_event_listener_breakpoint(
        &mut self,
        session_key: DevToolsSessionKey,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) {
        if enabled {
            self.event_listener_breakpoints
                .entry(session_key)
                .or_default()
                .insert(breakpoint);
            return;
        }
        let remove_session = self
            .event_listener_breakpoints
            .get_mut(&session_key)
            .is_some_and(|breakpoints| {
                breakpoints.remove(&breakpoint);
                breakpoints.is_empty()
            });
        if remove_session {
            self.event_listener_breakpoints.remove(&session_key);
        }
    }

    fn schedule_event_listener_pause(
        &self,
        event_name: &str,
        target_name: &str,
    ) -> Option<RendererDomDebuggerScheduledPause> {
        let session_keys = self
            .event_listener_breakpoints
            .iter()
            .filter(|(_, breakpoints)| {
                breakpoints
                    .iter()
                    .any(|breakpoint| breakpoint.matches(event_name, target_name))
            })
            .map(|(session_key, _)| session_key.clone())
            .collect::<Vec<_>>();
        if session_keys.is_empty() {
            return None;
        }
        let detail = json!({
            "eventName": format!("listener:{event_name}"),
            "targetName": target_name,
        })
        .to_string();
        Some(self.pause_scheduler.schedule_pause_on_next_statement(
            &session_keys,
            "EventListener",
            &detail,
        ))
    }

    fn configure_xhr_breakpoint(
        &mut self,
        session_key: DevToolsSessionKey,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) {
        if enabled {
            self.xhr_breakpoints
                .entry(session_key)
                .or_default()
                .insert(breakpoint);
            return;
        }
        let remove_session =
            self.xhr_breakpoints
                .get_mut(&session_key)
                .is_some_and(|breakpoints| {
                    breakpoints.remove(&breakpoint);
                    breakpoints.is_empty()
                });
        if remove_session {
            self.xhr_breakpoints.remove(&session_key);
        }
    }

    fn break_on_xhr_or_fetch_network_request(&self, request_url: &str) {
        let pauses = self
            .xhr_breakpoints
            .iter()
            .filter_map(|(session_key, breakpoints)| {
                let matched = breakpoints
                    .iter()
                    .find(|breakpoint| breakpoint.url.is_empty())
                    .or_else(|| {
                        breakpoints
                            .iter()
                            .find(|breakpoint| breakpoint.matches(request_url))
                    })?;
                Some((
                    session_key.clone(),
                    json!({
                        "breakpointURL": matched.url,
                        "url": request_url,
                    })
                    .to_string(),
                ))
            })
            .collect::<Vec<_>>();
        self.pause_scheduler
            .break_program_for_sessions(pauses, "XHR");
    }

    fn configure_dom_breakpoint(
        &mut self,
        session_key: DevToolsSessionKey,
        node: DomBreakpointNodeKey,
        breakpoint_type: RendererDomDebuggerDomBreakpointType,
        enabled: bool,
    ) {
        if enabled {
            self.dom_breakpoints
                .entry(session_key)
                .or_default()
                .entry(node)
                .or_default()
                .insert(breakpoint_type);
            return;
        }
        let remove_session = self
            .dom_breakpoints
            .get_mut(&session_key)
            .is_some_and(|nodes| {
                let remove_node = nodes.get_mut(&node).is_some_and(|breakpoints| {
                    breakpoints.remove(&breakpoint_type);
                    breakpoints.is_empty()
                });
                if remove_node {
                    nodes.remove(&node);
                }
                nodes.is_empty()
            });
        if remove_session {
            self.dom_breakpoints.remove(&session_key);
        }
    }

    fn direct_dom_breakpoint_pauses(
        &self,
        document_id: DocumentId,
        target: DomHandle,
        breakpoint_type: RendererDomDebuggerDomBreakpointType,
    ) -> Vec<DomBreakpointPause> {
        let key = DomBreakpointNodeKey {
            document_id,
            handle: target,
        };
        self.dom_breakpoints
            .iter()
            .filter(|(_, nodes)| {
                nodes
                    .get(&key)
                    .is_some_and(|breakpoints| breakpoints.contains(&breakpoint_type))
            })
            .map(|(session_key, _)| DomBreakpointPause {
                session_key: session_key.clone(),
                owner: target,
                target,
                breakpoint_type,
                insertion: false,
            })
            .collect()
    }

    fn subtree_dom_breakpoint_pauses(
        &self,
        dom_host: &crate::dom::native::DomHost,
        document_id: DocumentId,
        target: DomHandle,
        first_owner_candidate: Option<DomHandle>,
        insertion: bool,
    ) -> Vec<DomBreakpointPause> {
        self.dom_breakpoints
            .iter()
            .filter_map(|(session_key, nodes)| {
                let mut candidate = first_owner_candidate;
                let owner = loop {
                    let handle = candidate?;
                    let key = DomBreakpointNodeKey {
                        document_id,
                        handle,
                    };
                    if nodes.get(&key).is_some_and(|breakpoints| {
                        breakpoints.contains(&RendererDomDebuggerDomBreakpointType::SubtreeModified)
                    }) {
                        break handle;
                    }
                    candidate = dom_host.parent_node(handle);
                };
                Some(DomBreakpointPause {
                    session_key: session_key.clone(),
                    owner,
                    target,
                    breakpoint_type: RendererDomDebuggerDomBreakpointType::SubtreeModified,
                    insertion,
                })
            })
            .collect()
    }

    fn remove_breakpoints_in_subtree(&mut self, document_id: DocumentId, handles: &[DomHandle]) {
        self.dom_breakpoints.retain(|_, nodes| {
            for &handle in handles {
                nodes.remove(&DomBreakpointNodeKey {
                    document_id,
                    handle,
                });
            }
            !nodes.is_empty()
        });
    }

    fn clear_dom_breakpoints_for_session(&mut self, session_key: &DevToolsSessionKey) {
        self.dom_breakpoints.remove(session_key);
    }

    fn remove_session(&mut self, session_key: &DevToolsSessionKey) {
        self.event_listener_breakpoints.remove(session_key);
        self.xhr_breakpoints.remove(session_key);
        self.dom_breakpoints.remove(session_key);
    }
}

impl JsContextHost {
    pub(crate) fn has_dom_debugger_dom_breakpoints(&self) -> bool {
        !self.dom_debugger_state.dom_breakpoints.is_empty()
    }

    pub(crate) fn configure_dom_debugger_event_listener_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.dom_debugger_state.configure_event_listener_breakpoint(
            session_key,
            breakpoint,
            enabled,
        );
    }

    pub(crate) fn schedule_dom_debugger_event_listener_pause_for_target(
        &self,
        event_name: &str,
        target: EventTargetHandle,
    ) -> Option<RendererDomDebuggerScheduledPause> {
        if self
            .dom_debugger_state
            .event_listener_breakpoints
            .is_empty()
        {
            return None;
        }
        let target_name = match target {
            EventTargetHandle::Window
            | EventTargetHandle::ChildWindow(_)
            | EventTargetHandle::PopupWindow(_) => "Window".to_owned(),
            EventTargetHandle::Node(handle) => self
                .dom_host()
                .node(handle)
                .map(|node| node.node_name())
                .unwrap_or_default(),
        };
        self.schedule_dom_debugger_event_listener_pause_for_interface(event_name, &target_name)
    }

    pub(crate) fn schedule_dom_debugger_event_listener_pause_for_interface(
        &self,
        event_name: &str,
        target_name: &str,
    ) -> Option<RendererDomDebuggerScheduledPause> {
        self.dom_debugger_state
            .schedule_event_listener_pause(event_name, target_name)
    }

    pub(crate) fn configure_dom_debugger_xhr_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.dom_debugger_state
            .configure_xhr_breakpoint(session_key, breakpoint, enabled);
    }

    pub(crate) fn configure_dom_debugger_dom_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: DocumentId,
        handle: DomHandle,
        breakpoint_type: RendererDomDebuggerDomBreakpointType,
        enabled: bool,
    ) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.dom_debugger_state.configure_dom_breakpoint(
            session_key,
            DomBreakpointNodeKey {
                document_id,
                handle,
            },
            breakpoint_type,
            enabled,
        );
    }

    pub(crate) fn break_on_dom_debugger_will_insert_dom_node(&mut self, parent: DomHandle) {
        if self.dom_debugger_state.dom_breakpoints.is_empty() {
            return;
        }
        let Some(document_id) = self.document_id_for_backend_node_identity_handle(parent) else {
            return;
        };
        let pauses = self.dom_debugger_state.subtree_dom_breakpoint_pauses(
            self.dom_host(),
            document_id,
            parent,
            Some(parent),
            true,
        );
        let _ = self.break_on_dom_debugger_dom_pauses(document_id, pauses, None);
    }

    pub(crate) fn break_on_dom_debugger_character_data_modified(&mut self, target: DomHandle) {
        if self.dom_debugger_state.dom_breakpoints.is_empty() {
            return;
        }
        let Some(document_id) = self.document_id_for_backend_node_identity_handle(target) else {
            return;
        };
        let pauses = self.dom_debugger_state.subtree_dom_breakpoint_pauses(
            self.dom_host(),
            document_id,
            target,
            self.dom_host().parent_node(target),
            false,
        );
        let _ = self.break_on_dom_debugger_dom_pauses(document_id, pauses, None);
    }

    pub(crate) fn break_on_dom_debugger_will_modify_dom_attribute(&mut self, target: DomHandle) {
        if self.dom_debugger_state.dom_breakpoints.is_empty() {
            return;
        }
        let Some(document_id) = self.document_id_for_backend_node_identity_handle(target) else {
            return;
        };
        let pauses = self.dom_debugger_state.direct_dom_breakpoint_pauses(
            document_id,
            target,
            RendererDomDebuggerDomBreakpointType::AttributeModified,
        );
        let _ = self.break_on_dom_debugger_dom_pauses(document_id, pauses, None);
    }

    pub(crate) fn break_on_dom_debugger_will_remove_dom_node(
        &mut self,
        target: DomHandle,
    ) -> Vec<DevToolsDomPrepublishedRemoval> {
        if self.dom_debugger_state.dom_breakpoints.is_empty() {
            return Vec::new();
        }
        let Some(document_id) = self.document_id_for_backend_node_identity_handle(target) else {
            return Vec::new();
        };
        let direct = self.dom_debugger_state.direct_dom_breakpoint_pauses(
            document_id,
            target,
            RendererDomDebuggerDomBreakpointType::NodeRemoved,
        );
        let direct_sessions = direct
            .iter()
            .map(|pause| pause.session_key.clone())
            .collect::<BTreeSet<_>>();
        let mut pauses = direct;
        pauses.extend(
            self.dom_debugger_state
                .subtree_dom_breakpoint_pauses(
                    self.dom_host(),
                    document_id,
                    target,
                    self.dom_host().parent_node(target),
                    false,
                )
                .into_iter()
                .filter(|pause| !direct_sessions.contains(&pause.session_key)),
        );
        let prepublished_removals =
            self.break_on_dom_debugger_dom_pauses(document_id, pauses, Some(target));

        let mut removed = vec![target];
        let mut cursor = 0;
        while let Some(&handle) = removed.get(cursor) {
            cursor += 1;
            removed.extend(self.dom_host().child_handles(handle));
        }
        self.dom_debugger_state
            .remove_breakpoints_in_subtree(document_id, &removed);
        prepublished_removals
    }

    fn break_on_dom_debugger_dom_pauses(
        &mut self,
        document_id: DocumentId,
        pauses: Vec<DomBreakpointPause>,
        removal_target: Option<DomHandle>,
    ) -> Vec<DevToolsDomPrepublishedRemoval> {
        let mut scheduled_pauses = Vec::new();
        let mut prepublished_removals = Vec::new();
        for pause in pauses {
            let inspector_session_id = pause.session_key.wire_session_id();
            let Some((owner_node_id, mut preface)) = self.push_dom_debugger_node_path_to_frontend(
                inspector_session_id,
                document_id,
                pause.owner,
            ) else {
                continue;
            };
            let mut detail = serde_json::Map::from_iter([
                ("nodeId".to_owned(), json!(owner_node_id)),
                ("type".to_owned(), json!(pause.breakpoint_type.cdp_name())),
            ]);
            if pause.breakpoint_type == RendererDomDebuggerDomBreakpointType::SubtreeModified {
                let Some((target_node_id, target_preface)) = self
                    .push_dom_debugger_node_path_to_frontend(
                        inspector_session_id,
                        document_id,
                        pause.target,
                    )
                else {
                    continue;
                };
                preface.extend(target_preface);
                detail.insert("targetNodeId".to_owned(), json!(target_node_id));
                detail.insert("insertion".to_owned(), json!(pause.insertion));
            }
            if let Some(removal_target) = removal_target
                && let Some((mut removal_preface, removal)) = self
                    .prepublish_dom_node_removal_for_session(
                        inspector_session_id,
                        document_id,
                        removal_target,
                    )
            {
                preface.append(&mut removal_preface);
                prepublished_removals.push(removal);
            }
            scheduled_pauses.push((
                pause.session_key,
                Value::Object(detail).to_string(),
                preface,
            ));
        }
        self.dom_debugger_state
            .pause_scheduler
            .break_program_for_sessions_with_prefaces(scheduled_pauses, "DOM");
        prepublished_removals
    }

    fn prepublish_dom_node_removal_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: DocumentId,
        target: DomHandle,
    ) -> Option<(
        Vec<RendererRuntimeInspectorMessage>,
        DevToolsDomPrepublishedRemoval,
    )> {
        let parent = self.dom_host().parent_node(target)?;
        let (_, mut events) = self.push_dom_debugger_node_path_to_frontend(
            inspector_session_id,
            document_id,
            parent,
        )?;
        events.extend(self.push_dom_debugger_children_to_frontend(
            inspector_session_id,
            document_id,
            parent,
        )?);

        let parent_backend_node_id = self
            .dom_agent_state
            .backend_node_id_for_node(document_id, parent);
        let parent_node_id = self
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                Some(document_id),
                parent_backend_node_id,
            )?;
        let target_backend_node_id = self
            .dom_agent_state
            .backend_node_id_for_node(document_id, target);
        let target_node_id = self
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                Some(document_id),
                target_backend_node_id,
            )?;

        let include_whitespace = self
            .dom_agent_state
            .includes_whitespace(inspector_session_id, Some(document_id));
        let child_count = self
            .dom_host()
            .child_handles(parent)
            .filter(|&handle| {
                include_whitespace || !inspector_whitespace_text_node(self.dom_host(), handle)
            })
            .count();
        self.dom_agent_state.cache_child_count(
            inspector_session_id,
            Some(document_id),
            parent_backend_node_id,
            child_count.saturating_sub(1),
        );
        let mut removed = vec![target];
        let mut cursor = 0;
        while let Some(&handle) = removed.get(cursor) {
            cursor += 1;
            removed.extend(self.dom_host().child_handles(handle));
            if let Some(shadow_root) = self.dom_host().shadow_root_handle(handle) {
                removed.push(shadow_root);
            }
        }
        let removed_backend_node_ids = removed
            .into_iter()
            .map(|handle| {
                self.dom_agent_state
                    .backend_node_id_for_node(document_id, handle)
            })
            .collect::<Vec<_>>();
        self.dom_agent_state
            .remove_frontend_bindings_for_backend_node_ids(
                inspector_session_id,
                Some(document_id),
                removed_backend_node_ids,
            );
        events.push(RendererRuntimeInspectorMessage::protocol(json!({
            "method": "DOM.childNodeRemoved",
            "params": {
                "parentNodeId": parent_node_id,
                "nodeId": target_node_id,
            },
        })));
        Some((
            events,
            DevToolsDomPrepublishedRemoval::new(
                inspector_session_id.map(str::to_owned),
                parent,
                target,
            ),
        ))
    }

    fn push_dom_debugger_children_to_frontend(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: DocumentId,
        parent: DomHandle,
    ) -> Option<Vec<RendererRuntimeInspectorMessage>> {
        let parent_backend_node_id = self
            .dom_agent_state
            .backend_node_id_for_node(document_id, parent);
        if self.dom_agent_state.children_requested(
            inspector_session_id,
            Some(document_id),
            parent_backend_node_id,
        ) {
            return Some(Vec::new());
        }
        let parent_node_id = self
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                Some(document_id),
                parent_backend_node_id,
            )?;
        let include_whitespace = self
            .dom_agent_state
            .includes_whitespace(inspector_session_id, Some(document_id));
        let children = self
            .dom_host()
            .child_handles(parent)
            .filter(|&handle| {
                include_whitespace || !inspector_whitespace_text_node(self.dom_host(), handle)
            })
            .collect::<Vec<_>>();
        let mut payloads = Vec::with_capacity(children.len());
        for child in children.iter().copied() {
            let mut snapshot = live_inspector_document_node_snapshot(
                self.dom_host(),
                child,
                0,
                Some(parent),
                false,
                include_whitespace,
            )?;
            let backend_node_id = self
                .dom_agent_state
                .backend_node_id_for_node(document_id, child);
            let frontend_node_id = self.dom_agent_state.frontend_node_id_for_backend_node_id(
                inspector_session_id,
                Some(document_id),
                backend_node_id,
            );
            snapshot.backend_node_id = Some(backend_node_id);
            snapshot.frontend_node_id = Some(frontend_node_id);
            snapshot.parent_frontend_node_id = Some(parent_node_id);
            payloads.push(moli_protocol_cdp::node_snapshot_to_cdp(
                &snapshot, None, None,
            )?);
        }
        self.dom_agent_state.mark_children_requested(
            inspector_session_id,
            Some(document_id),
            parent_backend_node_id,
            children.len(),
        );
        Some(vec![RendererRuntimeInspectorMessage::protocol(json!({
            "method": "DOM.setChildNodes",
            "params": {
                "parentId": parent_node_id,
                "nodes": payloads,
            },
        }))])
    }

    fn push_dom_debugger_node_path_to_frontend(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: DocumentId,
        target: DomHandle,
    ) -> Option<(u32, Vec<RendererRuntimeInspectorMessage>)> {
        let target_backend_node_id = self
            .dom_agent_state
            .backend_node_id_for_node(document_id, target);
        if let Some(frontend_node_id) = self
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                Some(document_id),
                target_backend_node_id,
            )
        {
            return Some((frontend_node_id, Vec::new()));
        }

        let mut unbound_path = Vec::new();
        let mut current = target;
        let anchor = loop {
            let backend_node_id = self
                .dom_agent_state
                .backend_node_id_for_node(document_id, current);
            if let Some(frontend_node_id) = self
                .dom_agent_state
                .frontend_node_id_for_existing_backend_node_id(
                    inspector_session_id,
                    Some(document_id),
                    backend_node_id,
                )
            {
                break (current, frontend_node_id);
            }
            unbound_path.push(current);
            current = self.dom_host().parent_node(current)?;
        };

        let mut events = Vec::new();
        let mut parent = anchor.0;
        let mut parent_frontend_node_id = anchor.1;
        for next in unbound_path.iter().rev() {
            events.extend(self.push_dom_debugger_children_to_frontend(
                inspector_session_id,
                document_id,
                parent,
            )?);
            parent = *next;
            let next_backend_node_id = self
                .dom_agent_state
                .backend_node_id_for_node(document_id, parent);
            parent_frontend_node_id = self
                .dom_agent_state
                .frontend_node_id_for_existing_backend_node_id(
                    inspector_session_id,
                    Some(document_id),
                    next_backend_node_id,
                )?;
        }
        Some((parent_frontend_node_id, events))
    }

    pub(crate) fn clear_dom_debugger_dom_breakpoints_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.dom_debugger_state
            .clear_dom_breakpoints_for_session(&session_key);
    }

    pub(crate) fn break_on_dom_debugger_xhr_or_fetch_network_request(&self, request_url: &str) {
        if self.dom_debugger_state.xhr_breakpoints.is_empty() {
            return;
        }
        self.dom_debugger_state
            .break_on_xhr_or_fetch_network_request(request_url);
    }

    pub(crate) fn has_dom_debugger_event_listener_breakpoints(&self) -> bool {
        !self
            .dom_debugger_state
            .event_listener_breakpoints
            .is_empty()
    }

    pub(crate) fn remove_dom_debugger_session(&mut self, inspector_session_id: Option<&str>) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.dom_debugger_state.remove_session(&session_key);
        self.dom_agent_state.remove_session(inspector_session_id);
    }
}

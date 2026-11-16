use std::collections::{HashMap, HashSet};

use crate::{document_runtime::DomHandle, frame_owner_model::DocumentId};

use super::PageVm;

#[derive(Debug, Clone, Default)]
pub(super) struct RendererCssAgentSessionState {
    document_id: Option<DocumentId>,
    style_sheet_ids_by_owner: HashMap<RendererCssStyleSheetOwnerKey, String>,
    style_sheet_owners_by_id: HashMap<String, RendererCssStyleSheetOwnerKey>,
    next_style_sheet_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::runtime) struct RendererCssStyleSheetOwnerKey {
    pub(in crate::runtime) document_id: DocumentId,
    pub(in crate::runtime) handle: DomHandle,
}

impl RendererCssAgentSessionState {
    #[cfg(feature = "devtools")]
    pub(super) fn reset_for_document(&mut self, document_id: Option<DocumentId>) {
        if self.document_id == document_id {
            return;
        }
        self.document_id = document_id;
        self.style_sheet_ids_by_owner.clear();
        self.style_sheet_owners_by_id.clear();
        self.next_style_sheet_id = 0;
    }

    #[cfg(feature = "devtools")]
    pub(super) fn style_sheet_id_for_owner(
        &mut self,
        owner: RendererCssStyleSheetOwnerKey,
    ) -> (String, bool) {
        if let Some(style_sheet_id) = self.style_sheet_ids_by_owner.get(&owner) {
            return (style_sheet_id.clone(), false);
        }

        let style_sheet_id = format!("stylesheet:{}", self.next_style_sheet_id);
        self.next_style_sheet_id = self
            .next_style_sheet_id
            .checked_add(1)
            .expect("renderer CSS agent stylesheet id namespace exhausted");
        self.style_sheet_ids_by_owner
            .insert(owner, style_sheet_id.clone());
        self.style_sheet_owners_by_id
            .insert(style_sheet_id.clone(), owner);
        (style_sheet_id, true)
    }

    #[cfg(feature = "devtools")]
    pub(super) fn owner_for_style_sheet_id(
        &self,
        style_sheet_id: &str,
    ) -> Option<RendererCssStyleSheetOwnerKey> {
        self.style_sheet_owners_by_id.get(style_sheet_id).copied()
    }

    #[cfg(feature = "devtools")]
    pub(super) fn discard_style_sheet_id(&mut self, style_sheet_id: &str) {
        let Some(owner) = self.style_sheet_owners_by_id.remove(style_sheet_id) else {
            return;
        };
        if self
            .style_sheet_ids_by_owner
            .get(&owner)
            .map(String::as_str)
            == Some(style_sheet_id)
        {
            self.style_sheet_ids_by_owner.remove(&owner);
        }
    }

    #[cfg(feature = "devtools")]
    pub(super) fn discard_inactive_style_sheet_owners(
        &mut self,
        active_owners: &HashSet<RendererCssStyleSheetOwnerKey>,
    ) -> Vec<String> {
        let removed = self
            .style_sheet_ids_by_owner
            .iter()
            .filter_map(|(owner, style_sheet_id)| {
                if active_owners.contains(owner) {
                    None
                } else {
                    Some(style_sheet_id.clone())
                }
            })
            .collect::<Vec<_>>();
        for style_sheet_id in &removed {
            self.discard_style_sheet_id(style_sheet_id);
        }
        removed
    }
}

impl PageVm {
    #[cfg(feature = "devtools")]
    fn css_agent_state_for_session_mut(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> &mut RendererCssAgentSessionState {
        let document_id = self.current_dom_agent_document_id();
        let key = inspector_session_id.map(str::to_owned);
        let state = self.css_agent_sessions.entry(key).or_default();
        state.reset_for_document(document_id);
        state
    }

    #[cfg(feature = "devtools")]
    pub(in crate::runtime) fn register_document_style_sheet_owner(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
    ) -> Option<(String, bool)> {
        let document_id = self.vm().document_id_for_live_node_handle(handle)?;
        let owner = RendererCssStyleSheetOwnerKey {
            document_id,
            handle,
        };
        Some(
            self.css_agent_state_for_session_mut(inspector_session_id)
                .style_sheet_id_for_owner(owner),
        )
    }

    #[cfg(feature = "devtools")]
    pub(in crate::runtime) fn active_document_style_sheet_owner_key(
        &self,
        handle: DomHandle,
    ) -> Option<RendererCssStyleSheetOwnerKey> {
        let document_id = self.vm().document_id_for_live_node_handle(handle)?;
        Some(RendererCssStyleSheetOwnerKey {
            document_id,
            handle,
        })
    }

    #[cfg(feature = "devtools")]
    pub(in crate::runtime) fn discard_inactive_document_style_sheet_owners(
        &mut self,
        inspector_session_id: Option<&str>,
        active_owners: &HashSet<RendererCssStyleSheetOwnerKey>,
    ) -> Vec<String> {
        self.css_agent_state_for_session_mut(inspector_session_id)
            .discard_inactive_style_sheet_owners(active_owners)
    }

    #[cfg(feature = "devtools")]
    pub(in crate::runtime) fn reset_css_agent_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) {
        let key = inspector_session_id.map(str::to_owned);
        self.css_agent_sessions.remove(&key);
    }

    #[cfg(feature = "devtools")]
    pub(in crate::runtime) fn document_style_sheet_owner(
        &mut self,
        inspector_session_id: Option<&str>,
        style_sheet_id: &str,
    ) -> Option<DomHandle> {
        let owner = self
            .css_agent_state_for_session_mut(inspector_session_id)
            .owner_for_style_sheet_id(style_sheet_id)?;
        let still_current = self.vm().document_id_for_live_node_handle(owner.handle)
            == Some(owner.document_id)
            && self
                .vm()
                .document_runtime
                .dom_host()
                .node(owner.handle)
                .is_some();
        if still_current {
            return Some(owner.handle);
        }

        self.css_agent_state_for_session_mut(inspector_session_id)
            .discard_style_sheet_id(style_sheet_id);
        None
    }
}

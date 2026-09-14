use super::*;
use crate::document_runtime::DomHandle;
use crate::runtime::page_surface::{
    RendererStyleSheetHeader, RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};
use crate::stylesheet_blocking::link_rel_includes_token;
use std::collections::HashSet;

impl PageVm {
    pub(crate) fn set_inline_style_sheet_text_for_style_sheet_id(
        &mut self,
        inspector_session_id: Option<&str>,
        style_sheet_id: &str,
        text: &str,
    ) -> Result<bool> {
        let Some(node_id) = self.document_style_sheet_owner(inspector_session_id, style_sheet_id)
        else {
            return Ok(false);
        };
        self.set_inline_style_sheet_text_for_live_handle(node_id, text)
    }

    pub(crate) fn style_sheet_payload_for_style_sheet_id(
        &mut self,
        inspector_session_id: Option<&str>,
        style_sheet_id: &str,
    ) -> Option<RendererStyleSheetPayload> {
        let handle = self.document_style_sheet_owner(inspector_session_id, style_sheet_id)?;
        self.vm_mut().sync_live_document_style_sources();
        self.style_sheet_payload_for_live_handle(handle)
    }

    pub(crate) fn style_sheet_inventory_for_document(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> RendererStyleSheetInventoryUpdate {
        self.vm_mut().sync_live_document_style_sources();
        let handles = self.style_sheet_candidate_handles_for_document();
        let candidates = handles
            .into_iter()
            .filter_map(|handle| {
                let payload = self.style_sheet_payload_for_live_handle(handle)?;
                let owner = self.active_document_style_sheet_owner_key(handle)?;
                Some((handle, owner, payload))
            })
            .collect::<Vec<_>>();
        let active_owners = candidates
            .iter()
            .map(|(_, owner, _)| *owner)
            .collect::<HashSet<_>>();
        let removed =
            self.discard_inactive_document_style_sheet_owners(inspector_session_id, &active_owners);

        let added = candidates
            .into_iter()
            .filter_map(|(handle, _, payload)| {
                let (style_sheet_id, is_new) =
                    self.register_document_style_sheet_owner(inspector_session_id, handle)?;
                if !is_new {
                    return None;
                }
                let (end_line, end_column) = style_sheet_text_end_position(&payload.text);
                Some(RendererStyleSheetHeader {
                    style_sheet_id,
                    title: payload.title,
                    disabled: payload.disabled,
                    source_url: payload.source_url,
                    is_inline: payload.is_inline,
                    length: payload.text.len(),
                    end_line,
                    end_column,
                })
            })
            .collect();

        RendererStyleSheetInventoryUpdate { added, removed }
    }

    fn style_sheet_candidate_handles_for_document(&self) -> Vec<DomHandle> {
        let dom_host = self.vm().document_runtime.dom_host();
        dom_host
            .stylesheet_candidate_handles_for_tree_scope(dom_host.document_handle())
            .as_ref()
            .clone()
    }

    pub(crate) fn set_inline_style_sheet_text_for_live_handle(
        &mut self,
        handle: DomHandle,
        text: &str,
    ) -> Result<bool> {
        let text_json = serde_json::to_string(text)?;
        let payload = self.evaluate_expression_for_internal_node_reference(handle, false, |token| {
            format!(
            r#"(() => {{
                const node = __moliHostResolveInternalNodeReference({token});
                if (!node) {{
                    return false;
                }}
                if (!node || node.nodeType !== 1 || String(node.localName).toLowerCase() !== "style") {{
                    return false;
                }}
                node.textContent = {text};
                return true;
            }})()"#,
            text = text_json,
        )
        })?;
        Ok(payload
            .get("value")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    pub(crate) fn style_sheet_payload_for_live_handle(
        &self,
        handle: DomHandle,
    ) -> Option<RendererStyleSheetPayload> {
        let dom_host = self.vm().document_runtime.dom_host();
        let node = dom_host.node(handle)?;
        let element = node.as_element()?;
        if !crate::style_engine::stylesheet_owner_can_have_sheet(element) {
            return None;
        }

        if element.is_inline_style_element() {
            return Some(RendererStyleSheetPayload {
                text: self
                    .vm()
                    .owner_style_sheet_text(handle)
                    .unwrap_or_else(|| dom_host.text_content(handle).unwrap_or_default()),
                title: element.attribute("title").unwrap_or_default().to_owned(),
                disabled: element.has_attribute("disabled"),
                source_url: String::new(),
                is_inline: true,
            });
        }

        if !element.is_html_element("link")
            || !element
                .attribute("rel")
                .is_some_and(|rel| link_rel_includes_token(rel, "stylesheet"))
            || element.has_attribute("disabled")
            || !dom_host.is_connected(handle)
        {
            return None;
        }

        if let Some(source) = self.vm().linked_stylesheet_source_for_owner(handle) {
            return Some(RendererStyleSheetPayload {
                text: source.serialized_css_text().to_string(),
                title: element.attribute("title").unwrap_or_default().to_owned(),
                disabled: false,
                source_url: source.sheet_url().as_str().to_owned(),
                is_inline: false,
            });
        }

        if !self.vm().stylesheet_owner_is_csp_blocked(handle) {
            return None;
        }

        let source_url = crate::stylesheet_blocking::stylesheet_link_disposition(
            dom_host,
            moli_dom::NodeId::new(handle.index()),
        )
        .map(|disposition| disposition.url().as_str().to_owned())
        .unwrap_or_default();

        Some(RendererStyleSheetPayload {
            text: String::new(),
            title: element.attribute("title").unwrap_or_default().to_owned(),
            disabled: false,
            source_url,
            is_inline: false,
        })
    }
}

fn style_sheet_text_end_position(text: &str) -> (u32, u32) {
    let end_line = text.lines().count().saturating_sub(1) as u32;
    let end_column = text
        .lines()
        .last()
        .map(|line| line.chars().count())
        .unwrap_or(0) as u32;
    (end_line, end_column)
}

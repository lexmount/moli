use super::super::*;
use crate::runtime::page_surface::{
    RendererDocumentNodeAttributesResolution, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeTextResolution,
};

impl PageVm {
    pub(crate) fn computed_style_properties_for_live_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<Vec<(String, String)>>> {
        self.vm()
            .computed_style_properties_for_inspector_handle(handle)
    }

    pub(crate) fn computed_style_properties_for_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Result<Option<Vec<(String, String)>>> {
        let Some(node_id) = self.live_handle_for_document_node_reference(reference) else {
            return Ok(None);
        };
        self.computed_style_properties_for_live_handle(node_id)
    }

    pub(crate) fn computed_style_properties_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<Vec<(String, String)>>> {
        let Some(node_id) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.computed_style_properties_for_live_handle(node_id)
    }

    pub(crate) fn document_node_attributes(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> RendererDocumentNodeAttributesResolution {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return RendererDocumentNodeAttributesResolution::MissingNode;
        };
        let document = self.vm().document_runtime.dom_host().dom();
        let Some(node) = document.node(handle) else {
            return RendererDocumentNodeAttributesResolution::MissingNode;
        };
        let Some(element) = node.as_element() else {
            return RendererDocumentNodeAttributesResolution::NotElement;
        };
        let attributes = element
            .attributes()
            .iter()
            .map(|attribute| (attribute.name(), attribute.value().to_owned()))
            .collect();
        RendererDocumentNodeAttributesResolution::Found(attributes)
    }

    pub(crate) fn document_node_text_for_live_handle(
        &self,
        handle: DomHandle,
    ) -> RendererDocumentNodeTextResolution {
        let document = self.vm().document_runtime.dom_host().dom();
        match document.text_content(handle) {
            Some(text) => RendererDocumentNodeTextResolution::Found(text),
            None => RendererDocumentNodeTextResolution::MissingNode,
        }
    }

    pub(crate) fn document_node_text(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> RendererDocumentNodeTextResolution {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return RendererDocumentNodeTextResolution::MissingNode;
        };
        self.document_node_text_for_live_handle(handle)
    }

    pub(crate) fn document_node_property_for_live_handle(
        &self,
        handle: DomHandle,
        name: &str,
    ) -> RendererDocumentNodePropertyResolution {
        let document = self.vm().document_runtime.dom_host().dom();
        let Some(node) = document.node(handle) else {
            return RendererDocumentNodePropertyResolution::MissingNode;
        };
        let Some(element) = node.as_element() else {
            return RendererDocumentNodePropertyResolution::NotElement;
        };

        let value = match name {
            "tagName" => json!(element.node_name()),
            "nodeName" => json!(node.node_name()),
            "localName" => json!(element.local_name()),
            "namespaceURI" => json!(element.namespace()),
            "id" => json!(element.attribute("id").unwrap_or_default()),
            "className" => json!(element.attribute("class").unwrap_or_default()),
            "name" => json!(element.attribute("name").unwrap_or_default()),
            "title" => json!(element.attribute("title").unwrap_or_default()),
            "role" => json!(element.attribute("role").unwrap_or_default()),
            "textContent" | "innerText" => json!(document.text_content(handle).unwrap_or_default()),
            "innerHTML" => json!(document.inner_html(handle).unwrap_or_default()),
            "outerHTML" => json!(document.outer_html(handle).unwrap_or_default()),
            "value" if element.is_html_input() || element.is_html_textarea() => {
                json!(element.input_value())
            }
            "value" if element.is_html_option() => json!(element.option_value(document, handle)),
            "value" if element.is_html_select() => json!(
                document
                    .select_selected_option_elements(handle)
                    .into_iter()
                    .next()
                    .and_then(|option_id| document
                        .node(option_id)
                        .and_then(|node| node.as_element())
                        .map(|option| option.option_value(document, option_id)))
                    .unwrap_or_default()
            ),
            "checked" if element.is_html_input() => json!(element.checked()),
            "selected" if element.is_html_option() => {
                json!(document.option_effectively_selected(handle))
            }
            "disabled" => json!(element.has_attribute("disabled")),
            "readOnly" => json!(element.has_attribute("readonly")),
            "multiple" => json!(element.has_attribute("multiple")),
            "href" | "src" | "type" | "alt" | "placeholder" => {
                json!(element.attribute(name).unwrap_or_default())
            }
            "htmlFor" => json!(element.attribute("for").unwrap_or_default()),
            _ => Value::Null,
        };

        RendererDocumentNodePropertyResolution::Found(value)
    }

    pub(crate) fn document_node_property(
        &mut self,
        reference: RendererDomNodeReference,
        name: &str,
    ) -> RendererDocumentNodePropertyResolution {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return RendererDocumentNodePropertyResolution::MissingNode;
        };
        self.document_node_property_for_live_handle(handle, name)
    }

    pub(crate) fn outer_html_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        include_shadow_dom: bool,
    ) -> Result<Option<String>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        Ok(self.outer_html_for_live_handle(handle, include_shadow_dom))
    }
}

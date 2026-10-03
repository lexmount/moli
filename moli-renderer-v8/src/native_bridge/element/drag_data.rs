use crate::{
    document_runtime::DomHandle,
    dom::native::Node,
    native_bridge::JsContextHost,
    runtime::{RendererDragData, RendererDragDataItem},
};

/// Capture default element data before dispatching dragstart. URL resolution
/// follows the source Document without consulting author-defined JS properties.
pub(crate) fn native_element_drag_data(
    runtime: &JsContextHost,
    source: DomHandle,
) -> RendererDragData {
    let mut data = RendererDragData {
        items: Vec::new(),
        files: Vec::new(),
        directories: Vec::new(),
        drag_operations_mask: 1,
    };
    let Some(element) = runtime.dom_host().node(source).and_then(Node::as_element) else {
        return data;
    };
    let is_link = element.is_html_element("a");
    let attribute = if is_link {
        "href"
    } else if element.is_html_element("img") {
        "src"
    } else {
        return data;
    };
    let Some(url) = super::parsed_url_like_attribute(runtime, source, attribute) else {
        return data;
    };
    data.items.push(RendererDragDataItem {
        mime_type: "text/uri-list".to_owned(),
        data: url.to_string(),
        title: None,
        base_url: None,
    });
    if is_link {
        data.items.push(RendererDragDataItem {
            mime_type: "text/plain".to_owned(),
            data: url.into(),
            title: None,
            base_url: None,
        });
    }
    data
}

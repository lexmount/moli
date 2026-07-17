use style::values::specified::color::ColorSchemeFlags;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

/// Return the known schemes from the first valid document-tree
/// `<meta name=color-scheme>` in tree order.
///
/// The value is a page-level used-color input. It is deliberately not exposed
/// as the root element's computed `color-scheme`, which remains `normal`.
pub(crate) fn document_page_color_schemes(
    dom_host: &DomHost,
    document: DomHandle,
) -> ColorSchemeFlags {
    dom_host
        .html_elements_by_local_name_in_document_tree_order(document, "meta")
        .into_iter()
        .find_map(|handle| {
            let element = dom_host.node(handle).and_then(Node::as_element)?;
            if !element
                .attribute("name")
                .is_some_and(|name| name.eq_ignore_ascii_case("color-scheme"))
            {
                return None;
            }
            moli_css_parse::parse_css_color_scheme_flags(element.attribute("content")?)
        })
        .unwrap_or_else(ColorSchemeFlags::empty)
}

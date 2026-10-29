//! Native default drag data. DOM reads and markup cloning stay below the JS
//! wrapper layer, including when authors replace Selection/Range methods.

use std::cmp::Ordering;

use crate::context_bootstrap::NativeSelectionDragSnapshot;
use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, NativeDom, Node};
use crate::native_bridge::JsContextHost;
use crate::runtime::{RendererDragData, RendererDragDataItem};

fn empty_data() -> RendererDragData {
    RendererDragData {
        items: Vec::new(),
        files: Vec::new(),
        directories: Vec::new(),
        drag_operations_mask: 19,
    }
}

fn string_item(mime_type: &str, data: String) -> RendererDragDataItem {
    RendererDragDataItem {
        mime_type: mime_type.to_owned(),
        data,
        title: None,
        base_url: None,
    }
}

pub(super) fn selection_drag_data_at_point(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
) -> Option<RendererDragData> {
    let selection = crate::context_bootstrap::document_selection_drag_snapshot_at_point(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
    )?;
    let runtime = unsafe { &*runtime_ptr };
    let mut data = empty_data();
    if let Some(markup) = selected_html(runtime, &selection) {
        let mut html = string_item("text/html", markup);
        html.base_url = Some(
            runtime
                .document_url_for_handle(selection.document)
                .to_string(),
        );
        data.items.push(html);
    }
    // Blink's WriteSelection exports rendered text and replaces NBSP without
    // changing the selected HTML's whitespace or formatting.
    data.items.push(string_item(
        "text/plain",
        selection.text.replace('\u{00a0}', " "),
    ));
    Some(data)
}

pub(super) fn element_drag_data(runtime: &JsContextHost, source: DomHandle) -> RendererDragData {
    let mut data = empty_data();
    let host = runtime.dom_host();
    let Some(element) = host.node(source).and_then(Node::as_element) else {
        return data;
    };
    if element.is_html_element("img") {
        let Some(document) = host.owner_document_handle(source) else {
            return data;
        };
        let Some(image_url) = crate::native_bridge::element::image_selected_source(runtime, source)
            .and_then(|selected| {
                runtime
                    .document_base_url_for_handle(document)
                    .join(&selected)
                    .ok()
            })
        else {
            return data;
        };
        let mut ancestor = host.parent_node(source);
        let mut link_url = None;
        while let Some(handle) = ancestor {
            if host
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_element("a"))
                && let Some(url) = resolved_attribute_url(runtime, handle, "href")
            {
                link_url = Some(url);
                break;
            }
            ancestor = host
                .parent_node(handle)
                .or_else(|| host.shadow_root_host(handle));
        }
        let mut uri = string_item(
            "text/uri-list",
            link_url.as_ref().unwrap_or(&image_url).to_string(),
        );
        uri.title = Some(
            element
                .attribute_ns("", "alt")
                .unwrap_or_default()
                .to_owned(),
        );
        data.items.push(uri);
        if let Some(html) = element_html(runtime, source) {
            data.items.push(string_item("text/html", html));
        }
        if let Some(file) = runtime.image_drag_file(source) {
            data.files.push(file);
        }
    } else if element.is_html_element("a")
        && let Some(url) = resolved_attribute_url(runtime, source, "href")
    {
        let mut uri = string_item("text/uri-list", url.to_string());
        uri.title = Some(
            host.text_content(source)
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        );
        data.items.push(uri);
        data.items.push(string_item("text/plain", url.to_string()));
        if let Some(html) = element_html(runtime, source) {
            let mut item = string_item("text/html", html);
            item.base_url = Some(url.to_string());
            data.items.push(item);
        }
    }
    data
}

fn resolved_attribute_url(
    runtime: &JsContextHost,
    source: DomHandle,
    attribute: &str,
) -> Option<url::Url> {
    let host = runtime.dom_host();
    let value = host
        .node(source)?
        .as_element()?
        .attribute_ns("", attribute)?;
    let document = host.owner_document_handle(source)?;
    runtime
        .document_base_url_for_handle(document)
        .join(value)
        .ok()
}

fn element_html(runtime: &JsContextHost, source: DomHandle) -> Option<String> {
    let host = runtime.dom_host();
    let document = host.owner_document_handle(source)?;
    let base = runtime.document_base_url_for_handle(document);
    let mut scratch = DomHost::from_dom(NativeDom::new(base.clone()));
    let clone = scratch.import_foreign_node(scratch.document_handle(), host.dom(), source, true)?;
    resolve_markup_urls(&mut scratch, clone, &base);
    scratch.dom().outer_html(clone)
}

fn contains(host: &DomHost, ancestor: DomHandle, mut node: DomHandle) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        let Some(parent) = host.parent_node(node) else {
            return false;
        };
        node = parent;
    }
}

fn inline_container(host: &DomHost, node: DomHandle) -> bool {
    host.node(node)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            !matches!(
                element.local_name(),
                "html"
                    | "head"
                    | "body"
                    | "div"
                    | "p"
                    | "section"
                    | "article"
                    | "main"
                    | "header"
                    | "footer"
                    | "nav"
                    | "aside"
                    | "blockquote"
                    | "pre"
                    | "ul"
                    | "ol"
                    | "li"
                    | "table"
                    | "thead"
                    | "tbody"
                    | "tfoot"
                    | "tr"
                    | "td"
                    | "th"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            )
        })
}

fn selected_html(runtime: &JsContextHost, range: &NativeSelectionDragSnapshot) -> Option<String> {
    let host = runtime.dom_host();
    let mut common = range.start;
    while !contains(host, common, range.end) {
        common = host.parent_node(common)?;
    }
    // Range fragments omit the common container. Keep its surrounding inline
    // markup so selecting the text inside <b>/<em>/<a> retains its semantics.
    let mut root = common;
    while let Some(parent) = host.parent_node(root)
        && inline_container(host, parent)
    {
        root = parent;
    }
    let base = runtime.document_base_url_for_handle(range.document);
    let mut scratch = DomHost::from_dom(NativeDom::new(base.clone()));
    let fragment = scratch.create_document_fragment();
    if inline_container(host, root)
        || host.node(root)?.is_text()
        || host.node(root)?.is_cdata_section()
    {
        clone_selected_node(runtime, range, root, &mut scratch, fragment)?;
    } else {
        for child in host.child_nodes(root)? {
            clone_selected_node(runtime, range, child, &mut scratch, fragment)?;
        }
    }
    resolve_markup_urls(&mut scratch, fragment, &base);
    scratch.inner_html(fragment)
}

fn clone_selected_node(
    runtime: &JsContextHost,
    range: &NativeSelectionDragSnapshot,
    node: DomHandle,
    scratch: &mut DomHost,
    destination: DomHandle,
) -> Option<()> {
    let host = runtime.dom_host();
    let parent = host.parent_node(node)?;
    let index = u32::try_from(host.child_index(parent, node)?).ok()?;
    let order = |container, offset, other, other_offset| {
        crate::range_boundary::point_order_in_dom(host, container, offset, other, other_offset)
    };
    if order(range.end, range.end_offset, parent, index)? != Ordering::Greater
        || order(range.start, range.start_offset, parent, index + 1)? != Ordering::Less
    {
        return Some(());
    }
    let source = host.node(node)?;
    if source.as_element().is_some()
        && crate::context_bootstrap::selection_text_element_is_hidden(runtime, node)
    {
        return Some(());
    }
    let clone = if source.is_text() || source.is_cdata_section() {
        let units = runtime.character_data_utf16_units(node)?;
        let start = if node == range.start {
            range.start_offset as usize
        } else {
            0
        }
        .min(units.len());
        let end = if node == range.end {
            range.end_offset as usize
        } else {
            units.len()
        }
        .min(units.len());
        scratch.create_text_node(&String::from_utf16_lossy(&units[start..end.max(start)]))
    } else {
        // DomHost's native import reuses the DOM clone machinery without
        // invoking custom-element constructors or Range/Node JS overrides.
        // Even a fully selected subtree may contain non-rendered descendants.
        // Clone it shallowly and apply the same visibility rule as plain text.
        let clone =
            scratch.import_foreign_node(scratch.document_handle(), host.dom(), node, false)?;
        for child in host.child_nodes(node)? {
            clone_selected_node(runtime, range, child, scratch, clone)?;
        }
        clone
    };
    scratch.append_child(destination, clone).then_some(())
}

fn resolve_markup_urls(scratch: &mut DomHost, root: DomHandle, base: &url::Url) {
    let updates: Vec<_> = scratch
        .node(root)
        .and_then(Node::as_element)
        .map(|element| {
            element
                .attributes()
                .iter()
                .filter(|attribute| {
                    attribute.namespace().is_empty()
                        && matches!(
                            attribute.local_name(),
                            "href" | "src" | "action" | "formaction" | "poster" | "cite"
                        )
                })
                .filter_map(|attribute| {
                    base.join(attribute.value())
                        .ok()
                        .map(|url| (attribute.local_name().to_owned(), url.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    for (name, value) in updates {
        scratch.set_attribute(root, &name, &value);
    }
    for child in scratch.child_nodes(root).unwrap_or_default() {
        resolve_markup_urls(scratch, child, base);
    }
}

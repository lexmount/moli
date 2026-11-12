//! One synchronous, read-only export of the live DOM. Embedded Documents are
//! streamed into srcdoc attributes; only one optional base projection is stored.

use crate::dom::native::{
    Attribute, HtmlInsertionPoint, HtmlSerializationInsertion, HtmlSerializationPolicy, NativeDom,
    NativeNodeId, Node, escape_html_attribute_into_string,
};
use crate::runtime::page_surface::{RendererContentStripOptions, RendererHtmlSerializeOptions};

use super::PageVm;

impl PageVm {
    pub(crate) fn serialize_html(
        &self,
        options: RendererHtmlSerializeOptions,
    ) -> anyhow::Result<String> {
        let vm = self.vm();
        let dom = vm.document_runtime.dom_host().dom();
        let policy = HtmlDumpPolicy {
            strip: options.strip,
            base: options
                .with_base
                .then(|| BaseInsertion::new(dom, vm.document_runtime.document_url().as_str())),
        };
        Ok(vm.document_markup_for_export(
            dom.document_node_id(),
            (options.with_base || options.strip != RendererContentStripOptions::default())
                .then_some(&policy as &dyn HtmlSerializationPolicy),
            options.with_frames,
            options.max_output_bytes,
        )?)
    }
}

struct HtmlDumpPolicy {
    strip: RendererContentStripOptions,
    base: Option<BaseInsertion>,
}

impl HtmlSerializationPolicy for HtmlDumpPolicy {
    fn has_filters(&self) -> bool {
        self.strip != RendererContentStripOptions::default()
    }

    fn include_node(&self, node: &Node) -> bool {
        !super::content_filter::should_strip_node(node, self.strip)
    }

    fn include_attribute(&self, _: NativeNodeId, attribute: &Attribute) -> bool {
        let name_start = attribute
            .prefix()
            .filter(|prefix| !prefix.is_empty())
            .unwrap_or_else(|| attribute.local_name());
        !(self.strip.css && attribute.name_matches("style")
            || self.strip.js && name_start.starts_with("on"))
    }

    fn attribute_overrides(&self, node: NativeNodeId) -> &[Attribute] {
        if let Some(BaseInsertion::Override { target, attributes }) = &self.base
            && *target == node
        {
            return attributes;
        }
        &[]
    }

    fn insertion(&self) -> Option<HtmlSerializationInsertion<'_>> {
        match &self.base {
            Some(BaseInsertion::Insert {
                target,
                point,
                markup,
            }) => Some(HtmlSerializationInsertion {
                node: *target,
                point: *point,
                markup,
            }),
            _ => None,
        }
    }

    fn include_shadow_adopted_style_sheets(&self) -> bool {
        !self.strip.css
    }
}

enum BaseInsertion {
    Override {
        target: NativeNodeId,
        attributes: [Attribute; 1],
    },
    Insert {
        target: NativeNodeId,
        point: HtmlInsertionPoint,
        markup: String,
    },
}

impl BaseInsertion {
    fn new(dom: &NativeDom, href: &str) -> Self {
        if let Some(head) = dom.head_node_id()
            && let Some(base) = dom.find_child(head, |child| {
                dom.node(child)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("base"))
            })
        {
            return Self::Override {
                target: base,
                attributes: [attribute("href", href.to_owned())],
            };
        }
        let mut markup = String::from("<base href=\"");
        escape_html_attribute_into_string(href, &mut markup);
        markup.push_str("\">");
        let (target, point, markup) = if let Some(head) = dom.head_node_id() {
            (head, HtmlInsertionPoint::FirstChild, markup)
        } else if let Some(html) = dom.document_element_node_id() {
            let markup = format!("<head>{markup}</head>");
            match dom.body_node_id() {
                Some(body) => (body, HtmlInsertionPoint::BeforeNode, markup),
                None => (html, HtmlInsertionPoint::LastChild, markup),
            }
        } else {
            (
                dom.document_node_id(),
                HtmlInsertionPoint::LastChild,
                format!("<html><head>{markup}</head></html>"),
            )
        };
        Self::Insert {
            target,
            point,
            markup,
        }
    }
}

fn attribute(name: &str, value: String) -> Attribute {
    Attribute::new(name.to_owned(), String::new(), None, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::DomHost;

    #[test]
    fn virtual_base_preserves_existing_head_shapes_and_source_state() {
        let href = "https://example.test/page?a=1&b=2";
        let base = "<base href=\"https://example.test/page?a=1&amp;b=2\">";
        for shape in 0..6 {
            let url = url::Url::parse("https://example.test/").unwrap();
            let mut host = DomHost::from_dom(if shape == 5 {
                NativeDom::new_xml(url)
            } else {
                NativeDom::new_html(url)
            });
            let expected = match shape {
                0 => {
                    host.reset_html_document_shell();
                    let head = host.document_head_handle().unwrap();
                    let first = host.create_element("base");
                    assert!(host.set_attribute(first, "target", "_blank"));
                    assert!(host.set_attribute(first, "href", "/old"));
                    assert!(host.append_child(head, first));
                    let second = host.create_element("base");
                    assert!(host.set_attribute(second, "href", "/second"));
                    assert!(host.append_child(head, second));
                    "<html><head><base target=\"_blank\" href=\"https://example.test/page?a=1&amp;b=2\"><base href=\"/second\"></head><body></body></html>".to_owned()
                }
                1 => {
                    host.reset_html_document_shell();
                    let head = host.document_head_handle().unwrap();
                    let title = host.create_element("title");
                    let text = host.create_text_node("Title");
                    assert!(host.append_child(title, text));
                    assert!(host.append_child(head, title));
                    format!("<html><head>{base}<title>Title</title></head><body></body></html>")
                }
                2 | 3 => {
                    host.reset_html_document_shell();
                    let head = host.document_head_handle().unwrap();
                    let body = host.document_body_handle().unwrap();
                    let html = host.document_element_handle().unwrap();
                    assert!(host.remove_child(html, head));
                    if shape == 2 {
                        let comment = host.create_comment("before");
                        assert!(host.insert_before(html, comment, Some(body)));
                        format!("<html><!--before--><head>{base}</head><body></body></html>")
                    } else {
                        assert!(host.remove_child(html, body));
                        let content = host.create_element("div");
                        assert!(host.append_child(html, content));
                        format!("<html><div></div><head>{base}</head></html>")
                    }
                }
                4 => {
                    let comment = host.create_comment("empty");
                    assert!(host.append_child(host.document_handle(), comment));
                    format!("<!--empty--><html><head>{base}</head></html>")
                }
                5 => {
                    let root = host
                        .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
                        .unwrap();
                    assert!(host.append_child(host.document_handle(), root));
                    format!("<svg><head>{base}</head></svg>")
                }
                _ => unreachable!(),
            };
            let dom = host.dom();
            let before = dom.serialize_document();
            let count = dom.nodes().len();
            let version = host.dom_version();
            let policy = HtmlDumpPolicy {
                strip: Default::default(),
                base: Some(BaseInsertion::new(dom, href)),
            };
            let output = host
                .get_html_for_export(host.document_handle(), &|_| true, Some(&policy), None, None)
                .unwrap();
            assert_eq!(output, expected, "shape={shape}");
            assert_eq!(host.dom().serialize_document(), before);
            assert_eq!(host.dom().nodes().len(), count);
            assert_eq!(host.dom_version(), version);
        }
    }
}

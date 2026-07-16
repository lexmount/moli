use super::*;
use crate::native::serialize::is_void_html_element;

pub type ShadowRootRegistryAttributePolicy<'a> =
    dyn Fn(DomHandle, DomHandle, &ShadowRootInit) -> bool + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowRootInclusion<'a> {
    None,
    SerializableOrExplicit {
        serializable: bool,
        explicit: &'a [DomHandle],
    },
    AllAuthorForInspector,
}

impl ShadowRootInclusion<'_> {
    fn markup_profile(self) -> ShadowRootMarkupProfile {
        match self {
            Self::AllAuthorForInspector => ShadowRootMarkupProfile::Inspector,
            Self::None | Self::SerializableOrExplicit { .. } => ShadowRootMarkupProfile::WebApi,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HtmlSerializationTarget {
    IncludeNode,
    ChildrenOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadowRootMarkupProfile {
    WebApi,
    Inspector,
}

enum HostHtmlSerializationFrame<'a> {
    Node {
        handle: DomHandle,
        raw_text_parent: bool,
    },
    Children {
        handle: DomHandle,
        raw_text_parent: bool,
    },
    ShadowRootTemplate {
        host: DomHandle,
        shadow_root: DomHandle,
        init: ShadowRootInit,
    },
    CloseElement(&'a str),
    CloseShadowRootTemplate,
}

impl DomHost {
    pub fn get_html(
        &self,
        handle: DomHandle,
        serializable_shadow_roots: bool,
        explicit_shadow_roots: &[DomHandle],
    ) -> Option<String> {
        self.get_html_with_shadow_root_registry_attribute_policy(
            handle,
            serializable_shadow_roots,
            explicit_shadow_roots,
            None,
        )
    }

    pub fn get_html_with_shadow_root_registry_attribute_policy(
        &self,
        handle: DomHandle,
        serializable_shadow_roots: bool,
        explicit_shadow_roots: &[DomHandle],
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationTarget::ChildrenOnly,
            ShadowRootInclusion::SerializableOrExplicit {
                serializable: serializable_shadow_roots,
                explicit: explicit_shadow_roots,
            },
            registry_attribute_policy,
        )
    }

    pub fn outer_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationTarget::IncludeNode,
            shadow_root_inclusion,
            registry_attribute_policy,
        )
    }

    fn serialize_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        target: HtmlSerializationTarget,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        let node = self.node(handle)?;
        let mut html = String::new();
        let stack = match target {
            HtmlSerializationTarget::IncludeNode => vec![HostHtmlSerializationFrame::Node {
                handle,
                raw_text_parent: false,
            }],
            HtmlSerializationTarget::ChildrenOnly => {
                let mut stack = vec![HostHtmlSerializationFrame::Children {
                    handle,
                    raw_text_parent: false,
                }];
                if node.as_element().is_some()
                    && let Some((shadow_root, init)) =
                        self.serialized_shadow_root_for_host(handle, shadow_root_inclusion)
                {
                    stack.push(HostHtmlSerializationFrame::ShadowRootTemplate {
                        host: handle,
                        shadow_root,
                        init,
                    });
                }
                stack
            }
        };
        self.serialize_html_frames(
            &mut html,
            stack,
            shadow_root_inclusion,
            registry_attribute_policy,
        );
        Some(html)
    }

    fn serialize_html_frames<'a>(
        &'a self,
        out: &mut String,
        mut stack: Vec<HostHtmlSerializationFrame<'a>>,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) {
        while let Some(frame) = stack.pop() {
            match frame {
                HostHtmlSerializationFrame::Node {
                    handle,
                    raw_text_parent,
                } => self.serialize_html_node_frame(
                    handle,
                    out,
                    raw_text_parent,
                    shadow_root_inclusion,
                    &mut stack,
                ),
                HostHtmlSerializationFrame::Children {
                    handle,
                    raw_text_parent,
                } => self.push_html_child_frames(handle, raw_text_parent, &mut stack),
                HostHtmlSerializationFrame::ShadowRootTemplate {
                    host,
                    shadow_root,
                    init,
                } => {
                    self.write_shadow_root_template_start(
                        host,
                        shadow_root,
                        &init,
                        shadow_root_inclusion.markup_profile(),
                        out,
                        registry_attribute_policy,
                    );
                    stack.push(HostHtmlSerializationFrame::CloseShadowRootTemplate);
                    stack.push(HostHtmlSerializationFrame::Children {
                        handle: shadow_root,
                        raw_text_parent: false,
                    });
                }
                HostHtmlSerializationFrame::CloseElement(local_name) => {
                    out.push_str("</");
                    out.push_str(local_name);
                    out.push('>');
                }
                HostHtmlSerializationFrame::CloseShadowRootTemplate => {
                    out.push_str("</template>");
                }
            }
        }
    }

    fn push_html_child_frames<'a>(
        &self,
        handle: DomHandle,
        raw_text_parent: bool,
        stack: &mut Vec<HostHtmlSerializationFrame<'a>>,
    ) {
        let Some(node) = self.node(handle) else {
            return;
        };
        if let Some(template_contents) = node
            .as_element()
            .and_then(|element| element.template_contents())
        {
            let raw_text_child = node.as_element().is_some_and(|element| {
                is_raw_text_element(element.namespace(), element.local_name())
            });
            stack.push(HostHtmlSerializationFrame::Children {
                handle: template_contents,
                raw_text_parent: raw_text_child,
            });
            return;
        }
        stack.extend(self.child_handles_reversed(handle).map(|handle| {
            HostHtmlSerializationFrame::Node {
                handle,
                raw_text_parent,
            }
        }));
    }

    fn serialize_html_node_frame<'a>(
        &'a self,
        handle: DomHandle,
        out: &mut String,
        raw_text_parent: bool,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        stack: &mut Vec<HostHtmlSerializationFrame<'a>>,
    ) {
        let Some(node) = self.node(handle) else {
            return;
        };
        match node.data() {
            NodeData::Document(_) | NodeData::DocumentFragment(_) => {
                stack.push(HostHtmlSerializationFrame::Children {
                    handle,
                    raw_text_parent,
                });
            }
            NodeData::DocumentType(document_type) => {
                out.push_str("<!DOCTYPE ");
                out.push_str(document_type.name());
                if !document_type.public_id().is_empty() || !document_type.system_id().is_empty() {
                    out.push_str(" PUBLIC \"");
                    out.push_str(document_type.public_id());
                    out.push_str("\" \"");
                    out.push_str(document_type.system_id());
                    out.push('"');
                }
                out.push('>');
            }
            NodeData::Element(element) => {
                out.push('<');
                out.push_str(element.local_name());
                if let Some(is_name) = element.custom_element_is_name()
                    && !element.has_attribute("is")
                {
                    out.push_str(" is=\"");
                    escape_html_attribute(is_name, out);
                    out.push('"');
                }
                for attribute in element.attributes() {
                    out.push(' ');
                    attribute.push_html_serialized_name(|part| out.push_str(part));
                    out.push_str("=\"");
                    escape_html_attribute(attribute.value(), out);
                    out.push('"');
                }
                out.push('>');

                let raw_text_child = is_raw_text_element(element.namespace(), element.local_name());
                if !is_void_html_element(element.namespace(), element.local_name()) {
                    stack.push(HostHtmlSerializationFrame::CloseElement(
                        element.local_name(),
                    ));
                }
                stack.push(HostHtmlSerializationFrame::Children {
                    handle,
                    raw_text_parent: raw_text_child,
                });
                if let Some((shadow_root, init)) =
                    self.serialized_shadow_root_for_host(handle, shadow_root_inclusion)
                {
                    stack.push(HostHtmlSerializationFrame::ShadowRootTemplate {
                        host: handle,
                        shadow_root,
                        init,
                    });
                }
            }
            NodeData::Text(text) => {
                if raw_text_parent {
                    out.push_str(text.data());
                } else {
                    escape_html_text(text.data(), out);
                }
            }
            NodeData::CDataSection(cdata) => {
                out.push_str("<![CDATA[");
                out.push_str(cdata.data());
                out.push_str("]]>");
            }
            NodeData::Comment(comment) => {
                out.push_str("<!--");
                out.push_str(comment.data());
                out.push_str("-->");
            }
            NodeData::ProcessingInstruction(processing_instruction) => {
                out.push_str("<?");
                out.push_str(processing_instruction.target());
                if !processing_instruction.data().is_empty() {
                    out.push(' ');
                    out.push_str(processing_instruction.data());
                }
                out.push_str("?>");
            }
        }
    }

    fn serialized_shadow_root_for_host(
        &self,
        host: DomHandle,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
    ) -> Option<(DomHandle, ShadowRootInit)> {
        let state = self.shadow_roots_by_host.borrow().get(&host)?.clone();
        let included = match shadow_root_inclusion {
            ShadowRootInclusion::None => false,
            ShadowRootInclusion::SerializableOrExplicit {
                serializable,
                explicit,
            } => serializable && state.init.serializable() || explicit.contains(&state.handle),
            // DomHost stores author shadow roots. Generated user-agent trees are
            // Inspector projections owned by the renderer and never enter this map.
            ShadowRootInclusion::AllAuthorForInspector => true,
        };
        if included {
            Some((state.handle, state.init))
        } else {
            None
        }
    }

    fn write_shadow_root_template_start(
        &self,
        host: DomHandle,
        shadow_root: DomHandle,
        init: &ShadowRootInit,
        markup_profile: ShadowRootMarkupProfile,
        out: &mut String,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) {
        out.push_str("<template shadowrootmode=\"");
        escape_html_attribute(init.mode(), out);
        out.push('"');
        if init.delegates_focus() {
            out.push_str(" shadowrootdelegatesfocus=\"\"");
        }
        if init.serializable() {
            out.push_str(" shadowrootserializable=\"\"");
        }
        if markup_profile == ShadowRootMarkupProfile::WebApi && init.slot_assignment() != "named" {
            out.push_str(" shadowrootslotassignment=\"");
            escape_html_attribute(init.slot_assignment(), out);
            out.push('"');
        }
        if init.clonable() {
            out.push_str(" shadowrootclonable=\"\"");
        }
        let serialize_registry_attribute = registry_attribute_policy
            .map(|policy| policy(host, shadow_root, init))
            .unwrap_or_else(|| init.null_custom_element_registry());
        if serialize_registry_attribute {
            out.push_str(" shadowrootcustomelementregistry=\"\"");
        }
        if markup_profile == ShadowRootMarkupProfile::WebApi
            && let Some(reference_target) = init.reference_target()
        {
            out.push_str(" shadowrootreferencetarget=\"");
            escape_html_attribute(reference_target, out);
            out.push('"');
        }
        if markup_profile == ShadowRootMarkupProfile::WebApi
            && let Some(adopted_style_sheets) = init.adopted_style_sheets()
        {
            out.push_str(" shadowrootadoptedstylesheets=\"");
            escape_html_attribute(adopted_style_sheets, out);
            out.push('"');
        }
        out.push('>');
    }
}

fn is_raw_text_element(namespace: &str, local_name: &str) -> bool {
    namespace == "http://www.w3.org/1999/xhtml"
        && matches!(local_name, "script" | "style" | "textarea" | "title")
}

fn escape_html_text(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            _ => out.push(ch),
        }
    }
}

fn escape_html_attribute(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_url() -> url::Url {
        url::Url::parse("https://inspector-shadow-serialization.test/").expect("test URL")
    }

    #[test]
    fn inspector_outer_html_uses_chromium_shadow_template_attributes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let element = host.create_element("section");
        assert!(host.set_attribute(element, "id", "host"));
        assert!(host.append_child(host.document_node_id(), element));

        let mut init = ShadowRootInit::new("open");
        init.set_delegates_focus(true);
        init.set_serializable(true);
        init.set_slot_assignment("manual");
        init.set_clonable(true);
        init.set_reference_target(Some("target&name".to_owned()));
        init.set_adopted_style_sheets(Some("sheet-a sheet-b".to_owned()));
        let shadow_root = host
            .attach_shadow_root_with_init(element, init)
            .expect("shadow root");
        let shadow_child = host.create_element("span");
        let shadow_text = host.create_text_node("shadow <value>");
        assert!(host.append_child(shadow_root, shadow_child));
        assert!(host.append_child(shadow_child, shadow_text));
        let light_text = host.create_text_node("light & more");
        assert!(host.append_child(element, light_text));

        assert_eq!(
            host.outer_html_with_shadow_roots(element, ShadowRootInclusion::None, None)
                .as_deref(),
            Some("<section id=\"host\">light &amp; more</section>")
        );

        let include_registry = |_: DomHandle, _: DomHandle, _: &ShadowRootInit| true;
        assert_eq!(
            host.outer_html_with_shadow_roots(
                element,
                ShadowRootInclusion::AllAuthorForInspector,
                Some(&include_registry),
            )
            .as_deref(),
            Some(concat!(
                "<section id=\"host\"><template shadowrootmode=\"open\" ",
                "shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" ",
                "shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\">",
                "<span>shadow &lt;value&gt;</span></template>light &amp; more</section>"
            ))
        );

        assert_eq!(
            host.get_html_with_shadow_root_registry_attribute_policy(
                element,
                false,
                &[shadow_root],
                Some(&include_registry),
            )
            .as_deref(),
            Some(concat!(
                "<template shadowrootmode=\"open\" shadowrootdelegatesfocus=\"\" ",
                "shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" ",
                "shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\" ",
                "shadowrootreferencetarget=\"target&amp;name\" ",
                "shadowrootadoptedstylesheets=\"sheet-a sheet-b\">",
                "<span>shadow &lt;value&gt;</span></template>light &amp; more"
            ))
        );
    }

    #[test]
    fn inspector_outer_html_includes_nested_open_and_closed_author_roots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let outer_host = host.create_element("x-outer");
        assert!(host.append_child(host.document_node_id(), outer_host));
        let outer_root = host
            .attach_shadow_root(outer_host, "open")
            .expect("outer shadow root");
        let inner_host = host.create_element("x-inner");
        assert!(host.append_child(outer_root, inner_host));
        let inner_root = host
            .attach_shadow_root(inner_host, "closed")
            .expect("inner shadow root");
        let closed_child = host.create_element("b");
        let closed_text = host.create_text_node("closed");
        assert!(host.append_child(inner_root, closed_child));
        assert!(host.append_child(closed_child, closed_text));
        let inner_light = host.create_text_node("inner-light");
        let outer_light = host.create_text_node("outer-light");
        assert!(host.append_child(inner_host, inner_light));
        assert!(host.append_child(outer_host, outer_light));

        assert_eq!(
            host.outer_html_with_shadow_roots(outer_host, ShadowRootInclusion::None, None)
                .as_deref(),
            Some("<x-outer>outer-light</x-outer>")
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                outer_host,
                ShadowRootInclusion::SerializableOrExplicit {
                    serializable: false,
                    explicit: &[outer_root],
                },
                None,
            )
            .as_deref(),
            Some(concat!(
                "<x-outer><template shadowrootmode=\"open\">",
                "<x-inner>inner-light</x-inner></template>outer-light</x-outer>"
            ))
        );
        let all_author = concat!(
            "<x-outer><template shadowrootmode=\"open\"><x-inner>",
            "<template shadowrootmode=\"closed\"><b>closed</b></template>",
            "inner-light</x-inner></template>outer-light</x-outer>"
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                outer_host,
                ShadowRootInclusion::AllAuthorForInspector,
                None,
            )
            .as_deref(),
            Some(all_author)
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                host.document_node_id(),
                ShadowRootInclusion::AllAuthorForInspector,
                None,
            )
            .as_deref(),
            Some(all_author)
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                outer_root,
                ShadowRootInclusion::AllAuthorForInspector,
                None,
            )
            .as_deref(),
            Some(concat!(
                "<x-inner><template shadowrootmode=\"closed\"><b>closed</b></template>",
                "inner-light</x-inner>"
            ))
        );
    }

    #[test]
    fn inspector_outer_html_walks_deep_shadow_trees_iteratively() {
        const DEPTH: usize = 4096;

        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let root = host.create_element("div");
        assert!(host.append_child(host.document_node_id(), root));
        let mut shadow_host = root;
        for index in 0..DEPTH {
            let mode = if index % 2 == 0 { "open" } else { "closed" };
            let shadow_root = host
                .attach_shadow_root(shadow_host, mode)
                .expect("deep shadow root");
            let child_host = host.create_element("div");
            assert!(host.append_child(shadow_root, child_host));
            shadow_host = child_host;
        }
        let leaf = host.create_text_node("leaf");
        assert!(host.append_child(shadow_host, leaf));

        assert_eq!(
            host.outer_html_with_shadow_roots(root, ShadowRootInclusion::None, None)
                .as_deref(),
            Some("<div></div>")
        );
        let html = host
            .outer_html_with_shadow_roots(root, ShadowRootInclusion::AllAuthorForInspector, None)
            .expect("deep inspector outer HTML");
        assert_eq!(html.matches("<template shadowrootmode=").count(), DEPTH);
        assert!(html.starts_with("<div><template shadowrootmode=\"open\"><div>"));
        assert!(html.contains("<template shadowrootmode=\"closed\"><div>"));
        assert!(html.contains("leaf</div></template></div></template>"));
        assert!(html.ends_with("</div>"));
    }

    #[test]
    fn shadow_excluding_outer_html_matches_native_serializer() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let doctype = host.create_document_type("html", "public-id", "system-id");
        let root = host.create_element("main");
        assert!(host.set_attribute(root, "data-value", "<&\""));
        let script = host.create_element("script");
        let script_text = host.create_text_node("if (left < right && value > 0) {}");
        assert!(host.append_child(script, script_text));
        let template = host.create_element("template");
        let template_contents = host
            .parser_template_contents_handle(template)
            .expect("template contents");
        let template_child = host.create_element("span");
        let template_text = host.create_text_node("template <text> & value");
        assert!(host.append_child(template_contents, template_child));
        assert!(host.append_child(template_child, template_text));
        let input = host.create_element("input");
        let comment = host.create_comment("comment");
        let cdata = host.create_cdata_section("cdata <value>");
        let processing_instruction = host.create_processing_instruction("target", "value");

        assert!(host.append_child(document, doctype));
        assert!(host.append_child(document, root));
        for child in [
            script,
            template,
            input,
            comment,
            cdata,
            processing_instruction,
        ] {
            assert!(host.append_child(root, child));
        }

        let fragment = host.create_document_fragment();
        let fragment_child = host.create_element("aside");
        assert!(host.append_child(fragment, fragment_child));

        for handle in [
            document,
            doctype,
            root,
            script,
            script_text,
            template,
            template_contents,
            template_child,
            template_text,
            input,
            comment,
            cdata,
            processing_instruction,
            fragment,
            fragment_child,
        ] {
            assert_eq!(
                host.outer_html_with_shadow_roots(handle, ShadowRootInclusion::None, None),
                host.dom().outer_html(handle),
                "shadow-excluding host serializer diverged for {handle:?}"
            );
        }
    }
}

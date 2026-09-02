use super::*;
use crate::native::serialize::{
    HtmlSerializationOptions, HtmlSerializationTarget, HtmlSerializedShadowRoot,
    escape_html_attribute, serialize_html_into_sink,
};

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
enum ShadowRootMarkupProfile {
    WebApi,
    Inspector,
}

impl DomHost {
    pub fn get_html(
        &self,
        handle: DomHandle,
        scripting_enabled: bool,
        serializable_shadow_roots: bool,
        explicit_shadow_roots: &[DomHandle],
    ) -> Option<String> {
        self.get_html_with_shadow_root_registry_attribute_policy(
            handle,
            scripting_enabled,
            serializable_shadow_roots,
            explicit_shadow_roots,
            None,
        )
    }

    pub fn get_html_with_shadow_root_registry_attribute_policy(
        &self,
        handle: DomHandle,
        scripting_enabled: bool,
        serializable_shadow_roots: bool,
        explicit_shadow_roots: &[DomHandle],
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationTarget::ChildrenOnly,
            scripting_enabled,
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
        scripting_enabled: bool,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationTarget::IncludeNode,
            scripting_enabled,
            shadow_root_inclusion,
            registry_attribute_policy,
        )
    }

    fn serialize_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        target: HtmlSerializationTarget,
        scripting_enabled: bool,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        let shadow_root_provider = |host| {
            let (shadow_root, init) =
                self.serialized_shadow_root_for_host(host, shadow_root_inclusion)?;
            let mut template_start = String::new();
            self.write_shadow_root_template_start(
                host,
                shadow_root,
                &init,
                shadow_root_inclusion.markup_profile(),
                &mut template_start,
                registry_attribute_policy,
            );
            Some(HtmlSerializedShadowRoot::new(shadow_root, template_start))
        };
        let options = HtmlSerializationOptions::new(target, scripting_enabled)
            .with_shadow_root_provider(&shadow_root_provider);
        let mut html = String::new();
        serialize_html_into_sink(&self.dom, handle, options, &mut html).then_some(html)
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
            host.outer_html_with_shadow_roots(element, true, ShadowRootInclusion::None, None)
                .as_deref(),
            Some("<section id=\"host\">light &amp; more</section>")
        );

        let include_registry = |_: DomHandle, _: DomHandle, _: &ShadowRootInit| true;
        assert_eq!(
            host.outer_html_with_shadow_roots(
                element,
                true,
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
                true,
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
            host.outer_html_with_shadow_roots(outer_host, true, ShadowRootInclusion::None, None)
                .as_deref(),
            Some("<x-outer>outer-light</x-outer>")
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                outer_host,
                true,
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
                true,
                ShadowRootInclusion::AllAuthorForInspector,
                None,
            )
            .as_deref(),
            Some(all_author)
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                host.document_node_id(),
                true,
                ShadowRootInclusion::AllAuthorForInspector,
                None,
            )
            .as_deref(),
            Some(all_author)
        );
        assert_eq!(
            host.outer_html_with_shadow_roots(
                outer_root,
                true,
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
            host.outer_html_with_shadow_roots(root, true, ShadowRootInclusion::None, None)
                .as_deref(),
            Some("<div></div>")
        );
        let html = host
            .outer_html_with_shadow_roots(
                root,
                true,
                ShadowRootInclusion::AllAuthorForInspector,
                None,
            )
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
                host.outer_html_with_shadow_roots(handle, true, ShadowRootInclusion::None, None),
                host.dom().outer_html(handle),
                "shadow-excluding host serializer diverged for {handle:?}"
            );
        }
    }
}

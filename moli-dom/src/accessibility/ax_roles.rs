use crate::{
    NodeData, NodeId,
    forms::InputType,
    native::{Element, NativeDom, Node, select_display_size},
};

/// Chromium's ui::IsLink classification for computed AX roles.
pub(super) fn is_link(role: &str) -> bool {
    matches!(
        role,
        "link" | "doc-backlink" | "doc-biblioref" | "doc-glossref" | "doc-noteref"
    )
}

/// Blink's AXObject::IsImage excludes canvas from ui::IsImage.
pub(super) fn is_image(role: &str) -> bool {
    matches!(role, "image" | "doc-cover" | "graphics-symbol" | "SvgRoot")
}

#[derive(Clone, Copy)]
pub(super) enum AxRole {
    Fixed(&'static str),
    NamedRegion,
    Presentational,
}

impl AxRole {
    pub(super) fn for_name(self) -> &'static str {
        match self {
            Self::Fixed(role) => role,
            Self::NamedRegion => "region",
            Self::Presentational => "none",
        }
    }

    pub(super) fn with_name(self, name: &str) -> &'static str {
        if matches!(self, Self::NamedRegion) && name.is_empty() {
            "generic"
        } else {
            self.for_name()
        }
    }
}

/// Resolve native and ARIA semantics before traversing the accessible name.
pub(super) fn ax_role(document: &NativeDom, node: &Node, scrollable: bool) -> AxRole {
    AxRole::Fixed(match node.kind() {
        NodeData::Document(_) => "RootWebArea",
        NodeData::DocumentType(_) => "none",
        NodeData::Element(element) => {
            let role = match explicit_aria_role(document, node, element, scrollable) {
                Some("none") => return AxRole::Presentational,
                Some(role) => role,
                None => native_ax_role(element),
            };
            // HTML-AAM maps an unnamed section to generic. Core-AAM also
            // falls back to the host role for an explicit unnamed region.
            if role == "region" && element.is_html_element("section") {
                return AxRole::NamedRegion;
            }
            role
        }
        NodeData::Text(_) | NodeData::CDataSection(_) => "StaticText",
        NodeData::Comment(_) | NodeData::ProcessingInstruction(_) => "none",
        NodeData::DocumentFragment(_) => "generic",
    })
}

fn native_ax_role(element: &Element) -> &'static str {
    if element.hyperlink_href().is_some() {
        return "link";
    }
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return "generic";
    }
    match element.local_name() {
        "html" | "body" | "div" | "span" => "generic",
        "main" => "main",
        "nav" => "navigation",
        "aside" => "complementary",
        "header" => "banner",
        "footer" => "contentinfo",
        "section" => "region",
        "article" | "hgroup" => "article",
        "address" => "group",
        "button" => "button",
        "input" => match element.input_type() {
            InputType::Search => "searchbox",
            InputType::Number => "spinbutton",
            InputType::Checkbox => "checkbox",
            InputType::Radio => "radio",
            InputType::Button | InputType::Submit | InputType::Reset | InputType::Image => "button",
            InputType::Hidden => "none",
            InputType::Color => "color",
            InputType::File => "file",
            InputType::Month => "month",
            InputType::DatetimeLocal | InputType::Week | InputType::Time | InputType::Date => {
                "combobox"
            }
            _ => "textbox",
        },
        "form" => "form",
        "textarea" => "textbox",
        "select" => {
            if element.has_attribute_ns("", "multiple") || select_display_size(element) > 1 {
                "listbox"
            } else {
                "combobox"
            }
        }
        "option" => "option",
        "optgroup" | "fieldset" | "details" => "group",
        "summary" => "button",
        "datalist" => "listbox",
        "output" => "status",
        "progress" => "progressbar",
        "meter" => "meter",
        "hr" => "separator",
        "table" => "table",
        "caption" => "caption",
        "thead" | "tbody" | "tfoot" => "rowgroup",
        "tr" => "row",
        "th" => {
            if element
                .attribute("scope")
                .is_some_and(|scope| scope.eq_ignore_ascii_case("row"))
            {
                "rowheader"
            } else {
                "columnheader"
            }
        }
        "td" => "cell",
        "dialog" => "dialog",
        "iframe" | "frame" => "Iframe",
        "img" => "image",
        "figure" => "figure",
        "p" => "paragraph",
        "blockquote" => "blockquote",
        "code" => "code",
        "em" => "emphasis",
        "strong" => "strong",
        "s" | "del" => "deletion",
        "ins" => "insertion",
        "sub" => "subscript",
        "sup" => "superscript",
        "time" => "time",
        "ul" | "ol" => "list",
        "li" => "listitem",
        "dt" => "term",
        "dd" => "definition",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "br" => "LineBreak",
        "marquee" => "marquee",
        _ => "generic",
    }
}

fn explicit_aria_role(
    document: &NativeDom,
    node: &Node,
    element: &Element,
    scrollable: bool,
) -> Option<&'static str> {
    let role = element
        .attribute_ns("", "role")?
        .split_ascii_whitespace()
        .find_map(aria_role_to_ax_role)?;
    // Blink resolves presentational-role conflicts before selecting the native
    // role. Explicit semantic overrides such as button keep their precedence.
    if role == "none"
        && !element.is_html_element("iframe")
        && !element.is_html_element("frame")
        && (has_global_aria_property(element)
            || document.focus_support(node.id()).supports_focus(scrollable))
    {
        None
    } else {
        Some(role)
    }
}

fn has_global_aria_property(element: &Element) -> bool {
    // WAI-ARIA 1.2 sections 3.5 and 6.5: deprecated globals still participate
    // in presentational-role conflicts. Preserve the supported ARIA 1.3
    // braille/description globals too. HTML (including XHTML) ARIA attributes
    // have no namespace; another namespace's matching local name is unrelated.
    [
        "aria-atomic",
        "aria-braillelabel",
        "aria-brailleroledescription",
        "aria-busy",
        "aria-controls",
        "aria-current",
        "aria-describedby",
        "aria-description",
        "aria-details",
        "aria-disabled",
        "aria-dropeffect",
        "aria-errormessage",
        "aria-flowto",
        "aria-grabbed",
        "aria-haspopup",
        "aria-hidden",
        "aria-invalid",
        "aria-keyshortcuts",
        "aria-label",
        "aria-labelledby",
        "aria-live",
        "aria-owns",
        "aria-relevant",
        "aria-roledescription",
    ]
    .into_iter()
    .any(|name| element.has_attribute_ns("", name))
}

// Keep this mapping aligned with Blink's generated AriaRoleToInternalRole().
// Role attributes are fallback token lists, so the first recognized token is
// observable even when unsupported tokens precede it.
fn aria_role_to_ax_role(role: &str) -> Option<&'static str> {
    Some(match role.to_ascii_lowercase().as_str() {
        "alert" => "alert",
        "alertdialog" => "alertdialog",
        "application" => "application",
        "article" => "article",
        "banner" => "banner",
        "blockquote" => "blockquote",
        "button" => "button",
        "caption" => "caption",
        "cell" => "cell",
        "checkbox" => "checkbox",
        "code" => "code",
        "columnheader" => "columnheader",
        "combobox" => "combobox",
        "comment" => "comment",
        "complementary" => "complementary",
        "contentinfo" => "contentinfo",
        "definition" => "definition",
        "deletion" => "deletion",
        "dialog" => "dialog",
        "doc-abstract" => "doc-abstract",
        "doc-acknowledgments" => "doc-acknowledgments",
        "doc-afterword" => "doc-afterword",
        "doc-appendix" => "doc-appendix",
        "doc-backlink" => "doc-backlink",
        "doc-biblioentry" => "doc-biblioentry",
        "doc-bibliography" => "doc-bibliography",
        "doc-biblioref" => "doc-biblioref",
        "doc-chapter" => "doc-chapter",
        "doc-colophon" => "doc-colophon",
        "doc-conclusion" => "doc-conclusion",
        "doc-cover" => "doc-cover",
        "doc-credit" => "doc-credit",
        "doc-credits" => "doc-credits",
        "doc-dedication" => "doc-dedication",
        "doc-endnote" => "doc-endnote",
        "doc-endnotes" => "doc-endnotes",
        "doc-epigraph" => "doc-epigraph",
        "doc-epilogue" => "doc-epilogue",
        "doc-errata" => "doc-errata",
        "doc-example" => "doc-example",
        "doc-footnote" => "doc-footnote",
        "doc-foreword" => "doc-foreword",
        "doc-glossary" => "doc-glossary",
        "doc-glossref" => "doc-glossref",
        "doc-index" => "doc-index",
        "doc-introduction" => "doc-introduction",
        "doc-noteref" => "doc-noteref",
        "doc-notice" => "doc-notice",
        "doc-pagebreak" => "doc-pagebreak",
        "doc-pagefooter" => "doc-pagefooter",
        "doc-pageheader" => "doc-pageheader",
        "doc-pagelist" => "doc-pagelist",
        "doc-part" => "doc-part",
        "doc-preface" => "doc-preface",
        "doc-prologue" => "doc-prologue",
        "doc-pullquote" => "doc-pullquote",
        "doc-qna" => "doc-qna",
        "doc-subtitle" => "doc-subtitle",
        "doc-tip" => "doc-tip",
        "doc-toc" => "doc-toc",
        "document" => "document",
        "emphasis" => "emphasis",
        "feed" => "feed",
        "figure" => "figure",
        "form" => "form",
        "generic" => "generic",
        "graphics-document" => "graphics-document",
        "graphics-object" => "graphics-object",
        "graphics-symbol" => "graphics-symbol",
        "grid" => "grid",
        "gridcell" => "gridcell",
        "group" => "group",
        "heading" => "heading",
        "image" | "img" => "image",
        "insertion" => "insertion",
        "link" => "link",
        "list" | "directory" => "list",
        "listbox" => "listbox",
        "listitem" => "listitem",
        "log" => "log",
        "main" => "main",
        "mark" => "mark",
        "marquee" => "marquee",
        "math" => "math",
        "menu" => "menu",
        "menubar" => "menubar",
        "menuitem" => "menuitem",
        "menuitemcheckbox" => "menuitemcheckbox",
        "menuitemradio" => "menuitemradio",
        "meter" => "meter",
        "navigation" => "navigation",
        "none" | "presentation" => "none",
        "note" => "note",
        "option" => "option",
        "paragraph" => "paragraph",
        "progressbar" => "progressbar",
        "radio" => "radio",
        "radiogroup" => "radiogroup",
        "region" => "region",
        "row" => "row",
        "rowgroup" => "rowgroup",
        "rowheader" => "rowheader",
        "scrollbar" => "scrollbar",
        "search" => "search",
        "searchbox" => "searchbox",
        "sectionfooter" => "sectionfooter",
        "sectionheader" => "sectionheader",
        "separator" => "separator",
        "slider" => "slider",
        "spinbutton" => "spinbutton",
        "status" => "status",
        "strong" => "strong",
        "subscript" => "subscript",
        "suggestion" => "suggestion",
        "superscript" => "superscript",
        "switch" => "switch",
        "tab" => "tab",
        "table" => "table",
        "tablist" => "tablist",
        "tabpanel" => "tabpanel",
        "term" => "term",
        "textbox" => "textbox",
        "time" => "time",
        "timer" => "timer",
        "toolbar" => "toolbar",
        "tooltip" => "tooltip",
        "tree" => "tree",
        "treegrid" => "treegrid",
        "treeitem" => "treeitem",
        _ => return None,
    })
}

pub(super) fn heading_level(local_name: &str) -> Option<usize> {
    match local_name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

pub(super) fn listitem_level(document: &NativeDom, node: &Node) -> usize {
    let mut level = 0usize;
    let mut current = node.parent_node_id();
    while let Some(node_id) = current {
        let Some(parent) = document.node(node_id) else {
            break;
        };
        if let Some(element) = parent.as_element() {
            match element.local_name() {
                "ul" | "ol" | "menu" => level += 1,
                _ => {}
            }
        }
        current = parent.parent_node_id();
    }
    level
}

pub(super) fn ordered_list_item_index(
    document: &NativeDom,
    parent_id: NodeId,
    node_id: NodeId,
) -> Option<usize> {
    let mut count = 0usize;
    for child_id in document.child_ids(parent_id) {
        let child = document.node(child_id)?;
        let child_element = match child.as_element() {
            Some(element) => element,
            None => continue,
        };
        if !child_element.is_html_element("li") {
            continue;
        }
        count += 1;
        if child_id == node_id {
            return Some(count);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn presentation_conflicts_follow_standard_global_aria_properties() {
        // WAI-ARIA 1.2 section 6.5, including deprecated globals. Preserve the
        // three supported ARIA 1.3 globals as well.
        let properties = [
            "aria-atomic",
            "aria-busy",
            "aria-controls",
            "aria-current",
            "aria-describedby",
            "aria-details",
            "aria-disabled",
            "aria-dropeffect",
            "aria-errormessage",
            "aria-flowto",
            "aria-grabbed",
            "aria-haspopup",
            "aria-hidden",
            "aria-invalid",
            "aria-keyshortcuts",
            "aria-label",
            "aria-labelledby",
            "aria-live",
            "aria-owns",
            "aria-relevant",
            "aria-roledescription",
            "aria-braillelabel",
            "aria-brailleroledescription",
            "aria-description",
        ];
        let mut document = NativeDom::new_html(Url::parse("https://example.test/").unwrap());
        let root = document.create_element("main");
        assert!(document.append_child(document.document_node_id(), root));
        for role in ["none", "presentation"] {
            for property in properties {
                for (namespace, prefix) in [
                    (None, None),
                    (Some("urn:test"), None),
                    (Some("urn:test"), Some("x")),
                    (Some("http://www.w3.org/ns/wai-aria/"), None),
                    (Some("http://www.w3.org/ns/wai-aria/"), Some("aria")),
                ] {
                    let heading = document.create_element("h2");
                    assert!(document.append_child(root, heading));
                    assert!(document.set_attribute(heading, "role", role));
                    assert!(
                        document.set_attribute_ns(heading, namespace, prefix, property, "false")
                    );
                    assert_eq!(
                        ax_role(&document, document.node(heading).unwrap(), false).for_name(),
                        if namespace.is_none() {
                            "heading"
                        } else {
                            "none"
                        },
                        "role={role}, {property}, namespace={namespace:?}, prefix={prefix:?}"
                    );
                }
            }
            for property in [
                "aria-labeledby",
                "aria-checked",
                "aria-expanded",
                "aria-level",
                "aria-selected",
                "aria-valuenow",
                "aria-unknown",
            ] {
                let heading = document.create_element("h2");
                assert!(document.append_child(root, heading));
                assert!(document.set_attribute(heading, "role", role));
                assert!(document.set_attribute(heading, property, "false"));
                assert_eq!(
                    ax_role(&document, document.node(heading).unwrap(), false).for_name(),
                    "none",
                    "{property}"
                );
            }
        }
    }

    #[test]
    fn presentation_focus_conflicts_distinguish_html_spaces_from_vertical_tab() {
        let mut document = NativeDom::new_html(Url::parse("https://example.test/").unwrap());
        for role in ["none", "presentation"] {
            for (tabindex, expected) in [
                (" 0", "heading"),
                ("\t0", "heading"),
                ("\n0", "heading"),
                ("\r0", "heading"),
                ("\u{c}0", "heading"),
                ("\u{b}0", "none"),
                ("\u{a0}0", "none"),
                ("0\u{b}", "heading"),
            ] {
                let heading = document.create_element("h2");
                assert!(document.append_child(document.document_node_id(), heading));
                assert!(document.set_attribute(heading, "role", role));
                assert!(document.set_attribute(heading, "tabindex", tabindex));
                assert_eq!(
                    ax_role(&document, document.node(heading).unwrap(), false).for_name(),
                    expected,
                    "tabindex={tabindex:?}"
                );
            }
        }
    }

    #[test]
    fn presentation_conflicts_use_focus_support_and_global_aria_properties() {
        let cases = [
            ("a", vec![("href", "/go")], "link"),
            ("a", vec![], "none"),
            ("a", vec![("tabindex", "-1")], "generic"),
            ("button", vec![], "button"),
            ("button", vec![("disabled", ""), ("tabindex", "0")], "none"),
            ("input", vec![("type", "hidden"), ("tabindex", "0")], "none"),
            ("img", vec![("tabindex", "-1")], "image"),
            ("img", vec![("tabindex", "0tail")], "image"),
            ("img", vec![("tabindex", "invalid")], "none"),
            ("img", vec![("tabindex", "\u{a0}0")], "none"),
            ("img", vec![("aria-label", "")], "image"),
            ("img", vec![("aria-live", "off")], "image"),
            ("img", vec![("aria-hidden", "false")], "image"),
            ("img", vec![("aria-disabled", "true")], "image"),
            ("div", vec![("contenteditable", "true")], "generic"),
        ];
        let mut document = NativeDom::new_html(Url::parse("https://example.test/").unwrap());
        let root = document.create_element("main");
        assert!(document.append_child(document.document_node_id(), root));
        for role in ["presentation", "none"] {
            for (tag, attributes, expected) in &cases {
                let element = document.create_element(tag);
                assert!(document.append_child(root, element));
                for (name, value) in attributes {
                    assert!(document.set_attribute(element, name, value));
                }
                assert!(document.set_attribute(element, "role", role));
                assert_eq!(
                    ax_role(&document, document.node(element).unwrap(), false).for_name(),
                    *expected,
                    "{tag} role={role}, attributes={attributes:?}"
                );
            }
        }
    }

    #[test]
    fn presentation_conflicts_respect_disabled_fieldsets_and_the_first_legend() {
        let mut document = NativeDom::new_html(Url::parse("https://example.test/").unwrap());
        let fieldset = document.create_element("fieldset");
        document.append_child(document.document_node_id(), fieldset);
        document.set_attribute(fieldset, "disabled", "");
        let legend = document.create_element("legend");
        document.append_child(fieldset, legend);
        for (parent, expected) in [(legend, "button"), (fieldset, "none")] {
            let button = document.create_element("button");
            document.append_child(parent, button);
            document.set_attribute(button, "role", "presentation");
            assert_eq!(
                ax_role(&document, document.node(button).unwrap(), false).for_name(),
                expected
            );
        }
    }

    #[test]
    fn explicit_aria_role_uses_chromium_fallback_token_order() {
        let mut document =
            NativeDom::new_html(Url::parse("https://example.test/").expect("valid URL"));
        let element = document.create_element("div");
        assert!(document.append_child(document.document_node_id(), element));

        assert!(document.set_attribute(element, "role", "unknown STATUS"));
        assert_eq!(
            ax_role(&document, document.node(element).expect("element"), false).for_name(),
            "status"
        );

        assert!(document.set_attribute(element, "role", "button status"));
        assert_eq!(
            ax_role(&document, document.node(element).expect("element"), false).for_name(),
            "button"
        );
    }

    #[test]
    fn password_input_is_a_textbox_and_hidden_input_is_not_exposed() {
        let mut document =
            NativeDom::new_html(Url::parse("https://example.test/").expect("valid URL"));
        let password = document.create_element("input");
        assert!(document.append_child(document.document_node_id(), password));
        assert!(document.set_attribute(password, "type", "password"));
        assert_eq!(
            ax_role(&document, document.node(password).expect("element"), false).for_name(),
            "textbox"
        );

        let hidden = document.create_element("input");
        assert!(document.append_child(document.document_node_id(), hidden));
        assert!(document.set_attribute(hidden, "type", "hidden"));
        assert_eq!(
            ax_role(&document, document.node(hidden).expect("element"), false).for_name(),
            "none"
        );
    }
}

use super::RendererContentStripOptions;

pub(super) fn should_strip_node(
    node: &crate::dom::native::Node,
    strip: RendererContentStripOptions,
) -> bool {
    let Some(element) = node.as_element() else {
        return false;
    };
    let tag = element.local_name();
    (strip.js && matches!(tag, "script" | "noscript"))
        || (strip.css
            && (tag == "style"
                || (tag == "link"
                    && element
                        .attribute("rel")
                        .unwrap_or_default()
                        .split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("stylesheet")))))
        || (strip.ui
            && matches!(
                tag,
                "header"
                    | "footer"
                    | "nav"
                    | "aside"
                    | "form"
                    | "input"
                    | "button"
                    | "select"
                    | "textarea"
                    | "option"
                    | "dialog"
                    | "menu"
                    | "menuitem"
            ))
}

use crate::Dom;

pub(crate) fn input_text<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<&str> {
    let kind = dom.attribute(node, "type").unwrap_or("text");
    if kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio") {
        return Some(if dom.attribute(node, "checked").is_some() {
            "☑"
        } else {
            "☐"
        });
    }
    if ["hidden", "file", "password"]
        .iter()
        .any(|excluded| kind.eq_ignore_ascii_case(excluded))
    {
        return None;
    }
    if kind.eq_ignore_ascii_case("image") {
        return ["alt", "aria-label", "title"].into_iter().find_map(|name| {
            dom.attribute(node, name)
                .filter(|value| !value.trim().is_empty())
        });
    }
    if let Some(value) = dom
        .control_value(node)
        .or_else(|| dom.attribute(node, "value"))
        .filter(|value| !value.is_empty())
    {
        return Some(value);
    }
    if !["submit", "button", "reset"]
        .iter()
        .any(|button| kind.eq_ignore_ascii_case(button))
    {
        return None;
    }
    ["aria-label", "title"]
        .into_iter()
        .find_map(|name| dom.attribute(node, name).filter(|value| !value.is_empty()))
}

use std::borrow::Cow;

use crate::Dom;

pub(crate) fn input_text<'a, D: Dom + ?Sized>(dom: &'a D, node: D::NodeId) -> Option<Cow<'a, str>> {
    let kind = dom.attribute(node, "type").unwrap_or("text");
    if kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio") {
        return Some(Cow::Borrowed(if dom.attribute(node, "checked").is_some() {
            "☑"
        } else {
            "☐"
        }));
    }
    if ["hidden", "file"]
        .iter()
        .any(|excluded| kind.eq_ignore_ascii_case(excluded))
    {
        return None;
    }
    if kind.eq_ignore_ascii_case("image") {
        return dom
            .attribute(node, "alt")
            .filter(|value| !value.is_empty())
            .map(Cow::Borrowed);
    }
    let button = ["submit", "button", "reset"]
        .iter()
        .any(|v| kind.eq_ignore_ascii_case(v));
    let value = (!kind.eq_ignore_ascii_case("password"))
        .then(|| {
            dom.control_value(node)
                .or_else(|| dom.attribute(node, "value"))
        })
        .flatten()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            button
                .then(|| dom.attribute(node, "aria-label"))
                .flatten()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            button
                .then(|| dom.attribute(node, "title"))
                .flatten()
                .filter(|value| !value.is_empty())
        });
    value.map(Cow::Borrowed)
}

pub(crate) fn textarea_text<'a, D: Dom + ?Sized>(
    dom: &'a D,
    node: D::NodeId,
) -> Option<Cow<'a, str>> {
    dom.control_value(node).map(Cow::Borrowed)
}

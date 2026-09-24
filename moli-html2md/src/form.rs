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
    let value = (!kind.eq_ignore_ascii_case("password"))
        .then(|| dom.attribute(node, "value"))
        .flatten()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            dom.attribute(node, "placeholder")
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            dom.attribute(node, "aria-label")
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            dom.attribute(node, "title")
                .filter(|value| !value.is_empty())
        });
    value.map(Cow::Borrowed)
}

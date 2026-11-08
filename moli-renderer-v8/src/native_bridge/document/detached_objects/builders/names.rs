use crate::dom::native::{html_element_interface_name, svg_element_interface_name};

pub(in crate::native_bridge::document) fn qualified_name_parts(
    qualified_name: &str,
) -> (Option<String>, String) {
    match qualified_name.split_once(':') {
        Some((prefix, local_name)) => (Some(prefix.to_owned()), local_name.to_owned()),
        None => (None, qualified_name.to_owned()),
    }
}

pub(in crate::native_bridge::document) fn html_element_to_string_tag(
    local_name: &str,
) -> &'static str {
    html_element_interface_name(local_name)
}

pub(in crate::native_bridge::document) fn html_element_constructor_name(
    local_name: &str,
) -> Option<&'static str> {
    Some(html_element_interface_name(local_name))
}

pub(in crate::native_bridge::document) fn svg_element_to_string_tag(
    local_name: &str,
) -> &'static str {
    svg_element_interface_name(local_name)
}

pub(in crate::native_bridge::document) fn svg_element_constructor_name(
    local_name: &str,
) -> Option<&'static str> {
    Some(svg_element_interface_name(local_name))
}

pub(crate) fn is_valid_pi_target(target: &str) -> bool {
    let mut stream = xmlparser::Stream::from(target);
    stream.consume_name().is_ok() && stream.at_end()
}

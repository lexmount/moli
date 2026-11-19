mod abort_statics;
mod attr;
mod css_style;
mod prototypes;

use super::*;

pub(super) use abort_statics::install_abort_template_bindings;
pub(super) use attr::install_attr_template_bindings;
pub(super) use css_style::install_css_style_declaration_template_accessors;
pub(super) use moli_webapi::interface_constants::{
    install_constructor_constant_template_bindings, install_node_filter_constants,
};
pub(super) use prototypes::install_to_string_tag;

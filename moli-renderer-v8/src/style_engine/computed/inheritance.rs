use style::{
    computed_value_flags::ComputedValueFlags, invalidation::element::restyle_hints::RestyleHint,
    properties::ComputedValues,
};

/// Translate a canonical style commit into Stylo's child restyle hints.
/// Servo's rendering damage cannot distinguish reset-only changes or detect
/// every property that CSSOM can observe, so compare the computed structs.
pub(super) fn child_restyle_hint(
    old: Option<&ComputedValues>,
    new: &ComputedValues,
) -> RestyleHint {
    let Some(old) = old else {
        return RestyleHint::recascade_subtree();
    };
    if std::ptr::eq(old, new) {
        return RestyleHint::empty();
    }

    let old_display = old.clone_display();
    let new_display = new.clone_display();
    let display_changed = old_display != new_display;
    let old_container_type = old.clone_container_type();
    let new_container_type = new.clone_container_type();
    let font_changed = !same_style_struct(old.get_font(), new.get_font());
    let zoom_changed = old.effective_zoom != new.effective_zoom;

    // Container queries, root-relative units and blockification through
    // display: contents can depend on inputs beyond the immediate parent.
    if old.custom_properties() != new.custom_properties()
        || old.clone_container_name() != new.clone_container_name()
        || old_container_type != new_container_type
        || old.writing_mode != new.writing_mode
        || ((font_changed || zoom_changed)
            && (new
                .flags
                .contains(ComputedValueFlags::IS_ROOT_ELEMENT_STYLE)
                || new_container_type.is_size_container_type()))
        || (display_changed && (old_display.is_contents() || new_display.is_contents()))
    {
        return RestyleHint::restyle_subtree();
    }

    if old.flags.maybe_inherited() != new.flags.maybe_inherited()
        || zoom_changed
        || (display_changed
            && (old_display == style::values::specified::box_::Display::None
                || old_display.is_item_container() != new_display.is_item_container()))
        || font_changed
        || !same_style_struct(old.get_inherited_box(), new.get_inherited_box())
        || !same_style_struct(old.get_inherited_svg(), new.get_inherited_svg())
        || !same_style_struct(old.get_inherited_table(), new.get_inherited_table())
        || !same_style_struct(old.get_inherited_text(), new.get_inherited_text())
        || !same_style_struct(old.get_inherited_ui(), new.get_inherited_ui())
        || !same_style_struct(old.get_list(), new.get_list())
    {
        return RestyleHint::RECASCADE_SELF;
    }

    if !same_style_struct(old.get_background(), new.get_background())
        || !same_style_struct(old.get_border(), new.get_border())
        || !same_style_struct(old.get_box(), new.get_box())
        || !same_style_struct(old.get_column(), new.get_column())
        || !same_style_struct(old.get_counters(), new.get_counters())
        || !same_style_struct(old.get_effects(), new.get_effects())
        || !same_style_struct(old.get_margin(), new.get_margin())
        || !same_style_struct(old.get_outline(), new.get_outline())
        || !same_style_struct(old.get_padding(), new.get_padding())
        || !same_style_struct(old.get_page(), new.get_page())
        || !same_style_struct(old.get_position(), new.get_position())
        || !same_style_struct(old.get_svg(), new.get_svg())
        || !same_style_struct(old.get_table(), new.get_table())
        || !same_style_struct(old.get_text(), new.get_text())
        || !same_style_struct(old.get_ui(), new.get_ui())
    {
        return RestyleHint::RECASCADE_SELF_IF_INHERIT_RESET_STYLE;
    }

    RestyleHint::empty()
}

fn same_style_struct<T: PartialEq>(old: &T, new: &T) -> bool {
    std::ptr::eq(old, new) || old == new
}

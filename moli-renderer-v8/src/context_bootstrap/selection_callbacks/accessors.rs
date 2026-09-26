use super::*;

#[derive(Clone, Copy)]
enum SelectionAttribute {
    AnchorNode,
    AnchorOffset,
    FocusNode,
    FocusOffset,
    IsCollapsed,
    RangeCount,
    Type,
    Direction,
}

const SELECTION_ATTRIBUTES: &[SelectionAttribute] = &[
    SelectionAttribute::AnchorNode,
    SelectionAttribute::AnchorOffset,
    SelectionAttribute::FocusNode,
    SelectionAttribute::FocusOffset,
    SelectionAttribute::IsCollapsed,
    SelectionAttribute::RangeCount,
    SelectionAttribute::Type,
    SelectionAttribute::Direction,
];

pub(in crate::context_bootstrap) fn selection_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SELECTION_ATTRIBUTES,
        "Selection attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(selection_attribute_value(scope, args.this(), attribute));
}

fn selection_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    attribute: SelectionAttribute,
) -> v8::Local<'s, v8::Value> {
    match attribute {
        SelectionAttribute::AnchorNode => selection_anchor_node(scope, selection)
            .map(|node| node.into())
            .unwrap_or_else(|| v8::null(scope).into()),
        SelectionAttribute::AnchorOffset => {
            let offset = selection_anchor_offset(scope, selection) as f64;
            v8::Number::new(scope, offset).into()
        }
        SelectionAttribute::FocusNode => selection_focus_node(scope, selection)
            .map(|node| node.into())
            .unwrap_or_else(|| v8::null(scope).into()),
        SelectionAttribute::FocusOffset => {
            let offset = selection_focus_offset(scope, selection) as f64;
            v8::Number::new(scope, offset).into()
        }
        SelectionAttribute::RangeCount => {
            let count = if selection_has_range(scope, selection) {
                1
            } else {
                0
            };
            v8::Integer::new(scope, count).into()
        }
        SelectionAttribute::Type => {
            let value = if !selection_has_range(scope, selection) {
                "None"
            } else if selection_is_collapsed_internal(scope, selection)
                && !selection_spans_dom_roots(scope, selection)
            {
                "Caret"
            } else {
                "Range"
            };
            v8str(scope, value).into()
        }
        SelectionAttribute::IsCollapsed => {
            let collapsed = selection_is_collapsed_internal(scope, selection);
            v8::Boolean::new(scope, collapsed).into()
        }
        SelectionAttribute::Direction => selection_direction(scope, selection)
            .and_then(|direction| v8_string(scope, &direction))
            .map(|direction| direction.into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    }
}

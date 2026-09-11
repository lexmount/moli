use super::*;
use crate::web_api_interfaces;
use crate::webidl_iterator::{
    SnapshotWebIdlIteratorKind, invoke_webidl_collection_for_each_callback,
    new_snapshot_webidl_iterator, prepare_webidl_collection_for_each_callback,
};

const ELEMENT_COMPUTED_STYLE_MAP_SLOT: &str = "__moliElementComputedStyleMap";
const STYLE_PROPERTY_MAP_STYLE_SLOT: &str = "__moliStylePropertyMapStyle";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::StylePropertyMapReadOnly)]
struct ComputedStylePropertyMapDeclaration<'s> {
    #[webapi(slot = STYLE_PROPERTY_MAP_STYLE_SLOT)]
    style: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Element, enumerable)]
struct ElementComputedStyleMapPrototypeDeclaration {
    #[webapi(
        method = "computedStyleMap",
        callback = element_computed_style_map_callback,
        length = 0
    )]
    computed_style_map: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StylePropertyMapReadOnly, enumerable)]
struct StylePropertyMapReadOnlyPrototypeDeclaration {
    #[webapi(method, callback = style_property_map_get_callback, length = 1)]
    get: (),
    #[webapi(method = "getAll", callback = style_property_map_get_all_callback, length = 1)]
    get_all: (),
    #[webapi(method, callback = style_property_map_has_callback, length = 1)]
    has: (),
    #[webapi(method, callback = style_property_map_entries_callback, length = 0)]
    entries: (),
    #[webapi(method, callback = style_property_map_keys_callback, length = 0)]
    keys: (),
    #[webapi(method, callback = style_property_map_values_callback, length = 0)]
    values: (),
    #[webapi(method = "forEach", callback = style_property_map_for_each_callback, length = 1)]
    for_each: (),
    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
    #[webapi(accessor_property, getter = style_property_map_size_getter_callback)]
    size: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StylePropertyMapReadOnly")]
struct StylePropertyMapPropertyArgs {
    #[webidl(required)]
    property: String,
}

pub(super) fn install_computed_style_map_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Element" => {
            ElementComputedStyleMapPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "StylePropertyMapReadOnly" => {
            StylePropertyMapReadOnlyPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn element_computed_style_map_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((host_ptr, handle)) =
        native_bridge::node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_computed_style_map_illegal_invocation(scope);
        return;
    };
    if !unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_element())
    {
        throw_computed_style_map_illegal_invocation(scope);
        return;
    }
    if let Some(map) = get_private_object(scope, args.this(), ELEMENT_COMPUTED_STYLE_MAP_SLOT) {
        rv.set(map.into());
        return;
    }
    let Some(style) = window_host::build_computed_style_object(
        scope,
        host_ptr,
        handle,
        None,
        window_host::ComputedStylePseudoArgument::originating_element(),
    ) else {
        rv.set_undefined();
        return;
    };
    let map = ComputedStylePropertyMapDeclaration::new(style)
        .bind(scope)
        .expect("computed StylePropertyMap declaration should bind");
    set_private_value(
        scope,
        args.this(),
        ELEMENT_COMPUTED_STYLE_MAP_SLOT,
        map.into(),
    );
    rv.set(map.into());
}

fn style_property_map_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((style, property)) = map_style_and_property(scope, &args) else {
        return;
    };
    let values = style_values_for_property(scope, style, &property);
    if let Some(value) = values.first() {
        rv.set((*value).into());
    } else {
        rv.set_undefined();
    }
}

fn style_property_map_get_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((style, property)) = map_style_and_property(scope, &args) else {
        return;
    };
    rv.set(style_values_array(scope, style, &property).into());
}

fn style_property_map_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((style, property)) = map_style_and_property(scope, &args) else {
        return;
    };
    let has = style_property_text(scope, style, &property).is_some();
    rv.set_bool(has);
}

fn style_property_map_size_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(style) = map_style_object(scope, args.this()) else {
        throw_style_property_map_illegal_invocation(scope, "get size");
        return;
    };
    let size = computed_style_property_names(scope, style)
        .len()
        .min(u32::MAX as usize) as u32;
    rv.set(v8::Integer::new_from_unsigned(scope, size).into());
}

fn style_property_map_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(style) = map_style_object(scope, args.this()) else {
        throw_style_property_map_illegal_invocation(scope, "entries");
        return;
    };
    let entries = computed_style_property_names(scope, style)
        .into_iter()
        .map(|property| {
            let property_value = v8_string(scope, &property)
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::undefined(scope).into());
            let values = style_values_array(scope, style, &property);
            v8::Array::new_with_elements(scope, &[property_value, values.into()])
        })
        .map(v8::Local::<v8::Value>::from)
        .collect::<Vec<_>>();
    let entries = v8::Array::new_with_elements(scope, &entries);
    set_style_property_map_iterator(scope, entries, &mut rv);
}

fn style_property_map_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(style) = map_style_object(scope, args.this()) else {
        throw_style_property_map_illegal_invocation(scope, "keys");
        return;
    };
    let keys = computed_style_property_names(scope, style)
        .into_iter()
        .filter_map(|property| v8_string(scope, &property))
        .map(v8::Local::<v8::Value>::from)
        .collect::<Vec<_>>();
    let keys = v8::Array::new_with_elements(scope, &keys);
    set_style_property_map_iterator(scope, keys, &mut rv);
}

fn style_property_map_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(style) = map_style_object(scope, args.this()) else {
        throw_style_property_map_illegal_invocation(scope, "values");
        return;
    };
    let values = computed_style_property_names(scope, style)
        .into_iter()
        .map(|property| style_values_array(scope, style, &property))
        .map(v8::Local::<v8::Value>::from)
        .collect::<Vec<_>>();
    let values = v8::Array::new_with_elements(scope, &values);
    set_style_property_map_iterator(scope, values, &mut rv);
}

fn style_property_map_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(style) = map_style_object(scope, args.this()) else {
        throw_style_property_map_illegal_invocation(scope, "forEach");
        return;
    };
    let Some(callback) = prepare_webidl_collection_for_each_callback(
        scope,
        args.get(0),
        "StylePropertyMapReadOnly forEach",
    ) else {
        return;
    };
    let this_arg = args.get(1);
    let snapshot = computed_style_property_names(scope, style)
        .into_iter()
        .filter_map(|property| {
            let property_value = v8_string(scope, &property)?;
            let values = style_values_array(scope, style, &property);
            Some((property_value, values))
        })
        .collect::<Vec<_>>();
    for (property, values) in snapshot {
        if invoke_webidl_collection_for_each_callback(
            scope,
            &callback,
            this_arg,
            values.into(),
            property.into(),
            args.this(),
        )
        .is_none()
        {
            return;
        }
    }
    rv.set_undefined();
}

fn map_style_and_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(v8::Local<'s, v8::Object>, String)> {
    let style = map_style_object(scope, args.this()).or_else(|| {
        throw_style_property_map_illegal_invocation(scope, "property lookup");
        None
    })?;
    let parsed = webidl::parse_args::<StylePropertyMapPropertyArgs>(scope, args)?;
    let property = canonical_map_property_name(&parsed.property).or_else(|| {
        throw_type_error(scope, &format!("Invalid propertyName: {}", parsed.property));
        None
    })?;
    Some((style, property))
}

fn map_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, map, STYLE_PROPERTY_MAP_STYLE_SLOT)
}

fn canonical_map_property_name(property: &str) -> Option<String> {
    if property.trim() != property || property.is_empty() {
        return None;
    }
    if property.starts_with("--") {
        return moli_css_parse::is_cssom_custom_property_name(property)
            .then(|| property.to_owned());
    }
    let property = moli_css_parse::canonical_style_property_name(&property.to_ascii_lowercase());
    css_style_declaration_exposes_property_name(&property).then_some(property)
}

fn style_values_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> v8::Local<'s, v8::Array> {
    let values = style_values_for_property(scope, style, property)
        .into_iter()
        .map(v8::Local::<v8::Value>::from)
        .collect::<Vec<_>>();
    v8::Array::new_with_elements(scope, &values)
}

fn style_values_for_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(text) = style_property_text(scope, style, property) else {
        return Vec::new();
    };
    let components = if property.starts_with("--") {
        vec![text]
    } else {
        top_level_comma_separated_component_values(&text).unwrap_or_else(|| vec![text])
    };
    components
        .into_iter()
        .map(|component| {
            values::style_value_from_text(scope, component.trim(), !property.starts_with("--"))
        })
        .collect()
}

fn style_property_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let text =
        native_bridge::element::computed_style_property_value_from_object(scope, style, property)?;
    if !text.is_empty() {
        return Some(text);
    }
    computed_style_property_names(scope, style)
        .iter()
        .any(|name| name == property)
        .then_some(text)
}

fn computed_style_property_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let mut names = native_bridge::element::computed_style_property_names_from_object(scope, style)
        .unwrap_or_default();
    for property in css_style_declaration_standard_property_names() {
        if native_bridge::element::computed_style_property_is_shorthand(property)
            || names.iter().any(|name| name == property)
        {
            continue;
        }
        if native_bridge::element::computed_style_property_value_from_object(scope, style, property)
            .is_some_and(|value| !value.is_empty())
        {
            names.push((*property).to_owned());
        }
    }
    names.sort_by(|left, right| {
        typed_om_property_sort_category(left)
            .cmp(&typed_om_property_sort_category(right))
            .then_with(|| left.cmp(right))
    });
    names.dedup();
    names
}

fn typed_om_property_sort_category(property: &str) -> u8 {
    if property.starts_with("--") {
        2
    } else if property.starts_with('-') {
        1
    } else {
        0
    }
}

fn set_style_property_map_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(iterator) = new_snapshot_webidl_iterator(
        scope,
        array,
        SnapshotWebIdlIteratorKind::StylePropertyMapReadOnly,
    ) {
        rv.set(iterator.into());
    } else {
        rv.set_undefined();
    }
}

fn throw_computed_style_map_illegal_invocation(scope: &mut v8::PinScope<'_, '_>) {
    throw_type_error(
        scope,
        "Failed to execute 'computedStyleMap' on 'Element': Illegal invocation.",
    );
}

fn throw_style_property_map_illegal_invocation(scope: &mut v8::PinScope<'_, '_>, method: &str) {
    throw_type_error(
        scope,
        &format!("Failed to execute '{method}' on 'StylePropertyMapReadOnly': Illegal invocation."),
    );
}

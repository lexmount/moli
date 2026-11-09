use super::super::*;
use crate::util::call_script_visible_function;

pub(super) fn js_node_type(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<i32> {
    object
        .get(scope, v8str(scope, "nodeType").into())?
        .int32_value(scope)
}

pub(super) use crate::util::object_defined_string_property as js_string_property;

pub(super) fn js_object_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let key = v8_string(scope, key)?;
    let value = object.get(scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    v8::Local::<v8::Object>::try_from(value).ok()
}

pub(super) fn js_child_node_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let mut values = Vec::new();
    if let Some(children) = object.get(scope, v8str(scope, "childNodes").into())
        && let Some(children_obj) = children.to_object(scope)
    {
        let length = children_obj
            .get(scope, v8str(scope, "length").into())
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(0);
        values.reserve(length as usize);
        for index in 0..length {
            let Some(child) = children_obj.get_index(scope, index) else {
                continue;
            };
            let Ok(child) = v8::Local::<v8::Object>::try_from(child) else {
                continue;
            };
            values.push(child);
        }
    }
    if values.is_empty() {
        values = js_child_node_objects_by_pointers(scope, object);
    }
    values
}

fn js_child_node_objects_by_pointers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let mut values = Vec::new();
    let mut current = object.get(scope, v8str(scope, "firstChild").into());
    let mut steps = 0usize;
    while let Some(value) = current {
        if value.is_null_or_undefined() {
            break;
        }
        let Ok(child) = v8::Local::<v8::Object>::try_from(value) else {
            break;
        };
        values.push(child);
        steps += 1;
        if steps >= 4096 {
            break;
        }
        current = child.get(scope, v8str(scope, "nextSibling").into());
    }
    values
}

pub(super) fn js_attribute_names(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<String> {
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    let Some(get_attribute_names) = object.get(scope, v8str(scope, "getAttributeNames").into())
    else {
        return Vec::new();
    };
    let Ok(get_attribute_names) = v8::Local::<v8::Function>::try_from(get_attribute_names) else {
        return Vec::new();
    };
    let Some(result) = call_script_visible_function(
        scope,
        get_attribute_names,
        object.into(),
        &[],
        "foreign node getAttributeNames",
    ) else {
        return Vec::new();
    };
    let Ok(array) = v8::Local::<v8::Array>::try_from(result) else {
        return Vec::new();
    };
    let mut names = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        let Some(value) = array.get_index(scope, index) else {
            continue;
        };
        let Some(value) = value.to_string(scope) else {
            continue;
        };
        names.push(value.to_rust_string_lossy(scope));
    }
    names
}

pub(super) fn js_attribute_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Option<String> {
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    let get_attribute = object.get(scope, v8str(scope, "getAttribute").into())?;
    let get_attribute = v8::Local::<v8::Function>::try_from(get_attribute).ok()?;
    let name = v8_string(scope, name)?;
    let value = call_script_visible_function(
        scope,
        get_attribute,
        object.into(),
        &[name.into()],
        "foreign node getAttribute",
    )?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

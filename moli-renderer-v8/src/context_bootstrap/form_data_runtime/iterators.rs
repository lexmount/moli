use super::*;
use crate::context_bootstrap::form_data_runtime::storage::form_data_entries;
use crate::util::{
    get_private_object, get_private_value, materialize_hidden_function_template_prototype,
    set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FORM_DATA_ITERATOR_TARGET_SLOT: &str = "__moliFormDataIteratorTarget";
const FORM_DATA_ITERATOR_INDEX_SLOT: &str = "__moliFormDataIteratorIndex";
const FORM_DATA_ITERATOR_KIND_SLOT: &str = "__moliFormDataIteratorKind";
const FORM_DATA_ITERATOR_PROTOTYPE_SLOT: &str = "__moliFormDataIteratorPrototype";

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::FormDataIterator)]
struct FormDataIteratorDeclaration<'scope> {
    #[webapi(slot = FORM_DATA_ITERATOR_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
    #[webapi(slot = FORM_DATA_ITERATOR_INDEX_SLOT)]
    index: i32,
    #[webapi(slot = FORM_DATA_ITERATOR_KIND_SLOT)]
    kind: &'static str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::FormDataIterator,
    intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
    prototype_to_string_tag = "FormData Iterator",
    readonly_prototype,
    enumerable
)]
struct FormDataIteratorPrototypeDeclaration {
    #[webapi(method = "next", callback = form_data_iterator_next_callback)]
    next: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct FormDataIteratorResultDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    done: bool,
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(Clone, Copy)]
pub(super) enum FormDataIteratorKind {
    Keys,
    Values,
    Entries,
}

pub(super) fn live_form_data_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    kind: FormDataIteratorKind,
) -> Option<v8::Local<'s, v8::Value>> {
    let iterator = FormDataIteratorDeclaration::new(target, 0, iterator_kind_name(kind))
        .bind(scope)
        .ok()?;
    let prototype = form_data_iterator_prototype(scope)?;
    iterator.set_prototype(scope, prototype.into())?;
    Some(iterator.into())
}

fn form_data_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) = get_private_value(scope, global, FORM_DATA_ITERATOR_PROTOTYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }
    let template = FormDataIteratorPrototypeDeclaration::build(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(
        scope,
        global,
        FORM_DATA_ITERATOR_PROTOTYPE_SLOT,
        prototype.into(),
    );
    Some(prototype)
}

fn form_data_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = args.this();
    let Some(target) = get_private_object(scope, iterator, FORM_DATA_ITERATOR_TARGET_SLOT) else {
        rv.set_null();
        return;
    };
    let Some(kind_value) = get_private_value(scope, iterator, FORM_DATA_ITERATOR_KIND_SLOT) else {
        rv.set_null();
        return;
    };
    let Some(kind_name) = callback_value_string(scope, kind_value) else {
        rv.set_null();
        return;
    };
    let Some(index_value) = get_private_value(scope, iterator, FORM_DATA_ITERATOR_INDEX_SLOT)
    else {
        rv.set_null();
        return;
    };
    let index = index_value.integer_value(scope).unwrap_or(0).max(0) as usize;
    let entries = form_data_entries(scope, target);
    if index >= entries.len() {
        let result = form_data_iterator_result(scope, v8::undefined(scope).into(), true);
        rv.set(result.into());
        return;
    }

    let (key, entry_value) = &entries[index];
    let value: Option<v8::Local<'_, v8::Value>> = match kind_name.as_str() {
        "keys" => v8_string(scope, key).map(Into::into),
        "values" => Some(v8::Local::new(scope, entry_value)),
        _ => {
            let Some(key) = v8_string(scope, key) else {
                rv.set_null();
                return;
            };
            let value = v8::Local::new(scope, entry_value);
            Some(v8::Array::new_with_elements(scope, &[key.into(), value]).into())
        }
    };
    let Some(value) = value else {
        rv.set_null();
        return;
    };

    set_private_value(
        scope,
        iterator,
        FORM_DATA_ITERATOR_INDEX_SLOT,
        v8::Integer::new(scope, (index + 1) as i32).into(),
    );
    let result = form_data_iterator_result(scope, value, false);
    rv.set(result.into());
}

fn form_data_iterator_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
) -> v8::Local<'s, v8::Object> {
    FormDataIteratorResultDeclaration::new(done, value)
        .bind(scope)
        .expect("FormData iterator result declaration should bind")
}

fn iterator_kind_name(kind: FormDataIteratorKind) -> &'static str {
    match kind {
        FormDataIteratorKind::Keys => "keys",
        FormDataIteratorKind::Values => "values",
        FormDataIteratorKind::Entries => "entries",
    }
}

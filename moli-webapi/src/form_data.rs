//! FormData bindings. The runtime supplies live form entries and platform wrappers.

use crate::url_api::callback_value_string;
use moli_v8_util::{global_constructor_object, throw_type_error, v8_string};

mod callbacks;
mod iterators;
pub mod storage;
mod template;

pub use template::build_form_data_constructor_template;

pub struct FormDataRuntime {
    pub entries_for_form: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        Option<v8::Local<'s, v8::Object>>,
    ) -> Option<Vec<(String, v8::Global<v8::Value>)>>,
    pub object_owner: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
    ) -> v8::Local<'s, v8::Object>,
    pub value_in_realm: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Value>,
        v8::Local<'s, v8::Context>,
    ) -> Option<v8::Local<'s, v8::Value>>,
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) -> Option<&'static FormDataRuntime> {
    Some(&crate::embedding::realm_runtime(scope)?.form_data)
}

fn object_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    runtime(scope).map_or(object, |runtime| (runtime.object_owner)(scope, object))
}

pub fn new_world_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor =
        v8::Local::<v8::Function>::try_from(global_constructor_object(scope, "FormData")?).ok()?;
    constructor.new_instance(scope, &[])
}

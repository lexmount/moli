pub use moli_v8_util::{global_constructor_prototype, v8_string, v8str};

/// Complete description of one function installed by
/// `WebApiFunctionTemplate`.
///
/// The derive maps each Rust callback to V8's raw callback type at the call
/// site. Keeping that raw pointer in this short-lived value lets the shared
/// installers avoid monomorphization without adding a callback trampoline or
/// persistent callback data.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct FunctionTemplateMember<'s> {
    pub callback: v8::FunctionCallback,
    pub length: i32,
    pub data: Option<v8::Local<'s, v8::Value>>,
    pub class_name: Option<v8::Local<'s, v8::String>>,
}

#[inline(always)]
fn build_function_template_member<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    member: FunctionTemplateMember<'s>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let builder = v8::FunctionTemplate::builder_raw(member.callback)
        .length(member.length)
        .constructor_behavior(v8::ConstructorBehavior::Throw);
    let function = match member.data {
        Some(data) => builder.data(data).build(scope),
        None => builder.build(scope),
    };
    if let Some(class_name) = member.class_name {
        function.set_class_name(class_name);
    }
    function
}

/// Builds and installs a method declared by `WebApiFunctionTemplate`.
#[doc(hidden)]
#[inline(never)]
pub fn install_function_template_method<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::ObjectTemplate>,
    property_key: v8::Local<'s, v8::Name>,
    member: FunctionTemplateMember<'s>,
    attributes: v8::PropertyAttribute,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let function = build_function_template_member(scope, member);
    target.set_with_attr(property_key, function.into(), attributes);
    function
}

/// Builds and installs a static method declared by
/// `WebApiFunctionTemplate`.
#[doc(hidden)]
#[inline(never)]
pub fn install_function_template_static_method<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::FunctionTemplate>,
    property_key: v8::Local<'s, v8::Name>,
    member: FunctionTemplateMember<'s>,
    attributes: v8::PropertyAttribute,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let function = build_function_template_member(scope, member);
    target.set_with_attr(property_key, function.into(), attributes);
    function
}

/// Builds and installs an accessor declared by `WebApiFunctionTemplate`.
#[doc(hidden)]
#[inline(never)]
pub fn install_function_template_accessor<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::ObjectTemplate>,
    property_key: v8::Local<'s, v8::Name>,
    getter: FunctionTemplateMember<'s>,
    setter: Option<FunctionTemplateMember<'s>>,
    attributes: v8::PropertyAttribute,
) {
    let getter = build_function_template_member(scope, getter);
    let setter = setter.map(|setter| build_function_template_member(scope, setter));
    target.set_accessor_property(property_key, Some(getter), setter, attributes);
}

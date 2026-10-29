pub use crate::callback::{
    invoke_promise_callback, invoke_web_api_constructor, throw_illegal_invocation,
};
pub use moli_v8_util::{global_constructor_prototype, v8_string, v8str};

/// Complete description of one function installed by
/// `WebApiFunctionTemplate`.
///
/// The derive maps each Rust callback using V8's native adapter at installation
/// time. The shared installers need neither a call-time trampoline nor extra
/// persistent callback data.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct FunctionTemplateMember<'s> {
    pub callback: v8::FunctionCallback,
    pub length: i32,
    pub data: Option<v8::Local<'s, v8::Value>>,
    pub class_name: Option<v8::Local<'s, v8::String>>,
    pub receiver_access_check: bool,
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
    if member.receiver_access_check {
        function.set_accept_any_receiver(false);
    }
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

/// Creates a declared object method or accessor without repeating V8 builder
/// code at every installation site. Callback data remains the original value.
#[doc(hidden)]
#[inline(never)]
pub fn build_object_function<'s>(
    scope: &v8::PinScope<'s, '_>,
    callback: v8::FunctionCallback,
    length: i32,
    data: Option<v8::Local<'s, v8::Value>>,
    receiver_access_check: bool,
) -> Option<v8::Local<'s, v8::Function>> {
    if receiver_access_check {
        return build_function_template_member(
            scope,
            FunctionTemplateMember {
                callback,
                length,
                data,
                class_name: None,
                receiver_access_check,
            },
        )
        .get_function(scope);
    }
    let builder = v8::Function::builder_raw(callback)
        .length(length)
        .constructor_behavior(v8::ConstructorBehavior::Throw);
    match data {
        Some(data) => builder.data(data).build(scope),
        None => builder.build(scope),
    }
}

/// Static metadata for a string-named template method without callback data.
/// The callback mapping runs only during installation, not JavaScript calls.
#[doc(hidden)]
pub struct FunctionTemplateMethod {
    pub name: &'static str,
    pub callback: fn() -> v8::FunctionCallback,
    pub length: i32,
    pub enumerable: bool,
    pub readonly: bool,
    pub dont_delete: bool,
    pub receiver_access_check: bool,
}

/// Installs a run of methods from metadata instead of repeating call-site code.
#[doc(hidden)]
#[inline(never)]
pub fn install_function_template_methods<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::ObjectTemplate>,
    methods: &[FunctionTemplateMethod],
) {
    for method in methods {
        let name = v8str(scope, method.name);
        let mut attributes = v8::PropertyAttribute::NONE;
        if !method.enumerable {
            attributes = attributes | v8::PropertyAttribute::DONT_ENUM;
        }
        if method.readonly {
            attributes = attributes | v8::PropertyAttribute::READ_ONLY;
        }
        if method.dont_delete {
            attributes = attributes | v8::PropertyAttribute::DONT_DELETE;
        }
        install_function_template_method(
            scope,
            target,
            name.into(),
            FunctionTemplateMember {
                callback: (method.callback)(),
                length: method.length,
                data: None,
                class_name: Some(name),
                receiver_access_check: method.receiver_access_check,
            },
            attributes,
        );
    }
}

/// Static metadata for a string-named accessor without callback data.
/// Mappings preserve constant tables while using V8's original call-time path.
/// The derive prefixes getter names with "get "; the suffix is the property key.
#[doc(hidden)]
pub struct FunctionTemplateAccessor {
    pub getter_name: &'static str,
    /// Empty when no setter is declared.
    pub setter_name: &'static str,
    pub getter: fn() -> v8::FunctionCallback,
    pub setter: Option<fn() -> v8::FunctionCallback>,
    pub enumerable: bool,
    pub readonly: bool,
    pub dont_delete: bool,
    pub getter_receiver_access_check: bool,
    pub setter_receiver_access_check: bool,
}

/// Installs a run of accessors while preserving getter/setter function names.
#[doc(hidden)]
#[inline(never)]
pub fn install_function_template_accessors<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::ObjectTemplate>,
    accessors: &[FunctionTemplateAccessor],
) {
    for accessor in accessors {
        let key = v8str(scope, &accessor.getter_name["get ".len()..]);
        let mut attributes = v8::PropertyAttribute::NONE;
        if !accessor.enumerable {
            attributes = attributes | v8::PropertyAttribute::DONT_ENUM;
        }
        if accessor.readonly {
            attributes = attributes | v8::PropertyAttribute::READ_ONLY;
        }
        if accessor.dont_delete {
            attributes = attributes | v8::PropertyAttribute::DONT_DELETE;
        }
        let getter = FunctionTemplateMember {
            callback: (accessor.getter)(),
            length: 0,
            data: None,
            class_name: v8::String::new(scope, accessor.getter_name),
            receiver_access_check: accessor.getter_receiver_access_check,
        };
        let setter = accessor.setter.map(|mapping| FunctionTemplateMember {
            callback: mapping(),
            length: 1,
            data: None,
            class_name: v8::String::new(scope, accessor.setter_name),
            receiver_access_check: accessor.setter_receiver_access_check,
        });
        install_function_template_accessor(scope, target, key.into(), getter, setter, attributes);
    }
}

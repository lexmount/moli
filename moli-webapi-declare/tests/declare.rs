use std::pin::pin;

use moli_v8_test_util::ensure_v8;
use moli_webapi_declare::{
    ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiFunctionTemplateDeclaration,
    WebApiObject, WebApiValue, define_array_data_property,
};

fn install_template<'s, D: WebApiFunctionTemplateDeclaration>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Function> {
    let constructor = D::build(scope)
        .get_function(scope)
        .expect("build constructor");
    global
        .define_own_property(
            scope,
            v8::String::new(scope, D::NAME).unwrap().into(),
            constructor.into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .expect("publish constructor");
    constructor
}

fn run_script<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).expect("test source should allocate");
    v8::Script::compile(scope, source, None)
        .and_then(|script| script.run(scope))
        .expect("test script should run")
}

fn sample_now_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Number::new(scope, 7.0).into());
}

fn sample_data_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data().to_string(scope).unwrap().into());
}

fn sample_object_data_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Boolean::new(scope, args.data().strict_equals(args.this().into())).into());
}

fn sample_accessor_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data().to_string(scope).unwrap().into());
}

fn sample_accessor_setter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let payload_key = v8::String::new(scope, "setterPayload").unwrap();
    let written_key = v8::String::new(scope, "writtenValue").unwrap();
    let payload = args.data().to_string(scope).unwrap();
    let _ = args.this().set(scope, payload_key.into(), payload.into());
    let _ = args.this().set(scope, written_key.into(), args.get(0));
}

fn sample_receiver_getter(
    _scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.this().into());
}

fn sample_native_holder_getter(
    _scope: &mut v8::PinScope<'_, '_>,
    _key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.holder().into());
}

fn sample_getter_value_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(
        v8::String::new(scope, "declared-getter-value")
            .unwrap()
            .into(),
    );
}

fn sample_constructor_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

fn indexed_zero_getter(
    _scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if index != 0 {
        return v8::Intercepted::kNo;
    }
    rv.set_uint32(42);
    v8::Intercepted::kYes
}

fn indexed_zero_query(
    _scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if index != 0 {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

const SAMPLE_TIME_ORIGIN_SLOT: &str = "__lmSampleTimeOrigin";
const SAMPLE_ENTRIES_SLOT: &str = "__lmSampleEntries";
const SAMPLE_PRIVATE_COUNT_SLOT: &str = "__lmSamplePrivateCount";
const SAMPLE_PRIVATE_LABEL_SLOT: &str = "__lmSamplePrivateLabel";
const SAMPLE_OPTIONAL_SLOT: &str = "__lmSampleOptional";
const SAMPLE_MISSING_OPTIONAL_SLOT: &str = "__lmSampleMissingOptional";
const SAMPLE_HIDDEN_SLOT: &str = "__lmSampleHidden";
const SAMPLE_LOCKED_HIDDEN_SLOT: &str = "__lmSampleLockedHidden";
const SAMPLE_LOCAL_OBJECT_SLOT: &str = "__lmSampleLocalObject";

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "SamplePerformance",
    constructor = "illegal",
    readonly_prototype,
    prototype_to_string_tag = "SamplePerformance"
)]
struct SamplePerformanceInterface {
    #[webapi(method, length = 0, callback = sample_now_callback)]
    now: (),
    #[webapi(method, length = 0, callback = sample_now_callback)]
    sample_now: (),

    #[webapi(accessor_property, getter = sample_now_callback)]
    current_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = interfaces::ConstructibleSample,
    constructor_callback = sample_constructor_callback,
    constructor_length = 2
)]
struct ConstructibleSampleInterface {}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "InterfaceDescriptorSample", constructor = "illegal")]
struct InterfaceDescriptorSample {
    #[webapi(method, enumerable, readonly, dont_delete, callback = sample_now_callback)]
    locked_visible: (),
}

#[derive(WebApiObject)]
#[webapi(interface = interfaces::SamplePerformance)]
struct SamplePerformanceObject {
    #[webapi(data_property = SAMPLE_TIME_ORIGIN_SLOT)]
    time_origin: f64,

    #[webapi(data_property = "timeOrigin")]
    public_time_origin: f64,

    #[webapi(data_property)]
    enabled: bool,

    #[webapi(data_property)]
    label: &'static str,

    #[webapi(data_property = "visibleLabel", enumerable)]
    visible_label: &'static str,

    #[webapi(data_property = SAMPLE_ENTRIES_SLOT)]
    entries: Vec<u32>,

    #[webapi(data_property)]
    sized_entries: [u32; 2],

    #[webapi(data_property = "nullable", init = "null")]
    nullable: (),

    #[webapi(data_property = "emptyObject", init = "object")]
    empty_object: (),

    #[webapi(data_property = "nullObject", init = "null_object")]
    null_object: (),

    #[webapi(data_property = "emptyArray", init = "array")]
    empty_array: (),

    #[webapi(data_property = "undefinedValue", init = "undefined")]
    undefined_value: (),

    #[webapi(data_property = "defaultTrue", init = true)]
    default_true: (),

    #[webapi(data_property = "defaultFalse", init = false)]
    default_false: (),

    #[webapi(data_property = "defaultZero", init = 0)]
    default_zero: (),

    #[webapi(data_property = "defaultEmptyString", init = "")]
    default_empty_string: (),

    #[webapi(data_property = "defaultLiteralString", init = string("ready"))]
    default_literal_string: (),

    #[webapi(hidden = SAMPLE_HIDDEN_SLOT)]
    hidden_value: &'static str,

    #[webapi(hidden = SAMPLE_LOCKED_HIDDEN_SLOT)]
    #[webapi(readonly, dont_delete)]
    locked_hidden_value: &'static str,

    #[webapi(hidden)]
    hidden_by_field: &'static str,

    #[webapi(slot = SAMPLE_PRIVATE_COUNT_SLOT)]
    private_count: u32,

    #[webapi(slot = SAMPLE_PRIVATE_LABEL_SLOT)]
    private_label: &'static str,

    #[webapi(slot)]
    private_by_field: u32,

    #[webapi(slot = SAMPLE_OPTIONAL_SLOT)]
    optional_slot: Option<u32>,

    #[webapi(slot = SAMPLE_MISSING_OPTIONAL_SLOT)]
    missing_optional_slot: Option<u32>,
}

#[derive(WebApiObject)]
#[webapi(interface = interfaces::SamplePerformance)]
struct SampleLocalObjectSlot<'scope> {
    #[webapi(slot = SAMPLE_LOCAL_OBJECT_SLOT)]
    local_object: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct GeneratedConstructorObject<'scope> {
    #[webapi(data_property, init = true)]
    brand: (),

    #[webapi(data_property)]
    value: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, init = "array")]
    entries: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NestedValueRecord {
    #[webapi(data_property, enumerable)]
    name: &'static str,

    #[webapi(data_property, enumerable)]
    count: u32,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DefaultNameObject<'scope> {
    #[webapi(data_property, enumerable)]
    _visible_label: &'static str,

    cached_getter: v8::Local<'scope, v8::Function>,

    #[webapi(method, callback = sample_now_callback)]
    sample_method: (),

    #[webapi(method, callback = sample_now_callback, readonly, dont_delete)]
    locked_method: (),

    #[webapi(method, callback = sample_now_callback)]
    optional_method: Option<()>,

    #[webapi(method, callback = sample_now_callback)]
    missing_optional_method: Option<()>,

    #[webapi(method = "explicitMethod", callback = sample_now_callback)]
    explicit_method: (),

    #[webapi(
        method,
        callback = sample_data_callback,
        data = "declared-data"
    )]
    data_method: (),

    #[webapi(method, callback = sample_object_data_callback, data = object)]
    object_data_method: (),

    #[webapi(method, symbol = "iterator", callback = sample_now_callback)]
    symbol_iterator: (),

    #[webapi(method, symbol = "asyncIterator", callback = sample_now_callback)]
    symbol_async_iterator: (),

    #[webapi(
        accessor_property,
        getter = sample_accessor_getter,
        data = "declared-accessor"
    )]
    data_value: (),

    #[webapi(
        accessor_property = "mutableValue",
        getter = sample_accessor_getter,
        setter = sample_accessor_setter,
        data = "declared-setter",
        setter_data = "declared-setter-write",
        enumerable,
        dont_delete
    )]
    mutable_value: (),

    #[webapi(accessor_property, getter_value = self.cached_getter, enumerable, dont_delete)]
    cached_value: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct AliasObject {
    #[webapi(method, enumerable, length = 0, callback = sample_now_callback)]
    entries: (),

    #[webapi(alias = "entries", symbol = "iterator", enumerable)]
    iterator: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ConstantObject {
    #[webapi(constant = "READY", value = 4u32)]
    ready: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = interfaces::SampleTemplate,
    constructor_callback = sample_constructor_callback,
    constructor_length = 1,
    enumerable
)]
struct SampleFunctionTemplateDeclaration {
    #[webapi(
        static_method = "from",
        length = 1,
        callback = sample_data_callback,
        data = "template-static-data"
    )]
    from: (),

    #[webapi(static_method, callback = sample_now_callback, readonly, dont_delete)]
    locked_static: (),

    #[webapi(constant = "READY", value = 4u32)]
    ready: (),

    #[webapi(
        method,
        length = 2,
        callback = sample_data_callback,
        data = "template-method-data"
    )]
    append: (),

    #[webapi(method, callback = sample_now_callback, readonly, dont_delete)]
    locked_method: (),

    #[webapi(method, length = 0, callback = sample_now_callback)]
    entries: (),

    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),

    #[webapi(accessor_property, getter = sample_receiver_getter)]
    receiver: (),

    #[webapi(
        accessor_property,
        getter = sample_accessor_getter,
        setter = sample_accessor_setter,
        data = "template-getter-data",
        setter_data = "template-setter-data"
    )]
    data_value: (),

    #[webapi(native_data_property, getter = sample_native_holder_getter)]
    native_holder: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "IntrinsicArrayTemplate", enumerable)]
struct IntrinsicArrayTemplateDeclaration {
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoEntries)]
    entries: (),

    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues)]
    values: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "IntrinsicErrorParentTemplate",
    intrinsic_prototype_parent = v8::Intrinsic::ErrorPrototype
)]
struct IntrinsicErrorParentTemplateDeclaration {
    #[webapi(method, callback = sample_now_callback)]
    marker: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "Example Iterator",
    intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
    prototype_to_string_tag = "Example Iterator",
    readonly_prototype,
    enumerable
)]
struct IntrinsicIteratorTemplateDeclaration {
    #[webapi(method, callback = sample_now_callback)]
    next: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ExplicitFunctionNameSymbolMethodObject {
    #[webapi(
        method,
        symbol = "iterator",
        function_name = "values",
        callback = sample_now_callback
    )]
    iterator: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NativeDataPropertyObject {
    #[webapi(native_data_property, getter = sample_native_holder_getter)]
    native_holder: (),
}

#[derive(WebApiObject)]
#[webapi(plain, enumerable)]
struct DefaultEnumerableMembersObject {
    #[webapi(data_property)]
    property_value: &'static str,

    #[webapi(method, callback = sample_now_callback)]
    string_method: (),

    #[webapi(
        accessor_property,
        getter = sample_accessor_getter,
        data = "default-enumerable-accessor"
    )]
    accessor_value: (),

    #[webapi(method, symbol = "iterator", callback = sample_now_callback)]
    symbol_iterator: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DefaultEnumerablePropertiesObject {
    client_x: f64,
    is_2d: bool,
    related_target: Option<&'static str>,
    missing_target: Option<&'static str>,
}

#[derive(WebApiObject)]
#[webapi(interface = interfaces::SamplePerformance, data_properties, enumerable)]
struct PrototypeBackedEnumerablePropertiesObject {
    client_x: f64,
}

#[derive(WebApiObject)]
#[webapi(plain, rename_all = "none", data_properties, enumerable)]
struct ExplicitSnakeCasePropertiesObject {
    snake_case_name: &'static str,
}

#[derive(WebApiObject)]
#[webapi(
    interface = interfaces::SamplePerformance,
    own_to_string_tag = "ReadonlySample",
    readonly_to_string_tag,
)]
struct ReadonlyTagObject {}

#[derive(WebApiObject)]
#[webapi(interface = interfaces::MissingSampleInterface, require_prototype)]
struct RequiredPrototypeObject {}

#[derive(WebApiObject)]
#[webapi(plain, prototype = "SamplePerformance", require_prototype)]
struct PrototypeOnlyObject {}

#[derive(WebApiObject)]
#[webapi(plain, prototype = "MissingSampleInterface", require_prototype)]
struct MissingPrototypeOnlyObject {}

#[derive(WebApiObject)]
#[webapi(plain, scope_lifetime = 'scope)]
struct RuntimePrototypeTaggedObject<'scope, 'tag> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,

    #[webapi(to_string_tag)]
    to_string_tag: Option<&'tag str>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct RuntimeTagOnlyObject<'tag> {
    #[webapi(to_string_tag)]
    to_string_tag: Option<&'tag str>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct RuntimeReadonlyLockedTagObject {
    #[webapi(to_string_tag, readonly, dont_delete, init = string("LockedRuntimeTag"))]
    to_string_tag: (),
}

#[derive(WebApiObject)]
#[webapi(
    interface = interfaces::SamplePerformance,
    fallback_to_string_tag = "SampleFallback",
)]
struct PrototypeBackedFallbackTagObject {}

#[derive(WebApiObject)]
#[webapi(
    interface = interfaces::MissingSampleInterface,
    fallback_to_string_tag = "MissingSample",
)]
struct MissingPrototypeFallbackTagObject {}

#[test]
fn declared_template_and_object_have_expected_surface() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);

    install_template::<SamplePerformanceInterface>(scope, global);
    let declaration = SamplePerformanceObject {
        time_origin: 12.0,
        public_time_origin: 12.0,
        enabled: true,
        label: "declared",
        visible_label: "visible",
        entries: Vec::new(),
        sized_entries: [5, 7],
        nullable: (),
        empty_object: (),
        null_object: (),
        empty_array: (),
        undefined_value: (),
        default_true: (),
        default_false: (),
        default_zero: (),
        default_empty_string: (),
        default_literal_string: (),
        hidden_value: "hidden-data",
        locked_hidden_value: "locked-hidden-data",
        hidden_by_field: "field-hidden",
        private_count: 3,
        private_label: "hidden",
        private_by_field: 11,
        optional_slot: Some(9),
        missing_optional_slot: None,
    };
    let object = declaration.bind(scope).expect("bind object");
    let existing = v8::Object::new(scope);
    declaration
        .bind_into(scope, existing)
        .expect("bind existing object");
    let local_slot_value = v8::Object::new(scope);
    let local_object = SampleLocalObjectSlot {
        local_object: local_slot_value,
    }
    .bind(scope)
    .expect("bind object with local slot");
    let generated_constructor_value = v8::Boolean::new(scope, true).into();
    let generated_constructor_object = GeneratedConstructorObject::new(generated_constructor_value)
        .bind(scope)
        .expect("bind generated-constructor object");
    let getter_value = v8::Function::builder(sample_getter_value_callback)
        .build(scope)
        .expect("build cached getter");
    let default_name_object = DefaultNameObject {
        _visible_label: "renamed",
        cached_getter: getter_value,
        sample_method: (),
        locked_method: (),
        optional_method: Some(()),
        missing_optional_method: None,
        explicit_method: (),
        data_method: (),
        object_data_method: (),
        symbol_iterator: (),
        symbol_async_iterator: (),
        data_value: (),
        mutable_value: (),
        cached_value: (),
    }
    .bind(scope)
    .expect("bind object with default names");
    let alias_object = AliasObject {
        entries: (),
        iterator: (),
    }
    .bind(scope)
    .expect("bind alias object");
    let constant_object = ConstantObject { ready: () }
        .bind(scope)
        .expect("bind constant object");
    let default_enumerable_members_object = DefaultEnumerableMembersObject {
        property_value: "default-enumerable",
        string_method: (),
        accessor_value: (),
        symbol_iterator: (),
    }
    .bind(scope)
    .expect("bind object with default enumerable members");
    let default_properties_object = DefaultEnumerablePropertiesObject {
        client_x: 42.0,
        is_2d: true,
        related_target: Some("target"),
        missing_target: None,
    }
    .bind(scope)
    .expect("bind object with default enumerable properties");
    let explicit_snake_case_object = ExplicitSnakeCasePropertiesObject {
        snake_case_name: "snake",
    }
    .bind(scope)
    .expect("bind explicit snake-case properties object");

    let constructor = global
        .get(
            scope,
            v8::String::new(scope, "SamplePerformance").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("constructor");
    let prototype = constructor
        .get(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("prototype");
    let now = prototype
        .get(scope, v8::String::new(scope, "now").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("now function");
    assert_eq!(
        now.has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into()),
        Some(false),
        "Web IDL operations must be concise non-constructor functions"
    );
    let value = now.call(scope, object.into(), &[]).expect("call now");
    assert_eq!(value.number_value(scope), Some(7.0));
    let sample_now = prototype
        .get(scope, v8::String::new(scope, "sampleNow").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("sampleNow function");
    let value = sample_now
        .call(scope, object.into(), &[])
        .expect("call sampleNow");
    assert_eq!(value.number_value(scope), Some(7.0));
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "currentTime").unwrap().into())
            .and_then(|value| value.number_value(scope)),
        Some(7.0)
    );
    let current_time_descriptor = prototype
        .get_own_property_descriptor(scope, v8::String::new(scope, "currentTime").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("currentTime descriptor");
    let current_time_getter = current_time_descriptor
        .get(scope, v8::String::new(scope, "get").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("currentTime getter");
    assert_eq!(
        current_time_getter
            .has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into(),),
        Some(false),
        "Web IDL accessor functions must not expose a prototype"
    );
    assert_eq!(
        current_time_getter
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("get currentTime".to_owned())
    );

    let instance_constructor = object
        .get(scope, v8::String::new(scope, "constructor").unwrap().into())
        .expect("instance constructor");
    assert!(instance_constructor.strict_equals(constructor.into()));
    let constructor_descriptor = prototype
        .get_own_property_descriptor(scope, v8::String::new(scope, "constructor").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("prototype constructor descriptor");
    assert_eq!(
        constructor_descriptor
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .map(|value| value.strict_equals(constructor.into())),
        Some(true)
    );
    assert_eq!(
        constructor_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        constructor_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        constructor_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );

    let prototype_descriptor = constructor
        .get_own_property_descriptor(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("prototype descriptor");
    assert_eq!(
        prototype_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        prototype_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        prototype_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );

    let time_origin = object
        .get(scope, v8::String::new(scope, "timeOrigin").unwrap().into())
        .expect("timeOrigin");
    assert_eq!(time_origin.number_value(scope), Some(12.0));
    let label = object
        .get(scope, v8::String::new(scope, "label").unwrap().into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(label.as_deref(), Some("declared"));
    let visible_label = object
        .get(
            scope,
            v8::String::new(scope, "visibleLabel").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(visible_label.as_deref(), Some("visible"));
    assert_eq!(
        object
            .get_own_property_descriptor(
                scope,
                v8::String::new(scope, "visibleLabel").unwrap().into()
            )
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|descriptor| descriptor
                .get(scope, v8::String::new(scope, "enumerable").unwrap().into()))
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        object
            .get_own_property_descriptor(scope, v8::String::new(scope, "label").unwrap().into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|descriptor| descriptor
                .get(scope, v8::String::new(scope, "enumerable").unwrap().into()))
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let direct_hidden = v8::Object::new(scope);
    moli_webapi_declare::define_declared_hidden_property_with_descriptor(
        scope,
        direct_hidden,
        SAMPLE_LOCKED_HIDDEN_SLOT,
        &"direct-hidden",
        false,
        false,
    )
    .expect("direct locked hidden property should define");
    let direct_hidden_descriptor = direct_hidden
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, SAMPLE_LOCKED_HIDDEN_SLOT)
                .unwrap()
                .into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("direct locked hidden descriptor");
    assert_eq!(
        direct_hidden_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let locked_hidden_descriptor = object
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, SAMPLE_LOCKED_HIDDEN_SLOT)
                .unwrap()
                .into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("locked hidden descriptor");
    assert_eq!(
        locked_hidden_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_hidden_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_hidden_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "enabled").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        default_properties_object
            .get(scope, v8::String::new(scope, "clientX").unwrap().into())
            .and_then(|value| value.number_value(scope)),
        Some(42.0)
    );
    let data_method = default_name_object
        .get(scope, v8::String::new(scope, "dataMethod").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("dataMethod function");
    assert_eq!(
        data_method.has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into(),),
        Some(false),
        "operations with callback data must remain non-constructible"
    );
    let data_value = data_method
        .call(scope, default_name_object.into(), &[])
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(data_value.as_deref(), Some("declared-data"));
    let optional_method = default_name_object
        .get(
            scope,
            v8::String::new(scope, "optionalMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("optionalMethod function");
    let optional_method_value = optional_method
        .call(scope, default_name_object.into(), &[])
        .and_then(|value| value.number_value(scope));
    assert_eq!(optional_method_value, Some(7.0));
    assert!(
        !default_name_object
            .has_own_property(
                scope,
                v8::String::new(scope, "missingOptionalMethod")
                    .unwrap()
                    .into()
            )
            .unwrap_or(false)
    );
    let object_data_method = default_name_object
        .get(
            scope,
            v8::String::new(scope, "objectDataMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("objectDataMethod function");
    let object_data_matches_receiver = object_data_method
        .call(scope, default_name_object.into(), &[])
        .map(|value| value.boolean_value(scope));
    assert_eq!(object_data_matches_receiver, Some(true));
    let accessor_value = default_name_object
        .get(scope, v8::String::new(scope, "dataValue").unwrap().into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(accessor_value.as_deref(), Some("declared-accessor"));
    let cached_value = default_name_object
        .get(scope, v8::String::new(scope, "cachedValue").unwrap().into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(cached_value.as_deref(), Some("declared-getter-value"));
    let cached_descriptor = default_name_object
        .get_own_property_descriptor(scope, v8::String::new(scope, "cachedValue").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("cachedValue descriptor");
    assert_eq!(
        cached_descriptor
            .get(scope, v8::String::new(scope, "get").unwrap().into())
            .map(|value| value.strict_equals(getter_value.into())),
        Some(true)
    );
    assert_eq!(
        cached_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        cached_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let accessor_descriptor = default_name_object
        .get_own_property_descriptor(scope, v8::String::new(scope, "dataValue").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("dataValue descriptor");
    assert_eq!(
        accessor_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        accessor_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        accessor_descriptor
            .get(scope, v8::String::new(scope, "set").unwrap().into())
            .map(|value| value.is_undefined()),
        Some(true)
    );
    let mutable_key = v8::String::new(scope, "mutableValue").unwrap();
    let written = v8::String::new(scope, "written").unwrap();
    assert!(
        default_name_object
            .set(scope, mutable_key.into(), written.into())
            .unwrap_or(false)
    );
    let mutable_descriptor = default_name_object
        .get_own_property_descriptor(scope, mutable_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("mutableValue descriptor");
    assert_eq!(
        mutable_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        mutable_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let setter_payload = default_name_object
        .get(
            scope,
            v8::String::new(scope, "setterPayload").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(setter_payload.as_deref(), Some("declared-setter-write"));
    let written_value = default_name_object
        .get(
            scope,
            v8::String::new(scope, "writtenValue").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(written_value.as_deref(), Some("written"));
    assert_eq!(
        default_properties_object
            .get_own_property_descriptor(scope, v8::String::new(scope, "clientX").unwrap().into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|descriptor| descriptor
                .get(scope, v8::String::new(scope, "enumerable").unwrap().into()))
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    for key in ["propertyValue", "stringMethod", "accessorValue"] {
        assert_eq!(
            default_enumerable_members_object
                .get_own_property_descriptor(scope, v8::String::new(scope, key).unwrap().into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .and_then(|descriptor| descriptor
                    .get(scope, v8::String::new(scope, "enumerable").unwrap().into()))
                .map(|value| value.boolean_value(scope)),
            Some(true),
            "{key} should inherit the struct enumerable default"
        );
    }
    let default_enumerable_symbol_descriptor = default_enumerable_members_object
        .get_own_property_descriptor(scope, v8::Symbol::get_iterator(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("default enumerable Symbol.iterator descriptor");
    assert_eq!(
        default_enumerable_symbol_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        default_properties_object
            .get(scope, v8::String::new(scope, "is2D").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let related_target = default_properties_object
        .get(
            scope,
            v8::String::new(scope, "relatedTarget").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(related_target.as_deref(), Some("target"));
    let snake_case_name = explicit_snake_case_object
        .get(
            scope,
            v8::String::new(scope, "snake_case_name").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(snake_case_name.as_deref(), Some("snake"));
    assert!(
        !explicit_snake_case_object
            .has_own_property(
                scope,
                v8::String::new(scope, "snakeCaseName").unwrap().into()
            )
            .unwrap_or(false)
    );
    assert!(
        !default_properties_object
            .has_own_property(
                scope,
                v8::String::new(scope, "missingTarget").unwrap().into()
            )
            .unwrap_or(false)
    );
    let entries = object
        .get(
            scope,
            v8::String::new(scope, SAMPLE_ENTRIES_SLOT).unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("entries");
    assert_eq!(entries.length(), 0);
    let sized_entries = object
        .get(
            scope,
            v8::String::new(scope, "sizedEntries").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("sized entries");
    assert_eq!(sized_entries.length(), 2);
    assert_eq!(
        sized_entries
            .get_index(scope, 0)
            .and_then(|value| value.number_value(scope)),
        Some(5.0)
    );
    assert_eq!(
        sized_entries
            .get_index(scope, 1)
            .and_then(|value| value.number_value(scope)),
        Some(7.0)
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "nullable").unwrap().into())
            .map(|value| value.is_null()),
        Some(true)
    );
    let empty_object = object
        .get(scope, v8::String::new(scope, "emptyObject").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("empty object");
    assert_eq!(
        empty_object
            .get_prototype(scope)
            .map(|value| value.is_null()),
        Some(false)
    );
    let null_object = object
        .get(scope, v8::String::new(scope, "nullObject").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("null object");
    assert_eq!(
        null_object
            .get_prototype(scope)
            .map(|value| value.is_null()),
        Some(true)
    );
    let empty_array = object
        .get(scope, v8::String::new(scope, "emptyArray").unwrap().into())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("empty array");
    assert_eq!(empty_array.length(), 0);
    assert_eq!(
        object
            .get(
                scope,
                v8::String::new(scope, "undefinedValue").unwrap().into()
            )
            .map(|value| value.is_undefined()),
        Some(true)
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "defaultTrue").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        object
            .get(
                scope,
                v8::String::new(scope, "defaultFalse").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "defaultZero").unwrap().into())
            .and_then(|value| value.number_value(scope)),
        Some(0.0)
    );
    let default_empty_string = object
        .get(
            scope,
            v8::String::new(scope, "defaultEmptyString").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(default_empty_string.as_deref(), Some(""));
    let default_literal_string = object
        .get(
            scope,
            v8::String::new(scope, "defaultLiteralString")
                .unwrap()
                .into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(default_literal_string.as_deref(), Some("ready"));
    let hidden_time_origin = object
        .get(
            scope,
            v8::String::new(scope, SAMPLE_TIME_ORIGIN_SLOT)
                .unwrap()
                .into(),
        )
        .expect("hidden timeOrigin slot");
    assert_eq!(hidden_time_origin.number_value(scope), Some(12.0));
    let hidden_value = object
        .get(
            scope,
            v8::String::new(scope, SAMPLE_HIDDEN_SLOT).unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(hidden_value.as_deref(), Some("hidden-data"));
    assert_eq!(
        object
            .get_own_property_descriptor(
                scope,
                v8::String::new(scope, SAMPLE_HIDDEN_SLOT).unwrap().into()
            )
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|descriptor| descriptor
                .get(scope, v8::String::new(scope, "enumerable").unwrap().into()))
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let hidden_by_field = object
        .get(
            scope,
            v8::String::new(scope, "hidden_by_field").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(hidden_by_field.as_deref(), Some("field-hidden"));
    let private_count = get_private_value(scope, object, SAMPLE_PRIVATE_COUNT_SLOT)
        .expect("private count")
        .number_value(scope);
    assert_eq!(private_count, Some(3.0));
    let private_label = get_private_value(scope, object, SAMPLE_PRIVATE_LABEL_SLOT)
        .expect("private label")
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(private_label.as_deref(), Some("hidden"));
    let private_by_field = get_private_value(scope, object, "private_by_field")
        .expect("private field")
        .number_value(scope);
    assert_eq!(private_by_field, Some(11.0));
    let optional_slot = get_private_value(scope, object, SAMPLE_OPTIONAL_SLOT)
        .expect("optional slot")
        .number_value(scope);
    assert_eq!(optional_slot, Some(9.0));
    assert!(get_private_value(scope, object, SAMPLE_MISSING_OPTIONAL_SLOT).is_none());
    let local_object_slot = get_private_value(scope, local_object, SAMPLE_LOCAL_OBJECT_SLOT)
        .expect("local object slot");
    assert!(local_object_slot.strict_equals(local_slot_value.into()));
    assert_eq!(
        generated_constructor_object
            .get(scope, v8::String::new(scope, "brand").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        generated_constructor_object
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .map(|value| value.is_true()),
        Some(true)
    );
    assert!(
        generated_constructor_object
            .get(scope, v8::String::new(scope, "entries").unwrap().into())
            .is_some_and(|value| value.is_array())
    );
    let default_visible_label = default_name_object
        .get(
            scope,
            v8::String::new(scope, "visibleLabel").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(default_visible_label.as_deref(), Some("renamed"));
    let sample_method = default_name_object
        .get(
            scope,
            v8::String::new(scope, "sampleMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("sampleMethod function");
    let value = sample_method
        .call(scope, default_name_object.into(), &[])
        .expect("call sampleMethod");
    assert_eq!(value.number_value(scope), Some(7.0));
    let locked_method = default_name_object
        .get(
            scope,
            v8::String::new(scope, "lockedMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("lockedMethod function");
    let value = locked_method
        .call(scope, default_name_object.into(), &[])
        .expect("call lockedMethod");
    assert_eq!(value.number_value(scope), Some(7.0));
    let locked_method_descriptor = default_name_object
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, "lockedMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("lockedMethod descriptor");
    assert_eq!(
        locked_method_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_method_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_method_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let explicit_method = default_name_object
        .get(
            scope,
            v8::String::new(scope, "explicitMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("explicitMethod function");
    let value = explicit_method
        .call(scope, default_name_object.into(), &[])
        .expect("call explicitMethod");
    assert_eq!(value.number_value(scope), Some(7.0));
    assert!(
        !default_name_object
            .has_own_property(
                scope,
                v8::String::new(scope, "symbolIterator").unwrap().into()
            )
            .unwrap_or(false)
    );
    assert!(
        !default_name_object
            .has_own_property(
                scope,
                v8::String::new(scope, "symbolAsyncIterator")
                    .unwrap()
                    .into()
            )
            .unwrap_or(false)
    );
    let iterator_key = v8::Symbol::get_iterator(scope);
    let symbol_iterator = default_name_object
        .get(scope, iterator_key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("Symbol.iterator method");
    let value = symbol_iterator
        .call(scope, default_name_object.into(), &[])
        .expect("call Symbol.iterator method");
    assert_eq!(value.number_value(scope), Some(7.0));
    let symbol_iterator_descriptor = default_name_object
        .get_own_property_descriptor(scope, iterator_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Symbol.iterator descriptor");
    assert_eq!(
        symbol_iterator_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        symbol_iterator_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        symbol_iterator_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let async_iterator_key = v8::Symbol::get_async_iterator(scope);
    let symbol_async_iterator = default_name_object
        .get(scope, async_iterator_key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("Symbol.asyncIterator method");
    let value = symbol_async_iterator
        .call(scope, default_name_object.into(), &[])
        .expect("call Symbol.asyncIterator method");
    assert_eq!(value.number_value(scope), Some(7.0));
    let symbol_async_iterator_descriptor = default_name_object
        .get_own_property_descriptor(scope, async_iterator_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Symbol.asyncIterator descriptor");
    assert_eq!(
        symbol_async_iterator_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        symbol_async_iterator_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        symbol_async_iterator_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let alias_entries = alias_object
        .get(scope, v8::String::new(scope, "entries").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("entries alias source");
    let alias_iterator_key = v8::Symbol::get_iterator(scope);
    let alias_iterator = alias_object
        .get(scope, alias_iterator_key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("Symbol.iterator alias");
    assert!(alias_iterator.strict_equals(alias_entries.into()));
    let alias_value = alias_iterator
        .call(scope, alias_object.into(), &[])
        .expect("call aliased Symbol.iterator");
    assert_eq!(alias_value.number_value(scope), Some(7.0));
    let alias_iterator_descriptor = alias_object
        .get_own_property_descriptor(scope, alias_iterator_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Symbol.iterator alias descriptor");
    assert_eq!(
        alias_iterator_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        alias_iterator_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        alias_iterator_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let constant_descriptor = constant_object
        .get_own_property_descriptor(scope, v8::String::new(scope, "READY").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("READY object constant descriptor");
    assert_eq!(
        constant_descriptor
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(4)
    );
    assert_eq!(
        constant_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        constant_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        constant_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let existing_time_origin = existing
        .get(scope, v8::String::new(scope, "timeOrigin").unwrap().into())
        .expect("existing timeOrigin");
    assert_eq!(existing_time_origin.number_value(scope), Some(12.0));
    let prototype_only_object = PrototypeOnlyObject {}
        .bind(scope)
        .expect("prototype-only object");
    assert_eq!(
        prototype_only_object
            .get_prototype(scope)
            .map(|value| value.strict_equals(prototype.into())),
        Some(true)
    );
    let runtime_prototype = v8::Object::new(scope);
    let runtime_tagged_object = RuntimePrototypeTaggedObject {
        prototype: runtime_prototype,
        to_string_tag: Some("RuntimeDeclared"),
    }
    .bind(scope)
    .expect("runtime prototype tagged object");
    assert_eq!(
        runtime_tagged_object
            .get_prototype(scope)
            .map(|value| value.strict_equals(runtime_prototype.into())),
        Some(true)
    );
    let runtime_tag_descriptor = runtime_tagged_object
        .get_own_property_descriptor(scope, v8::Symbol::get_to_string_tag(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("runtime tag descriptor");
    assert_eq!(
        runtime_tag_descriptor
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("RuntimeDeclared")
    );
    assert_eq!(
        runtime_tag_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let runtime_untagged_object = RuntimePrototypeTaggedObject {
        prototype: runtime_prototype,
        to_string_tag: None,
    }
    .bind(scope)
    .expect("runtime prototype untagged object");
    assert_eq!(
        runtime_untagged_object
            .has_own_property(scope, v8::Symbol::get_to_string_tag(scope).into()),
        Some(false)
    );
    let runtime_tag_only_object = RuntimeTagOnlyObject {
        to_string_tag: Some("RuntimeTagOnly"),
    }
    .bind(scope)
    .expect("runtime tag-only object");
    let runtime_tag_only = runtime_tag_only_object
        .get_own_property_descriptor(scope, v8::Symbol::get_to_string_tag(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|descriptor| {
            descriptor.get(scope, v8::String::new(scope, "value").unwrap().into())
        })
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(runtime_tag_only.as_deref(), Some("RuntimeTagOnly"));
    let locked_runtime_tag = RuntimeReadonlyLockedTagObject { to_string_tag: () }
        .bind(scope)
        .expect("bind readonly locked runtime tag");
    let locked_runtime_tag_descriptor = locked_runtime_tag
        .get_own_property_descriptor(scope, v8::Symbol::get_to_string_tag(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("locked runtime tag descriptor");
    assert_eq!(
        locked_runtime_tag_descriptor
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("LockedRuntimeTag")
    );
    assert_eq!(
        locked_runtime_tag_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_runtime_tag_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_runtime_tag_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );

    let tag = object
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|prototype| prototype.get(scope, v8::Symbol::get_to_string_tag(scope).into()))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(tag.as_deref(), Some("SamplePerformance"));
    assert_eq!(
        object.has_own_property(scope, v8::Symbol::get_to_string_tag(scope).into()),
        Some(false)
    );

    let readonly_tag_object = ReadonlyTagObject {}
        .bind(scope)
        .expect("readonly tag object");
    let readonly_tag_descriptor = readonly_tag_object
        .get_own_property_descriptor(scope, v8::Symbol::get_to_string_tag(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("readonly tag descriptor");
    assert_eq!(
        readonly_tag_descriptor
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("ReadonlySample")
    );
    assert_eq!(
        readonly_tag_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        readonly_tag_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );

    let prototype_backed_fallback = PrototypeBackedFallbackTagObject {}
        .bind(scope)
        .expect("prototype backed fallback tag object");
    assert_eq!(
        prototype_backed_fallback
            .has_own_property(scope, v8::Symbol::get_to_string_tag(scope).into()),
        Some(false)
    );
    let missing_prototype_fallback = MissingPrototypeFallbackTagObject {}
        .bind(scope)
        .expect("missing prototype fallback tag object");
    let fallback_tag = missing_prototype_fallback
        .get_own_property_descriptor(scope, v8::Symbol::get_to_string_tag(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|descriptor| {
            descriptor.get(scope, v8::String::new(scope, "value").unwrap().into())
        })
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(fallback_tag.as_deref(), Some("MissingSample"));
}

#[test]
fn declared_constructor_callback_uses_configured_length() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);

    let constructor = install_template::<ConstructibleSampleInterface>(scope, global);
    let length = constructor
        .get(scope, v8::String::new(scope, "length").unwrap().into())
        .and_then(|value| value.number_value(scope));
    assert_eq!(length, Some(2.0));
}

#[test]
fn declared_native_data_property_uses_native_holder() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let object = NativeDataPropertyObject::new()
        .bind(scope)
        .expect("bind native data property object");
    let value = object
        .get(
            scope,
            v8::String::new(scope, "nativeHolder").unwrap().into(),
        )
        .expect("read native data property");

    assert!(
        value.strict_equals(object.into()),
        "native data property callback must receive the defining holder"
    );
}

#[test]
fn declared_template_method_descriptor_attributes_are_applied() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);

    let constructor = install_template::<InterfaceDescriptorSample>(scope, global);
    let prototype = constructor
        .get(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("descriptor sample prototype");
    let descriptor = prototype
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, "lockedVisible").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("lockedVisible descriptor");

    assert_eq!(
        descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    let method = descriptor
        .get(scope, v8::String::new(scope, "value").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("lockedVisible method");
    assert_eq!(
        method
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("lockedVisible".to_owned())
    );
}

#[test]
fn indexed_property_handler_flags_are_forwarded_to_v8() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let object_prototype = context
        .global(scope)
        .get(scope, v8::String::new(scope, "Object").unwrap().into())
        .and_then(|value| value.to_object(scope))
        .and_then(|object| {
            object
                .get(scope, v8::String::new(scope, "prototype").unwrap().into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        })
        .expect("Object.prototype");
    object_prototype
        .define_own_property(
            scope,
            v8::String::new(scope, "0").unwrap().into(),
            v8::Integer::new(scope, 5).into(),
            v8::PropertyAttribute::NONE,
        )
        .expect("define Object.prototype[0]");

    let masking_template = v8::ObjectTemplate::new(scope);
    masking_template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_zero_getter)
            .query(indexed_zero_query),
    );
    let masking = masking_template
        .new_instance(scope)
        .expect("masking indexed interceptor instance");
    assert_eq!(
        masking
            .get(scope, v8::Integer::new(scope, 0).into())
            .and_then(|value| value.uint32_value(scope)),
        Some(42)
    );

    let non_masking_template = v8::ObjectTemplate::new(scope);
    non_masking_template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_zero_getter)
            .query(indexed_zero_query)
            .flags(v8::PropertyHandlerFlags::NON_MASKING),
    );
    let non_masking = non_masking_template
        .new_instance(scope)
        .expect("non-masking indexed interceptor instance");
    assert_eq!(
        non_masking
            .get(scope, v8::Integer::new(scope, 0).into())
            .and_then(|value| value.uint32_value(scope)),
        Some(5)
    );
}

#[test]
fn webapi_value_converts_tuple_sequences_to_arrays() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let pairs = vec![
        ("name".to_owned(), "value".to_owned()),
        ("second".to_owned(), "entry".to_owned()),
    ];
    let array = pairs
        .to_v8_value(scope)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("pairs array");

    assert_eq!(array.length(), 2);
    let first = array
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("first pair");
    assert_eq!(first.length(), 2);
    assert_eq!(
        first
            .get_index(scope, 0)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("name")
    );
    assert_eq!(
        first
            .get_index(scope, 1)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("value")
    );

    let triple = ("event", 7_u32, true)
        .to_v8_value(scope)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("triple array");
    assert_eq!(triple.length(), 3);
    assert_eq!(
        triple
            .get_index(scope, 0)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("event")
    );
    assert_eq!(
        triple
            .get_index(scope, 1)
            .and_then(|value| value.uint32_value(scope)),
        Some(7)
    );
    assert_eq!(
        triple
            .get_index(scope, 2)
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );

    let quad = ("x", "y", 1_u32, 2_u32)
        .to_v8_value(scope)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("quad array");
    assert_eq!(quad.length(), 4);
    assert_eq!(
        quad.get_index(scope, 3)
            .and_then(|value| value.uint32_value(scope)),
        Some(2)
    );

    run_script(
        scope,
        r#"
        globalThis.__arraySetterHit = null;
        Object.defineProperty(Array.prototype, "1", {
          configurable: true,
          set(value) { globalThis.__arraySetterHit = this; }
        });
        "#,
    );
    let guarded = vec![("guard".to_owned(), "value".to_owned())]
        .to_v8_value(scope)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("guarded pair array");
    assert_eq!(guarded.length(), 1);
    let hit = run_script(
        scope,
        r#"
        const hit = globalThis.__arraySetterHit;
        delete Array.prototype["1"];
        hit;
        "#,
    );
    assert!(hit.is_null());
}

#[test]
fn define_array_data_property_does_not_trigger_inherited_indexed_setters() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    run_script(
        scope,
        r#"
        globalThis.__arrayDataPropertySetterHit = null;
        Object.defineProperty(Array.prototype, "0", {
          configurable: true,
          set(value) { globalThis.__arrayDataPropertySetterHit = value; }
        });
        "#,
    );

    let array = v8::Array::new(scope, 0);
    let value = v8::String::new(scope, "stored").expect("test string should allocate");
    define_array_data_property(scope, array, 0, value.into())
        .expect("indexed data property should define");

    assert_eq!(array.length(), 1);
    assert_eq!(
        array
            .get_index(scope, 0)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("stored")
    );
    let hit = run_script(
        scope,
        r#"
        const hit = globalThis.__arrayDataPropertySetterHit;
        delete Array.prototype["0"];
        hit;
        "#,
    );
    assert!(hit.is_null());
}

#[test]
fn declared_enumerable_properties_do_not_trigger_inherited_setters() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    run_script(
        scope,
        r#"
        globalThis.__declaredDataPropertySetterHit = null;
        Object.defineProperty(Object.prototype, "clientX", {
          configurable: true,
          set(value) { globalThis.__declaredDataPropertySetterHit = value; }
        });
        "#,
    );

    let object = DefaultEnumerablePropertiesObject {
        client_x: 42.0,
        is_2d: true,
        related_target: Some("target"),
        missing_target: None,
    }
    .bind(scope)
    .expect("default enumerable properties object should bind");

    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "clientX").unwrap().into())
            .and_then(|value| value.number_value(scope)),
        Some(42.0)
    );
    let descriptor = object
        .get_own_property_descriptor(scope, v8::String::new(scope, "clientX").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("clientX descriptor");
    assert_eq!(
        descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );

    let hit = run_script(
        scope,
        r#"
        const hit = globalThis.__declaredDataPropertySetterHit;
        delete Object.prototype.clientX;
        hit;
        "#,
    );
    assert!(hit.is_null());
}

#[test]
fn webapi_value_binds_declared_objects_inside_arrays() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let records = vec![
        NestedValueRecord {
            name: "alpha",
            count: 1,
        },
        NestedValueRecord {
            name: "beta",
            count: 2,
        },
    ];
    let array = records
        .to_v8_value(scope)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("records array");

    assert_eq!(array.length(), 2);
    let first = array
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("first record");
    assert_eq!(
        first
            .get(scope, v8::String::new(scope, "name").unwrap().into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("alpha")
    );
    assert_eq!(
        first
            .get(scope, v8::String::new(scope, "count").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(1)
    );
}

#[test]
fn declared_object_can_require_interface_prototype() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let error = RequiredPrototypeObject {}
        .bind(scope)
        .expect_err("missing required prototype should fail");
    assert_eq!(
        error.to_string(),
        "missing `MissingSampleInterface` prototype"
    );
    let error = MissingPrototypeOnlyObject {}
        .bind(scope)
        .expect_err("missing required explicit prototype should fail");
    assert_eq!(
        error.to_string(),
        "missing `MissingSampleInterface` prototype"
    );
}

#[test]
fn declared_enumerable_properties_bypass_prototype_assignment_semantics() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);

    install_template::<SamplePerformanceInterface>(scope, global);
    let constructor = global
        .get(
            scope,
            v8::String::new(scope, "SamplePerformance").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("constructor");
    let prototype = constructor
        .get(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("prototype");
    let client_x_name = v8::String::new(scope, "clientX").unwrap();
    let prototype_value = v8::String::new(scope, "prototype-blocked").unwrap();
    prototype
        .define_own_property(
            scope,
            client_x_name.into(),
            prototype_value.into(),
            v8::PropertyAttribute::READ_ONLY,
        )
        .expect("readonly prototype property should define");

    let object = PrototypeBackedEnumerablePropertiesObject { client_x: 42.0 }
        .bind(scope)
        .expect("bind prototype-backed enumerable object");

    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "clientX").unwrap().into())
            .and_then(|value| value.number_value(scope)),
        Some(42.0)
    );
    assert_eq!(
        object.has_own_property(scope, v8::String::new(scope, "clientX").unwrap().into()),
        Some(true)
    );
    assert_eq!(
        object
            .get_own_property_descriptor(scope, v8::String::new(scope, "clientX").unwrap().into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|descriptor| {
                descriptor.get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            })
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
}

#[test]
fn symbol_method_can_declare_explicit_function_name() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let object = ExplicitFunctionNameSymbolMethodObject { iterator: () }
        .bind(scope)
        .expect("bind explicit function name method object");
    let iterator_key = v8::Symbol::get_iterator(scope);
    let iterator = object
        .get(scope, iterator_key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("Symbol.iterator method");
    assert_eq!(
        iterator
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("values".to_owned())
    );
    assert_eq!(
        iterator
            .get(scope, v8::String::new(scope, "length").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(0)
    );
    assert_eq!(
        object.has_own_property(scope, v8::String::new(scope, "values").unwrap().into()),
        Some(false)
    );
    let descriptor = object
        .get_own_property_descriptor(scope, iterator_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Symbol.iterator descriptor");
    let descriptor_value = descriptor
        .get(scope, v8::String::new(scope, "value").unwrap().into())
        .expect("descriptor value");
    assert!(descriptor_value.strict_equals(iterator.into()));
    assert_eq!(
        descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
}

#[test]
fn function_template_declaration_installs_webidl_method_descriptors() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let template = SampleFunctionTemplateDeclaration::build(scope);
    let constructor = template
        .get_function(scope)
        .expect("SampleTemplate function");
    assert_eq!(
        constructor
            .get(scope, v8::String::new(scope, "length").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(1)
    );
    let from_descriptor = constructor
        .get_own_property_descriptor(scope, v8::String::new(scope, "from").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("from static method descriptor");
    assert_eq!(
        from_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        from_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        from_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let from = from_descriptor
        .get(scope, v8::String::new(scope, "value").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("from static method function");
    assert_eq!(
        from.has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into()),
        Some(false),
        "static Web IDL operations must be concise non-constructor functions"
    );
    assert_eq!(
        from.get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("from".to_owned())
    );
    assert_eq!(
        from.get(scope, v8::String::new(scope, "length").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(1)
    );
    let from_result = from
        .call(scope, constructor.into(), &[])
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(from_result.as_deref(), Some("template-static-data"));

    let locked_static_descriptor = constructor
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, "lockedStatic").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("lockedStatic descriptor");
    assert_eq!(
        locked_static_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        locked_static_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_static_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );

    let prototype = constructor
        .get(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("SampleTemplate.prototype");

    for owner in [constructor.into(), prototype] {
        let ready_descriptor = owner
            .get_own_property_descriptor(scope, v8::String::new(scope, "READY").unwrap().into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("READY constant descriptor");
        assert_eq!(
            ready_descriptor
                .get(scope, v8::String::new(scope, "value").unwrap().into())
                .and_then(|value| value.uint32_value(scope)),
            Some(4)
        );
        assert_eq!(
            ready_descriptor
                .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
                .map(|value| value.boolean_value(scope)),
            Some(true)
        );
        assert_eq!(
            ready_descriptor
                .get(scope, v8::String::new(scope, "writable").unwrap().into())
                .map(|value| value.boolean_value(scope)),
            Some(false)
        );
        assert_eq!(
            ready_descriptor
                .get(
                    scope,
                    v8::String::new(scope, "configurable").unwrap().into()
                )
                .map(|value| value.boolean_value(scope)),
            Some(false)
        );
    }

    let append_descriptor = prototype
        .get_own_property_descriptor(scope, v8::String::new(scope, "append").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("append descriptor");
    assert_eq!(
        append_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        append_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        append_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let append = append_descriptor
        .get(scope, v8::String::new(scope, "value").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("append function");
    assert_eq!(
        append.has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into(),),
        Some(false),
        "prototype Web IDL operations must be concise non-constructor functions"
    );
    assert_eq!(
        append
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("append".to_owned())
    );
    assert_eq!(
        append
            .get(scope, v8::String::new(scope, "length").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(2)
    );
    let append_result = append
        .call(scope, prototype.into(), &[])
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(append_result.as_deref(), Some("template-method-data"));

    let locked_descriptor = prototype
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, "lockedMethod").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("lockedMethod descriptor");
    assert_eq!(
        locked_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        locked_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        locked_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );

    let entries = prototype
        .get(scope, v8::String::new(scope, "entries").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("entries function");
    let iterator_key = v8::Symbol::get_iterator(scope);
    let iterator_descriptor = prototype
        .get_own_property_descriptor(scope, iterator_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Symbol.iterator descriptor");
    assert_eq!(
        iterator_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        iterator_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        iterator_descriptor
            .get(
                scope,
                v8::String::new(scope, "configurable").unwrap().into()
            )
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let iterator = iterator_descriptor
        .get(scope, v8::String::new(scope, "value").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("Symbol.iterator function");
    assert!(iterator.strict_equals(entries.into()));
    assert_eq!(
        iterator
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("entries".to_owned())
    );

    let instance = constructor
        .new_instance(scope, &[])
        .expect("SampleTemplate instance");
    let receiver_key = v8::String::new(scope, "receiver").unwrap();
    let receiver = instance
        .get(scope, receiver_key.into())
        .expect("receiver accessor value");
    assert!(
        receiver.strict_equals(instance.into()),
        "function accessor must observe the actual receiver"
    );
    assert!(
        !instance
            .has_own_property(scope, receiver_key.into())
            .unwrap_or(false),
        "prototype accessor must not create an instance property"
    );
    let receiver_descriptor = prototype
        .get_own_property_descriptor(scope, receiver_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("receiver descriptor");
    let receiver_getter = receiver_descriptor
        .get(scope, v8::String::new(scope, "get").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("receiver getter");
    assert_eq!(
        receiver_getter
            .has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into(),),
        Some(false),
        "template accessor getters must not expose a prototype"
    );
    assert_eq!(
        receiver_getter
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("get receiver".to_owned())
    );
    assert_eq!(
        receiver_getter
            .get(scope, v8::String::new(scope, "length").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(0)
    );

    let data_key = v8::String::new(scope, "dataValue").unwrap();
    let getter_data = instance
        .get(scope, data_key.into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(getter_data.as_deref(), Some("template-getter-data"));
    let written = v8::String::new(scope, "template-written").unwrap();
    assert_eq!(
        instance.set(scope, data_key.into(), written.into()),
        Some(true)
    );
    let setter_payload = instance
        .get(
            scope,
            v8::String::new(scope, "setterPayload").unwrap().into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    assert_eq!(setter_payload.as_deref(), Some("template-setter-data"));
    let data_descriptor = prototype
        .get_own_property_descriptor(scope, data_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("dataValue descriptor");
    let data_setter = data_descriptor
        .get(scope, v8::String::new(scope, "set").unwrap().into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .expect("dataValue setter");
    assert_eq!(
        data_setter.has_own_property(scope, v8::String::new(scope, "prototype").unwrap().into(),),
        Some(false),
        "template accessor setters with callback data must not expose a prototype"
    );
    assert_eq!(
        data_setter
            .get_name(scope)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope)),
        Some("set dataValue".to_owned())
    );
    assert_eq!(
        data_setter
            .get(scope, v8::String::new(scope, "length").unwrap().into())
            .and_then(|value| value.uint32_value(scope)),
        Some(1)
    );

    let native_holder = instance
        .get(
            scope,
            v8::String::new(scope, "nativeHolder").unwrap().into(),
        )
        .expect("native holder accessor value");
    assert!(
        native_holder.strict_equals(prototype.into()),
        "native accessor must retain explicit holder semantics"
    );
    let native_holder_descriptor = prototype
        .get_own_property_descriptor(
            scope,
            v8::String::new(scope, "nativeHolder").unwrap().into(),
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("nativeHolder descriptor");
    assert_eq!(
        native_holder_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true),
        "template-level enumerable must apply to native accessors"
    );
}

#[test]
fn function_template_intrinsic_properties_do_not_read_public_array() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    run_script(
        scope,
        r#"
        globalThis.__originalArrayEntries = Array.prototype.entries;
        globalThis.__originalArrayValues = Array.prototype.values;
        globalThis.Array = undefined;
        "#,
    );
    let global = context.global(scope);
    let original_entries = global
        .get(
            scope,
            v8::String::new(scope, "__originalArrayEntries")
                .unwrap()
                .into(),
        )
        .expect("saved Array.prototype.entries");
    let original_values = global
        .get(
            scope,
            v8::String::new(scope, "__originalArrayValues")
                .unwrap()
                .into(),
        )
        .expect("saved Array.prototype.values");

    let template = IntrinsicArrayTemplateDeclaration::build(scope);
    let constructor = template
        .get_function(scope)
        .expect("IntrinsicArrayTemplate function");
    let prototype = constructor
        .get(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("IntrinsicArrayTemplate.prototype");

    let entries = prototype
        .get(scope, v8::String::new(scope, "entries").unwrap().into())
        .expect("entries intrinsic");
    let values = prototype
        .get(scope, v8::String::new(scope, "values").unwrap().into())
        .expect("values intrinsic");
    let iterator_key = v8::Symbol::get_iterator(scope);
    let iterator = prototype
        .get(scope, iterator_key.into())
        .expect("Symbol.iterator intrinsic");
    assert!(entries.strict_equals(original_entries));
    assert!(values.strict_equals(original_values));
    assert!(iterator.strict_equals(original_values));

    let values_descriptor = prototype
        .get_own_property_descriptor(scope, v8::String::new(scope, "values").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("values descriptor");
    assert_eq!(
        values_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let iterator_descriptor = prototype
        .get_own_property_descriptor(scope, iterator_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Symbol.iterator descriptor");
    assert_eq!(
        iterator_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
}

#[test]
fn function_template_intrinsic_parent_does_not_read_public_error() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    run_script(
        scope,
        r#"
        globalThis.__originalErrorPrototype = Error.prototype;
        globalThis.Error = undefined;
        "#,
    );
    let global = context.global(scope);
    let original_error_prototype = global
        .get(
            scope,
            v8::String::new(scope, "__originalErrorPrototype")
                .unwrap()
                .into(),
        )
        .expect("saved Error.prototype");

    let template = v8::FunctionTemplate::builder(sample_constructor_callback).build(scope);
    IntrinsicErrorParentTemplateDeclaration::initialize_template(scope, template);
    let constructor = template
        .get_function(scope)
        .expect("IntrinsicErrorParentTemplate function");
    let prototype = constructor
        .get(scope, v8::String::new(scope, "prototype").unwrap().into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("IntrinsicErrorParentTemplate.prototype");
    assert!(
        prototype
            .get_prototype(scope)
            .is_some_and(|parent| parent.strict_equals(original_error_prototype))
    );
}

#[test]
fn function_template_declares_complete_intrinsic_iterator_shape() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    run_script(
        scope,
        r#"
        globalThis.__originalIteratorPrototype =
          Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
        Array.prototype[Symbol.iterator] = function poisonedIterator() {
          throw new Error("public iterator observed");
        };
        Object.getPrototypeOf = function poisonedGetPrototypeOf() {
          throw new Error("public Object.getPrototypeOf observed");
        };
        "#,
    );
    let global = context.global(scope);
    let original_iterator_prototype = global
        .get(
            scope,
            v8::String::new(scope, "__originalIteratorPrototype")
                .unwrap()
                .into(),
        )
        .expect("saved IteratorPrototype");

    let template = IntrinsicIteratorTemplateDeclaration::build(scope);
    let constructor = template
        .get_function(scope)
        .expect("Example Iterator function");
    let prototype_key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, prototype_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Example Iterator prototype");
    assert!(
        prototype
            .get_prototype(scope)
            .is_some_and(|parent| parent.strict_equals(original_iterator_prototype))
    );

    let constructor_prototype_descriptor = constructor
        .get_own_property_descriptor(scope, prototype_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("constructor prototype descriptor");
    assert_eq!(
        constructor_prototype_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );

    let tag = v8::Symbol::get_to_string_tag(scope);
    let tag_descriptor = prototype
        .get_own_property_descriptor(scope, tag.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("iterator toStringTag descriptor");
    assert_eq!(
        tag_descriptor
            .get(scope, v8::String::new(scope, "value").unwrap().into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("Example Iterator")
    );
    assert_eq!(
        tag_descriptor
            .get(scope, v8::String::new(scope, "writable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
    assert_eq!(
        tag_descriptor
            .get(scope, v8::String::new(scope, "enumerable").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(false)
    );
}

#[test]
fn object_literal_declaration_sets_runtime_keys_and_copies_snapshot_values() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    run_script(
        scope,
        r#"
        globalThis.__objectLiteralSetterHit = null;
        Object.defineProperty(Object.prototype, "runtimeKey", {
          configurable: true,
          set(value) { globalThis.__objectLiteralSetterHit = value; }
        });
        "#,
    );

    let source = v8::Object::new(scope);
    let _ = source.set(
        scope,
        v8::String::new(scope, "copied").unwrap().into(),
        v8::Number::new(scope, 42.0).into(),
    );

    let dynamic_key = v8::String::new(scope, "runtimeKey").unwrap();
    let literal = ObjectLiteralDeclaration::bind(scope);
    let dynamic_value = v8::Boolean::new(scope, true).into();
    literal.set_value_property(scope, dynamic_key.into(), dynamic_value);
    let string_value = v8::String::new(scope, "value").unwrap().into();
    literal.set_string_property(scope, "stringKey", string_value);
    let proto_key = v8::String::new(scope, "__proto__").unwrap();
    let attempted_prototype = v8::Object::new(scope);
    literal.set_value_property(scope, proto_key.into(), attempted_prototype.into());
    literal.copy_string_property(scope, source, "copied");
    literal.copy_string_property(scope, source, "missing");

    let object_view = literal.as_object();
    assert_eq!(
        object_view.has_own_property(scope, proto_key.into()),
        Some(true)
    );
    assert_eq!(
        object_view
            .get_prototype(scope)
            .map(|prototype| prototype.strict_equals(attempted_prototype.into())),
        Some(false)
    );
    let object = literal.into_object();
    assert!(object.strict_equals(object_view.into()));
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "runtimeKey").unwrap().into())
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "stringKey").unwrap().into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref(),
        Some("value")
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "copied").unwrap().into())
            .and_then(|value| value.number_value(scope)),
        Some(42.0)
    );
    assert_eq!(
        object.has_own_property(scope, v8::String::new(scope, "missing").unwrap().into()),
        Some(true)
    );
    assert_eq!(
        object
            .get(scope, v8::String::new(scope, "missing").unwrap().into())
            .map(|value| value.is_undefined()),
        Some(true)
    );
    assert_eq!(
        object
            .get_own_property_descriptor(scope, v8::String::new(scope, "copied").unwrap().into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|descriptor| descriptor
                .get(scope, v8::String::new(scope, "enumerable").unwrap().into()))
            .map(|value| value.boolean_value(scope)),
        Some(true)
    );
    let hit = run_script(
        scope,
        r#"
        const hit = globalThis.__objectLiteralSetterHit;
        delete Object.prototype.runtimeKey;
        hit;
        "#,
    );
    assert!(hit.is_null());
}

fn get_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let name = v8::String::new(scope, slot)?;
    let key = v8::Private::for_api(scope, Some(name));
    let value = object.get_private(scope, key)?;
    if value.is_undefined() {
        None
    } else {
        Some(value)
    }
}

mod interfaces {
    moli_webapi_declare::declare_web_api_interfaces! {
        pub(super) ConstructibleSample;
        pub(super) MissingSampleInterface;
        pub(super) SamplePerformance;
        pub(super) SampleTemplate;
    }
}

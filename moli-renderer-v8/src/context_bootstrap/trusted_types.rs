use super::*;
use crate::content_security_policy::TrustedTypesForScriptRequirements;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::{WebApiInterface, WebApiObject};

mod policy_callbacks;
mod realm_state;

use policy_callbacks::{
    TrustedTypePolicyCallbackOutcome, invoke_policy_callback, parse_policy_callback_carriers,
};

#[cfg(test)]
pub(crate) use realm_state::trusted_types_lazy_state_materialized;

const TRUSTED_TYPE_KIND_SLOT: &str = "__moliTrustedTypeKind";
const TRUSTED_TYPE_VALUE_SLOT: &str = "__moliTrustedTypeValue";
const TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT: &str = "__moliTrustedScriptCodeLikeConstructor";
const TRUSTED_TYPE_HTML_PROTOTYPE_SLOT: &str = "__moliTrustedHTMLPrototype";
const TRUSTED_TYPE_SCRIPT_PROTOTYPE_SLOT: &str = "__moliTrustedScriptPrototype";
const TRUSTED_TYPE_SCRIPT_URL_PROTOTYPE_SLOT: &str = "__moliTrustedScriptURLPrototype";
const TRUSTED_TYPE_HTML_CONSTRUCTOR_SLOT: &str = "__moliTrustedHTMLConstructor";
const TRUSTED_TYPE_SCRIPT_CONSTRUCTOR_SLOT: &str = "__moliTrustedScriptConstructor";
const TRUSTED_TYPE_SCRIPT_URL_CONSTRUCTOR_SLOT: &str = "__moliTrustedScriptURLConstructor";
const TRUSTED_TYPE_POLICY_CONSTRUCTOR_SLOT: &str = "__moliTrustedTypePolicyConstructor";
const TRUSTED_TYPE_POLICY_FACTORY_CONSTRUCTOR_SLOT: &str =
    "__moliTrustedTypePolicyFactoryConstructor";
const TRUSTED_TYPES_DEFAULT_POLICY_SLOT: &str = "__moliTrustedTypesDefaultPolicy";
const TRUSTED_TYPES_CREATE_HTML_SLOT: &str = "__moliTrustedTypesCreateHTML";
const TRUSTED_TYPES_CREATE_SCRIPT_SLOT: &str = "__moliTrustedTypesCreateScript";
const TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT: &str = "__moliTrustedTypesCreateScriptURL";
const TRUSTED_TYPES_POLICY_NAME_SLOT: &str = "__moliTrustedTypesPolicyName";
const TRUSTED_TYPES_CREATED_POLICY_NAMES_SLOT: &str = "__moliTrustedTypesCreatedPolicyNames";
const TRUSTED_TYPES_EMPTY_HTML_SLOT: &str = "__moliTrustedTypesEmptyHTML";
const TRUSTED_TYPES_EMPTY_SCRIPT_SLOT: &str = "__moliTrustedTypesEmptyScript";
const TRUSTED_TYPES_EMPTY_VALUE_SLOTS: [&str; 2] = [
    TRUSTED_TYPES_EMPTY_HTML_SLOT,
    TRUSTED_TYPES_EMPTY_SCRIPT_SLOT,
];

#[derive(WebApiInterface)]
#[webapi(
    name = "TrustedTypePolicyFactory",
    constructor = "illegal",
    constructor_length = 0
)]
struct TrustedTypePolicyFactoryInterfaceDeclaration {
    #[webapi(
        method,
        callback = trusted_types_create_policy_callback,
        length = 2,
        enumerable
    )]
    create_policy: (),
    #[webapi(
        method = "isHTML",
        callback = trusted_types_is_html_callback,
        length = 1,
        enumerable
    )]
    is_html: (),
    #[webapi(
        method,
        callback = trusted_types_is_script_callback,
        length = 1,
        enumerable
    )]
    is_script: (),
    #[webapi(
        method = "isScriptURL",
        callback = trusted_types_is_script_url_callback,
        length = 1,
        enumerable
    )]
    is_script_url: (),
    #[webapi(
        accessor_property = "emptyHTML",
        getter = trusted_types_empty_value_getter_callback,
        data = crate::util::callback_data_index_value(scope, 0),
        enumerable
    )]
    empty_html: (),
    #[webapi(
        accessor_property = "emptyScript",
        getter = trusted_types_empty_value_getter_callback,
        data = crate::util::callback_data_index_value(scope, 1),
        enumerable
    )]
    empty_script: (),
    #[webapi(
        method = "getAttributeType",
        callback = trusted_types_get_attribute_type_callback,
        length = 2,
        enumerable
    )]
    get_attribute_type: (),
    #[webapi(
        method = "getPropertyType",
        callback = trusted_types_get_property_type_callback,
        length = 2,
        enumerable
    )]
    get_property_type: (),
    #[webapi(
        accessor_property = "defaultPolicy",
        getter = trusted_types_default_policy_getter_callback,
        enumerable
    )]
    default_policy: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "TrustedTypePolicyFactory", require_prototype)]
struct TrustedTypesFactoryObjectDeclaration<'scope> {
    #[webapi(slot = TRUSTED_TYPES_EMPTY_HTML_SLOT)]
    empty_html: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TRUSTED_TYPES_EMPTY_SCRIPT_SLOT)]
    empty_script: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TRUSTED_TYPES_CREATED_POLICY_NAMES_SLOT)]
    created_policy_names: v8::Local<'scope, v8::Array>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TrustedTypePolicyFactory.getAttributeType")]
struct TrustedTypesGetAttributeTypeArgs {
    #[webidl(required)]
    tag_name: String,
    #[webidl(required)]
    attribute: String,
    #[webidl(nullable)]
    element_namespace: Option<String>,
    #[webidl(nullable)]
    attribute_namespace: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TrustedTypePolicyFactory.getPropertyType")]
struct TrustedTypesGetPropertyTypeArgs {
    #[webidl(required)]
    tag_name: String,
    #[webidl(required)]
    property: String,
    #[webidl(nullable)]
    element_namespace: Option<String>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct TrustedTypeObjectDeclaration<'scope> {
    #[webapi(slot = TRUSTED_TYPE_KIND_SLOT)]
    kind: v8::Local<'scope, v8::String>,
    #[webapi(slot = TRUSTED_TYPE_VALUE_SLOT)]
    value: v8::Local<'scope, v8::String>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct TrustedTypePrototypeDeclaration {
    #[webapi(method = "toString", callback = trusted_type_to_string_callback, length = 0)]
    to_string: (),
    #[webapi(method = "toJSON", callback = trusted_type_to_string_callback, length = 0)]
    to_json: (),
    #[webapi(method, callback = trusted_type_to_string_callback, length = 0)]
    value_of: (),
}

#[derive(WebApiInterface)]
#[webapi(
    name = "TrustedTypePolicy",
    constructor = "illegal",
    constructor_length = 0
)]
struct TrustedTypePolicyInterfaceDeclaration {
    #[webapi(
        accessor_property,
        getter = trusted_type_policy_name_getter_callback,
        enumerable
    )]
    name: (),
    #[webapi(
        method = "createHTML",
        callback = trusted_type_policy_create_callback,
        data = crate::util::callback_data_index_value(scope, 0),
        length = 1,
        enumerable
    )]
    create_html: (),
    #[webapi(
        method,
        callback = trusted_type_policy_create_callback,
        data = crate::util::callback_data_index_value(scope, 1),
        length = 1,
        enumerable
    )]
    create_script: (),
    #[webapi(
        method = "createScriptURL",
        callback = trusted_type_policy_create_callback,
        data = crate::util::callback_data_index_value(scope, 2),
        length = 1,
        enumerable
    )]
    create_script_url: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "TrustedTypePolicy", require_prototype)]
struct TrustedTypePolicyObjectDeclaration<'scope> {
    #[webapi(slot = TRUSTED_TYPES_POLICY_NAME_SLOT)]
    name: v8::Local<'scope, v8::String>,
    #[webapi(slot = TRUSTED_TYPES_CREATE_HTML_SLOT)]
    create_html_callback: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = TRUSTED_TYPES_CREATE_SCRIPT_SLOT)]
    create_script_callback: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT)]
    create_script_url_callback: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrustedTypeKind {
    Html,
    Script,
    ScriptUrl,
}

impl TrustedTypeKind {
    fn constructor_name(self) -> &'static str {
        match self {
            Self::Html => "TrustedHTML",
            Self::Script => "TrustedScript",
            Self::ScriptUrl => "TrustedScriptURL",
        }
    }

    fn create_method_name(self) -> &'static str {
        match self {
            Self::Html => "createHTML",
            Self::Script => "createScript",
            Self::ScriptUrl => "createScriptURL",
        }
    }

    fn callback_slot(self) -> &'static str {
        match self {
            Self::Html => TRUSTED_TYPES_CREATE_HTML_SLOT,
            Self::Script => TRUSTED_TYPES_CREATE_SCRIPT_SLOT,
            Self::ScriptUrl => TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT,
        }
    }

    fn prototype_slot(self) -> &'static str {
        match self {
            Self::Html => TRUSTED_TYPE_HTML_PROTOTYPE_SLOT,
            Self::Script => TRUSTED_TYPE_SCRIPT_PROTOTYPE_SLOT,
            Self::ScriptUrl => TRUSTED_TYPE_SCRIPT_URL_PROTOTYPE_SLOT,
        }
    }

    fn constructor_slot(self) -> &'static str {
        match self {
            Self::Html => TRUSTED_TYPE_HTML_CONSTRUCTOR_SLOT,
            Self::Script => TRUSTED_TYPE_SCRIPT_CONSTRUCTOR_SLOT,
            Self::ScriptUrl => TRUSTED_TYPE_SCRIPT_URL_CONSTRUCTOR_SLOT,
        }
    }

    fn as_slot_value(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Script => "script",
            Self::ScriptUrl => "script-url",
        }
    }

    fn from_callback_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Html),
            1 => Some(Self::Script),
            2 => Some(Self::ScriptUrl),
            _ => None,
        }
    }
}

const TRUSTED_TYPE_KINDS: [TrustedTypeKind; 3] = [
    TrustedTypeKind::Html,
    TrustedTypeKind::Script,
    TrustedTypeKind::ScriptUrl,
];

pub(crate) fn install_trusted_types_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    realm_state::install_lazy_trusted_types_runtime_state(scope, global)
}

pub(crate) fn trusted_script_url_string_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    requirements: TrustedTypesForScriptRequirements,
    sink: &str,
    api_name: &'static str,
) -> Option<String> {
    trusted_type_string_or_throw(
        scope,
        value,
        TrustedTypeKind::ScriptUrl,
        requirements,
        sink,
        api_name,
        TrustedTypeErrorKind::Type,
    )
}

pub(crate) fn trusted_html_string_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    requirements: TrustedTypesForScriptRequirements,
    sink: &str,
    api_name: &'static str,
) -> Option<String> {
    trusted_type_string_or_throw(
        scope,
        value,
        TrustedTypeKind::Html,
        requirements,
        sink,
        api_name,
        TrustedTypeErrorKind::Type,
    )
}

pub(crate) fn trusted_html_value_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    trusted_type_string(scope, value, TrustedTypeKind::Html)
}

pub(crate) fn trusted_script_string_or_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    requirements: TrustedTypesForScriptRequirements,
    sink: &str,
    api_name: &'static str,
) -> Option<String> {
    trusted_type_string_or_throw(
        scope,
        value,
        TrustedTypeKind::Script,
        requirements,
        sink,
        api_name,
        TrustedTypeErrorKind::Type,
    )
}

pub(crate) fn trusted_script_string_for_script_element_execution(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
    sink: &str,
) -> Option<String> {
    let default_value = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        apply_default_trusted_type_policy(
            &mut scope,
            original,
            TrustedTypeKind::Script,
            sink,
            TrustedTypeErrorKind::ScriptExecution,
        )
    };
    if let Some(default_value) = default_value {
        return Some(default_value);
    }
    dispatch_trusted_types_sink_violation_event_without_stack(scope, sink, original);
    None
}

/// Result of applying the Trusted Types pre-navigation conversion to a
/// decoded `javascript:` source.
///
/// Owner-specific CSP event dispatch deliberately remains outside this
/// module. This layer owns only Trusted Type values and default-policy calls.
pub(crate) struct JavascriptUrlTrustedTypesCheck {
    pub(crate) source: Option<String>,
    pub(crate) violated: bool,
}

/// Apply the Trusted Types pre-navigation conversion for a decoded
/// `javascript:` source. Policy exceptions are consumed because this runs in
/// the later navigation task, not in the API call that queued the navigation.
pub(crate) fn check_javascript_url_trusted_types(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
    requirements: TrustedTypesForScriptRequirements,
) -> JavascriptUrlTrustedTypesCheck {
    if !requirements.requires_conversion() {
        return JavascriptUrlTrustedTypesCheck {
            source: Some(original.to_owned()),
            violated: false,
        };
    }
    let outcome = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        apply_default_trusted_type_policy_outcome(
            &mut scope,
            original,
            TrustedTypeKind::Script,
            "Location href",
        )
    };
    match outcome {
        DefaultTrustedTypePolicyOutcome::Value(value)
            if url::Url::parse(&format!("javascript:{value}")).is_ok() =>
        {
            JavascriptUrlTrustedTypesCheck {
                source: Some(value),
                violated: false,
            }
        }
        DefaultTrustedTypePolicyOutcome::Value(_)
        | DefaultTrustedTypePolicyOutcome::Unavailable
        | DefaultTrustedTypePolicyOutcome::Rejected
        | DefaultTrustedTypePolicyOutcome::Exception => JavascriptUrlTrustedTypesCheck {
            source: (!requirements.is_enforced()).then(|| original.to_owned()),
            violated: true,
        },
    }
}

pub(crate) enum TrustedTypesCodeGenerationCheck {
    AllowOriginal,
    AllowModified(String),
    Block,
}

pub(crate) fn trusted_types_code_generation_check<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Value>,
    is_code_like: bool,
    requirements: TrustedTypesForScriptRequirements,
) -> TrustedTypesCodeGenerationCheck {
    if let Some(value) = trusted_type_string(scope, source, TrustedTypeKind::Script) {
        return TrustedTypesCodeGenerationCheck::AllowModified(value);
    }
    if is_code_like {
        return js_value_to_string(scope, source)
            .map(TrustedTypesCodeGenerationCheck::AllowModified)
            .unwrap_or(TrustedTypesCodeGenerationCheck::Block);
    }
    if !source.is_string() {
        return TrustedTypesCodeGenerationCheck::AllowOriginal;
    }
    let Some(original) = js_value_to_string(scope, source) else {
        return TrustedTypesCodeGenerationCheck::Block;
    };
    if !requirements.requires_conversion() {
        return TrustedTypesCodeGenerationCheck::AllowModified(original);
    }
    trusted_script_string_for_code_generation(scope, &original, requirements)
        .map(TrustedTypesCodeGenerationCheck::AllowModified)
        .unwrap_or(TrustedTypesCodeGenerationCheck::Block)
}

fn trusted_script_string_for_code_generation(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
    requirements: TrustedTypesForScriptRequirements,
) -> Option<String> {
    let Some(function_source) = function_constructor_code_generation_source(original) else {
        return trusted_script_string_for_eval_source(scope, original, requirements);
    };
    trusted_script_string_for_code_generation_sink(
        scope,
        original,
        function_source.default_policy_input,
        "Function",
        function_source.violation_sample,
        requirements,
        "Trusted Types default policy must not transform strings passed to Function.",
    )
}

struct FunctionConstructorCodeGenerationSource<'a> {
    default_policy_input: &'a str,
    violation_sample: &'a str,
}

fn function_constructor_code_generation_source(
    source: &str,
) -> Option<FunctionConstructorCodeGenerationSource<'_>> {
    let default_policy_input = source
        .strip_prefix('(')
        .and_then(|source| source.strip_suffix(')'))?;
    let generated_prefix = [
        "(function anonymous",
        "(async function anonymous",
        "(function* anonymous",
        "(async function* anonymous",
    ]
    .into_iter()
    .find(|prefix| source.starts_with(prefix))?;
    Some(FunctionConstructorCodeGenerationSource {
        default_policy_input,
        violation_sample: &source[generated_prefix.len()..],
    })
}

fn trusted_script_string_for_eval_source(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
    requirements: TrustedTypesForScriptRequirements,
) -> Option<String> {
    trusted_script_string_for_code_generation_sink(
        scope,
        original,
        original,
        "eval",
        original,
        requirements,
        "Trusted Types default policy must not transform strings passed to eval.",
    )
}

fn trusted_script_string_for_code_generation_sink(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
    default_policy_input: &str,
    sink: &'static str,
    violation_sample: &str,
    requirements: TrustedTypesForScriptRequirements,
    transformed_value_error: &'static str,
) -> Option<String> {
    let outcome = apply_default_trusted_type_policy_outcome(
        scope,
        default_policy_input,
        TrustedTypeKind::Script,
        sink,
    );
    match outcome {
        DefaultTrustedTypePolicyOutcome::Value(value) if value == default_policy_input => {
            Some(original.to_owned())
        }
        DefaultTrustedTypePolicyOutcome::Value(_) => {
            dispatch_trusted_types_sink_violation_event(scope, sink, violation_sample);
            throw_eval_error(scope, transformed_value_error);
            None
        }
        DefaultTrustedTypePolicyOutcome::Exception => None,
        outcome @ (DefaultTrustedTypePolicyOutcome::Unavailable
        | DefaultTrustedTypePolicyOutcome::Rejected) => {
            dispatch_trusted_types_sink_violation_event(scope, sink, violation_sample);
            if requirements.is_enforced() {
                if matches!(outcome, DefaultTrustedTypePolicyOutcome::Rejected) {
                    throw_trusted_type_policy_result_error(
                        scope,
                        TrustedTypeErrorKind::Eval,
                        TrustedTypeKind::Script,
                    );
                } else {
                    throw_trusted_type_error(
                        scope,
                        TrustedTypeErrorKind::Eval,
                        sink,
                        TrustedTypeKind::Script,
                        sink,
                    );
                }
                None
            } else {
                Some(original.to_owned())
            }
        }
    }
}

#[derive(Clone, Copy)]
enum TrustedTypeErrorKind {
    Type,
    Eval,
    ScriptExecution,
}

enum DefaultTrustedTypePolicyOutcome {
    Unavailable,
    Value(String),
    Rejected,
    Exception,
}

fn trusted_type_string_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: TrustedTypeKind,
    requirements: TrustedTypesForScriptRequirements,
    sink: &str,
    api_name: &'static str,
    error_kind: TrustedTypeErrorKind,
) -> Option<String> {
    if let Some(value) = trusted_type_string(scope, value, kind) {
        return Some(value);
    }
    if !requirements.requires_conversion() {
        return js_value_to_string(scope, value);
    }
    let original = js_value_to_string(scope, value)?;
    let default_policy = apply_default_trusted_type_policy_outcome(scope, &original, kind, sink);
    let default_policy_rejected = match default_policy {
        DefaultTrustedTypePolicyOutcome::Value(value) => return Some(value),
        DefaultTrustedTypePolicyOutcome::Exception => return None,
        DefaultTrustedTypePolicyOutcome::Unavailable => false,
        DefaultTrustedTypePolicyOutcome::Rejected => true,
    };
    dispatch_trusted_types_sink_violation_event(scope, sink, &original);
    if requirements.is_enforced() {
        if default_policy_rejected {
            throw_trusted_type_policy_result_error(scope, error_kind, kind);
        } else {
            throw_trusted_type_error(scope, error_kind, api_name, kind, sink);
        }
        None
    } else {
        Some(original)
    }
}

fn apply_default_trusted_type_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: &str,
    kind: TrustedTypeKind,
    sink: &str,
    error_kind: TrustedTypeErrorKind,
) -> Option<String> {
    match apply_default_trusted_type_policy_outcome(scope, input, kind, sink) {
        DefaultTrustedTypePolicyOutcome::Value(value) => Some(value),
        DefaultTrustedTypePolicyOutcome::Rejected => {
            throw_trusted_type_policy_result_error(scope, error_kind, kind);
            None
        }
        DefaultTrustedTypePolicyOutcome::Unavailable
        | DefaultTrustedTypePolicyOutcome::Exception => None,
    }
}

fn apply_default_trusted_type_policy_outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: &str,
    kind: TrustedTypeKind,
    sink: &str,
) -> DefaultTrustedTypePolicyOutcome {
    let global = scope.get_current_context().global(scope);
    let Some(policy) = get_private_value(scope, global, TRUSTED_TYPES_DEFAULT_POLICY_SLOT)
        .and_then(|policy| v8::Local::<v8::Object>::try_from(policy).ok())
    else {
        return DefaultTrustedTypePolicyOutcome::Unavailable;
    };
    let Some(input) = v8_string(scope, input) else {
        return DefaultTrustedTypePolicyOutcome::Exception;
    };
    let type_name = v8str(scope, kind.constructor_name());
    let Some(sink) = v8_string(scope, sink) else {
        return DefaultTrustedTypePolicyOutcome::Exception;
    };
    let args = [input.into(), type_name.into(), sink.into()];
    match invoke_policy_callback(scope, policy, kind, &args) {
        TrustedTypePolicyCallbackOutcome::Missing => DefaultTrustedTypePolicyOutcome::Unavailable,
        TrustedTypePolicyCallbackOutcome::Returned(Some(value)) => {
            DefaultTrustedTypePolicyOutcome::Value(value)
        }
        TrustedTypePolicyCallbackOutcome::Returned(None) => {
            DefaultTrustedTypePolicyOutcome::Rejected
        }
        TrustedTypePolicyCallbackOutcome::Abrupt => DefaultTrustedTypePolicyOutcome::Exception,
    }
}

fn trusted_type_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: TrustedTypeKind,
) -> Option<String> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let kind_value = get_private_value(scope, object, TRUSTED_TYPE_KIND_SLOT)?;
    let kind_string = kind_value.to_string(scope)?.to_rust_string_lossy(scope);
    if kind_string != kind.as_slot_value() {
        return None;
    }
    let value = get_private_value(scope, object, TRUSTED_TYPE_VALUE_SLOT)?;
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn js_value_to_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

struct TrustedTypeConstructorBinding<'s> {
    kind: TrustedTypeKind,
    constructor: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
}

fn build_trusted_type_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: TrustedTypeKind,
) -> Result<TrustedTypeConstructorBinding<'s>> {
    let name = kind.constructor_name();
    let constructor = v8::Function::builder(trusted_type_illegal_constructor_callback)
        .length(0)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create {name} constructor"))?;
    constructor.set_name(v8str(scope, name));
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("{name}.prototype missing"))?;
    TrustedTypePrototypeDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize {name}.prototype declaration: {error}"))?;
    prototype
        .define_own_property(
            scope,
            v8::Symbol::get_to_string_tag(scope).into(),
            v8str(scope, name).into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to install {name} @@toStringTag"))?;
    Ok(TrustedTypeConstructorBinding {
        kind,
        constructor,
        prototype,
    })
}

fn install_trusted_script_code_like_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let template = v8::FunctionTemplate::new(scope, trusted_script_code_like_constructor_callback);
    template.instance_template(scope).set_code_like();
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to create TrustedScript code-like constructor"))?;
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("TrustedScript code-like constructor prototype missing"))?;
    TrustedTypePrototypeDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| {
            anyhow!("failed to initialize TrustedScript code-like carrier: {error}")
        })?;
    set_private_value(
        scope,
        global,
        TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT,
        constructor.into(),
    );
    Ok(())
}

fn build_trusted_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: TrustedTypeKind,
    value: String,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let prototype = get_private_value(scope, global, kind.prototype_slot())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let value = v8_string(scope, &value)?;
    let declaration = TrustedTypeObjectDeclaration::new(v8str(scope, kind.as_slot_value()), value);
    let object = if kind == TrustedTypeKind::Script {
        get_private_value(scope, global, TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?
            .new_instance(scope, &[])?
    } else {
        v8::Object::new(scope)
    };
    declaration
        .initialize(scope, object)
        .expect("TrustedType object declaration should initialize");
    let _ = object.set_prototype(scope, prototype.into());
    Some(object)
}

fn build_trusted_script_code_like_carrier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: String,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let object = get_private_value(scope, global, TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?
        .new_instance(scope, &[])?;
    let value = v8_string(scope, &value)?;
    TrustedTypeObjectDeclaration::new(v8str(scope, TrustedTypeKind::Script.as_slot_value()), value)
        .initialize(scope, object)
        .expect("TrustedScript code-like carrier declaration should initialize");
    Some(object)
}

fn trusted_type_illegal_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor.");
}

fn trusted_script_code_like_constructor_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

fn trusted_type_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some(value) = get_private_value(scope, this, TRUSTED_TYPE_VALUE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

fn trusted_types_empty_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    let Some(slot) = crate::util::callback_data_item(
        scope,
        &args,
        &TRUSTED_TYPES_EMPTY_VALUE_SLOTS,
        "TrustedTypePolicyFactory empty values",
    ) else {
        return;
    };
    let Some(value) = get_private_value(scope, args.this(), slot) else {
        return;
    };
    rv.set(value);
}

fn trusted_types_default_policy_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    let global = scope.get_current_context().global(scope);
    rv.set(
        get_private_value(scope, global, TRUSTED_TYPES_DEFAULT_POLICY_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn trusted_types_factory_receiver_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if get_private_value(scope, receiver, TRUSTED_TYPES_EMPTY_HTML_SLOT).is_some() {
        return true;
    }
    throw_type_error(scope, "Illegal invocation");
    false
}

fn trusted_types_create_policy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let factory = args.this();
    if !trusted_types_factory_receiver_is_valid(scope, factory) {
        return;
    }
    let Some(name) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("TrustedTypePolicyFactory.createPolicy", 1),
        "Failed to execute 'createPolicy' on 'TrustedTypePolicyFactory': 1 argument required.",
    ) else {
        return;
    };
    let name: String = name.into();
    let Some(callbacks) = parse_policy_callback_carriers(scope, args.get(1)) else {
        return;
    };

    let is_duplicate = trusted_types_policy_name_was_created(scope, factory, &name);
    if !trusted_types_policy_name_allowed(scope, &name, is_duplicate) {
        throw_type_error(
            scope,
            "Failed to execute 'createPolicy' on 'TrustedTypePolicyFactory': Content Security Policy disallows creating a policy with the given name.",
        );
        return;
    }
    if name == "default" && is_duplicate {
        throw_type_error(
            scope,
            "Failed to execute 'createPolicy' on 'TrustedTypePolicyFactory': Policy with name \"default\" already exists.",
        );
        return;
    }

    let policy_name = v8_string(scope, &name).unwrap_or_else(|| v8::String::empty(scope));
    let policy = TrustedTypePolicyObjectDeclaration {
        name: policy_name,
        create_html_callback: callbacks.create_html,
        create_script_callback: callbacks.create_script,
        create_script_url_callback: callbacks.create_script_url,
    }
    .bind(scope)
    .expect("TrustedTypePolicy declaration should bind");

    if name == "default" {
        let global = scope.get_current_context().global(scope);
        set_private_value(
            scope,
            global,
            TRUSTED_TYPES_DEFAULT_POLICY_SLOT,
            policy.into(),
        );
    }
    trusted_types_record_created_policy_name(scope, factory, &name, is_duplicate);
    rv.set(policy.into());
}

fn trusted_types_policy_name_was_created<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    let Some(names) = get_private_value(scope, factory, TRUSTED_TYPES_CREATED_POLICY_NAMES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return false;
    };
    (0..names.length()).any(|index| {
        names
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .is_some_and(|value| value.to_rust_string_lossy(scope) == name)
    })
}

fn trusted_types_record_created_policy_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
    name: &str,
    was_duplicate: bool,
) {
    if was_duplicate {
        return;
    }
    let Some(names) = get_private_value(scope, factory, TRUSTED_TYPES_CREATED_POLICY_NAMES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    let Some(name) = v8_string(scope, name) else {
        return;
    };
    let _ = names.set_index(scope, names.length(), name.into());
}

fn trusted_types_policy_name_allowed(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
    is_duplicate: bool,
) -> bool {
    if let Some(allowed) =
        crate::worker::worker_allows_trusted_type_policy_name_by_csp(scope, name, is_duplicate)
    {
        return allowed;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    unsafe { &mut *host_ptr }.allows_trusted_type_policy_name_by_csp(scope, name, is_duplicate)
}

fn trusted_type_policy_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = get_private_value(scope, args.this(), TRUSTED_TYPES_POLICY_NAME_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(name);
}

fn trusted_type_policy_create_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(kind) = crate::util::callback_data_item(
        scope,
        &args,
        &TRUSTED_TYPE_KINDS,
        "Trusted Type policy methods",
    )
    .or_else(|| {
        args.data()
            .uint32_value(scope)
            .and_then(|index| TrustedTypeKind::from_callback_index(index as usize))
    }) else {
        return;
    };
    let policy = args.this();
    if get_private_value(scope, policy, TRUSTED_TYPES_POLICY_NAME_SLOT).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(input) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument(kind.create_method_name(), 1),
        "TrustedTypePolicy creation methods require an input value.",
    ) else {
        return;
    };
    let Some(input) = v8_string(scope, &String::from(input)) else {
        return;
    };
    let mut callback_arguments = Vec::with_capacity(args.length().max(1) as usize);
    callback_arguments.push(input.into());
    for index in 1..args.length() {
        callback_arguments.push(args.get(index));
    }
    let value = match invoke_policy_callback(scope, policy, kind, &callback_arguments) {
        TrustedTypePolicyCallbackOutcome::Missing => {
            throw_type_error(
                scope,
                &format!(
                    "Policy's TrustedTypePolicyOptions did not specify a '{}' member.",
                    kind.create_method_name()
                ),
            );
            return;
        }
        TrustedTypePolicyCallbackOutcome::Returned(value) => value.unwrap_or_default(),
        TrustedTypePolicyCallbackOutcome::Abrupt => return,
    };
    let Some(object) = build_trusted_type_object(scope, kind, value) else {
        return;
    };
    rv.set(object.into());
}

fn trusted_types_is_html_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    rv.set_bool(trusted_type_string(scope, args.get(0), TrustedTypeKind::Html).is_some());
}

fn trusted_types_is_script_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    rv.set_bool(trusted_type_string(scope, args.get(0), TrustedTypeKind::Script).is_some());
}

fn trusted_types_is_script_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    rv.set_bool(trusted_type_string(scope, args.get(0), TrustedTypeKind::ScriptUrl).is_some());
}

fn trusted_types_get_attribute_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<TrustedTypesGetAttributeTypeArgs>(scope, &args) else {
        return;
    };
    let element_namespace = parsed
        .element_namespace
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or_else(|| crate::native_bridge::document::XHTML_NS.to_owned());
    let attribute_namespace = parsed
        .attribute_namespace
        .filter(|namespace| !namespace.is_empty());
    let tag_name = parsed.tag_name.to_ascii_lowercase();
    let attribute = parsed.attribute.to_ascii_lowercase();
    let Some(type_name) = crate::native_bridge::element::trusted_attribute_type_name_for_names(
        &element_namespace,
        &tag_name,
        attribute_namespace.as_deref(),
        &attribute,
    ) else {
        rv.set_null();
        return;
    };
    let Some(type_name) = v8_string(scope, type_name) else {
        return;
    };
    rv.set(type_name.into());
}

fn trusted_types_get_property_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !trusted_types_factory_receiver_is_valid(scope, args.this()) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<TrustedTypesGetPropertyTypeArgs>(scope, &args) else {
        return;
    };
    let element_namespace = parsed
        .element_namespace
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or_else(|| crate::native_bridge::document::XHTML_NS.to_owned());
    let tag_name = parsed.tag_name.to_ascii_lowercase();
    let Some(type_name) = crate::native_bridge::element::trusted_property_type_name_for_names(
        &element_namespace,
        &tag_name,
        &parsed.property,
    ) else {
        rv.set_null();
        return;
    };
    let Some(type_name) = v8_string(scope, type_name) else {
        return;
    };
    rv.set(type_name.into());
}

#[derive(Clone, Copy)]
enum FunctionConstructorKind {
    Function,
    AsyncFunction,
    GeneratorFunction,
    AsyncGeneratorFunction,
}

impl FunctionConstructorKind {
    fn name(self) -> &'static str {
        match self {
            Self::Function => "Function",
            Self::AsyncFunction => "AsyncFunction",
            Self::GeneratorFunction => "GeneratorFunction",
            Self::AsyncGeneratorFunction => "AsyncGeneratorFunction",
        }
    }

    fn prototype_expression(self) -> &'static str {
        match self {
            Self::Function => "Function.prototype",
            Self::AsyncFunction => "Object.getPrototypeOf(async function() {})",
            Self::GeneratorFunction => "Object.getPrototypeOf(function*() {})",
            Self::AsyncGeneratorFunction => "Object.getPrototypeOf(async function*() {})",
        }
    }
}

const FUNCTION_CONSTRUCTOR_KINDS: [FunctionConstructorKind; 4] = [
    FunctionConstructorKind::Function,
    FunctionConstructorKind::AsyncFunction,
    FunctionConstructorKind::GeneratorFunction,
    FunctionConstructorKind::AsyncGeneratorFunction,
];

fn install_function_constructor_brand_guards<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    // V8's Function constructors report one combined source string to the
    // code-generation callback. Preserve the original argument list here so
    // an overridden TrustedScript string conversion can still be downgraded
    // to the default-policy path. The traps always delegate compilation to the
    // intrinsic constructors; they do not compile source themselves.
    let reflect_construct = global
        .get(scope, v8str(scope, "Reflect").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|reflect| reflect.get(scope, v8str(scope, "construct").into()))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow!("Reflect.construct missing during Trusted Types bootstrap"))?;
    let apply = v8::Function::builder(function_constructor_apply_trap_callback)
        .length(3)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create Function apply brand guard"))?;
    let construct = v8::Function::builder(function_constructor_construct_trap_callback)
        .data(reflect_construct.into())
        .length(3)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create Function construct brand guard"))?;
    let handler = v8::Object::new(scope);
    if !handler
        .define_own_property(
            scope,
            v8str(scope, "apply").into(),
            apply.into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap_or(false)
        || !handler
            .define_own_property(
                scope,
                v8str(scope, "construct").into(),
                construct.into(),
                v8::PropertyAttribute::DONT_ENUM,
            )
            .unwrap_or(false)
    {
        return Err(anyhow!(
            "failed to initialize Function constructor brand guard"
        ));
    }

    for kind in FUNCTION_CONSTRUCTOR_KINDS {
        let prototype = eval_object(scope, kind.prototype_expression())
            .ok_or_else(|| anyhow!("{} prototype missing", kind.name()))?;
        let target = prototype
            .get(scope, v8str(scope, "constructor").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| anyhow!("{} constructor missing", kind.name()))?;
        let proxy = v8::Proxy::new(scope, target.into(), handler)
            .ok_or_else(|| anyhow!("failed to proxy {} constructor", kind.name()))?;
        if !prototype
            .define_own_property(
                scope,
                v8str(scope, "constructor").into(),
                proxy.into(),
                v8::PropertyAttribute::DONT_ENUM,
            )
            .unwrap_or(false)
        {
            return Err(anyhow!(
                "failed to install {} constructor brand guard",
                kind.name()
            ));
        }
        if matches!(kind, FunctionConstructorKind::Function) {
            define_global_value(scope, global, "Function", proxy.into())?;
        }
    }
    Ok(())
}

fn eval_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = v8str(scope, source);
    v8::Script::compile(scope, source, None)
        .and_then(|script| script.run(scope))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn function_constructor_apply_trap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Function>::try_from(args.get(0)) else {
        return;
    };
    let Ok(arguments) = v8::Local::<v8::Array>::try_from(args.get(2)) else {
        return;
    };
    let Some(arguments) = prepare_function_constructor_arguments(scope, arguments) else {
        return;
    };
    if let Some(value) = target.call(scope, args.get(1), &arguments) {
        rv.set(value);
    }
}

fn function_constructor_construct_trap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(arguments) = v8::Local::<v8::Array>::try_from(args.get(1)) else {
        return;
    };
    let Some(arguments) = prepare_function_constructor_arguments(scope, arguments) else {
        return;
    };
    let Ok(reflect_construct) = v8::Local::<v8::Function>::try_from(args.data()) else {
        return;
    };
    let arguments = v8::Array::new_with_elements(scope, &arguments);
    let receiver = v8::undefined(scope);
    let forwarded = [args.get(0), arguments.into(), args.get(2)];
    if let Some(value) = reflect_construct.call(scope, receiver.into(), &forwarded) {
        rv.set(value);
    }
}

fn prepare_function_constructor_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    arguments: v8::Local<'s, v8::Array>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    let original = (0..arguments.length())
        .map(|index| arguments.get_index(scope, index))
        .collect::<Option<Vec<_>>>()?;
    if !trusted_types_for_script_is_required(scope) || trusted_types_eval_is_allowed(scope) {
        return Some(original);
    }

    let mut trusted = Vec::with_capacity(original.len());
    for value in &original {
        let Some(value) = trusted_type_string(scope, *value, TrustedTypeKind::Script) else {
            return Some(original);
        };
        trusted.push(value);
    }
    let stringified = original
        .iter()
        .map(|value| js_value_to_string(scope, *value))
        .collect::<Option<Vec<_>>>()?;
    if stringified != trusted {
        // Plain strings force V8's combined Function source through the normal
        // Trusted Types/default-policy check. Reusing the already stringified
        // values also avoids invoking a page-defined conversion twice.
        return stringified
            .into_iter()
            .map(|value| v8_string(scope, &value).map(Into::into))
            .collect();
    }
    // Private carriers retain V8's code-like brand while using an inaccessible
    // native string conversion, so the original page objects are converted
    // exactly once before the intrinsic constructor runs.
    trusted
        .into_iter()
        .map(|value| build_trusted_script_code_like_carrier(scope, value).map(Into::into))
        .collect()
}

fn trusted_types_for_script_is_required(scope: &mut v8::PinScope<'_, '_>) -> bool {
    if let Some(required) = crate::worker::worker_requires_trusted_types_for_script(scope) {
        return required;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    unsafe { &*host_ptr }.requires_trusted_types_for_script(scope)
}

fn trusted_types_eval_is_allowed(scope: &mut v8::PinScope<'_, '_>) -> bool {
    if let Some(allowed) = crate::worker::worker_allows_trusted_types_eval(scope) {
        return allowed;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    unsafe { &*host_ptr }.allows_trusted_types_eval(scope)
}

fn dispatch_trusted_types_sink_violation_event(
    scope: &mut v8::PinScope<'_, '_>,
    sink: &str,
    sample: &str,
) {
    if crate::worker::get_worker_state(scope).is_some() {
        crate::worker::dispatch_worker_trusted_types_sink_violation_event(scope, sink, sample);
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }
        .dispatch_trusted_types_sink_csp_violation_event_best_effort(scope, host_ptr, sink, sample);
}

fn dispatch_trusted_types_sink_violation_event_without_stack(
    scope: &mut v8::PinScope<'_, '_>,
    sink: &str,
    sample: &str,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }
        .dispatch_trusted_types_sink_csp_violation_event_without_stack_best_effort(
            scope, host_ptr, sink, sample,
        );
}

fn throw_trusted_type_error(
    scope: &mut v8::PinScope<'_, '_>,
    error_kind: TrustedTypeErrorKind,
    api_name: &'static str,
    kind: TrustedTypeKind,
    sink: &str,
) {
    let message = format!(
        "Failed to execute '{api_name}': This document requires '{}' assignment for the '{}' sink.",
        kind.constructor_name(),
        sink
    );
    match error_kind {
        TrustedTypeErrorKind::Type => throw_type_error(scope, &message),
        TrustedTypeErrorKind::Eval => throw_eval_error(scope, &message),
        TrustedTypeErrorKind::ScriptExecution => {}
    }
}

fn throw_trusted_type_policy_result_error(
    scope: &mut v8::PinScope<'_, '_>,
    error_kind: TrustedTypeErrorKind,
    kind: TrustedTypeKind,
) {
    let message = format!(
        "Trusted Types default policy did not return a {} value.",
        kind.constructor_name()
    );
    match error_kind {
        TrustedTypeErrorKind::Type => throw_type_error(scope, &message),
        TrustedTypeErrorKind::Eval => throw_eval_error(scope, &message),
        TrustedTypeErrorKind::ScriptExecution => {}
    }
}

fn throw_eval_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let global = scope.get_current_context().global(scope);
    if let Some(constructor) = global
        .get(scope, v8str(scope, "EvalError").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let message = v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
        if let Some(error) = constructor.new_instance(scope, &[message.into()]) {
            scope.throw_exception(error.into());
            return;
        }
    }
    let message = v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    scope.throw_exception(v8::Exception::error(scope, message));
}

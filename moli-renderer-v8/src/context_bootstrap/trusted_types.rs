use super::*;
use crate::content_security_policy::TrustedTypesForScriptRequirements;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

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
const TRUSTED_TYPES_DEFAULT_POLICY_SLOT: &str = "__moliTrustedTypesDefaultPolicy";
const TRUSTED_TYPES_CREATE_HTML_SLOT: &str = "__moliTrustedTypesCreateHTML";
const TRUSTED_TYPES_CREATE_SCRIPT_SLOT: &str = "__moliTrustedTypesCreateScript";
const TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT: &str = "__moliTrustedTypesCreateScriptURL";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct TrustedTypesFactoryDeclaration {
    #[webapi(method, callback = trusted_types_create_policy_callback, length = 2)]
    create_policy: (),
    #[webapi(method = "isHTML", callback = trusted_types_is_html_callback, length = 1)]
    is_html: (),
    #[webapi(method, callback = trusted_types_is_script_callback, length = 1)]
    is_script: (),
    #[webapi(method = "isScriptURL", callback = trusted_types_is_script_url_callback, length = 1)]
    is_script_url: (),
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

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct TrustedTypePolicyDeclaration<'scope> {
    #[webapi(data_property)]
    name: v8::Local<'scope, v8::String>,
    #[webapi(slot = TRUSTED_TYPES_CREATE_HTML_SLOT)]
    create_html_callback: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(
        method = "createHTML",
        callback = trusted_type_policy_create_callback,
        data = crate::util::callback_data_index_value(scope, 0),
        length = 1
    )]
    create_html: (),
    #[webapi(slot = TRUSTED_TYPES_CREATE_SCRIPT_SLOT)]
    create_script_callback: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(
        method,
        callback = trusted_type_policy_create_callback,
        data = crate::util::callback_data_index_value(scope, 1),
        length = 1
    )]
    create_script: (),
    #[webapi(slot = TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT)]
    create_script_url_callback: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(
        method = "createScriptURL",
        callback = trusted_type_policy_create_callback,
        data = crate::util::callback_data_index_value(scope, 2),
        length = 1
    )]
    create_script_url: (),
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

pub(crate) fn install_trusted_types_eval_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    install_function_constructor_wrappers(scope, global)
}

pub(crate) fn trusted_script_url_string_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    requirements: TrustedTypesForScriptRequirements,
    sink: &'static str,
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
    sink: &'static str,
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
    sink: &'static str,
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
    sink: &'static str,
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

/// Runs Trusted Types' pre-navigation check for a decoded JavaScript-URL
/// script in the currently entered Window realm.
///
/// Navigation is not a synchronous JavaScript sink: a missing/rejected
/// default policy blocks an enforcing Document without surfacing an exception
/// to the producer. Report-only policy still reports the mismatch and lets the
/// original source continue. A successful default policy may rewrite the
/// source, but its result must still form a valid `javascript:` URL.
pub(crate) fn trusted_script_string_for_javascript_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
    requirements: TrustedTypesForScriptRequirements,
) -> Option<String> {
    if !requirements.requires_conversion() {
        return Some(original.to_owned());
    }

    let default_policy = {
        // Default-policy callbacks are invoked by the later navigation task.
        // Their exceptions are deliberately consumed at this boundary rather
        // than escaping through the API call that scheduled the navigation.
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        apply_default_trusted_type_policy_outcome(
            &mut scope,
            original,
            TrustedTypeKind::Script,
            "Location href",
        )
    };
    if let DefaultTrustedTypePolicyOutcome::Value(value) = &default_policy
        && javascript_navigation_default_policy_result_is_valid(value)
    {
        return Some(value.clone());
    }

    dispatch_trusted_types_sink_violation_event_without_stack(scope, "Location href", original);
    (!requirements.is_enforced()).then(|| original.to_owned())
}

fn javascript_navigation_default_policy_result_is_valid(source: &str) -> bool {
    url::Url::parse(&format!("javascript:{source}")).is_ok()
}

#[cfg(test)]
#[test]
fn javascript_navigation_default_policy_rejects_an_invalid_reconstructed_url() {
    assert!(javascript_navigation_default_policy_result_is_valid(
        "globalThis.accepted = true"
    ));
    assert!(!javascript_navigation_default_policy_result_is_valid(
        "//make:invalid/"
    ));
}

pub(crate) enum TrustedTypesCodeGenerationCheck {
    AllowOriginal,
    AllowModified(String),
    Block,
}

pub(crate) unsafe extern "C" fn trusted_types_code_generation_check_callback(
    context: v8::Local<'_, v8::Context>,
    source: v8::Local<'_, v8::Value>,
    is_code_like: bool,
    modified_source: *mut *const v8::String,
) -> bool {
    v8::callback_scope!(unsafe scope, context);
    if trusted_types_eval_is_allowed(scope) && source.is_string() {
        return true;
    }
    match trusted_types_code_generation_check(scope, source, is_code_like) {
        TrustedTypesCodeGenerationCheck::AllowOriginal => true,
        TrustedTypesCodeGenerationCheck::AllowModified(source) => {
            let Some(source) = v8_string(scope, &source) else {
                return false;
            };
            if !modified_source.is_null() {
                unsafe {
                    *modified_source = &*source;
                }
            }
            true
        }
        TrustedTypesCodeGenerationCheck::Block => false,
    }
}

pub(crate) fn trusted_types_code_generation_check<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Value>,
    is_code_like: bool,
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
    trusted_script_string_for_code_generation(scope, &original)
        .map(TrustedTypesCodeGenerationCheck::AllowModified)
        .unwrap_or(TrustedTypesCodeGenerationCheck::Block)
}

fn trusted_script_string_for_code_generation(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
) -> Option<String> {
    let Some(callback_input) = function_constructor_callback_input(original) else {
        return trusted_script_string_for_eval_source(scope, original);
    };
    if let Some(default_value) = apply_default_trusted_type_policy(
        scope,
        callback_input,
        TrustedTypeKind::Script,
        "Function",
        TrustedTypeErrorKind::Eval,
    ) {
        if default_value == callback_input {
            return Some(original.to_owned());
        }
        dispatch_trusted_types_sink_violation_event(scope, "Function", callback_input);
        throw_eval_error(
            scope,
            "Trusted Types default policy must not transform strings passed to Function.",
        );
        return None;
    }
    dispatch_trusted_types_sink_violation_event(scope, "Function", callback_input);
    throw_trusted_type_error(
        scope,
        TrustedTypeErrorKind::Eval,
        "Function",
        TrustedTypeKind::Script,
        "Function",
    );
    None
}

fn function_constructor_callback_input(source: &str) -> Option<&str> {
    let source = source
        .strip_prefix('(')
        .and_then(|source| source.strip_suffix(')'))?;
    [
        "function anonymous",
        "async function anonymous",
        "function* anonymous",
        "async function* anonymous",
    ]
    .into_iter()
    .any(|prefix| source.starts_with(prefix))
    .then_some(source)
}

fn trusted_script_string_for_eval_source(
    scope: &mut v8::PinScope<'_, '_>,
    original: &str,
) -> Option<String> {
    if let Some(default_value) = apply_default_trusted_type_policy(
        scope,
        original,
        TrustedTypeKind::Script,
        "eval",
        TrustedTypeErrorKind::Eval,
    ) {
        if default_value == original {
            return Some(default_value);
        }
        dispatch_trusted_types_sink_violation_event(scope, "eval", original);
        throw_eval_error(
            scope,
            "Trusted Types default policy must not transform strings passed to eval.",
        );
        return None;
    }
    dispatch_trusted_types_sink_violation_event(scope, "eval", original);
    throw_trusted_type_error(
        scope,
        TrustedTypeErrorKind::Eval,
        "eval",
        TrustedTypeKind::Script,
        "eval",
    );
    None
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
    sink: &'static str,
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
    sink: &'static str,
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
    sink: &'static str,
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
    let sink = v8str(scope, sink);
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
    let object = if kind == TrustedTypeKind::Script {
        get_private_value(scope, global, TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?
            .new_instance(scope, &[])?
    } else {
        v8::Object::new(scope)
    };
    TrustedTypeObjectDeclaration::new(v8str(scope, kind.as_slot_value()), value)
        .initialize(scope, object)
        .expect("TrustedType object declaration should initialize");
    let _ = object.set_prototype(scope, prototype.into());
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

fn trusted_types_create_policy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
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

    if !trusted_types_policy_name_allowed(scope, &name) {
        throw_type_error(
            scope,
            "Failed to execute 'createPolicy' on 'TrustedTypePolicyFactory': Content Security Policy disallows creating a policy with the given name.",
        );
        return;
    }

    let policy_name = v8_string(scope, &name).unwrap_or_else(|| v8::String::empty(scope));
    let policy = TrustedTypePolicyDeclaration::new(
        policy_name,
        callbacks.create_html,
        callbacks.create_script,
        callbacks.create_script_url,
    )
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
    rv.set(policy.into());
}

fn trusted_types_policy_name_allowed(scope: &mut v8::PinScope<'_, '_>, name: &str) -> bool {
    if let Some(allowed) = crate::worker::worker_allows_trusted_type_policy_name(scope, name) {
        return allowed;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    unsafe { &*host_ptr }.allows_trusted_type_policy_name(scope, name)
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
    rv.set_bool(trusted_type_string(scope, args.get(0), TrustedTypeKind::Html).is_some());
}

fn trusted_types_is_script_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(trusted_type_string(scope, args.get(0), TrustedTypeKind::Script).is_some());
}

fn trusted_types_is_script_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(trusted_type_string(scope, args.get(0), TrustedTypeKind::ScriptUrl).is_some());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrustedFunctionConstructorKind {
    Function,
    AsyncFunction,
    GeneratorFunction,
    AsyncGeneratorFunction,
}

impl TrustedFunctionConstructorKind {
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

    fn source_wrapper(self, params: &str, body: &str) -> String {
        match self {
            Self::Function => format!("(function anonymous({params}) {{\n{body}\n}})"),
            Self::AsyncFunction => format!("(async function anonymous({params}) {{\n{body}\n}})"),
            Self::GeneratorFunction => {
                format!("(function* anonymous({params}) {{\n{body}\n}})")
            }
            Self::AsyncGeneratorFunction => {
                format!("(async function* anonymous({params}) {{\n{body}\n}})")
            }
        }
    }

    fn default_policy_source(self, params: &str, body: &str) -> String {
        let prefix = match self {
            Self::Function => "function anonymous",
            Self::AsyncFunction => "async function anonymous",
            Self::GeneratorFunction => "function* anonymous",
            Self::AsyncGeneratorFunction => "async function* anonymous",
        };
        format!("{prefix}({params}\n) {{\n{body}\n}}")
    }
}

const TRUSTED_FUNCTION_CONSTRUCTOR_KINDS: [TrustedFunctionConstructorKind; 4] = [
    TrustedFunctionConstructorKind::Function,
    TrustedFunctionConstructorKind::AsyncFunction,
    TrustedFunctionConstructorKind::GeneratorFunction,
    TrustedFunctionConstructorKind::AsyncGeneratorFunction,
];

fn install_function_constructor_wrappers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    for (index, kind) in TRUSTED_FUNCTION_CONSTRUCTOR_KINDS.into_iter().enumerate() {
        let data = crate::util::callback_data_index_value(scope, index);
        let constructor = v8::Function::builder(trusted_types_function_constructor_callback)
            .data(data)
            .length(1)
            .build(scope)
            .ok_or_else(|| anyhow!("failed to build {} Trusted Types wrapper", kind.name()))?;
        constructor.set_name(v8str(scope, kind.name()));
        if let Some(prototype) = eval_object(scope, kind.prototype_expression()) {
            let _ = prototype.define_own_property(
                scope,
                v8str(scope, "constructor").into(),
                constructor.into(),
                v8::PropertyAttribute::DONT_ENUM,
            );
        }
        if kind == TrustedFunctionConstructorKind::Function {
            define_global_value(scope, global, "Function", constructor.into())?;
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

fn trusted_types_function_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(kind) = crate::util::callback_data_item(
        scope,
        &args,
        &TRUSTED_FUNCTION_CONSTRUCTOR_KINDS,
        "Trusted Types Function constructor wrappers",
    ) else {
        return;
    };
    let Some((params, body)) = function_constructor_source_parts(scope, &args, kind) else {
        return;
    };
    let source = kind.source_wrapper(&params, &body);
    let Some(source) = v8_string(scope, &source) else {
        return;
    };
    let Some(script) = v8::Script::compile(scope, source, None) else {
        return;
    };
    if let Some(function) = script.run(scope) {
        rv.set(function);
    }
}

fn function_constructor_source_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: TrustedFunctionConstructorKind,
) -> Option<(String, String)> {
    let argument_count = args.length();
    if argument_count == 0 {
        let empty = v8::String::empty(scope);
        let body = trusted_script_string_for_function_constructor(scope, empty.into(), "", kind)?;
        return Some((String::new(), body));
    }
    let mut params = Vec::new();
    for index in 0..argument_count.saturating_sub(1) {
        params.push(js_value_to_string(scope, args.get(index))?);
    }
    let params = params.join(",");
    let body = trusted_script_string_for_function_constructor(
        scope,
        args.get(argument_count.saturating_sub(1)),
        &params,
        kind,
    )?;
    Some((params, body))
}

fn trusted_script_string_for_function_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    params: &str,
    kind: TrustedFunctionConstructorKind,
) -> Option<String> {
    if let Some(value) = trusted_type_string(scope, value, TrustedTypeKind::Script) {
        return Some(value);
    }
    let original = js_value_to_string(scope, value)?;
    if trusted_types_eval_is_allowed(scope) {
        return Some(original);
    }
    let violation_sample = function_constructor_violation_sample(params, &original);
    let default_policy_source = kind.default_policy_source(params, &original);
    if let Some(default_value) = apply_default_trusted_type_policy(
        scope,
        &default_policy_source,
        TrustedTypeKind::Script,
        "Function",
        TrustedTypeErrorKind::Eval,
    ) {
        if default_value == default_policy_source {
            return Some(original);
        }
        dispatch_trusted_types_sink_violation_event(scope, "Function", &violation_sample);
        throw_eval_error(
            scope,
            "Trusted Types default policy must not transform strings passed to Function.",
        );
        return None;
    }
    dispatch_trusted_types_sink_violation_event(scope, "Function", &violation_sample);
    throw_trusted_type_error(
        scope,
        TrustedTypeErrorKind::Eval,
        "Function",
        TrustedTypeKind::Script,
        "Function",
    );
    None
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

fn function_constructor_violation_sample(params: &str, body: &str) -> String {
    format!("({params}\n) {{\n{body}\n}}")
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
    sink: &'static str,
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

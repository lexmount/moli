use crate::interfaces as web_api_interfaces;
use anyhow::{Result, anyhow};
use moli_v8_util::{get_private_value, set_private_value, throw_type_error, v8str};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;
pub const TRUSTED_TYPE_VALUE_SLOT: &str = "__moliTrustedTypeValue";
pub const TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT: &str =
    "__moliTrustedScriptCodeLikeConstructor";
pub const TRUSTED_TYPE_HTML_PROTOTYPE_SLOT: &str = "__moliTrustedHTMLPrototype";
pub const TRUSTED_TYPE_SCRIPT_PROTOTYPE_SLOT: &str = "__moliTrustedScriptPrototype";
pub const TRUSTED_TYPE_SCRIPT_URL_PROTOTYPE_SLOT: &str = "__moliTrustedScriptURLPrototype";
pub const TRUSTED_TYPE_HTML_CONSTRUCTOR_SLOT: &str = "__moliTrustedHTMLConstructor";
pub const TRUSTED_TYPE_SCRIPT_CONSTRUCTOR_SLOT: &str = "__moliTrustedScriptConstructor";
pub const TRUSTED_TYPE_SCRIPT_URL_CONSTRUCTOR_SLOT: &str = "__moliTrustedScriptURLConstructor";
pub const TRUSTED_TYPES_CREATE_HTML_SLOT: &str = "__moliTrustedTypesCreateHTML";
pub const TRUSTED_TYPES_CREATE_SCRIPT_SLOT: &str = "__moliTrustedTypesCreateScript";
pub const TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT: &str = "__moliTrustedTypesCreateScriptURL";
#[derive(WebApiObject)]
#[webapi(plain)]
pub struct TrustedTypeObjectDeclaration<'scope> {
    #[webapi(slot = TRUSTED_TYPE_VALUE_SLOT)]
    value: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(fragment, enumerable)]
struct TrustedTypePrototypeDeclaration<'scope> {
    kind: v8::Local<'scope, v8::String>,

    #[webapi(method = "toString", callback = trusted_type_to_string_callback, data = self.kind, length = 0)]
    to_string: (),
    #[webapi(method = "toJSON", callback = trusted_type_to_string_callback, data = self.kind, length = 0)]
    to_json: (),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustedTypeKind {
    Html,
    Script,
    ScriptUrl,
}

impl TrustedTypeKind {
    pub fn interface(self) -> moli_webapi_declare::WebApiInterfaceDescriptor {
        match self {
            Self::Html => web_api_interfaces::TrustedHTML::DESCRIPTOR,
            Self::Script => web_api_interfaces::TrustedScript::DESCRIPTOR,
            Self::ScriptUrl => web_api_interfaces::TrustedScriptURL::DESCRIPTOR,
        }
    }

    pub fn constructor_name(self) -> &'static str {
        self.interface().name()
    }

    pub fn create_method_name(self) -> &'static str {
        match self {
            Self::Html => "createHTML",
            Self::Script => "createScript",
            Self::ScriptUrl => "createScriptURL",
        }
    }

    pub fn callback_slot(self) -> &'static str {
        match self {
            Self::Html => TRUSTED_TYPES_CREATE_HTML_SLOT,
            Self::Script => TRUSTED_TYPES_CREATE_SCRIPT_SLOT,
            Self::ScriptUrl => TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT,
        }
    }

    pub fn prototype_slot(self) -> &'static str {
        match self {
            Self::Html => TRUSTED_TYPE_HTML_PROTOTYPE_SLOT,
            Self::Script => TRUSTED_TYPE_SCRIPT_PROTOTYPE_SLOT,
            Self::ScriptUrl => TRUSTED_TYPE_SCRIPT_URL_PROTOTYPE_SLOT,
        }
    }

    pub fn constructor_slot(self) -> &'static str {
        match self {
            Self::Html => TRUSTED_TYPE_HTML_CONSTRUCTOR_SLOT,
            Self::Script => TRUSTED_TYPE_SCRIPT_CONSTRUCTOR_SLOT,
            Self::ScriptUrl => TRUSTED_TYPE_SCRIPT_URL_CONSTRUCTOR_SLOT,
        }
    }

    pub fn from_callback_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Html),
            1 => Some(Self::Script),
            2 => Some(Self::ScriptUrl),
            _ => None,
        }
    }
}

pub const TRUSTED_TYPE_KINDS: [TrustedTypeKind; 3] = [
    TrustedTypeKind::Html,
    TrustedTypeKind::Script,
    TrustedTypeKind::ScriptUrl,
];

pub fn trusted_type_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<TrustedTypeKind> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    TRUSTED_TYPE_KINDS
        .into_iter()
        .find(|kind| kind.interface().is_instance(scope, object))
}

pub struct TrustedTypeConstructorBinding<'s> {
    pub kind: TrustedTypeKind,
    pub constructor: v8::Local<'s, v8::Function>,
    pub prototype: v8::Local<'s, v8::Object>,
}

pub fn trusted_type_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: TrustedTypeKind,
) -> Option<String> {
    trusted_type_string16(scope, value, kind).map(|units| String::from_utf16_lossy(&units))
}

pub fn trusted_type_string16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: TrustedTypeKind,
) -> Option<Vec<u16>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if !kind.interface().is_instance(scope, object) {
        return None;
    }
    let value = get_private_value(scope, object, TRUSTED_TYPE_VALUE_SLOT)?;
    value
        .to_string(scope)
        .map(|value| moli_v8_util::v8_string_to_u16_string(scope, value).into_vec())
}

pub fn build_trusted_type_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: TrustedTypeKind,
) -> Result<TrustedTypeConstructorBinding<'s>> {
    let name = kind.constructor_name();
    let template = v8::FunctionTemplate::new(scope, trusted_type_illegal_constructor_callback);
    template.read_only_prototype();
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to create {name} constructor"))?;
    constructor.set_name(v8str(scope, name));
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("{name}.prototype missing"))?;
    TrustedTypePrototypeDeclaration::new(v8str(scope, kind.constructor_name()))
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize {name}.prototype declaration: {error}"))?;
    prototype
        .define_own_property(
            scope,
            v8::Symbol::get_to_string_tag(scope).into(),
            v8str(scope, name).into(),
            v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
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
pub fn install_trusted_script_code_like_constructor<'s>(
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
    TrustedTypePrototypeDeclaration::new(v8str(scope, TrustedTypeKind::Script.constructor_name()))
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
pub fn build_trusted_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: TrustedTypeKind,
    value: &[u16],
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let prototype = get_private_value(scope, global, kind.prototype_slot())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let value = moli_v8_util::v8_string_from_utf16_units(scope, value)?;
    let object = if kind == TrustedTypeKind::Script {
        get_private_value(scope, global, TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?
            .new_instance(scope, &[])?
    } else {
        v8::Object::new(scope)
    };
    TrustedTypeObjectDeclaration::new(value)
        .initialize(scope, object)
        .expect("TrustedType object declaration should initialize");
    kind.interface().initialize(scope, object).ok()?;
    object
        .set_prototype(scope, prototype.into())?
        .then_some(())?;
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
    if !trusted_type_kind(scope, this.into())
        .is_some_and(|kind| v8str(scope, kind.constructor_name()).strict_equals(args.data()))
    {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(value) = get_private_value(scope, this, TRUSTED_TYPE_VALUE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TrustedTypePolicyOptions")]
pub struct TrustedTypePolicyOptionsMembers {
    #[webidl(name = "createHTML", converter = "callback_function")]
    pub create_html: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "createScript", converter = "callback_function")]
    pub create_script: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "createScriptURL", converter = "callback_function")]
    pub create_script_url: Option<webidl::WebIdlCallbackFunction>,
}

pub fn parse_policy_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<TrustedTypePolicyOptionsMembers> {
    let members = match webidl::parse_dictionary::<TrustedTypePolicyOptionsMembers>(
        scope,
        options,
        webidl::Context::argument("TrustedTypePolicyFactory.createPolicy", 2),
    ) {
        Ok(Some(members)) => members,
        Ok(None) => TrustedTypePolicyOptionsMembers::default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    Some(members)
}

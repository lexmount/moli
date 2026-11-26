use crate::interfaces as web_api_interfaces;
use moli_browser_profile::BrowserIdentityProfile;
use moli_v8_util::{get_private_value, set_private_value, v8_string};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
use webidl::{SnapshotPrototype, WebIdlDictionarySerialize};

const NAVIGATOR_UA_DATA_USER_AGENT_SLOT: &str = "__moliNavigatorUADataUserAgent";

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigatorUABrandVersion", snapshot)]
struct NavigatorUaBrandVersion {
    #[webidl(required)]
    brand: String,
    #[webidl(required)]
    version: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigatorUAData)]
struct NavigatorUaDataObjectDeclaration {
    #[webapi(slot = NAVIGATOR_UA_DATA_USER_AGENT_SLOT)]
    user_agent: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "UALowEntropyJSON", snapshot)]
struct NavigatorUaDataLowEntropy {
    #[webidl(required, sequence, converter = "dictionary")]
    brands: Vec<NavigatorUaBrandVersion>,
    #[webidl(required)]
    mobile: bool,
    #[webidl(required)]
    platform: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "UADataValues", snapshot)]
struct NavigatorUaDataHighEntropy {
    architecture: Option<String>,
    bitness: Option<String>,
    #[webidl(required, sequence, converter = "dictionary")]
    brands: Vec<NavigatorUaBrandVersion>,
    #[webidl(sequence)]
    form_factors: Option<Vec<String>>,
    #[webidl(sequence, converter = "dictionary")]
    full_version_list: Option<Vec<NavigatorUaBrandVersion>>,
    #[webidl(required)]
    mobile: bool,
    model: Option<String>,
    #[webidl(required)]
    platform: String,
    platform_version: Option<String>,
    ua_full_version: Option<String>,
    wow64: Option<bool>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NavigatorUAData.getHighEntropyValues")]
struct NavigatorUaDataGetHighEntropyValuesArgs {
    #[webidl(required, with = navigator_ua_data_high_entropy_hints_arg)]
    hints: webidl::Sequence<webidl::DomString>,
}

fn navigator_ua_data_high_entropy_hints_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<webidl::Sequence<webidl::DomString>, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to execute 'getHighEntropyValues' on 'NavigatorUAData': 1 argument required, but only 0 present.",
        ));
    }
    webidl::convert(
        scope,
        args.get(index),
        webidl::Context::argument("NavigatorUAData.getHighEntropyValues", (index + 1) as usize),
    )
}

pub fn navigator_ua_data_brands_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated NavigatorUAData receiver");
    let context = receiver.get_creation_context(scope).expect("UAData realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let identity = navigator_ua_data_identity(scope, args.this());
    // The FrozenArray value belongs to the receiver, even when its getter
    // comes from another realm. Keep the immutable profile as its source.
    let values = build_ua_brand_records(identity.brands())
        .iter()
        .map(|brand| {
            brand
                .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
                .expect("UA brand dictionary snapshot")
                .into()
        })
        .collect::<Vec<_>>();
    let array = v8::Array::new_with_elements(scope, &values);
    assert_eq!(
        array.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true)
    );
    rv.set(array.into());
}

pub fn navigator_ua_data_mobile_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(navigator_ua_data_identity(scope, args.this()).mobile());
}

pub fn navigator_ua_data_platform_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let identity = navigator_ua_data_identity(scope, args.this());
    rv.set(
        v8_string(scope, identity.platform())
            .expect("UA platform string")
            .into(),
    );
}

pub fn navigator_ua_data_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let identity = navigator_ua_data_identity(scope, args.this());
    rv.set(build_navigator_ua_data_snapshot(scope, &identity, None).into());
}

pub fn navigator_ua_data_get_high_entropy_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let requested = match navigator_ua_data_high_entropy_keys(scope, &args) {
        Ok(requested) => requested,
        Err(exception) => {
            let _ = resolver.reject(scope, exception);
            rv.set(resolver.get_promise(scope).into());
            return;
        }
    };
    let identity = navigator_ua_data_identity(scope, args.this());
    let values = build_navigator_ua_data_snapshot(scope, &identity, Some(&requested));
    let _ = resolver.resolve(scope, values.into());
    rv.set(resolver.get_promise(scope).into());
}

fn navigator_ua_data_high_entropy_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<std::collections::BTreeSet<String>, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut conversion_scope = try_catch.init();
    match webidl::try_parse_args::<NavigatorUaDataGetHighEntropyValuesArgs>(
        &mut conversion_scope,
        args,
    ) {
        Ok(parsed) => Ok(parsed.hints.0.into_iter().map(|hint| hint.0).collect()),
        Err(error) => {
            webidl::throw_error(&mut conversion_scope, &error);
            let exception = conversion_scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&conversion_scope).into());
            conversion_scope.reset();
            Err(exception)
        }
    }
}

pub fn build_navigator_ua_data_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &BrowserIdentityProfile,
) -> v8::Local<'s, v8::Object> {
    let object = NavigatorUaDataObjectDeclaration::new(identity.user_agent().to_owned())
        .bind(scope)
        .expect("NavigatorUAData declaration should bind");
    set_navigator_identity_profile(scope, object, identity);
    object
}

fn build_navigator_ua_data_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &BrowserIdentityProfile,
    requested: Option<&std::collections::BTreeSet<String>>,
) -> v8::Local<'s, v8::Object> {
    let brands = build_ua_brand_records(identity.brands());
    if let Some(requested) = requested {
        let requested_key = |key: &str| requested.contains(key);
        return NavigatorUaDataHighEntropy {
            architecture: requested_key("architecture").then(|| identity.architecture().to_owned()),
            bitness: requested_key("bitness").then(|| identity.bitness().to_owned()),
            brands,
            form_factors: requested_key("formFactors").then(|| identity.form_factors().to_vec()),
            full_version_list: requested_key("fullVersionList")
                .then(|| build_ua_brand_records(identity.full_version_list())),
            mobile: identity.mobile(),
            model: requested_key("model").then(|| identity.model().to_owned()),
            platform: identity.platform().to_owned(),
            platform_version: requested_key("platformVersion")
                .then(|| identity.platform_version().to_owned()),
            ua_full_version: requested_key("uaFullVersion")
                .then(|| identity.full_version().to_owned()),
            wow64: requested_key("wow64").then(|| identity.wow64()),
        }
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .expect("NavigatorUAData high entropy dictionary snapshot");
    }
    NavigatorUaDataLowEntropy {
        brands,
        mobile: identity.mobile(),
        platform: identity.platform().to_owned(),
    }
    .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
    .expect("NavigatorUAData low entropy dictionary snapshot")
}

fn navigator_ua_data_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> BrowserIdentityProfile {
    let object = moli_webapi_declare::web_api_object_target(scope, object)
        .expect("validated NavigatorUAData receiver");
    if let Some(identity) = navigator_identity_profile(scope, object) {
        return identity;
    }
    get_private_value(scope, object, NAVIGATOR_UA_DATA_USER_AGENT_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .map(|user_agent| {
            BrowserIdentityProfile::new(user_agent, moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE)
        })
        .unwrap_or_default()
}

fn build_ua_brand_records(
    brands: &[moli_browser_profile::BrowserBrandVersion],
) -> Vec<NavigatorUaBrandVersion> {
    brands
        .iter()
        .map(|entry| NavigatorUaBrandVersion {
            brand: entry.brand.clone(),
            version: entry.version.clone(),
        })
        .collect()
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigatorUAData, enumerable)]
struct NavigatorUaDataPrototypeMethodsDeclaration {
    #[webapi(accessor_property, getter = navigator_ua_data_brands_getter)]
    brands: (),
    #[webapi(accessor_property, getter = navigator_ua_data_mobile_getter)]
    mobile: (),
    #[webapi(accessor_property, getter = navigator_ua_data_platform_getter)]
    platform: (),

    #[webapi(method, name = "toJSON", length = 0, callback = navigator_ua_data_to_json_callback)]
    to_json: (),

    #[webapi(method, length = 1, callback = navigator_ua_data_get_high_entropy_values_callback, returns_promise)]
    get_high_entropy_values: (),
}

const NAVIGATOR_IDENTITY_PROFILE_SLOT: &str = "__moliNavigatorIdentityProfile";
pub fn set_navigator_identity_profile<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    identity: &BrowserIdentityProfile,
) {
    let Ok(serialized) = serde_json::to_string(identity) else {
        return;
    };
    let Some(value) = v8_string(scope, &serialized) else {
        return;
    };
    set_private_value(scope, object, NAVIGATOR_IDENTITY_PROFILE_SLOT, value.into());
}

pub fn navigator_identity_profile<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BrowserIdentityProfile> {
    let serialized = get_private_value(scope, object, NAVIGATOR_IDENTITY_PROFILE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&serialized).ok()
}
pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    NavigatorUaDataPrototypeMethodsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

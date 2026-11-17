use crate::interfaces as web_api_interfaces;
use moli_browser_profile::BrowserIdentityProfile;
use moli_v8_util::{get_private_value, set_private_value, v8_string};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigatorUaBrandEntryDeclaration {
    #[webapi(data_property, enumerable)]
    brand: String,

    #[webapi(data_property, enumerable)]
    version: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigatorUAData)]
struct NavigatorUaDataObjectDeclaration {
    #[webapi(slot = NAVIGATOR_UA_DATA_USER_AGENT_SLOT)]
    user_agent: String,

    #[webapi(data_property, enumerable)]
    brands: Vec<NavigatorUaBrandEntryDeclaration>,

    #[webapi(data_property, enumerable)]
    mobile: bool,

    #[webapi(data_property, enumerable)]
    platform: String,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigatorUaDataSnapshotDeclaration {
    #[webapi(data_property, enumerable)]
    brands: Vec<NavigatorUaBrandEntryDeclaration>,

    #[webapi(data_property, enumerable)]
    mobile: bool,

    #[webapi(data_property, enumerable)]
    platform: String,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigatorUaDataHighEntropySnapshotDeclaration {
    #[webapi(data_property, enumerable)]
    architecture: Option<String>,

    #[webapi(data_property, enumerable)]
    bitness: Option<String>,

    #[webapi(data_property, enumerable)]
    brands: Vec<NavigatorUaBrandEntryDeclaration>,

    #[webapi(data_property = "formFactors", enumerable)]
    form_factors: Option<Vec<String>>,

    #[webapi(data_property, enumerable)]
    full_version_list: Option<Vec<NavigatorUaBrandEntryDeclaration>>,

    #[webapi(data_property, enumerable)]
    mobile: bool,

    #[webapi(data_property, enumerable)]
    model: Option<String>,

    #[webapi(data_property, enumerable)]
    platform: String,

    #[webapi(data_property, enumerable)]
    platform_version: Option<String>,

    #[webapi(data_property, enumerable)]
    ua_full_version: Option<String>,

    #[webapi(data_property, enumerable)]
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
    let brands = build_ua_brand_records(identity.brands());
    let object = NavigatorUaDataObjectDeclaration::new(
        identity.user_agent().to_owned(),
        brands,
        identity.mobile(),
        identity.platform().to_owned(),
    )
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
        return NavigatorUaDataHighEntropySnapshotDeclaration::new(
            requested_key("architecture").then(|| identity.architecture().to_owned()),
            requested_key("bitness").then(|| identity.bitness().to_owned()),
            brands,
            requested_key("formFactors").then(|| identity.form_factors().to_vec()),
            requested_key("fullVersionList")
                .then(|| build_ua_brand_records(identity.full_version_list())),
            identity.mobile(),
            requested_key("model").then(|| identity.model().to_owned()),
            identity.platform().to_owned(),
            requested_key("platformVersion").then(|| identity.platform_version().to_owned()),
            requested_key("uaFullVersion").then(|| identity.full_version().to_owned()),
            requested_key("wow64").then(|| identity.wow64()),
        )
        .bind(scope)
        .expect("NavigatorUAData high entropy snapshot declaration should bind");
    }
    NavigatorUaDataSnapshotDeclaration::new(
        brands,
        identity.mobile(),
        identity.platform().to_owned(),
    )
    .bind(scope)
    .expect("NavigatorUAData snapshot declaration should bind")
}

fn navigator_ua_data_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> BrowserIdentityProfile {
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
) -> Vec<NavigatorUaBrandEntryDeclaration> {
    brands
        .iter()
        .map(|entry| {
            NavigatorUaBrandEntryDeclaration::new(entry.brand.clone(), entry.version.clone())
        })
        .collect()
}

const NAVIGATOR_UA_DATA_USER_AGENT_SLOT: &str = "__moliNavigatorUADataUserAgent";
#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigatorUAData, enumerable, receiver)]
struct NavigatorUaDataPrototypeMethodsDeclaration {
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

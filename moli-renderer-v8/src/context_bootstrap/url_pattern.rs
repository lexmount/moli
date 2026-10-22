//! Native URLPattern bindings shared by Window and Worker realms.
//!
//! The URL parser owns component canonicalization and pattern syntax. Compiled
//! V8 expressions live only in private slots, away from author RegExp methods
//! and legacy RegExp statics. Constructors use the ordinary V8 allocation path.

use crate::{
    util::{get_private_value, v8_string},
    web_api_interfaces, webidl,
};
use moli_v8_util::NativeRegExp;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use v8::{self, RegExpCreationFlags};

mod input;
mod parser;
use input::{ConstructorArgs, Input, MatchArgs};

const COMPONENTS: &str = "__moliURLPatternComponents";
const HAS_GROUPS: &str = "__moliURLPatternHasGroups";
const PATTERN: &str = "__moliURLPatternString";
const REGEXP: &str = "__moliURLPatternRegExp";
const NAMES: &str = "__moliURLPatternGroupNames";
const KEYS: [&str; 8] = [
    "protocol", "username", "password", "hostname", "port", "pathname", "search", "hash",
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::URLPattern)]
struct PatternState<'s> {
    #[webapi(slot = COMPONENTS)]
    components: v8::Local<'s, v8::Array>,
    #[webapi(slot = HAS_GROUPS)]
    has_groups: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ComponentState<'s> {
    #[webapi(slot = PATTERN)]
    pattern: String,
    #[webapi(slot = REGEXP)]
    regexp: v8::Local<'s, v8::RegExp>,
    #[webapi(slot = NAMES)]
    names: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ComponentResult<'s> {
    groups: v8::Local<'s, v8::Object>,
    input: String,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MatchResult<'s> {
    hash: v8::Local<'s, v8::Object>,
    hostname: v8::Local<'s, v8::Object>,
    inputs: v8::Local<'s, v8::Array>,
    password: v8::Local<'s, v8::Object>,
    pathname: v8::Local<'s, v8::Object>,
    port: v8::Local<'s, v8::Object>,
    protocol: v8::Local<'s, v8::Object>,
    search: v8::Local<'s, v8::Object>,
    username: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::URLPattern, constructor_callback = constructor, constructor_length = 0, enumerable, receiver)]
struct PatternTemplate {
    #[webapi(method, callback = test, length = 0)]
    test: (),
    #[webapi(method, callback = exec, length = 0)]
    exec: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 0))]
    protocol: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 1))]
    username: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 2))]
    password: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 3))]
    hostname: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 4))]
    port: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 5))]
    pathname: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 6))]
    search: (),
    #[webapi(accessor_property, getter = component_getter, data = v8::Integer::new(scope, 7))]
    hash: (),
    #[webapi(accessor_property, getter = has_groups_getter)]
    has_reg_exp_groups: (),
}

pub(super) fn build_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    PatternTemplate::build(scope)
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        webidl::throw_type_error(scope, "URLPattern requires the new operator");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let (base, options) = parsed.options;
    let pattern = match parser::parse(scope, parsed.input, base.as_deref(), options.ignore_case) {
        Ok(pattern) => pattern,
        Err(error) => {
            webidl::throw_type_error(scope, &format!("Invalid URLPattern: {error}"));
            return;
        }
    };
    let has_groups = pattern.has_regexp_groups;
    let parts = [
        pattern.protocol,
        pattern.username,
        pattern.password,
        pattern.hostname,
        pattern.port,
        pattern.pathname,
        pattern.search,
        pattern.hash,
    ];
    let mut components = Vec::with_capacity(8);
    for (index, part) in parts.into_iter().enumerate() {
        let Some(source) = v8_string(scope, &part.regexp_string) else {
            return;
        };
        let mut flags = RegExpCreationFlags::UNICODE_SETS;
        // Protocol, authority and port already use their canonical URL forms.
        if options.ignore_case && index >= 5 {
            flags |= RegExpCreationFlags::IGNORE_CASE;
        }
        let Some(regexp) = NativeRegExp::new(scope, source, flags) else {
            webidl::throw_type_error(
                scope,
                &format!("Invalid URLPattern {} expression", KEYS[index]),
            );
            return;
        };
        let Some(names) = crate::util::serialize_v8_array(scope, part.group_name_list) else {
            return;
        };
        let Ok(component) =
            ComponentState::new(part.pattern_string, regexp.compiled(), names).bind(scope)
        else {
            return;
        };
        components.push(component.into());
    }
    let components = v8::Array::new_with_elements(scope, &components);
    if PatternState::new(components, has_groups)
        .initialize(scope, args.this())
        .is_ok()
    {
        rv.set(args.this().into());
    }
}

fn component<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let parts =
        v8::Local::<v8::Array>::try_from(get_private_value(scope, object, COMPONENTS)?).ok()?;
    v8::Local::<v8::Object>::try_from(parts.get_index(scope, index)?).ok()
}

fn component_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(index) = args.data().uint32_value(scope)
        && let Some(part) = component(scope, args.this(), index)
        && let Some(value) = get_private_value(scope, part, PATTERN)
    {
        rv.set(value);
    }
}

fn has_groups_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), HAS_GROUPS) {
        rv.set(value);
    }
}

fn test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = matches(scope, &args, false) {
        rv.set(value);
    }
}

fn exec<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = matches(scope, &args, true) {
        rv.set(value);
    }
}

fn matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    result_needed: bool,
) -> Option<v8::Local<'s, v8::Value>> {
    let parsed = webidl::parse_args::<MatchArgs>(scope, args)?;
    let miss = if result_needed {
        v8::null(scope).into()
    } else {
        v8::Boolean::new(scope, false).into()
    };
    let input = parsed.input;
    let normalized = match urlpattern::quirks::process_match_input(
        input.library_input(),
        parsed.base_url.as_deref(),
    ) {
        Ok(Some((normalized, _))) => normalized,
        Ok(None) => return Some(miss),
        Err(error) => {
            webidl::throw_type_error(scope, &format!("Invalid URLPattern input: {error}"));
            return None;
        }
    };
    let Some(values) = urlpattern::quirks::parse_match_input(normalized) else {
        return Some(miss);
    };
    let values = [
        values.protocol,
        values.username,
        values.password,
        values.hostname,
        values.port,
        values.pathname,
        values.search,
        values.hash,
    ];
    let mut results = Vec::with_capacity(8);
    for (index, value) in values.into_iter().enumerate() {
        let part = component(scope, args.this(), index as u32)?;
        let regexp =
            v8::Local::<v8::RegExp>::try_from(get_private_value(scope, part, REGEXP)?).ok()?;
        let regexp = NativeRegExp::from_compiled(scope, regexp);
        let subject = v8_string(scope, &value)?;
        // Host-regexp execution failures (including resource limits) are a
        // failed match, rather than an author exception or an undefined result.
        let Some(matched) = regexp.exec(scope, subject) else {
            return Some(miss);
        };
        if matched.is_null() {
            return Some(miss);
        }
        if !result_needed {
            continue;
        }
        let captures = v8::Local::<v8::Array>::try_from(matched).ok()?;
        let names =
            v8::Local::<v8::Array>::try_from(get_private_value(scope, part, NAMES)?).ok()?;
        let groups = v8::Object::new(scope);
        for i in 0..names.length() {
            let name = v8::Local::<v8::Name>::try_from(names.get_index(scope, i)?).ok()?;
            let capture = captures.get_index(scope, i + 1)?;
            if groups.create_data_property(scope, name, capture) != Some(true) {
                return None;
            }
        }
        results.push(
            ComponentResult {
                groups,
                input: value,
            }
            .bind(scope)
            .ok()?,
        );
    }
    if !result_needed {
        return Some(v8::Boolean::new(scope, true).into());
    }
    let first = input.snapshot(scope)?;
    let mut inputs = vec![first];
    if let Some(base) = parsed.base_url {
        inputs.push(v8_string(scope, &base)?.into());
    }
    let inputs = v8::Array::new_with_elements(scope, &inputs);
    let [
        protocol,
        username,
        password,
        hostname,
        port,
        pathname,
        search,
        hash,
    ]: [_; 8] = results.try_into().ok()?;
    MatchResult {
        hash,
        hostname,
        inputs,
        password,
        pathname,
        port,
        protocol,
        search,
        username,
    }
    .bind(scope)
    .ok()
    .map(Into::into)
}

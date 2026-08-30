use super::{
    identity::class_list_runtime_handle_and_kind_from_object,
    tokens::{class_list_tokens, set_class_list_tokens, token_list_attribute_name},
    *,
};
use crate::{util::throw_type_error, webidl};
use indexmap::IndexSet;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMTokenList.item")]
struct ClassListItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMTokenList")]
struct ClassListTokenArgs {
    #[webidl(required)]
    token: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMTokenList.add")]
struct ClassListAddArgs {
    #[webidl(variadic)]
    tokens: Vec<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMTokenList.remove")]
struct ClassListRemoveArgs {
    #[webidl(variadic)]
    tokens: Vec<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMTokenList.toggle")]
struct ClassListToggleArgs {
    #[webidl(required)]
    token: String,
    force: Option<bool>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMTokenList.replace")]
struct ClassListReplaceArgs {
    #[webidl(required)]
    from: String,
    #[webidl(required)]
    to: String,
}

const LINK_REL_LIST_SUPPORTED_TOKENS: &[&str] = &[
    "preload",
    "preconnect",
    "dns-prefetch",
    "stylesheet",
    "icon",
    "alternate",
    "prefetch",
    "prerender",
    "next",
    "manifest",
    "apple-touch-icon",
    "apple-touch-icon-precomposed",
    "canonical",
    "modulepreload",
    "allowed-alt-sxg",
    "compression-dictionary",
];

const NAVIGATION_REL_LIST_SUPPORTED_TOKENS: &[&str] = &["noreferrer", "noopener", "opener"];

const SANDBOX_SUPPORTED_TOKENS: &[&str] = &[
    "allow-downloads",
    "allow-forms",
    "allow-modals",
    "allow-orientation-lock",
    "allow-pointer-lock",
    "allow-popups",
    "allow-popups-to-escape-sandbox",
    "allow-presentation",
    "allow-same-origin",
    "allow-scripts",
    "allow-storage-access-by-user-activation",
    "allow-top-navigation",
    "allow-top-navigation-by-user-activation",
    "allow-top-navigation-to-custom-protocols",
];

fn rel_list_supports_token(runtime: &JsContextHost, handle: DomHandle, token: &str) -> bool {
    let supported_tokens = if runtime.dom_host().is_html_element_named(handle, "link") {
        LINK_REL_LIST_SUPPORTED_TOKENS
    } else if ["a", "area", "form"]
        .into_iter()
        .any(|name| runtime.dom_host().is_html_element_named(handle, name))
        || runtime
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.is_svg_element("a"))
    {
        NAVIGATION_REL_LIST_SUPPORTED_TOKENS
    } else {
        return false;
    };
    supported_tokens
        .iter()
        .any(|supported| token.eq_ignore_ascii_case(supported))
}

fn sandbox_supports_token(token: &str) -> bool {
    SANDBOX_SUPPORTED_TOKENS
        .iter()
        .any(|supported| token.eq_ignore_ascii_case(supported))
}

pub(super) fn class_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListItemArgs>(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(value) = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind)
        .get(parsed.index as usize)
        .and_then(|token| v8_string(scope, token))
    else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(super) fn class_list_contains_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListTokenArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let token = parsed.token;
    rv.set_bool(
        class_list_tokens(unsafe { &*runtime_ptr }, handle, kind)
            .iter()
            .any(|value| value == &token),
    );
}

pub(super) fn class_list_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListAddArgs>(scope, &args) else {
        return;
    };
    let had_attribute = element_attribute(
        unsafe { &*runtime_ptr },
        handle,
        token_list_attribute_name(kind),
    )
    .is_some();
    let mut tokens = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind);
    let mut changed = false;
    for token in parsed.tokens {
        if let Err((name, code, message)) = validate_class_list_token(&token) {
            throw_dom_exception(scope, name, code, message);
            return;
        }
        if !tokens.iter().any(|value| value == &token) {
            tokens.push(token);
            changed = true;
        }
    }
    // Spec: DOMTokenList update steps run only when something changed OR the
    // attribute already existed (the latter still re-serialises to normalise).
    if changed || had_attribute {
        set_class_list_tokens(scope, runtime_ptr, handle, kind, &tokens);
    }
}

pub(super) fn class_list_remove_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListRemoveArgs>(scope, &args) else {
        return;
    };
    let had_attribute = element_attribute(
        unsafe { &*runtime_ptr },
        handle,
        token_list_attribute_name(kind),
    )
    .is_some();
    let mut tokens = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind);
    for token in parsed.tokens {
        if let Err((name, code, message)) = validate_class_list_token(&token) {
            throw_dom_exception(scope, name, code, message);
            return;
        }
        tokens.retain(|value| value != &token);
    }
    // Spec: only re-serialise when the attribute previously existed (or when
    // we have content to write). If the class attribute was null and remove()
    // is a no-op, keep it null.
    if had_attribute || !tokens.is_empty() {
        set_class_list_tokens(scope, runtime_ptr, handle, kind, &tokens);
    }
}

pub(super) fn class_list_toggle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListToggleArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let token = parsed.token;
    if let Err((name, code, message)) = validate_class_list_token(&token) {
        throw_dom_exception(scope, name, code, message);
        return;
    }
    let mut tokens = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind);
    let contains = tokens.iter().any(|value| value == &token);
    let next_value = parsed.force.unwrap_or(!contains);
    let mutated;
    if next_value {
        if !contains {
            tokens.push(token);
            mutated = true;
        } else {
            mutated = false;
        }
    } else if contains {
        tokens.retain(|value| value != &token);
        mutated = true;
    } else {
        mutated = false;
    }
    // Spec: only update the attribute when the token list actually changed.
    // This preserves the original `class` attribute value (including
    // non-normalised whitespace) when toggle() is a no-op.
    if mutated {
        set_class_list_tokens(scope, runtime_ptr, handle, kind, &tokens);
    }
    rv.set_bool(next_value);
}

pub(super) fn class_list_replace_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListReplaceArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let from = parsed.from;
    let to = parsed.to;
    if let Err((name, code, message)) = validate_class_list_token_pair(&from, &to) {
        throw_dom_exception(scope, name, code, message);
        return;
    }
    let mut tokens = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind);
    let Some(position) = tokens.iter().position(|value| value == &from) else {
        rv.set_bool(false);
        return;
    };
    // Per DOM spec, replace() always re-serializes the attribute when from
    // is found — even if from == to or to is already present. This is the
    // distinguishing behaviour from toggle()'s no-op-skip optimisation.
    tokens[position] = to;
    tokens = dedupe_tokens(tokens);
    set_class_list_tokens(scope, runtime_ptr, handle, kind, &tokens);
    rv.set_bool(true);
}

fn dedupe_tokens(tokens: Vec<String>) -> Vec<String> {
    tokens
        .into_iter()
        .collect::<IndexSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn class_list_supports_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let Some(parsed) = webidl::parse_args::<ClassListTokenArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    match kind {
        DomTokenListKind::Class
        | DomTokenListKind::Part
        | DomTokenListKind::HtmlFor
        | DomTokenListKind::Sizes => {
            throw_type_error(scope, "DOMTokenList has no supported tokens.");
            return;
        }
        DomTokenListKind::Rel => {}
        DomTokenListKind::Sandbox => {
            rv.set_bool(sandbox_supports_token(&parsed.token));
            return;
        }
    }
    rv.set_bool(rel_list_supports_token(
        unsafe { &*runtime_ptr },
        handle,
        &parsed.token,
    ));
}

pub(super) fn class_list_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = element_attribute(
        unsafe { &*runtime_ptr },
        handle,
        token_list_attribute_name(kind),
    )
    .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

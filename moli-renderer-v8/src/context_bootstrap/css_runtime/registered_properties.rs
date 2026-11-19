use super::*;
use crate::{
    context_bootstrap::range::current_document_object,
    document_runtime::DomHandle,
    native_bridge::{callback_value_dom_handle, child_window_handle_from_marker_data},
    style_engine::{CssCustomPropertyRegistration, CssCustomPropertyRegistrationError},
    util::{get_private_value, set_private_value},
    webidl,
};
use moli_webapi::css_declarations::options::PropertyDefinition;
use style::stylist::RegisterCustomPropertyResult;

const CSS_OWNER_DOCUMENT_HANDLE_SLOT: &str = "__moliCssOwnerDocumentHandle";

pub(super) fn css_register_property_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let raw = args.get(0);
    let parsed = match webidl::parse_dictionary::<PropertyDefinition>(
        scope,
        raw,
        webidl::Context::argument("CSS.registerProperty", 1),
    ) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            webidl::throw_type_error(
                scope,
                "CSS.registerProperty requires a PropertyDefinition dictionary.",
            );
            return;
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };

    let registration = CssCustomPropertyRegistration {
        name: parsed.name,
        syntax: parsed.syntax,
        inherits: parsed.inherits,
        initial_value: parsed.initial_value,
    };

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(document_handle) = css_register_property_document_handle(scope, args.this()) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    match host
        .validate_css_custom_property_registration_for_document(document_handle, &registration)
    {
        RegisterCustomPropertyResult::SuccessfullyRegistered => {}
        RegisterCustomPropertyResult::AlreadyRegistered => {
            webidl::throw_dom_exception(
                scope,
                "InvalidModificationError",
                "CSS custom property is already registered.",
            );
            return;
        }
        error => {
            throw_css_property_registration_error(scope, error);
            return;
        }
    }

    match host.register_css_custom_property_for_document(document_handle, registration) {
        Ok(()) => {}
        Err(CssCustomPropertyRegistrationError::AlreadyRegistered) => {
            webidl::throw_dom_exception(
                scope,
                "InvalidModificationError",
                "CSS custom property is already registered.",
            );
        }
    }
}

pub(super) fn set_css_owner_document_handle(
    scope: &mut v8::PinScope<'_, '_>,
    css: v8::Local<'_, v8::Object>,
    document_handle: DomHandle,
) {
    let value = v8::BigInt::new_from_u64(scope, document_handle.index() as u64);
    set_private_value(scope, css, CSS_OWNER_DOCUMENT_HANDLE_SLOT, value.into());
}

fn css_register_property_document_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    css_owner_document_handle(scope, receiver).or_else(|| {
        current_document_object(scope)
            .and_then(|document| callback_value_dom_handle(scope, document.into()))
    })
}

fn css_owner_document_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, receiver, CSS_OWNER_DOCUMENT_HANDLE_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
}

fn throw_css_property_registration_error(
    scope: &mut v8::PinScope<'_, '_>,
    error: RegisterCustomPropertyResult,
) {
    let message = match error {
        RegisterCustomPropertyResult::InvalidName => {
            "CSS.registerProperty name must be a custom property name."
        }
        RegisterCustomPropertyResult::InvalidSyntax => {
            "CSS.registerProperty syntax is invalid or unsupported."
        }
        RegisterCustomPropertyResult::NoInitialValue => {
            "CSS.registerProperty requires initialValue for non-universal syntax."
        }
        RegisterCustomPropertyResult::InvalidInitialValue => {
            "CSS.registerProperty initialValue does not match syntax."
        }
        RegisterCustomPropertyResult::InitialValueNotComputationallyIndependent => {
            "CSS.registerProperty initialValue must be computationally independent."
        }
        RegisterCustomPropertyResult::AlreadyRegistered => {
            webidl::throw_dom_exception(
                scope,
                "InvalidModificationError",
                "CSS custom property is already registered.",
            );
            return;
        }
        RegisterCustomPropertyResult::SuccessfullyRegistered => {
            unreachable!("successful CSS.registerProperty validation is not an error")
        }
    };
    webidl::throw_dom_exception(scope, "SyntaxError", message);
}

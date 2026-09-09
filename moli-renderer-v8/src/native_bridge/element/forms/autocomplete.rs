use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AutofillCategory {
    None,
    Off,
    Automatic,
    Normal,
    Contact,
    Credential,
}

impl AutofillCategory {
    fn maximum_token_count(self) -> usize {
        match self {
            Self::None => 0,
            Self::Off | Self::Automatic => 1,
            Self::Normal => 3,
            Self::Contact => 4,
            Self::Credential => 5,
        }
    }
}

fn autofill_category(token: &str) -> AutofillCategory {
    match token {
        "off" => AutofillCategory::Off,
        "on" => AutofillCategory::Automatic,
        "name"
        | "honorific-prefix"
        | "given-name"
        | "additional-name"
        | "family-name"
        | "honorific-suffix"
        | "nickname"
        | "organization-title"
        | "username"
        | "new-password"
        | "current-password"
        | "one-time-code"
        | "organization"
        | "street-address"
        | "address-line1"
        | "address-line2"
        | "address-line3"
        | "address-level4"
        | "address-level3"
        | "address-level2"
        | "address-level1"
        | "country"
        | "country-name"
        | "postal-code"
        | "cc-name"
        | "cc-given-name"
        | "cc-additional-name"
        | "cc-family-name"
        | "cc-number"
        | "cc-exp"
        | "cc-exp-month"
        | "cc-exp-year"
        | "cc-csc"
        | "cc-type"
        | "transaction-currency"
        | "transaction-amount"
        | "language"
        | "bday"
        | "bday-day"
        | "bday-month"
        | "bday-year"
        | "sex"
        | "url"
        | "photo" => AutofillCategory::Normal,
        "tel" | "tel-country-code" | "tel-national" | "tel-area-code" | "tel-local"
        | "tel-local-prefix" | "tel-local-suffix" | "tel-extension" | "email" | "impp" => {
            AutofillCategory::Contact
        }
        "webauthn" => AutofillCategory::Credential,
        _ => AutofillCategory::None,
    }
}

fn is_html_ascii_whitespace(value: char) -> bool {
    matches!(value, '\t' | '\n' | '\u{000C}' | '\r' | ' ')
}

fn prepend_autofill_token(token: &str, value: String) -> String {
    format!("{token} {value}")
}

// HTML's autofill processing model defines the canonical value exposed by the
// autocomplete IDL attribute; it is not a plain DOMString reflection.
fn idl_exposed_autofill_value(raw: Option<&str>, wears_autofill_anchor_mantle: bool) -> String {
    let Some(raw) = raw else {
        return String::new();
    };
    let canonical = raw.to_ascii_lowercase();
    let tokens: Vec<&str> = canonical
        .split(is_html_ascii_whitespace)
        .filter(|token| !token.is_empty())
        .collect();
    let Some(field) = tokens.last() else {
        return String::new();
    };

    let mut index = tokens.len() - 1;
    let mut category = autofill_category(field);
    if category == AutofillCategory::None
        || tokens.len() > category.maximum_token_count()
        || (wears_autofill_anchor_mantle
            && matches!(
                category,
                AutofillCategory::Off | AutofillCategory::Automatic
            ))
    {
        return String::new();
    }
    if category == AutofillCategory::Off {
        return "off".to_owned();
    }
    if category == AutofillCategory::Automatic {
        return "on".to_owned();
    }

    let mut idl_value = (*field).to_owned();
    if category == AutofillCategory::Credential && index != 0 {
        index -= 1;
        category = autofill_category(tokens[index]);
        if !matches!(
            category,
            AutofillCategory::Normal | AutofillCategory::Contact
        ) || index > category.maximum_token_count() - 1
        {
            return String::new();
        }
        idl_value = prepend_autofill_token(tokens[index], idl_value);
    }

    if index != 0 {
        index -= 1;
        if category == AutofillCategory::Contact
            && matches!(tokens[index], "home" | "work" | "mobile" | "fax" | "pager")
        {
            idl_value = prepend_autofill_token(tokens[index], idl_value);
            if index == 0 {
                return idl_value;
            }
            index -= 1;
        }

        if matches!(tokens[index], "shipping" | "billing") {
            idl_value = prepend_autofill_token(tokens[index], idl_value);
            if index == 0 {
                return idl_value;
            }
            index -= 1;
        }

        if index != 0 || !tokens[index].starts_with("section-") {
            return String::new();
        }
        idl_value = prepend_autofill_token(tokens[index], idl_value);
    }

    idl_value
}

pub(crate) fn autocomplete_field_name(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    let wears_autofill_anchor_mantle =
        element.is_html_input() && element.input_type() == InputType::Hidden;
    let canonical = idl_exposed_autofill_value(
        element.attribute("autocomplete"),
        wears_autofill_anchor_mantle,
    );
    canonical
        .split_ascii_whitespace()
        .next_back()
        .map(str::to_owned)
}

fn autocomplete_getter_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    owner: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, receiver, owner, "autocomplete", local_name)
    else {
        rv.set_empty_string();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let wears_autofill_anchor_mantle = local_name == "input"
        && element_attribute(runtime, handle, "type")
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"));
    let raw = element_attribute(runtime, handle, "autocomplete");
    let value = idl_exposed_autofill_value(raw.as_deref(), wears_autofill_anchor_mantle);
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn set_autocomplete_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, receiver, owner, "autocomplete", local_name)
    else {
        return;
    };
    let Some(value) = form_dom_string_property_value(scope, value, owner, "autocomplete", false)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "autocomplete", &value);
}

macro_rules! autocomplete_accessors {
    ($getter:ident, $setter:ident, $owner:literal, $local_name:literal) => {
        pub(in crate::native_bridge) fn $getter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            autocomplete_getter_for_receiver(scope, args.this(), rv, $owner, $local_name);
        }

        pub(in crate::native_bridge) fn $setter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            set_autocomplete_for_receiver(scope, args.this(), args.get(0), $owner, $local_name);
            rv.set_undefined();
        }
    };
}

autocomplete_accessors!(
    input_autocomplete_getter_function,
    input_autocomplete_setter_function,
    "HTMLInputElement",
    "input"
);
autocomplete_accessors!(
    select_autocomplete_getter_function,
    select_autocomplete_setter_function,
    "HTMLSelectElement",
    "select"
);
autocomplete_accessors!(
    textarea_autocomplete_getter_function,
    textarea_autocomplete_setter_function,
    "HTMLTextAreaElement",
    "textarea"
);

#[cfg(test)]
mod tests {
    use super::idl_exposed_autofill_value;

    #[test]
    fn autofill_parser_canonicalizes_valid_field_combinations() {
        for (raw, expected) in [
            (" NAME\t", "name"),
            ("\u{000C}NAME", "name"),
            (" HOME\ntel", "home tel"),
            ("shipping   country", "shipping country"),
            ("billing  work  email", "billing work email"),
            ("  section-FOO  bday", "section-foo bday"),
            ("\tusername webauthn", "username webauthn"),
            (
                "section-LOGIN shipping work tel webauthn",
                "section-login shipping work tel webauthn",
            ),
        ] {
            assert_eq!(idl_exposed_autofill_value(Some(raw), false), expected);
        }
    }

    #[test]
    fn autofill_parser_rejects_invalid_and_anchor_mantle_values() {
        for raw in [
            "",
            " \n\t",
            "call-sign",
            "\u{000B}name",
            "foo off",
            "foo section-foo billing name",
            "foo section-bar billing work tel",
            "foo section-bar billing work tel webauthn",
        ] {
            assert_eq!(idl_exposed_autofill_value(Some(raw), false), "");
        }
        assert_eq!(idl_exposed_autofill_value(Some("on"), true), "");
        assert_eq!(idl_exposed_autofill_value(Some("off"), true), "");
        assert_eq!(idl_exposed_autofill_value(None, false), "");
    }
}

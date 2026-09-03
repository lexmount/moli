use anyhow::Result;

use super::ScriptVm;
use crate::{
    document_runtime::DomHandle,
    dom::{forms::InputType, native::Node},
    native_bridge::element::{
        autocomplete_field_name, autofill_related_form_control_elements, construct_simple_event,
        dispatch_public_event, form_control_is_effectively_disabled,
    },
    runtime::{
        RendererAutofillCreditCard, RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
    },
};

#[derive(Clone, Copy)]
enum CreditCardField {
    Number,
    FullName,
    GivenName,
    AdditionalName,
    FamilyName,
    ExpiryMonth,
    ExpiryYear,
    ExpiryDate,
    VerificationCode,
}

fn normalized_identity(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn explicit_credit_card_field(token: &str) -> Option<CreditCardField> {
    match token {
        "cc-number" => Some(CreditCardField::Number),
        "cc-name" => Some(CreditCardField::FullName),
        "cc-given-name" => Some(CreditCardField::GivenName),
        "cc-additional-name" => Some(CreditCardField::AdditionalName),
        "cc-family-name" => Some(CreditCardField::FamilyName),
        "cc-exp-month" => Some(CreditCardField::ExpiryMonth),
        "cc-exp-year" => Some(CreditCardField::ExpiryYear),
        "cc-exp" => Some(CreditCardField::ExpiryDate),
        "cc-csc" => Some(CreditCardField::VerificationCode),
        _ => None,
    }
}

fn inferred_credit_card_field(identity: &str) -> Option<CreditCardField> {
    match identity {
        "creditcardnumber" | "cardnumber" | "ccnumber" => Some(CreditCardField::Number),
        "creditcardnamefull" | "cardholdername" | "nameoncard" | "ccname" => {
            Some(CreditCardField::FullName)
        }
        "creditcardexpmonth" | "cardexpmonth" | "ccexpmonth" | "ccmonth" => {
            Some(CreditCardField::ExpiryMonth)
        }
        "creditcardexp2digityear"
        | "creditcardexp4digityear"
        | "creditcardexpyear"
        | "cardexpyear"
        | "ccexpyear"
        | "ccyear" => Some(CreditCardField::ExpiryYear),
        "creditcardexpdate2digityear"
        | "creditcardexpdate4digityear"
        | "creditcardexpiration"
        | "cardexpiration"
        | "ccexp" => Some(CreditCardField::ExpiryDate),
        "creditcardverificationcode"
        | "cardverificationcode"
        | "securitycode"
        | "cccsc"
        | "cccvc"
        | "csc"
        | "cvc"
        | "cvv" => Some(CreditCardField::VerificationCode),
        _ => None,
    }
}

fn credit_card_field(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> Option<CreditCardField> {
    if let Some(token) = autocomplete_field_name(runtime, handle)
        && let Some(field) = explicit_credit_card_field(&token)
    {
        return Some(field);
    }
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    [element.attribute("id"), element.attribute("name")]
        .into_iter()
        .flatten()
        .find_map(|identity| inferred_credit_card_field(&normalized_identity(identity)))
}

fn name_component(name: &str, field: CreditCardField) -> String {
    let components = name.split_whitespace().collect::<Vec<_>>();
    match field {
        CreditCardField::GivenName => components.first().copied().unwrap_or_default().to_owned(),
        CreditCardField::FamilyName => components.last().copied().unwrap_or_default().to_owned(),
        CreditCardField::AdditionalName => components
            .get(1..components.len().saturating_sub(1))
            .unwrap_or_default()
            .join(" "),
        _ => name.to_owned(),
    }
}

fn expiry_year_for_control(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    year: &str,
) -> String {
    let uses_two_digits = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(|element| element.attribute("maxlength"))
        .and_then(|value| value.parse::<usize>().ok())
        == Some(2);
    if uses_two_digits {
        year.chars()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        year.to_owned()
    }
}

fn credit_card_value(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    card: &RendererAutofillCreditCard,
) -> Option<String> {
    let field = credit_card_field(runtime, handle)?;
    Some(match field {
        CreditCardField::Number => card.number.clone(),
        CreditCardField::FullName => card.name.clone(),
        CreditCardField::GivenName
        | CreditCardField::AdditionalName
        | CreditCardField::FamilyName => name_component(&card.name, field),
        CreditCardField::ExpiryMonth => card.expiry_month.clone(),
        CreditCardField::ExpiryYear => expiry_year_for_control(runtime, handle, &card.expiry_year),
        CreditCardField::ExpiryDate => {
            let year = expiry_year_for_control(runtime, handle, &card.expiry_year);
            format!("{}/{}", card.expiry_month, year)
        }
        CreditCardField::VerificationCode => card.cvc.clone(),
    })
}

fn control_value(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if element.is_html_input() || element.is_html_textarea() {
        return Some(element.input_value());
    }
    if element.is_html_select() {
        return Some(element.select_value(runtime.dom_host(), handle, |option| {
            runtime
                .dom_host()
                .node(option)
                .and_then(Node::as_element)
                .is_some_and(|element| element.selected())
        }));
    }
    None
}

fn fill_credit_card_control(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
    card: &RendererAutofillCreditCard,
) -> bool {
    let Some((value, is_select, previous_value)) = (|| {
        let runtime = unsafe { &*runtime_ptr };
        let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
        if form_control_is_effectively_disabled(runtime, handle)
            || element.has_attribute("readonly")
            || (element.is_html_input()
                && matches!(element.input_type(), InputType::File | InputType::Hidden))
        {
            return None;
        }
        Some((
            credit_card_value(runtime, handle, card)?,
            element.is_html_select(),
            control_value(runtime, handle)?,
        ))
    })() else {
        return false;
    };

    if is_select {
        let _ = unsafe { &mut *runtime_ptr }.set_select_value(scope, runtime_ptr, handle, &value);
    } else {
        let _ = unsafe { &mut *runtime_ptr }.set_input_value(handle, &value);
    }
    let Some(current_value) = control_value(unsafe { &*runtime_ptr }, handle) else {
        return false;
    };
    if current_value != value {
        return false;
    }
    let _ = unsafe { &mut *runtime_ptr }.set_autofilled(handle, true);
    if current_value == previous_value {
        return true;
    }

    if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}

fn fill_credit_card(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    anchor: DomHandle,
    card: &RendererAutofillCreditCard,
) -> RendererAutofillTriggerOutcome {
    let controls = autofill_related_form_control_elements(unsafe { &*runtime_ptr }, anchor);
    if !controls.contains(&anchor) {
        return RendererAutofillTriggerOutcome::FieldNotFound;
    }
    let filled_field_count = controls
        .into_iter()
        .filter(|handle| fill_credit_card_control(scope, runtime_ptr, *handle, card))
        .count();
    RendererAutofillTriggerOutcome::Applied { filled_field_count }
}

impl ScriptVm {
    pub(crate) fn trigger_autofill(
        &mut self,
        anchor: DomHandle,
        request: RendererAutofillTriggerRequest,
    ) -> Result<RendererAutofillTriggerOutcome> {
        let anchor_is_form_control = {
            let runtime = self._context_host.borrow();
            autofill_related_form_control_elements(&runtime, anchor).contains(&anchor)
        };
        if !anchor_is_form_control {
            return Ok(RendererAutofillTriggerOutcome::FieldNotFound);
        }
        match (&request.card, &request.address) {
            (Some(_), Some(_)) => {
                return Ok(RendererAutofillTriggerOutcome::CardAndAddressProvided);
            }
            (None, None) => return Ok(RendererAutofillTriggerOutcome::MissingCardOrAddress),
            (None, Some(_)) => return Ok(RendererAutofillTriggerOutcome::AddressNotSupported),
            (Some(_), None) => {}
        }
        let card = request.card.expect("card presence checked above");
        let body = match self.child_execution_context_id_for_live_dom_handle(anchor) {
            Some(Some(execution_context_id)) => self.with_child_frame_realm_context_scope(
                execution_context_id,
                move |scope, runtime_ptr| Ok(fill_credit_card(scope, runtime_ptr, anchor, &card)),
            ),
            Some(None) => Ok(RendererAutofillTriggerOutcome::FieldNotFound),
            None => self.with_default_context_scope(move |scope, runtime_ptr| {
                Ok(fill_credit_card(scope, runtime_ptr, anchor, &card))
            }),
        };
        self.finish_devtools_live_dom_command(body)
    }
}

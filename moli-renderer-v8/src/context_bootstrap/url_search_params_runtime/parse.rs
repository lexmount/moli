use super::storage::{url_search_params_is_object, url_search_params_pairs};
use super::*;
use crate::context_bootstrap::url_form::url_href_slot;
use crate::webidl;
use moli_url::search_params::{SearchParamPair, parse_search_params};

pub(super) fn url_search_params_pairs_from_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Vec<SearchParamPair>> {
    if value.is_null_or_undefined() {
        return Some(Vec::new());
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        if form_data_is_object(scope, object) {
            return Some(
                form_data_entries(scope, object)
                    .into_iter()
                    .filter_map(|(key, value)| {
                        callback_value_string(scope, v8::Local::new(scope, &value))
                            .map(|value| (key, value))
                    })
                    .collect(),
            );
        }
        let sequence = match webidl::convert_optional_sequence::<webidl::Sequence<webidl::UsvString>>(
            scope,
            value,
            webidl::Context::argument("URLSearchParams", 1),
            &webidl::StringOptions::default(),
        ) {
            Ok(sequence) => sequence,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
        if let Some(sequence) = sequence {
            // Pair lengths are checked after the complete WebIDL conversion.
            let mut pairs = Vec::with_capacity(sequence.0.len());
            for pair in sequence.0 {
                let Ok([key, value]) = <[_; 2]>::try_from(pair.0) else {
                    webidl::throw_error(
                        scope,
                        &webidl::WebIdlError::custom_message(
                            "URLSearchParams sequence pairs must contain exactly two items",
                        ),
                    );
                    return None;
                };
                pairs.push((key.0, value.0));
            }
            return Some(pairs);
        }
        if url_search_params_is_object(scope, object) {
            return Some(url_search_params_pairs(scope, object));
        }
        if url_href_slot(scope, object).is_some() {
            return Some(
                callback_arg_url_like_string(scope, value)
                    .as_deref()
                    .map(parse_search_params)
                    .unwrap_or_default(),
            );
        }
        return record_string_pairs(scope, object.into());
    }
    Some(
        url_search_params_usv_string(scope, value)
            .as_deref()
            .map(parse_search_params)
            .unwrap_or_default(),
    )
}

fn record_string_pairs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Vec<SearchParamPair>> {
    match webidl::convert::<webidl::Record<webidl::UsvString, webidl::UsvString>>(
        scope,
        value,
        webidl::Context::argument("URLSearchParams", 1),
    ) {
        Ok(record) => Some(
            record
                .0
                .into_iter()
                .map(|(key, value)| (key.0, value.0))
                .collect(),
        ),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn url_search_params_usv_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    match webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::argument("URLSearchParams", 1),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

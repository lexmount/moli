use super::*;
use crate::webidl;
use moli_url::search_params::{SearchParamPair, parse_search_params};

struct UrlSearchParamsSequencePair(SearchParamPair);

impl<'s> webidl::WebIdlConverter<'s> for UrlSearchParamsSequencePair {
    type Options = webidl::StringOptions;

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let pair = <webidl::Sequence<webidl::UsvString> as webidl::WebIdlConverter>::convert(
            scope, value, context, options,
        )?;
        if pair.0.len() != 2 {
            return Err(webidl::WebIdlError::custom_message(
                "URLSearchParams sequence pairs must contain exactly two items",
            ));
        }
        let mut values = pair.0.into_iter();
        let key = values.next().expect("validated sequence pair key").0;
        let value = values.next().expect("validated sequence pair value").0;
        Ok(Self((key, value)))
    }
}

pub(super) fn url_search_params_pairs_from_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Vec<SearchParamPair>> {
    if value.is_null_or_undefined() {
        return Some(Vec::new());
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        let sequence = match webidl::convert_optional_sequence::<UrlSearchParamsSequencePair>(
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
            return Some(sequence.0.into_iter().map(|pair| pair.0).collect());
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

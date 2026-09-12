use super::entries::normalized_header_entry_or_throw;
use crate::webidl;

pub(in crate::network_host) fn headers_entries_from_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init_arg: v8::Local<'s, v8::Value>,
) -> Result<Vec<(String, String)>, webidl::WebIdlError> {
    if init_arg.is_undefined() {
        return Ok(vec![]);
    }
    if init_arg.is_null() {
        return Err(webidl::WebIdlError::custom_message(
            "Headers initializer must be an object",
        ));
    }

    let Ok(init_obj) = v8::Local::<v8::Object>::try_from(init_arg) else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers initializer must be an object",
        ));
    };

    if let Some(entries) = headers_entries_from_iterable_init(scope, init_arg, init_obj)? {
        return Ok(entries);
    }

    let record = webidl::convert::<webidl::Record<webidl::ByteString, webidl::ByteString>>(
        scope,
        init_obj.into(),
        webidl::Context::argument("Headers", 1),
    )?;
    let mut entries = Vec::with_capacity(record.0.len());
    for (key, value) in record.0 {
        let Some(entry) = normalized_header_entry_or_throw(scope, key.into(), value.into()) else {
            return Err(webidl::WebIdlError::custom_message(
                "Headers initializer contains an invalid header",
            ));
        };
        entries.push(entry);
    }
    Ok(entries)
}

fn headers_entries_from_iterable_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init_arg: v8::Local<'s, v8::Value>,
    init_obj: v8::Local<'s, v8::Object>,
) -> Result<Option<Vec<(String, String)>>, webidl::WebIdlError> {
    let iterator_key = v8::Symbol::get_iterator(scope);
    let Some(iterator_value) = webidl::symbol_property_result(
        scope,
        init_obj,
        iterator_key,
        webidl::Context::member("Headers", "@@iterator"),
    )?
    else {
        return Ok(None);
    };
    if iterator_value.is_null_or_undefined() {
        return Ok(None);
    }
    let Ok(iterator_method) = v8::Local::<v8::Function>::try_from(iterator_value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers iterable initializer @@iterator must be callable",
        ));
    };
    let Some(iterator_value) = call_function_result(
        scope,
        iterator_method,
        init_arg,
        &[],
        webidl::Context::member("Headers", "@@iterator"),
    )?
    else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers iterable initializer did not return an iterator",
        ));
    };
    let Ok(iterator) = v8::Local::<v8::Object>::try_from(iterator_value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers iterable initializer did not return an iterator",
        ));
    };
    let Some(next_method) = webidl::property_result(
        scope,
        iterator,
        "next",
        webidl::Context::member("Headers", "next"),
    )?
    else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers iterable initializer iterator must have next()",
        ));
    };
    let Ok(next_method) = v8::Local::<v8::Function>::try_from(next_method) else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers iterable initializer iterator must have next()",
        ));
    };

    let mut entries = Vec::new();
    loop {
        let Some(step_value) = call_function_result(
            scope,
            next_method,
            iterator.into(),
            &[],
            webidl::Context::member("Headers", "next"),
        )?
        else {
            return Err(webidl::WebIdlError::custom_message(
                "Headers iterable initializer next() must return an object",
            ));
        };
        let Ok(step) = v8::Local::<v8::Object>::try_from(step_value) else {
            return Err(webidl::WebIdlError::custom_message(
                "Headers iterable initializer next() must return an object",
            ));
        };
        let done = webidl::property_result(
            scope,
            step,
            "done",
            webidl::Context::member("Headers", "done"),
        )?
        .is_some_and(|value| value.boolean_value(scope));
        if done {
            break;
        }
        let Some(pair) = webidl::property_result(
            scope,
            step,
            "value",
            webidl::Context::member("Headers", "value"),
        )?
        else {
            return Err(webidl::WebIdlError::custom_message(
                "Headers iterable initializer could not read an entry",
            ));
        };
        entries.push(header_sequence_pair_from_value(scope, pair)?);
    }
    Ok(Some(entries))
}

fn header_sequence_pair_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pair_val: v8::Local<'s, v8::Value>,
) -> Result<(String, String), webidl::WebIdlError> {
    let Ok(pair) = v8::Local::<v8::Object>::try_from(pair_val) else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers sequence initializer must contain pairs",
        ));
    };
    if webidl::property_result(
        scope,
        pair,
        "length",
        webidl::Context::member("Headers", "length"),
    )?
    .and_then(|value| value.uint32_value(scope))
        != Some(2)
    {
        return Err(webidl::WebIdlError::custom_message(
            "Headers sequence initializer pairs must have length 2",
        ));
    }
    let key_value = get_index_result(scope, pair, 0)?;
    let key = header_init_byte_string(scope, key_value, "name")?;
    let value_value = get_index_result(scope, pair, 1)?;
    let value = header_init_byte_string(scope, value_value, "value")?;
    normalized_header_entry_or_throw(scope, key, value).ok_or_else(|| {
        webidl::WebIdlError::custom_message("Headers initializer contains an invalid header")
    })
}

fn header_init_byte_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
    member: &'static str,
) -> Result<String, webidl::WebIdlError> {
    let value = value.ok_or_else(|| {
        webidl::WebIdlError::custom_message("Headers initializer could not read a value")
    })?;
    webidl::convert::<webidl::ByteString>(scope, value, webidl::Context::member("Headers", member))
        .map(Into::into)
}

fn get_index_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
) -> Result<Option<v8::Local<'s, v8::Value>>, webidl::WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match object.get_index(&scope, index) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(webidl::WebIdlError::pending_exception(
                webidl::Context::member("Headers", "sequence item"),
            ))
        }
        None => Ok(None),
    }
}

fn call_function_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
    context: webidl::Context,
) -> Result<Option<v8::Local<'s, v8::Value>>, webidl::WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match function.call(&scope, receiver, args) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(webidl::WebIdlError::pending_exception(context))
        }
        None => Ok(None),
    }
}

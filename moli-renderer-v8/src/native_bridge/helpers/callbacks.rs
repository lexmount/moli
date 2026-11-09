pub(crate) use self::callback_arg_optional_string as callback_arg_namespace;

pub(crate) fn callback_arg_optional_string(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> Option<String> {
    let value = args.get(index);
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(crate) fn encode_tag_name_ns_query(namespace: Option<&str>, local_name: &str) -> String {
    let namespace = namespace.unwrap_or_default();
    format!("{namespace}\u{0}{local_name}")
}

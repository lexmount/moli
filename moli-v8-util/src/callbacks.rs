use v8::{FunctionCallbackArguments, Integer, Local, PinScope, Value};

pub fn callback_data_index(
    scope: &mut PinScope<'_, '_>,
    args: &FunctionCallbackArguments<'_>,
) -> Option<usize> {
    args.data()
        .int32_value(scope)
        .and_then(|value| usize::try_from(value).ok())
}

pub fn indexed_callback_data<T: Copy>(items: &[T], index: usize, label: &'static str) -> Option<T> {
    debug_assert!(
        index < items.len(),
        "callback data index {index} out of bounds for {label} (len {})",
        items.len()
    );
    items.get(index).copied()
}

pub fn callback_data_item<T: Copy>(
    scope: &mut PinScope<'_, '_>,
    args: &FunctionCallbackArguments<'_>,
    items: &[T],
    label: &'static str,
) -> Option<T> {
    callback_data_index(scope, args).and_then(|index| indexed_callback_data(items, index, label))
}

pub fn callback_data_index_value<'s>(
    scope: &mut PinScope<'s, '_, ()>,
    index: usize,
) -> Local<'s, Value> {
    let index = i32::try_from(index).expect("callback data index exceeds i32");
    Integer::new(scope, index).into()
}

/// Forward a converted first argument while retaining the original trailing values.
pub fn callback_arguments_with_first<'s>(
    args: &FunctionCallbackArguments<'s>,
    first: Local<'s, Value>,
) -> Vec<Local<'s, Value>> {
    let mut values = Vec::with_capacity(args.length().max(1) as usize);
    values.push(first);
    for index in 1..args.length() {
        values.push(args.get(index));
    }
    values
}

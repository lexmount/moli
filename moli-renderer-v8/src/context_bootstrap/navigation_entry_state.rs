use super::history_runtime::native;
use super::*;
use crate::structured_clone::{deserialize_history_state, serialize_history_state};

pub(super) fn navigation_entry_state_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = native::entry(scope, entry)?
        .borrow()
        .navigation_state
        .clone()?;
    deserialize_history_state(scope, &state)
}

pub(super) fn history_entry_state_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = native::entry(scope, entry)?
        .borrow()
        .history_state
        .clone()?;
    deserialize_history_state(scope, &state)
}

pub(super) fn clone_navigation_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    navigation_entry_state_snapshot(scope, entry)
}

pub(super) fn clone_history_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    history_entry_state_snapshot(scope, entry)
}

pub(super) fn copy_entry_serialized_states<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: v8::Local<'s, v8::Object>,
    to: v8::Local<'s, v8::Object>,
) {
    let Some(from) = native::entry(scope, from) else {
        return;
    };
    let Some(to) = native::entry(scope, to) else {
        return;
    };
    let (history_state, navigation_state) = {
        let from = from.borrow();
        (from.history_state.clone(), from.navigation_state.clone())
    };
    let mut to = to.borrow_mut();
    to.history_state = history_state;
    to.navigation_state = navigation_state;
}

pub(super) fn set_navigation_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let Some(record) = native::entry(scope, entry) else {
        return;
    };
    if let Some(state) = serialize_history_state(scope, state) {
        record.borrow_mut().navigation_state = Some(state);
    }
}

pub(super) fn clone_navigation_state_arg_for_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<'s, v8::Object>>,
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    let Some(raw_state) =
        options.and_then(|options| options.get(scope, v8str(scope, "state").into()))
    else {
        return Ok(None);
    };
    if raw_state.is_undefined() {
        return Ok(None);
    }

    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let cloned_state = structured_clone_value_for_storage(&mut scope, raw_state);
    if let Some(error) = scope.exception() {
        scope.reset();
        return Err(error);
    }
    Ok(cloned_state)
}

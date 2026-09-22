use super::location_history_storage::{
    HISTORY_ENTRY_STATE_SNAPSHOT_SLOT, NAVIGATION_ENTRY_STATE_SNAPSHOT_SLOT,
};
use super::navigation_entry::{
    navigation_entry_state_slot_value, set_navigation_entry_private_slot_value,
};
use super::*;

pub(super) fn navigation_entry_state_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    navigation_entry_state_slot_value(scope, entry, NAVIGATION_ENTRY_STATE_SNAPSHOT_SLOT)
        .filter(|value| !value.is_undefined())
}

pub(super) fn history_entry_state_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    navigation_entry_state_slot_value(scope, entry, HISTORY_ENTRY_STATE_SNAPSHOT_SLOT)
}

pub(super) fn clone_navigation_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let snapshot = navigation_entry_state_snapshot(scope, entry)?;
    structured_clone_value(scope, snapshot).or(Some(snapshot))
}

pub(super) fn clone_history_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let snapshot = history_entry_state_snapshot(scope, entry)?;
    structured_clone_value(scope, snapshot).or(Some(snapshot))
}

pub(super) fn set_history_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    set_navigation_entry_private_slot_value(scope, entry, HISTORY_ENTRY_STATE_SNAPSHOT_SLOT, state);
}

pub(super) fn set_navigation_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let exposed_state = structured_clone_value(scope, state).unwrap_or(state);
    set_navigation_entry_private_slot_value(
        scope,
        entry,
        NAVIGATION_ENTRY_STATE_SNAPSHOT_SLOT,
        state,
    );
    set_navigation_entry_private_slot_value(scope, entry, "state", exposed_state);
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

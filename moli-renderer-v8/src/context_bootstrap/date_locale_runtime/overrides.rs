use crate::util::date_locale_runtime_state_from_context;

/// Copies the target-owned Date/Intl emulation state for the current realm.
///
/// Every Window realm installed by a `ScriptVm` receives the same reference-
/// counted runtime-state slot, so main, child, and isolated worlds observe one
/// applied renderer state without mirroring values into each V8 global object
/// or dereferencing the re-entrant `JsContextHost` callback pointer.
pub(super) fn current_date_locale_overrides(
    scope: &mut v8::PinScope<'_, '_>,
) -> (Option<String>, Option<String>) {
    let context = scope.get_current_context();
    let Some(state) = date_locale_runtime_state_from_context(context) else {
        return (None, None);
    };
    state.snapshot()
}

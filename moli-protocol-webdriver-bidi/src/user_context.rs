use moli_protocol::devtools_runtime::DevToolsBrowserContextId;

pub(crate) const DEFAULT_BIDI_USER_CONTEXT: &str = "default";
pub(crate) const DEFAULT_MOLI_BROWSER_CONTEXT_ID: &str = "BID-default";

pub(crate) fn bidi_user_context_to_browser_context_id(
    user_context: &str,
) -> Option<DevToolsBrowserContextId> {
    (user_context != DEFAULT_BIDI_USER_CONTEXT)
        .then(|| DevToolsBrowserContextId::from(user_context))
}

pub(crate) fn bidi_user_context_from_browser_context_id(browser_context_id: Option<&str>) -> &str {
    match browser_context_id {
        Some(browser_context_id) if browser_context_id != DEFAULT_MOLI_BROWSER_CONTEXT_ID => {
            browser_context_id
        }
        _ => DEFAULT_BIDI_USER_CONTEXT,
    }
}

pub(crate) fn explicit_bidi_user_context_to_browser_context_id(
    user_context: &str,
) -> DevToolsBrowserContextId {
    if user_context == DEFAULT_BIDI_USER_CONTEXT {
        DevToolsBrowserContextId::from(DEFAULT_MOLI_BROWSER_CONTEXT_ID)
    } else {
        DevToolsBrowserContextId::from(user_context)
    }
}

use crate::automation::{DevToolsError, DevToolsErrorKind};
use crate::conn::CdpConnection;

pub(crate) fn resolve_bidi_browser_context_ids(
    conn: &mut CdpConnection,
    browser_context_ids: &[crate::automation::DevToolsBrowserContextId],
) -> Result<Vec<String>, DevToolsError> {
    let mut resolved = Vec::new();
    for browser_context_id in browser_context_ids {
        let browser_context_id = browser_context_id.as_str();
        if browser_context_id == "default" {
            let mut default_context_ids = conn
                .browser_contexts()
                .filter(|context| is_moli_internal_default_user_context(&context.id))
                .map(|context| context.id.clone())
                .collect::<Vec<_>>();
            if default_context_ids.is_empty() {
                let id = conn.default_browser_context_id().to_owned();
                conn.insert_browser_context(conn.new_browser_context(id.clone()));
                default_context_ids.push(id);
            }
            resolved.extend(default_context_ids);
            continue;
        }
        if !conn.has_browser_context_id(browser_context_id) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            ));
        }
        resolved.push(browser_context_id.to_owned());
    }
    resolved.sort();
    resolved.dedup();
    Ok(resolved)
}

pub(crate) fn is_moli_internal_default_user_context(browser_context_id: &str) -> bool {
    browser_context_id == "BID-default"
        || browser_context_id
            .strip_prefix("BID-")
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
}

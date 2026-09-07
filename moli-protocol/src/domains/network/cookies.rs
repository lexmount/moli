use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::*;
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsCommand, DevToolsDeleteCookiesCommand,
    DevToolsGetCookiesCommand, DevToolsSetCookiesCommand,
};
use crate::domains::storage::{
    StorageCommandTaskStep, normalize_partition_key, start_devtools_storage_command,
};

pub(super) fn start_get_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_network_get_cookies_command(conn, cmd, true) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::GetCookies(command))
}

pub(super) fn start_get_all_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_network_get_cookies_command(conn, cmd, false) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::GetCookies(command))
}

fn build_cdp_network_get_cookies_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    parse_urls: bool,
) -> Result<DevToolsGetCookiesCommand, CommandOutputPlan> {
    let urls = if parse_urls {
        network_get_cookie_urls(cmd)?
    } else {
        None
    };
    let (_, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(_, target_id)| ((), target_id))
        .unwrap_or(((), None));
    Ok(DevToolsGetCookiesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), Option::<&str>::None),
        browser_context_id: None,
        urls,
        filter: None,
    })
}

fn network_get_cookie_urls(cmd: &Cmd<'_>) -> Result<Option<Vec<String>>, CommandOutputPlan> {
    let Some(urls) = cmd
        .params
        .and_then(|params| params.get("urls"))
        .and_then(Value::as_array)
    else {
        return Ok(None);
    };
    let mut parsed = Vec::new();
    for url in urls.iter().filter_map(Value::as_str) {
        if Url::parse(url).is_err() {
            return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
        }
        parsed.push(url.to_owned());
    }
    Ok(Some(parsed))
}

pub(super) fn start_delete_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_network_delete_cookies_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::DeleteCookies(command))
}

pub(super) fn start_clear_browser_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let (_, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(_, target_id)| ((), target_id))
        .unwrap_or(((), None));
    let command = DevToolsDeleteCookiesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), Option::<&str>::None),
        browser_context_id: None,
        name: None,
        url: None,
        domain: None,
        path: None,
        partition_key: None,
        filter: None,
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::DeleteCookies(command))
}

fn build_cdp_network_delete_cookies_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsDeleteCookiesCommand, CommandOutputPlan> {
    let params: DeleteCookiesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    let partition_key = normalize_partition_key(params.partition_key.as_ref(), false)
        .map_err(|_| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    let (_, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(_, target_id)| ((), target_id))
        .unwrap_or(((), None));
    let browser_context_id = params
        .browser_context_id
        .map(DevToolsBrowserContextId::from);
    Ok(DevToolsDeleteCookiesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.clone()),
        browser_context_id,
        name: params.name,
        url: params.url,
        domain: params.domain,
        path: params.path,
        partition_key,
        filter: None,
    })
}

pub(super) fn start_set_cookie_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let cookie: CdpCookieParam = match cmd.get_params() {
        Ok(Some(cookie)) => cookie,
        _ => {
            return StorageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let command = build_devtools_set_cookies_command(conn, cmd, None, vec![cookie]);
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::SetCookies(command))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetCookiesParams {
    cookies: Vec<CdpCookieParam>,
}

pub(super) fn start_set_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let params: SetCookiesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return StorageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let command = build_devtools_set_cookies_command(conn, cmd, None, params.cookies);
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::SetCookies(command))
}

fn build_devtools_set_cookies_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    browser_context_id: Option<String>,
    cookies: Vec<CdpCookieParam>,
) -> DevToolsSetCookiesCommand {
    let (_, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(_, target_id)| ((), target_id))
        .unwrap_or(((), None));
    let browser_context_id = browser_context_id.map(DevToolsBrowserContextId::from);
    DevToolsSetCookiesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.clone()),
        browser_context_id,
        cookies: cookies.into_iter().map(Into::into).collect(),
    }
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::DevToolsProtocol;
    use serde_json::{Value, json};

    use crate::conn::Cmd;
    use crate::domains::storage::StorageCommandTaskStep;

    use super::{
        build_cdp_network_delete_cookies_command, build_cdp_network_get_cookies_command,
        start_clear_browser_cookies_command, start_delete_cookies_command,
        start_get_cookies_command, start_set_cookie_command,
    };

    #[test]
    fn cdp_network_get_cookies_builds_protocol_neutral_command_with_urls() {
        let conn = crate::test_support::connection();
        let params = json!({
            "urls": [
                "https://example.com/",
                42,
                "https://example.org/path"
            ]
        });
        let cmd = Cmd::for_test(
            Some(142),
            "Network.getCookies",
            &params,
            Some("SID-network"),
            r#"{"id":142,"method":"Network.getCookies"}"#,
        );

        let command = build_cdp_network_get_cookies_command(&conn, &cmd, true);
        let Ok(command) = command else {
            panic!("valid Network.getCookies command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-network")
        );
        assert_eq!(command.browser_context_id, None);
        assert_eq!(
            command.urls,
            Some(vec![
                "https://example.com/".to_owned(),
                "https://example.org/path".to_owned()
            ])
        );
    }

    #[test]
    fn cdp_network_get_cookies_rejects_invalid_url_params_before_owner_entry() {
        let conn = crate::test_support::connection();
        let params = json!({
            "urls": ["not a url"]
        });
        let cmd = Cmd::for_test(
            Some(143),
            "Network.getCookies",
            &params,
            None,
            r#"{"id":143,"method":"Network.getCookies"}"#,
        );

        let command = build_cdp_network_get_cookies_command(&conn, &cmd, true);

        let Err(plan) = command else {
            panic!("invalid URL should be rejected while building the shared command");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(143));
        assert_eq!(out[0]["error"]["code"], json!(-32602));
        assert_eq!(out[0]["error"]["message"], json!("InvalidParams"));
    }

    #[test]
    fn network_get_cookies_routes_to_shared_storage_entry() {
        let mut conn = crate::test_support::connection();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(144),
            "Network.getCookies",
            &params,
            None,
            r#"{"id":144,"method":"Network.getCookies"}"#,
        );

        let step = start_get_cookies_command(&mut conn, &cmd);

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("Network.getCookies should route through shared storage entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(144));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn cdp_network_delete_cookies_builds_protocol_neutral_command() {
        let conn = crate::test_support::connection();
        let params = json!({
            "name": "sid",
            "url": "https://example.com/",
            "domain": "example.com",
            "path": "/",
            "partitionKey": {
                "topLevelSite": "https://top.example",
                "hasCrossSiteAncestor": true
            }
        });
        let cmd = Cmd::for_test(
            Some(147),
            "Network.deleteCookies",
            &params,
            Some("SID-network"),
            r#"{"id":147,"method":"Network.deleteCookies"}"#,
        );

        let command = build_cdp_network_delete_cookies_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid Network.deleteCookies command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-network")
        );
        assert_eq!(command.name.as_deref(), Some("sid"));
        assert_eq!(command.url.as_deref(), Some("https://example.com/"));
        assert_eq!(command.domain.as_deref(), Some("example.com"));
        assert_eq!(command.path.as_deref(), Some("/"));
        assert_eq!(
            command.partition_key,
            Some(moli_cookie_jar::StoredCookiePartitionKey::site(
                "https://top.example".to_owned(),
                true,
            ))
        );
    }

    #[test]
    fn network_delete_cookies_routes_to_shared_storage_entry() {
        let mut conn = crate::test_support::connection();
        let params = json!({ "name": "sid" });
        let cmd = Cmd::for_test(
            Some(148),
            "Network.deleteCookies",
            &params,
            None,
            r#"{"id":148,"method":"Network.deleteCookies"}"#,
        );

        let step = start_delete_cookies_command(&mut conn, &cmd);

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("Network.deleteCookies should route through shared storage entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(148));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn network_clear_browser_cookies_routes_to_shared_storage_entry() {
        let mut conn = crate::test_support::connection();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(154),
            "Network.clearBrowserCookies",
            &params,
            None,
            r#"{"id":154,"method":"Network.clearBrowserCookies"}"#,
        );

        let step = start_clear_browser_cookies_command(&mut conn, &cmd);

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("Network.clearBrowserCookies should route through shared storage entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(154));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn network_set_cookie_routes_to_shared_storage_entry() {
        let mut conn = crate::test_support::connection();
        let params = json!({
            "name": "sid",
            "value": "1",
            "url": "https://example.com/"
        });
        let cmd = Cmd::for_test(
            Some(151),
            "Network.setCookie",
            &params,
            None,
            r#"{"id":151,"method":"Network.setCookie"}"#,
        );

        let step = start_set_cookie_command(&mut conn, &cmd);

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("missing browser context should fail synchronously");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(151));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }
}

use moli_cookie_jar::{StoredCookieDomainMatch, StoredCookiePartitionFilter};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::*;
use crate::automation::{
    AutomationCommand, DevToolsBrowserContextId, DevToolsDeleteCookiesCommand,
    DevToolsGetCookiesCommand, DevToolsSetCookiesCommand,
};
use crate::domains::storage::{StorageCommandTaskStep, start_devtools_storage_command};

pub(super) fn start_get_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_network_get_cookies_command(conn, cmd, true) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, AutomationCommand::GetCookies(command))
}

pub(super) fn start_get_all_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_network_get_cookies_command(conn, cmd, false) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, AutomationCommand::GetCookies(command))
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
        context: cmd.automation_context(target_id.as_deref(), Option::<&str>::None),
        scope: Default::default(),
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
    start_devtools_storage_command(conn, cmd.id, AutomationCommand::DeleteCookies(command))
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
        context: cmd.automation_context(target_id.as_deref(), Option::<&str>::None),
        scope: crate::automation::DevToolsCookieScope {
            browser_context_id: None,
            partition_filter: StoredCookiePartitionFilter::All,
        },
        filter: crate::automation::DevToolsCookieDeleteFilter::Scope {
            name: None,
            url: None,
            domain: None,
            domain_match: StoredCookieDomainMatch::Canonical,
            path: None,
        },
    };
    start_devtools_storage_command(conn, cmd.id, AutomationCommand::DeleteCookies(command))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkDeleteCookiesParams {
    name: String,
    #[serde(default, deserialize_with = "deserialize_present")]
    url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    domain: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    partition_key: Option<NetworkCookiePartitionKey>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkCookiePartitionKey {
    top_level_site: String,
    has_cross_site_ancestor: bool,
}

// Optional CDP fields may be absent, but explicit null still has to satisfy
// the field's type, just like other malformed JSON values.
fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn build_cdp_network_delete_cookies_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsDeleteCookiesCommand, CommandOutputPlan> {
    let params: NetworkDeleteCookiesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    // Chromium uses a nonempty domain verbatim and only falls back to the URL
    // hostname when the domain is absent or empty.
    let domain = match params.domain.filter(|domain| !domain.is_empty()) {
        Some(domain) => domain,
        None => {
            let url = params.url.as_deref().and_then(|url| Url::parse(url).ok());
            let Some(url) = url.filter(|url| matches!(url.scheme(), "http" | "https")) else {
                return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
            };
            let Some(host) = url.host_str() else {
                return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
            };
            host.to_owned()
        }
    };
    // Match the stored serialization without parsing or canonicalizing the
    // client's string. A malformed or noncanonical site naturally matches none.
    let partition_filter =
        params
            .partition_key
            .map_or(StoredCookiePartitionFilter::Unpartitioned, |key| {
                StoredCookiePartitionFilter::Serialized {
                    top_level_site: Some(key.top_level_site),
                    has_cross_site_ancestor: Some(key.has_cross_site_ancestor),
                }
            });
    let (_, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(_, target_id)| ((), target_id))
        .unwrap_or(((), None));
    Ok(DevToolsDeleteCookiesCommand {
        context: cmd.automation_context(target_id.as_deref(), Option::<&str>::None),
        scope: crate::automation::DevToolsCookieScope {
            browser_context_id: None,
            partition_filter,
        },
        filter: crate::automation::DevToolsCookieDeleteFilter::Scope {
            name: Some(params.name),
            url: None,
            domain: Some(domain),
            domain_match: StoredCookieDomainMatch::Serialized,
            path: params.path.filter(|path| !path.is_empty()),
        },
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
    start_devtools_storage_command(conn, cmd.id, AutomationCommand::SetCookies(command))
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
    start_devtools_storage_command(conn, cmd.id, AutomationCommand::SetCookies(command))
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
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.clone()),
        scope: crate::automation::DevToolsCookieScope {
            browser_context_id,
            ..Default::default()
        },
        cookies: cookies.into_iter().map(Into::into).collect(),
    }
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::automation::FrontendProtocol;
    use serde_json::{Value, json};

    use crate::conn::{CdpConnection, Cmd};
    use crate::domains::storage::StorageCommandTaskStep;

    use super::{
        build_cdp_network_delete_cookies_command, build_cdp_network_get_cookies_command,
        start_clear_browser_cookies_command, start_delete_cookies_command,
        start_get_cookies_command, start_set_cookie_command,
    };

    #[test]
    fn cdp_network_get_cookies_builds_protocol_neutral_command_with_urls() {
        let conn = CdpConnection::new();
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

        assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-network")
        );
        assert_eq!(command.scope.browser_context_id, None);
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
        let conn = CdpConnection::new();
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
        let mut conn = CdpConnection::new();
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
        let conn = CdpConnection::new();
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

        assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-network")
        );
        let crate::automation::DevToolsCookieDeleteFilter::Scope {
            name,
            url,
            domain,
            domain_match,
            path,
        } = command.filter
        else {
            panic!("scope filter");
        };
        assert_eq!(name.as_deref(), Some("sid"));
        assert_eq!(url, None);
        assert_eq!(domain.as_deref(), Some("example.com"));
        assert_eq!(domain_match, super::StoredCookieDomainMatch::Serialized);
        assert_eq!(command.scope.browser_context_id, None);
        assert_eq!(path.as_deref(), Some("/"));
        assert_eq!(
            command.scope.partition_filter,
            moli_cookie_jar::StoredCookiePartitionFilter::Serialized {
                top_level_site: Some("https://top.example".to_owned()),
                has_cross_site_ancestor: Some(true),
            }
        );
    }

    #[test]
    fn network_delete_cookies_routes_to_shared_storage_entry() {
        let mut conn = CdpConnection::new();
        let params = json!({ "name": "sid", "url": "https://example.com/" });
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
    fn cdp_network_delete_cookies_preserves_unparseable_partition_key_for_storage_matching() {
        let conn = CdpConnection::new();
        // Shape sent by Puppeteer's page.setCookie()/deleteCookie().
        let params = json!({
            "name": "sid",
            "url": "https://example.com/",
            "partitionKey": {
                "topLevelSite": "https//example.com",
                "hasCrossSiteAncestor": false
            }
        });
        let cmd = Cmd::for_test(
            Some(149),
            "Network.deleteCookies",
            &params,
            None,
            r#"{"id":149,"method":"Network.deleteCookies"}"#,
        );

        let Ok(command) = build_cdp_network_delete_cookies_command(&conn, &cmd) else {
            panic!("a structurally valid partitionKey should reach storage matching");
        };
        assert_eq!(
            command.scope.partition_filter,
            moli_cookie_jar::StoredCookiePartitionFilter::Serialized {
                top_level_site: Some("https//example.com".to_owned()),
                has_cross_site_ancestor: Some(false),
            }
        );
    }

    #[test]
    fn cdp_network_delete_cookies_rejects_invalid_partition_key_structure() {
        for partition_key in [
            Value::Null,
            json!("https://example.com"),
            json!(42),
            json!(false),
            json!([]),
            json!({}),
            json!({ "topLevelSite": "https://example.com" }),
            json!({ "hasCrossSiteAncestor": false }),
            json!({ "topLevelSite": 42, "hasCrossSiteAncestor": false }),
            json!({ "topLevelSite": null, "hasCrossSiteAncestor": false }),
            json!({ "topLevelSite": "https://example.com", "hasCrossSiteAncestor": "false" }),
            json!({ "topLevelSite": "https://example.com", "hasCrossSiteAncestor": null }),
            // Structural errors take priority even when the site cannot be parsed.
            json!({ "topLevelSite": "https//example.com" }),
            json!({ "topLevelSite": "https//example.com", "hasCrossSiteAncestor": 0 }),
        ] {
            let mut conn = CdpConnection::new();
            let params = json!({
                "name": "sid",
                "url": "https://example.com/",
                "partitionKey": partition_key,
            });
            let cmd = Cmd::for_test(
                Some(150),
                "Network.deleteCookies",
                &params,
                Some("SID-network"),
                r#"{"id":150,"method":"Network.deleteCookies"}"#,
            );
            let StorageCommandTaskStep::Complete(plan) =
                start_delete_cookies_command(&mut conn, &cmd)
            else {
                panic!("invalid partitionKey structure must fail before storage dispatch");
            };
            let mut out = Vec::new();
            plan.emit_into(&mut out, cmd.id, cmd.session_id);
            assert_eq!(out[0]["error"]["code"], json!(-32602), "{params}");
            assert_eq!(
                out[0]["error"]["message"],
                json!("InvalidParams"),
                "{params}"
            );
            assert_eq!(out[0]["sessionId"], json!("SID-network"));
        }
    }

    #[test]
    fn network_clear_browser_cookies_routes_to_shared_storage_entry() {
        let mut conn = CdpConnection::new();
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
        let mut conn = CdpConnection::new();
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

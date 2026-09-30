use moli_protocol::automation::{
    AutomationCommand, DevToolsCookieParam, DevToolsDeleteCookiesCommand,
    DevToolsGetCookiesCommand, DevToolsGetCookiesResult, DevToolsSetCookiesCommand,
};
use serde_json::{Value, json};

use crate::{ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode};

use super::parsing::{
    optional_object_bool, optional_object_expiry, optional_object_string, required_object_string,
};

pub fn get_cookies_command(
    context: &ClassicDevToolsCommandContext,
    current_url: impl Into<String>,
) -> AutomationCommand {
    AutomationCommand::GetCookies(DevToolsGetCookiesCommand {
        context: context.command_context(),
        browser_context_id: None,
        urls: Some(vec![current_url.into()]),
        filter: None,
    })
}

pub fn add_cookie_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    current_url: impl Into<String>,
) -> Result<AutomationCommand, ClassicError> {
    let cookie = params
        .get("cookie")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "cookie must be an object",
            )
        })?;
    let name = required_object_string(cookie, "name")?.to_owned();
    let value = required_object_string(cookie, "value")?.to_owned();
    let same_site = optional_object_string(cookie, "sameSite")?.map(str::to_owned);
    if let Some(same_site) = same_site.as_deref()
        && !matches!(same_site, "None" | "Lax" | "Strict")
    {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "sameSite must be None, Lax, or Strict",
        ));
    }
    Ok(AutomationCommand::SetCookies(DevToolsSetCookiesCommand {
        context: context.command_context(),
        browser_context_id: None,
        cookies: vec![DevToolsCookieParam {
            name,
            value,
            url: Some(current_url.into()),
            domain: optional_object_string(cookie, "domain")?.map(str::to_owned),
            path: optional_object_string(cookie, "path")?.map(str::to_owned),
            secure: Some(optional_object_bool(cookie, "secure")?.unwrap_or(false)),
            http_only: optional_object_bool(cookie, "httpOnly")?.unwrap_or(false),
            same_site,
            priority: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
            partition_key_opaque: None,
            expires: optional_object_expiry(cookie)?,
        }],
    }))
}

pub fn delete_all_cookies_command(
    context: &ClassicDevToolsCommandContext,
    current_url: impl Into<String>,
) -> AutomationCommand {
    AutomationCommand::DeleteCookies(DevToolsDeleteCookiesCommand {
        context: context.command_context(),
        browser_context_id: None,
        name: None,
        url: Some(current_url.into()),
        domain: None,
        path: None,
        partition_key: None,
        filter: None,
    })
}

pub fn delete_cookie_command(
    context: &ClassicDevToolsCommandContext,
    name: impl Into<String>,
    current_url: impl Into<String>,
) -> AutomationCommand {
    AutomationCommand::DeleteCookies(DevToolsDeleteCookiesCommand {
        context: context.command_context(),
        browser_context_id: None,
        name: Some(name.into()),
        url: Some(current_url.into()),
        domain: None,
        path: None,
        partition_key: None,
        filter: None,
    })
}

pub fn classic_cookies_from_devtools(result: DevToolsGetCookiesResult) -> Vec<Value> {
    result.cookies.into_iter().map(classic_cookie).collect()
}

pub fn classic_cookie_by_name(
    result: DevToolsGetCookiesResult,
    name: &str,
) -> Result<Value, ClassicError> {
    classic_cookies_from_devtools(result)
        .into_iter()
        .find(|cookie| cookie.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| ClassicError::new(ClassicErrorCode::NoSuchCookie, "cookie not found"))
}

fn classic_cookie(cookie: Value) -> Value {
    let mut classic = json!({
        "name": cookie.get("name").and_then(Value::as_str).unwrap_or_default(),
        "value": cookie.get("value").and_then(Value::as_str).unwrap_or_default(),
        "path": cookie.get("path").and_then(Value::as_str).unwrap_or("/"),
        "domain": cookie.get("domain").and_then(Value::as_str).unwrap_or_default(),
        "secure": cookie.get("secure").and_then(Value::as_bool).unwrap_or(false),
        "httpOnly": cookie.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
    });
    classic["sameSite"] = json!(
        cookie
            .get("sameSite")
            .and_then(Value::as_str)
            .unwrap_or("None")
    );
    if let Some(expires) = cookie.get("expires").and_then(Value::as_f64)
        && expires >= 0.0
    {
        classic["expiry"] = json!(expires.trunc() as i64);
    }
    classic
}

use moli_protocol::automation::{
    DevToolsBrowserContextId, DevToolsCookieFilter, DevToolsCookieParam,
    DevToolsDeleteCookiesCommand, DevToolsGetCookiesCommand, DevToolsSetCookiesCommand,
    DevToolsTargetId,
};
use serde_json::{Value, json};

use super::{BidiCommand, BidiDevToolsCommandContext, BidiError, BidiErrorCode};
use crate::commands::{
    optional_object_bool, optional_object_string, optional_object_uint, optional_object_value,
    required_network_bytes_value, required_object_string, required_object_value,
};
use crate::user_context::bidi_user_context_to_browser_context_id;

#[derive(Debug, Clone)]
struct BidiStoragePartition {
    target_id: Option<DevToolsTargetId>,
    browser_context_id: Option<DevToolsBrowserContextId>,
}

pub(crate) fn bidi_get_cookies_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsGetCookiesCommand, BidiError> {
    let partition = bidi_storage_partition(&command.params, context)?;
    let command_context = context.command_context_with_browser_context_id(
        partition.target_id.clone(),
        partition.browser_context_id.clone(),
    );
    Ok(DevToolsGetCookiesCommand {
        context: command_context,
        browser_context_id: partition.browser_context_id,
        urls: None,
        filter: bidi_cookie_filter(command.params.get("filter"))?,
    })
}

pub(crate) fn bidi_set_cookie_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetCookiesCommand, BidiError> {
    let partition = bidi_storage_partition(&command.params, context)?;
    let command_context = context.command_context_with_browser_context_id(
        partition.target_id.clone(),
        partition.browser_context_id.clone(),
    );
    let Some(cookie) = command.params.get("cookie").and_then(Value::as_object) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "cookie must be an object",
        ));
    };
    let same_site = optional_object_string(cookie, "sameSite")?
        .map(bidi_cookie_same_site_to_cdp)
        .transpose()?
        .flatten();
    Ok(DevToolsSetCookiesCommand {
        context: command_context,
        browser_context_id: partition.browser_context_id,
        cookies: vec![DevToolsCookieParam {
            name: required_object_string(cookie, "name")?.to_owned(),
            value: required_network_bytes_value(required_object_value(cookie, "value")?, "value")?,
            url: None,
            domain: Some(required_object_string(cookie, "domain")?.to_owned()),
            path: optional_object_string(cookie, "path")?.map(str::to_owned),
            secure: optional_object_bool(cookie, "secure")?,
            http_only: optional_object_bool(cookie, "httpOnly")?.unwrap_or(false),
            same_site,
            priority: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
            partition_key_opaque: None,
            expires: optional_object_uint(cookie, "expiry")?.map(|expiry| expiry as f64),
        }],
    })
}

pub(crate) fn bidi_delete_cookies_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsDeleteCookiesCommand, BidiError> {
    let partition = bidi_storage_partition(&command.params, context)?;
    let command_context = context.command_context_with_browser_context_id(
        partition.target_id.clone(),
        partition.browser_context_id.clone(),
    );
    let filter = bidi_cookie_filter(command.params.get("filter"))?;
    Ok(DevToolsDeleteCookiesCommand {
        context: command_context,
        browser_context_id: partition.browser_context_id,
        name: filter.as_ref().and_then(|filter| filter.name.clone()),
        url: None,
        domain: filter.as_ref().and_then(|filter| filter.domain.clone()),
        path: filter.as_ref().and_then(|filter| filter.path.clone()),
        partition_key: None,
        filter,
    })
}

pub(crate) fn bidi_cookie_from_cdp_cookie(cookie: Value) -> Value {
    let expires = cookie.get("expires").and_then(Value::as_f64);
    let mut bidi = json!({
        "name": cookie.get("name").and_then(Value::as_str).unwrap_or_default(),
        "value": {
            "type": "string",
            "value": cookie.get("value").and_then(Value::as_str).unwrap_or_default(),
        },
        "domain": cookie
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim_start_matches('.'),
        "path": cookie.get("path").and_then(Value::as_str).unwrap_or("/"),
        "size": cookie.get("size").and_then(Value::as_u64).unwrap_or_else(|| {
            cookie.get("name").and_then(Value::as_str).unwrap_or_default().len() as u64
                + cookie.get("value").and_then(Value::as_str).unwrap_or_default().len() as u64
        }),
        "httpOnly": cookie.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
        "secure": cookie.get("secure").and_then(Value::as_bool).unwrap_or(false),
        "sameSite": bidi_same_site_from_cdp_cookie(&cookie),
    });
    if let Some(expires) = expires.filter(|value| value.is_finite() && *value >= 0.0) {
        bidi["expiry"] = json!(expires.trunc() as i64);
    }
    bidi
}

fn bidi_storage_partition(
    params: &Value,
    context: &BidiDevToolsCommandContext,
) -> Result<BidiStoragePartition, BidiError> {
    let mut partition = BidiStoragePartition {
        target_id: None,
        browser_context_id: context
            .browser_context_id
            .as_deref()
            .map(DevToolsBrowserContextId::from),
    };
    let Some(value) = params.get("partition") else {
        return Ok(partition);
    };
    let Some(descriptor) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "partition must be an object",
        ));
    };
    match required_object_string(descriptor, "type")? {
        "context" => {
            partition.target_id = Some(DevToolsTargetId::from(required_object_string(
                descriptor, "context",
            )?));
        }
        "storageKey" => {
            if let Some(user_context) = optional_object_string(descriptor, "userContext")? {
                partition.browser_context_id =
                    bidi_user_context_to_browser_context_id(user_context);
            }
            optional_object_string(descriptor, "sourceOrigin")?;
        }
        _ => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "partition type must be context or storageKey",
            ));
        }
    }
    Ok(partition)
}

fn bidi_cookie_filter(value: Option<&Value>) -> Result<Option<DevToolsCookieFilter>, BidiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(filter) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "filter must be an object",
        ));
    };
    let same_site = optional_object_string(filter, "sameSite")?
        .map(bidi_cookie_same_site_for_filter)
        .transpose()?;
    Ok(Some(DevToolsCookieFilter {
        name: optional_object_string(filter, "name")?.map(str::to_owned),
        value: optional_object_value(filter, "value")
            .map(|value| required_network_bytes_value(value, "filter.value"))
            .transpose()?,
        domain: optional_object_string(filter, "domain")?.map(str::to_owned),
        path: optional_object_string(filter, "path")?.map(str::to_owned),
        secure: optional_object_bool(filter, "secure")?,
        http_only: optional_object_bool(filter, "httpOnly")?,
        same_site,
        size: optional_object_uint(filter, "size")?,
        expires: optional_object_uint(filter, "expiry")?
            .and_then(|value| i64::try_from(value).ok()),
    }))
}

fn bidi_cookie_same_site_to_cdp(value: &str) -> Result<Option<String>, BidiError> {
    match value {
        "none" => Ok(Some("None".to_owned())),
        "lax" => Ok(Some("Lax".to_owned())),
        "strict" => Ok(Some("Strict".to_owned())),
        "default" => Ok(None),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "sameSite must be none, lax, strict, or default",
        )),
    }
}

fn bidi_cookie_same_site_for_filter(value: &str) -> Result<String, BidiError> {
    match value {
        "none" | "lax" | "strict" | "default" => Ok(value.to_owned()),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "sameSite must be none, lax, strict, or default",
        )),
    }
}

fn bidi_same_site_from_cdp_cookie(cookie: &Value) -> &'static str {
    match cookie.get("sameSite").and_then(Value::as_str) {
        Some("None") => "none",
        Some("Lax") => "lax",
        Some("Strict") => "strict",
        _ => "default",
    }
}

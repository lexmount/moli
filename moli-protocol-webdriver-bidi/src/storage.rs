use moli_protocol::automation::{
    AutomationContext, DevToolsBrowserContextId, DevToolsCookieDeleteFilter, DevToolsCookieFilter,
    DevToolsCookieParam, DevToolsCookieScope, DevToolsDeleteCookiesCommand,
    DevToolsGetCookiesCommand, DevToolsSetCookiesCommand, DevToolsTargetId,
    StoredCookiePartitionFilter,
};
use serde_json::{Value, json};
use url::Url;

use super::{BidiCommand, BidiDevToolsCommandContext, BidiError, BidiErrorCode};
use crate::commands::{
    optional_object_bool, optional_object_string, optional_object_uint, optional_object_value,
    required_network_bytes_value, required_object_string, required_object_value,
};
use crate::user_context::bidi_user_context_to_browser_context_id;

#[derive(Debug, Clone)]
struct BidiStoragePartition {
    target_id: Option<DevToolsTargetId>,
    scope: DevToolsCookieScope,
}

impl BidiStoragePartition {
    fn command_context(&self, context: &BidiDevToolsCommandContext) -> AutomationContext {
        context.command_context_with_browser_context_id(
            self.target_id.clone(),
            self.scope.browser_context_id.clone(),
        )
    }
}

pub(crate) fn bidi_get_cookies_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsGetCookiesCommand, BidiError> {
    let partition = bidi_storage_partition(&command.params, context)?;
    let command_context = partition.command_context(context);
    Ok(DevToolsGetCookiesCommand {
        context: command_context,
        scope: partition.scope,
        urls: None,
        filter: bidi_cookie_filter(command.params.get("filter"))?,
    })
}

pub(crate) fn bidi_set_cookie_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetCookiesCommand, BidiError> {
    let partition = bidi_storage_partition(&command.params, context)?;
    let command_context = partition.command_context(context);
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
    let domain = required_object_string(cookie, "domain")?;
    let secure = optional_object_bool(cookie, "secure")?;
    // BiDi storage writes address the supplied cookie domain, independently of
    // the currently active document in the selected user context.
    let write_url = format!(
        "{}://{}/",
        if secure == Some(true) {
            "https"
        } else {
            "http"
        },
        domain.trim_start_matches('.')
    );
    let cookie = DevToolsCookieParam {
        name: required_object_string(cookie, "name")?.to_owned(),
        value: required_network_bytes_value(required_object_value(cookie, "value")?, "value")?,
        url: Some(write_url),
        domain: Some(domain.to_owned()),
        path: optional_object_string(cookie, "path")?.map(str::to_owned),
        secure,
        http_only: optional_object_bool(cookie, "httpOnly")?.unwrap_or(false),
        same_site,
        priority: None,
        source_scheme: None,
        source_port: None,
        // The shared write boundary derives the partition from scope.
        partition_key: None,
        partition_key_opaque: None,
        expires: optional_object_uint(cookie, "expiry")?.map(|expiry| expiry as f64),
    };
    if matches!(
        &partition.scope.partition_filter,
        StoredCookiePartitionFilter::Serialized {
            top_level_site: None,
            has_cross_site_ancestor: Some(_)
        }
    ) {
        return Err(BidiError::new(
            BidiErrorCode::UnableToSetCookie,
            "Cannot set goog:hasCrossSiteAncestor without sourceOrigin",
        ));
    }
    Ok(DevToolsSetCookiesCommand {
        context: command_context,
        scope: partition.scope,
        cookies: vec![cookie],
    })
}

pub(crate) fn bidi_delete_cookies_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsDeleteCookiesCommand, BidiError> {
    let partition = bidi_storage_partition(&command.params, context)?;
    let command_context = partition.command_context(context);
    let filter = bidi_cookie_filter(command.params.get("filter"))?;
    Ok(DevToolsDeleteCookiesCommand {
        context: command_context,
        scope: partition.scope,
        filter: DevToolsCookieDeleteFilter::Properties(filter.unwrap_or_default()),
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
        scope: DevToolsCookieScope {
            browser_context_id: context
                .browser_context_id
                .as_deref()
                .map(DevToolsBrowserContextId::from),
            ..Default::default()
        },
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
            partition.scope.browser_context_id = None;
            partition.target_id = Some(DevToolsTargetId::from(required_object_string(
                descriptor, "context",
            )?));
        }
        "storageKey" => {
            if let Some(user_context) = optional_object_string(descriptor, "userContext")? {
                partition.scope.browser_context_id =
                    bidi_user_context_to_browser_context_id(user_context);
            }
            let top_level_site = optional_object_string(descriptor, "sourceOrigin")?
                .map(|origin| -> Result<String, BidiError> {
                    let url = Url::parse(origin).map_err(|_| {
                        BidiError::new(
                            BidiErrorCode::InvalidArgument,
                            "sourceOrigin must be a valid absolute URL",
                        )
                    })?;
                    // Chrome's BiDi storage descriptor retains the hostname,
                    // including subdomains, and compares it to the serialized
                    // cookie site. Canonicalizing to eTLD+1 here would broaden
                    // get/delete beyond Chrome's selection.
                    Ok(format!(
                        "{}://{}",
                        url.scheme(),
                        url.host_str().unwrap_or_default()
                    ))
                })
                .transpose()?;
            let has_cross_site_ancestor =
                optional_object_bool(descriptor, "goog:hasCrossSiteAncestor")?;
            if top_level_site.is_some() || has_cross_site_ancestor.is_some() {
                partition.scope.partition_filter = StoredCookiePartitionFilter::Serialized {
                    top_level_site,
                    has_cross_site_ancestor,
                };
            }
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
            .map(|value| {
                i64::try_from(value).map_err(|_| {
                    BidiError::new(
                        BidiErrorCode::InvalidArgument,
                        "expiry must fit in a signed 64-bit timestamp",
                    )
                })
            })
            .transpose()?,
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

pub(crate) fn bidi_cookie_partition_key(scope: &DevToolsCookieScope) -> Value {
    let mut result = json!({
        "userContext": crate::user_context::bidi_user_context_from_browser_context_id(
            scope.browser_context_id.as_ref().map(DevToolsBrowserContextId::as_str)),
    });
    if let StoredCookiePartitionFilter::Serialized {
        top_level_site,
        has_cross_site_ancestor,
    } = &scope.partition_filter
    {
        if let Some(site) = top_level_site {
            result["sourceOrigin"] = json!(site);
        }
        if let Some(ancestor) = has_cross_site_ancestor {
            result["goog:hasCrossSiteAncestor"] = json!(ancestor);
        }
    }
    result
}

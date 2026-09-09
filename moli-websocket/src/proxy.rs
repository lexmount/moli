use url::Url;

use crate::ConnectOptions;

pub(crate) fn websocket_proxy_url(
    url: &Url,
    context: &ConnectOptions,
) -> Result<Option<Url>, String> {
    websocket_proxy_url_with_env(url, context, |name| std::env::var(name).ok())
}

pub(crate) fn websocket_proxy_url_with_env(
    url: &Url,
    context: &ConnectOptions,
    mut env: impl FnMut(&str) -> Option<String>,
) -> Result<Option<Url>, String> {
    let proxy = match context.http_proxy.as_deref() {
        Some("") => return Ok(None),
        Some(proxy) => Some(proxy.to_owned()),
        None => websocket_env_proxy_for_scheme(url.scheme(), &mut env),
    };
    let Some(proxy) = proxy.filter(|proxy| !proxy.is_empty()) else {
        return Ok(None);
    };
    let host = url
        .host_str()
        .ok_or_else(|| "WebSocket URL is missing host".to_owned())?;
    let no_proxy = match context.http_no_proxy.as_deref() {
        Some(no_proxy) => Some(no_proxy.to_owned()),
        None => websocket_env_no_proxy(&mut env),
    };
    if no_proxy_matches(host, url.port(), no_proxy.as_deref()) {
        return Ok(None);
    }
    let proxy_url = Url::parse(&proxy)
        .map_err(|error| format!("failed to parse WebSocket proxy URL `{proxy}`: {error}"))?;
    if proxy_url.scheme() != "http" {
        return Err(format!(
            "unsupported WebSocket proxy scheme `{}`; only http proxies are supported",
            proxy_url.scheme()
        ));
    }
    if proxy_url.host_str().is_none() {
        return Err("WebSocket proxy URL is missing host".to_owned());
    }
    Ok(Some(proxy_url))
}

fn websocket_env_proxy_for_scheme(
    scheme: &str,
    env: &mut impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    let mut names: &[&str] = match scheme {
        "ws" => &["http_proxy"],
        "wss" => &["https_proxy", "HTTPS_PROXY"],
        _ => &[],
    };
    for name in names {
        if let Some(value) = env(name).filter(|value| !value.is_empty()) {
            return Some(value);
        }
    }
    names = &["all_proxy", "ALL_PROXY"];
    for name in names {
        if let Some(value) = env(name).filter(|value| !value.is_empty()) {
            return Some(value);
        }
    }
    None
}

fn websocket_env_no_proxy(env: &mut impl FnMut(&str) -> Option<String>) -> Option<String> {
    env("no_proxy")
        .filter(|value| !value.is_empty())
        .or_else(|| env("NO_PROXY").filter(|value| !value.is_empty()))
}

pub(crate) fn no_proxy_matches(host: &str, port: Option<u16>, no_proxy: Option<&str>) -> bool {
    let Some(no_proxy) = no_proxy else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    no_proxy.split(',').any(|token| {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        if token == "*" {
            return true;
        }
        let (token_host, token_port) = split_no_proxy_host_port(token);
        if let Some(token_port) = token_port
            && Some(token_port) != port
        {
            return false;
        }
        let token_host = token_host
            .trim_matches(['[', ']'])
            .trim_start_matches('.')
            .to_ascii_lowercase();
        !token_host.is_empty()
            && (host == token_host
                || host
                    .strip_suffix(&token_host)
                    .is_some_and(|prefix| prefix.ends_with('.')))
    })
}

fn split_no_proxy_host_port(token: &str) -> (&str, Option<u16>) {
    let Some((host, port)) = token.rsplit_once(':') else {
        return (token, None);
    };
    match port.parse::<u16>() {
        Ok(port) if !host.contains(':') => (host, Some(port)),
        _ => (token, None),
    }
}

pub(crate) fn append_proxy_connect_header(
    request: &mut String,
    name: &str,
    value: &str,
) -> Result<(), String> {
    if value.bytes().any(|byte| matches!(byte, b'\r' | b'\n')) {
        return Err(format!(
            "invalid WebSocket proxy CONNECT header `{name}` contains a newline"
        ));
    }
    request.push_str(name);
    request.push_str(": ");
    request.push_str(value);
    request.push_str("\r\n");
    Ok(())
}

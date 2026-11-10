pub(crate) fn resolve_service_worker_script_url(
    document_url: &url::Url,
    script: &str,
) -> Option<url::Url> {
    let mut script_url = document_url.join(script).ok()?;
    script_url.set_fragment(None);
    Some(script_url)
}

pub(crate) fn service_worker_scope_url(
    document_url: &url::Url,
    script_url: &url::Url,
    requested_scope: Option<&str>,
) -> Option<url::Url> {
    let Some(requested_scope) = requested_scope else {
        return Some(default_service_worker_scope_url(script_url));
    };
    let mut scope_url = document_url.join(requested_scope).ok()?;
    scope_url.set_fragment(None);
    Some(scope_url)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientUrlError {
    Invalid,
    CrossOrigin,
}

pub(crate) fn service_worker_client_url(
    api_base_url: &url::Url,
    client_origin: &moli_url::WebOrigin,
    client: &str,
) -> std::result::Result<url::Url, ServiceWorkerClientUrlError> {
    let mut client_url = api_base_url
        .join(client)
        .map_err(|_| ServiceWorkerClientUrlError::Invalid)?;
    client_url.set_fragment(None);
    if !client_origin.same_origin_url(&client_url) {
        return Err(ServiceWorkerClientUrlError::CrossOrigin);
    }
    Ok(client_url)
}

pub(crate) fn default_service_worker_scope_url(script_url: &url::Url) -> url::Url {
    let mut scope_url = script_url.clone();
    let path = script_url.path();
    let scope_path = path.rfind('/').map(|index| &path[..=index]).unwrap_or("/");
    scope_url.set_path(scope_path);
    scope_url.set_query(None);
    scope_url.set_fragment(None);
    scope_url
}

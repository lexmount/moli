use moli_fetch::{FetchCancelHandle, RequestCredentialsMode};
use moli_url::{origin_ascii_serialization, same_origin};
use serde_json::{Map, Value};
use url::Url;

use crate::{
    network::{
        RendererNetworkResourceLoadOutcome, RendererPreparedNetworkResourceLoad,
        context::DocumentResourceLoaderIdentity, loads::ResourceLoadLease,
    },
    types::{
        SubresourceNetworkRecord, SubresourceRequestInitiatorType, SubresourceResourceType,
        SubresourceResponseBody,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererAppManifestDisplayMode {
    Undefined,
    Browser,
    MinimalUi,
    Standalone,
    Fullscreen,
    WindowControlsOverlay,
    Tabbed,
    Borderless,
    PictureInPicture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererAppManifestOrientation {
    Default,
    Any,
    Natural,
    Landscape,
    LandscapePrimary,
    LandscapeSecondary,
    Portrait,
    PortraitPrimary,
    PortraitSecondary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifestImageResource {
    pub url: String,
    pub sizes: String,
    pub mime_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifestProtocolHandler {
    pub protocol: String,
    pub url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifestRelatedApplication {
    pub id: Option<String>,
    pub url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifestShortcut {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifest {
    pub background_color: Option<String>,
    pub description: Option<String>,
    pub display: RendererAppManifestDisplayMode,
    pub display_overrides: Vec<RendererAppManifestDisplayMode>,
    pub icons: Vec<RendererAppManifestImageResource>,
    pub id: String,
    pub name: Option<String>,
    pub orientation: RendererAppManifestOrientation,
    pub prefer_related_applications: bool,
    pub protocol_handlers: Vec<RendererAppManifestProtocolHandler>,
    pub related_applications: Vec<RendererAppManifestRelatedApplication>,
    pub scope: String,
    pub shortcuts: Vec<RendererAppManifestShortcut>,
    pub start_url: String,
    pub theme_color: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifestError {
    pub message: String,
    pub critical: i32,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAppManifestQueryResult {
    pub url: String,
    pub errors: Vec<RendererAppManifestError>,
    pub data: Option<String>,
    pub manifest: RendererAppManifest,
}

pub enum RendererAppManifestLoadPreparation {
    Complete(Box<RendererAppManifestQueryResult>),
    Ready(Box<RendererPreparedAppManifestLoad>),
}

pub struct RendererAppManifestLoadOutcome {
    result: RendererAppManifestQueryResult,
    publication: RendererAppManifestLoadPublication,
}

pub struct RendererAppManifestLoadPublication {
    network_record: SubresourceNetworkRecord,
    successful_result: Option<RendererAppManifestSuccessfulResult>,
}

struct RendererAppManifestSuccessfulResult {
    link_identity: RendererAppManifestLinkIdentity,
    result: RendererAppManifestQueryResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererAppManifestLinkIdentity {
    node_id: u32,
    rel: String,
    href: String,
    resolved_url: Url,
    link_change_epoch: u64,
    // Async fetch publication must still belong to the Document that started it.
    document_resource_loader: Option<DocumentResourceLoaderIdentity>,
}

impl RendererAppManifestLinkIdentity {
    pub(crate) fn new(
        node_id: u32,
        rel: String,
        href: String,
        resolved_url: Url,
        link_change_epoch: u64,
        document_resource_loader: Option<DocumentResourceLoaderIdentity>,
    ) -> Self {
        Self {
            node_id,
            rel,
            href,
            resolved_url,
            link_change_epoch,
            document_resource_loader,
        }
    }
}

impl RendererAppManifestLoadOutcome {
    pub fn into_parts(
        self,
    ) -> (
        RendererAppManifestQueryResult,
        RendererAppManifestLoadPublication,
    ) {
        (self.result, self.publication)
    }
}

impl RendererAppManifestLoadPublication {
    pub(crate) fn into_parts(
        self,
    ) -> (
        SubresourceNetworkRecord,
        Option<(
            RendererAppManifestLinkIdentity,
            RendererAppManifestQueryResult,
        )>,
    ) {
        let successful_result = self
            .successful_result
            .map(|successful| (successful.link_identity, successful.result));
        (self.network_record, successful_result)
    }
}

pub struct RendererPreparedAppManifestLoad {
    document_url: Url,
    requested_manifest_url: Url,
    link_identity: RendererAppManifestLinkIdentity,
    resource: RendererPreparedNetworkResourceLoad,
    observation: RendererAppManifestNetworkObservation,
}

pub(crate) struct RendererAppManifestNetworkObservation {
    frame_id: Option<String>,
    request_headers: Vec<(String, String)>,
    credentials_mode: RequestCredentialsMode,
    load: ResourceLoadLease,
    cancel_handle: FetchCancelHandle,
}

impl RendererAppManifestNetworkObservation {
    pub(crate) fn new(
        frame_id: Option<String>,
        request_headers: Vec<(String, String)>,
        credentials_mode: RequestCredentialsMode,
        load: ResourceLoadLease,
        cancel_handle: FetchCancelHandle,
    ) -> Self {
        Self {
            frame_id,
            request_headers,
            credentials_mode,
            load,
            cancel_handle,
        }
    }
}

impl RendererPreparedAppManifestLoad {
    pub(crate) fn new(
        document_url: Url,
        requested_manifest_url: Url,
        link_identity: RendererAppManifestLinkIdentity,
        resource: RendererPreparedNetworkResourceLoad,
        observation: RendererAppManifestNetworkObservation,
    ) -> Self {
        Self {
            document_url,
            requested_manifest_url,
            link_identity,
            resource,
            observation,
        }
    }

    pub async fn execute(self) -> RendererAppManifestLoadOutcome {
        let Self {
            document_url,
            requested_manifest_url,
            link_identity,
            resource,
            observation,
        } = self;
        let RendererAppManifestNetworkObservation {
            frame_id,
            request_headers,
            credentials_mode,
            load,
            cancel_handle,
        } = observation;
        let outcome = resource.execute_with_cancel(cancel_handle).await;
        drop(load);
        let (result, network_record, cacheable) = match outcome {
            RendererNetworkResourceLoadOutcome::FailedBeforeResponse(error) => (
                default_query_result(&document_url, Some(&requested_manifest_url)),
                SubresourceNetworkRecord::failure(
                    frame_id,
                    document_url.clone(),
                    requested_manifest_url,
                    "GET".to_owned(),
                    request_headers,
                    None,
                    SubresourceResourceType::Manifest,
                    error,
                )
                .with_request_initiator_type(SubresourceRequestInitiatorType::Other),
                false,
            ),
            RendererNetworkResourceLoadOutcome::Response(response) => {
                let response = *response;
                let security_error = response.completion_error.clone().or_else(|| {
                    crate::network_host::validate_cors_response(
                        &document_url,
                        &response.final_url,
                        &response.headers,
                        credentials_mode,
                    )
                    .err()
                });
                if let Some(error) = security_error {
                    (
                        default_query_result(&document_url, Some(&response.final_url)),
                        SubresourceNetworkRecord::failure(
                            frame_id,
                            document_url.clone(),
                            requested_manifest_url,
                            "GET".to_owned(),
                            request_headers,
                            None,
                            SubresourceResourceType::Manifest,
                            error,
                        )
                        .with_request_initiator_type(SubresourceRequestInitiatorType::Other),
                        false,
                    )
                } else {
                    let (result, cacheable) = if (200..400).contains(&response.status) {
                        let source = decode_manifest_source(&response.body, &response.headers);
                        let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
                        let result = parse_app_manifest(&document_url, &response.final_url, source);
                        let cacheable = result.data.is_some();
                        (result, cacheable)
                    } else {
                        (
                            default_query_result(&document_url, Some(&response.final_url)),
                            false,
                        )
                    };
                    let response_body = SubresourceResponseBody::from_bytes(response.body);
                    let record = SubresourceNetworkRecord::success_with_body(
                        frame_id,
                        document_url.clone(),
                        requested_manifest_url,
                        "GET".to_owned(),
                        request_headers,
                        None,
                        SubresourceResourceType::Manifest,
                        response.request_cookie_report,
                        response.redirect_chain,
                        response.final_url,
                        response.status,
                        response.headers,
                        response_body,
                        response.cookie_set_reports,
                    )
                    .with_from_cache(response.from_cache)
                    .with_negotiated_http_version(response.negotiated_http_version)
                    .with_network_request_headers(response.network_request_headers)
                    .with_request_initiator_type(SubresourceRequestInitiatorType::Other);
                    (result, record, cacheable)
                }
            }
        };
        let successful_result = cacheable.then(|| RendererAppManifestSuccessfulResult {
            link_identity,
            result: result.clone(),
        });
        RendererAppManifestLoadOutcome {
            result,
            publication: RendererAppManifestLoadPublication {
                network_record,
                successful_result,
            },
        }
    }
}

fn decode_manifest_source(body: &[u8], headers: &[(String, String)]) -> String {
    let content_type = moli_web_mime::response_content_type(headers);
    let charset = content_type
        .as_deref()
        .and_then(moli_web_mime::mime_charset);
    moli_encoding::decode_text_for_legacy_web(body, charset.as_deref())
}

pub(crate) fn complete_default_app_manifest(
    document_url: &Url,
    manifest_url: Option<&Url>,
) -> RendererAppManifestLoadPreparation {
    RendererAppManifestLoadPreparation::Complete(Box::new(default_query_result(
        document_url,
        manifest_url,
    )))
}

fn default_query_result(
    document_url: &Url,
    manifest_url: Option<&Url>,
) -> RendererAppManifestQueryResult {
    RendererAppManifestQueryResult {
        url: manifest_url.map_or_else(String::new, ToString::to_string),
        errors: Vec::new(),
        data: Some(String::new()),
        manifest: default_manifest(document_url),
    }
}

fn default_manifest(document_url: &Url) -> RendererAppManifest {
    let start_url = document_url.clone();
    let mut id = start_url.clone();
    id.set_fragment(None);
    let scope = default_scope(&start_url);
    RendererAppManifest {
        background_color: None,
        description: None,
        display: RendererAppManifestDisplayMode::Undefined,
        display_overrides: Vec::new(),
        icons: Vec::new(),
        id: id.to_string(),
        name: None,
        orientation: RendererAppManifestOrientation::Default,
        prefer_related_applications: false,
        protocol_handlers: Vec::new(),
        related_applications: Vec::new(),
        scope: scope.to_string(),
        shortcuts: Vec::new(),
        start_url: start_url.to_string(),
        theme_color: None,
    }
}

fn parse_app_manifest(
    document_url: &Url,
    manifest_url: &Url,
    source: &str,
) -> RendererAppManifestQueryResult {
    let root = match serde_json::from_str::<Value>(source) {
        Ok(Value::Object(root)) => root,
        Ok(_) => {
            return syntax_error_result(
                document_url,
                manifest_url,
                1,
                1,
                "Manifest root is not an object.",
            );
        }
        Err(error) => {
            let column = error.column().saturating_add(1);
            return syntax_error_result(
                document_url,
                manifest_url,
                error.line(),
                column,
                &format!("Line: {}, column: {}, Syntax error.", error.line(), column),
            );
        }
    };

    let mut errors = Vec::new();
    let mut manifest = default_manifest(document_url);
    // Blink resolves relative members in an embedded data: manifest against
    // the embedding Document because data: URLs cannot be a relative base.
    let member_base_url = if manifest_url.scheme() == "data" {
        document_url
    } else {
        manifest_url
    };
    let start_url = parse_start_url(&root, member_base_url, document_url, &mut errors);
    let id = parse_id(&root, &start_url, document_url, &mut errors);
    let scope = parse_scope(
        &root,
        member_base_url,
        document_url,
        &start_url,
        &mut errors,
    );

    manifest.start_url = start_url.to_string();
    manifest.id = id.to_string();
    manifest.scope = scope.to_string();
    manifest.name = parse_optional_trimmed_string(&root, "name", &mut errors);
    manifest.description = parse_optional_trimmed_string(&root, "description", &mut errors);
    manifest.display = parse_display(&root, &mut errors);
    manifest.display_overrides = parse_display_overrides(&root);
    manifest.orientation = parse_orientation(&root, &mut errors);
    manifest.prefer_related_applications = parse_prefer_related_applications(&root, &mut errors);
    manifest.background_color = parse_color(&root, "background_color", &mut errors);
    manifest.theme_color = parse_color(&root, "theme_color", &mut errors);
    manifest.icons = parse_icons(&root, member_base_url, &mut errors);
    manifest.shortcuts = parse_shortcuts(&root, member_base_url, &scope, &mut errors);
    manifest.related_applications = parse_related_applications(&root, member_base_url, &mut errors);
    manifest.protocol_handlers = parse_protocol_handlers(&root, member_base_url, &mut errors);

    RendererAppManifestQueryResult {
        url: manifest_url.to_string(),
        errors,
        data: Some(source.to_owned()),
        manifest,
    }
}

fn syntax_error_result(
    document_url: &Url,
    manifest_url: &Url,
    line: usize,
    column: usize,
    message: &str,
) -> RendererAppManifestQueryResult {
    RendererAppManifestQueryResult {
        url: manifest_url.to_string(),
        errors: vec![RendererAppManifestError {
            message: message.to_owned(),
            critical: 1,
            line,
            column,
        }],
        data: None,
        manifest: default_manifest(document_url),
    }
}

fn parse_start_url(
    root: &Map<String, Value>,
    manifest_url: &Url,
    document_url: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Url {
    let Some(raw) = string_member(root, "start_url", errors) else {
        return document_url.clone();
    };
    let Ok(url) = manifest_url.join(raw) else {
        noncritical(errors, "property 'start_url' ignored, URL is invalid.");
        return document_url.clone();
    };
    if !same_origin(&url, document_url) {
        noncritical(
            errors,
            "property 'start_url' ignored, should be same origin as document.",
        );
        return document_url.clone();
    }
    url
}

fn parse_id(
    root: &Map<String, Value>,
    start_url: &Url,
    document_url: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Url {
    let mut default_id = start_url.clone();
    default_id.set_fragment(None);
    let Some(raw) = string_member(root, "id", errors).filter(|raw| !raw.is_empty()) else {
        return default_id;
    };
    let origin = origin_ascii_serialization(start_url);
    let Ok(origin_root) = Url::parse(&format!("{origin}/")) else {
        return default_id;
    };
    let Ok(mut id) = origin_root.join(raw) else {
        noncritical(errors, "property 'id' ignored, URL is invalid.");
        return default_id;
    };
    if !same_origin(&id, document_url) {
        noncritical(
            errors,
            "property 'id' ignored, should be same origin as document.",
        );
        return default_id;
    }
    id.set_fragment(None);
    id
}

fn parse_scope(
    root: &Map<String, Value>,
    manifest_url: &Url,
    document_url: &Url,
    start_url: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Url {
    let fallback = default_scope(start_url);
    let Some(raw) = string_member(root, "scope", errors) else {
        return fallback;
    };
    let Ok(mut scope) = manifest_url.join(raw) else {
        noncritical(errors, "property 'scope' ignored, URL is invalid.");
        return fallback;
    };
    scope.set_query(None);
    scope.set_fragment(None);
    if !same_origin(&scope, document_url) || !url_is_within_scope(start_url, &scope) {
        noncritical(
            errors,
            "property 'scope' ignored. Start url should be within scope of scope URL.",
        );
        return fallback;
    }
    scope
}

fn default_scope(start_url: &Url) -> Url {
    let mut scope = start_url.clone();
    let path = scope.path().to_owned();
    let end = path.rfind('/').map_or(1, |index| index + 1);
    scope.set_path(&path[..end]);
    scope.set_query(None);
    scope.set_fragment(None);
    scope
}

fn url_is_within_scope(url: &Url, scope: &Url) -> bool {
    same_origin(url, scope) && url.path().starts_with(scope.path())
}

fn parse_optional_trimmed_string(
    root: &Map<String, Value>,
    name: &str,
    errors: &mut Vec<RendererAppManifestError>,
) -> Option<String> {
    string_member(root, name, errors)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn string_member<'a>(
    root: &'a Map<String, Value>,
    name: &str,
    errors: &mut Vec<RendererAppManifestError>,
) -> Option<&'a str> {
    match root.get(name) {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.trim()),
        Some(_) => {
            noncritical(
                errors,
                &format!("property '{name}' ignored, type string expected."),
            );
            None
        }
    }
}

fn parse_display(
    root: &Map<String, Value>,
    errors: &mut Vec<RendererAppManifestError>,
) -> RendererAppManifestDisplayMode {
    let Some(raw) = string_member(root, "display", errors) else {
        return RendererAppManifestDisplayMode::Undefined;
    };
    parse_display_mode(raw).unwrap_or_else(|| {
        noncritical(errors, "unknown 'display' value ignored.");
        RendererAppManifestDisplayMode::Undefined
    })
}

fn parse_display_overrides(root: &Map<String, Value>) -> Vec<RendererAppManifestDisplayMode> {
    root.get("display_override")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(parse_display_mode)
        .collect()
}

fn parse_display_mode(raw: &str) -> Option<RendererAppManifestDisplayMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "browser" => Some(RendererAppManifestDisplayMode::Browser),
        "minimal-ui" => Some(RendererAppManifestDisplayMode::MinimalUi),
        "standalone" => Some(RendererAppManifestDisplayMode::Standalone),
        "fullscreen" => Some(RendererAppManifestDisplayMode::Fullscreen),
        "window-controls-overlay" => Some(RendererAppManifestDisplayMode::WindowControlsOverlay),
        "tabbed" => Some(RendererAppManifestDisplayMode::Tabbed),
        "borderless" => Some(RendererAppManifestDisplayMode::Borderless),
        "picture-in-picture" => Some(RendererAppManifestDisplayMode::PictureInPicture),
        _ => None,
    }
}

fn parse_orientation(
    root: &Map<String, Value>,
    errors: &mut Vec<RendererAppManifestError>,
) -> RendererAppManifestOrientation {
    let Some(raw) = string_member(root, "orientation", errors) else {
        return RendererAppManifestOrientation::Default;
    };
    match raw.to_ascii_lowercase().as_str() {
        "any" => RendererAppManifestOrientation::Any,
        "natural" => RendererAppManifestOrientation::Natural,
        "landscape" => RendererAppManifestOrientation::Landscape,
        "landscape-primary" => RendererAppManifestOrientation::LandscapePrimary,
        "landscape-secondary" => RendererAppManifestOrientation::LandscapeSecondary,
        "portrait" => RendererAppManifestOrientation::Portrait,
        "portrait-primary" => RendererAppManifestOrientation::PortraitPrimary,
        "portrait-secondary" => RendererAppManifestOrientation::PortraitSecondary,
        _ => {
            noncritical(errors, "unknown 'orientation' value ignored.");
            RendererAppManifestOrientation::Default
        }
    }
}

fn parse_prefer_related_applications(
    root: &Map<String, Value>,
    errors: &mut Vec<RendererAppManifestError>,
) -> bool {
    match root.get("prefer_related_applications") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            noncritical(
                errors,
                "property 'prefer_related_applications' ignored, type boolean expected.",
            );
            false
        }
    }
}

fn parse_color(
    root: &Map<String, Value>,
    name: &str,
    errors: &mut Vec<RendererAppManifestError>,
) -> Option<String> {
    let raw = string_member(root, name, errors)?;
    let [red, green, blue, alpha] =
        moli_css_parse::parse_css_color_to_srgb_bytes(raw).or_else(|| {
            noncritical(
                errors,
                &format!("property '{name}' ignored, '{raw}' is not a valid color."),
            );
            None
        })?;
    let alpha = f64::from(alpha) / 255.0;
    Some(format!("rgba({red},{green},{blue},{alpha})"))
}

fn parse_icons(
    root: &Map<String, Value>,
    manifest_url: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Vec<RendererAppManifestImageResource> {
    let Some(entries) = root.get("icons").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|icon| {
            let src = string_member(icon, "src", errors)?;
            let url = manifest_url.join(src).ok()?;
            let mime_type = string_member(icon, "type", errors)
                .unwrap_or_default()
                .to_owned();
            let sizes = icon
                .get("sizes")
                .and_then(Value::as_str)
                .map(normalize_icon_sizes)
                .unwrap_or_default();
            Some(RendererAppManifestImageResource {
                url: url.to_string(),
                sizes,
                mime_type,
            })
        })
        .collect()
}

fn normalize_icon_sizes(raw: &str) -> String {
    raw.split_ascii_whitespace()
        .filter_map(|token| {
            if token.eq_ignore_ascii_case("any") {
                return Some("0x0".to_owned());
            }
            let (width, height) = token.split_once(['x', 'X'])?;
            let width = width.parse::<u32>().ok()?;
            let height = height.parse::<u32>().ok()?;
            (width > 0 && height > 0).then(|| format!("{width}x{height}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_shortcuts(
    root: &Map<String, Value>,
    manifest_url: &Url,
    scope: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Vec<RendererAppManifestShortcut> {
    let Some(entries) = root.get("shortcuts").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|shortcut| {
            let name = string_member(shortcut, "name", errors)?;
            if name.is_empty() {
                return None;
            }
            let url = string_member(shortcut, "url", errors)
                .and_then(|raw| manifest_url.join(raw).ok())
                .filter(|url| url_is_within_scope(url, scope));
            let Some(url) = url else {
                noncritical(
                    errors,
                    "property 'url' of 'shortcut' not present or is outside the manifest scope.",
                );
                return None;
            };
            Some(RendererAppManifestShortcut {
                name: name.to_owned(),
                url: url.to_string(),
            })
        })
        .collect()
}

fn parse_related_applications(
    root: &Map<String, Value>,
    manifest_url: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Vec<RendererAppManifestRelatedApplication> {
    let Some(entries) = root.get("related_applications").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|application| {
            let raw_url = string_member(application, "url", errors)?;
            let url = manifest_url.join(raw_url).ok()?;
            let id = string_member(application, "id", errors).map(str::to_owned);
            Some(RendererAppManifestRelatedApplication {
                id,
                url: url.to_string(),
            })
        })
        .collect()
}

fn parse_protocol_handlers(
    root: &Map<String, Value>,
    manifest_url: &Url,
    errors: &mut Vec<RendererAppManifestError>,
) -> Vec<RendererAppManifestProtocolHandler> {
    let Some(entries) = root.get("protocol_handlers").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|handler| {
            let protocol = string_member(handler, "protocol", errors)?;
            let raw_url = string_member(handler, "url", errors)?;
            let url = manifest_url.join(raw_url).ok()?;
            Some(RendererAppManifestProtocolHandler {
                protocol: protocol.to_owned(),
                url: url.to_string(),
            })
        })
        .collect()
}

fn noncritical(errors: &mut Vec<RendererAppManifestError>, message: &str) {
    errors.push(RendererAppManifestError {
        message: message.to_owned(),
        critical: 0,
        line: 0,
        column: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn default_manifest_uses_document_url_and_directory_scope() {
        let result = default_query_result(&url("https://example.test/app/page?x=1#fragment"), None);
        assert_eq!(result.url, "");
        assert_eq!(result.data.as_deref(), Some(""));
        assert_eq!(
            result.manifest.start_url,
            "https://example.test/app/page?x=1#fragment"
        );
        assert_eq!(result.manifest.id, "https://example.test/app/page?x=1");
        assert_eq!(result.manifest.scope, "https://example.test/app/");
    }

    #[test]
    fn parser_resolves_core_members_like_chromium() {
        let result = parse_app_manifest(
            &url("https://example.test/app/page"),
            &url("https://example.test/manifests/app.webmanifest"),
            r##"{
                "name":" Manifest Name ",
                "id":"./identity?x=1#ignored",
                "start_url":"./start?x=2#frag",
                "scope":"./",
                "display":"standalone",
                "display_override":["fullscreen","browser","bogus"],
                "orientation":"portrait-primary",
                "prefer_related_applications":true,
                "background_color":"#11223380",
                "theme_color":"red",
                "icons":[{"src":"icons/a.png","sizes":"16x16 any 32x32","type":"image/png"}],
                "shortcuts":[{"name":"Shortcut","url":"./shortcut"}],
                "related_applications":[{"url":"https://store.test/app","id":"pkg"}],
                "protocol_handlers":[{"protocol":"web+demo","url":"./handler?u=%s"}]
            }"##,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.manifest.name.as_deref(), Some("Manifest Name"));
        assert_eq!(result.manifest.id, "https://example.test/identity?x=1");
        assert_eq!(
            result.manifest.start_url,
            "https://example.test/manifests/start?x=2#frag"
        );
        assert_eq!(result.manifest.scope, "https://example.test/manifests/");
        assert_eq!(
            result.manifest.background_color.as_deref(),
            Some("rgba(17,34,51,0.5019607843137255)")
        );
        assert_eq!(
            result.manifest.theme_color.as_deref(),
            Some("rgba(255,0,0,1)")
        );
        assert_eq!(result.manifest.icons[0].sizes, "16x16 0x0 32x32");
        assert_eq!(result.manifest.shortcuts.len(), 1);
        assert_eq!(result.manifest.related_applications.len(), 1);
        assert_eq!(result.manifest.protocol_handlers.len(), 1);
    }

    #[test]
    fn parser_reports_critical_json_errors_and_returns_default_manifest() {
        let result = parse_app_manifest(
            &url("https://example.test/app/page"),
            &url("https://example.test/app.webmanifest"),
            "{\"name\":",
        );
        assert_eq!(result.data, None);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].critical, 1);
        assert_eq!(result.errors[0].line, 1);
        assert_eq!(result.errors[0].column, 9);
        assert_eq!(result.manifest.id, "https://example.test/app/page");
    }

    #[test]
    fn cross_origin_start_and_id_fall_back_to_document() {
        let result = parse_app_manifest(
            &url("https://example.test/app/page"),
            &url("https://example.test/app.webmanifest"),
            r#"{"start_url":"https://other.test/start","id":"https://other.test/id"}"#,
        );
        assert_eq!(result.manifest.start_url, "https://example.test/app/page");
        assert_eq!(result.manifest.id, "https://example.test/app/page");
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn data_manifest_resolves_relative_members_against_document_url() {
        let result = parse_app_manifest(
            &url("https://example.test/app/page"),
            &url("data:application/manifest+json,%7B%7D"),
            r#"{
                "start_url":"relative-start",
                "scope":"./",
                "icons":[{"src":"icon.png"}]
            }"#,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.manifest.start_url,
            "https://example.test/app/relative-start"
        );
        assert_eq!(result.manifest.scope, "https://example.test/app/");
        assert_eq!(
            result.manifest.icons[0].url,
            "https://example.test/app/icon.png"
        );
    }

    #[test]
    fn manifest_source_honors_response_charset() {
        let source = decode_manifest_source(
            b"{\"name\":\"caf\xe9\"}",
            &[(
                "content-type".to_owned(),
                "application/manifest+json; charset=windows-1252".to_owned(),
            )],
        );
        assert_eq!(source, "{\"name\":\"caf\u{e9}\"}");
    }
}

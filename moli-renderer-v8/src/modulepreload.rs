use url::Url;

use crate::dom::native::Element;
use crate::link_as::{LinkAsDestination, link_as_destination};
use crate::module_runtime::{
    ModuleAttributesKey, ModuleFetchMetadata, ModuleMapKey, NativeModuleSingleFetchRequest,
};
use crate::stylesheet_blocking::link_rel_includes_token;

#[derive(Clone)]
pub(crate) struct ModulepreloadFetchCandidate {
    pub(crate) key: ModuleMapKey,
    pub(crate) request: NativeModuleSingleFetchRequest,
}

#[derive(Debug, Default)]
pub(crate) struct ParserDiscoveredModulepreloadResult {
    requests: Vec<NativeModuleSingleFetchRequest>,
    runtime_warnings: Vec<String>,
    link_error_tasks: usize,
}

impl ParserDiscoveredModulepreloadResult {
    pub(crate) fn push_request(&mut self, request: NativeModuleSingleFetchRequest) {
        self.requests.push(request);
    }

    pub(crate) fn push_runtime_warning(&mut self, warning: impl Into<String>) {
        self.runtime_warnings.push(warning.into());
    }

    pub(crate) fn push_link_error_task(&mut self) {
        self.link_error_tasks += 1;
    }

    pub(crate) fn into_parts(self) -> (Vec<NativeModuleSingleFetchRequest>, Vec<String>, usize) {
        (self.requests, self.runtime_warnings, self.link_error_tasks)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModulepreloadAsState {
    ScriptLike,
    Style,
    Json,
    Text,
    Invalid,
}

pub(crate) fn modulepreload_href(element: &Element) -> Option<&str> {
    if !element.is_html_element("link") {
        return None;
    }
    if !element
        .attribute("rel")
        .is_some_and(|rel| link_rel_includes_token(rel, "modulepreload"))
    {
        return None;
    }
    element
        .attribute("href")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn modulepreload_as_state(element: &Element) -> ModulepreloadAsState {
    modulepreload_as_state_from_attribute(element.attribute("as"))
}

pub(crate) fn modulepreload_as_state_from_attribute(
    as_attribute: Option<&str>,
) -> ModulepreloadAsState {
    match link_as_destination(as_attribute) {
        LinkAsDestination::None
        | LinkAsDestination::Script
        | LinkAsDestination::AudioWorklet
        | LinkAsDestination::PaintWorklet
        | LinkAsDestination::ServiceWorker
        | LinkAsDestination::SharedWorker
        | LinkAsDestination::Worker => ModulepreloadAsState::ScriptLike,
        LinkAsDestination::Style => ModulepreloadAsState::Style,
        LinkAsDestination::Json => ModulepreloadAsState::Json,
        LinkAsDestination::Text => ModulepreloadAsState::Text,
        LinkAsDestination::Audio
        | LinkAsDestination::Document
        | LinkAsDestination::Embed
        | LinkAsDestination::Fetch
        | LinkAsDestination::Font
        | LinkAsDestination::Frame
        | LinkAsDestination::IFrame
        | LinkAsDestination::Image
        | LinkAsDestination::Manifest
        | LinkAsDestination::Object
        | LinkAsDestination::Report
        | LinkAsDestination::Track
        | LinkAsDestination::Video
        | LinkAsDestination::WebIdentity
        | LinkAsDestination::Xslt => ModulepreloadAsState::Invalid,
    }
}

pub(crate) fn invalid_modulepreload_as_value(element: &Element) -> Option<String> {
    if modulepreload_as_state(element) != ModulepreloadAsState::Invalid {
        return None;
    }
    element
        .attribute("as")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

pub(crate) fn invalid_modulepreload_as_warning(value: &str) -> String {
    format!("<link rel=modulepreload> has an invalid `as` value {value}")
}

fn modulepreload_attributes_for_state(state: ModulepreloadAsState) -> Option<ModuleAttributesKey> {
    match state {
        ModulepreloadAsState::ScriptLike | ModulepreloadAsState::Text => {
            Some(ModuleAttributesKey::empty())
        }
        ModulepreloadAsState::Style => Some(ModuleAttributesKey::from_pairs(vec![(
            "type".to_owned(),
            "css".to_owned(),
        )])),
        ModulepreloadAsState::Json => Some(ModuleAttributesKey::from_pairs(vec![(
            "type".to_owned(),
            "json".to_owned(),
        )])),
        ModulepreloadAsState::Invalid => None,
    }
}

pub(crate) fn modulepreload_media_matches(element: &Element) -> bool {
    modulepreload_media_attribute_matches(element.attribute("media"))
}

pub(crate) fn modulepreload_media_attribute_matches(media: Option<&str>) -> bool {
    let Some(media) = media.map(str::trim) else {
        return true;
    };
    if media.is_empty() {
        return true;
    }
    crate::style_engine::media_list::evaluate_media_query_list(
        media,
        None,
        crate::style_engine::StyleViewport::default(),
    )
}

fn modulepreload_fetch_metadata_with_import_map_integrity(
    mut fetch_metadata: crate::planning::ScriptFetchMetadata,
    integrity_attribute: Option<&str>,
    import_map_integrity: Option<String>,
) -> crate::planning::ScriptFetchMetadata {
    if integrity_attribute.is_none() && fetch_metadata.integrity.is_none() {
        fetch_metadata.integrity = import_map_integrity;
    }
    fetch_metadata
}

pub(crate) fn modulepreload_fetch_candidate(
    element: &Element,
    request_url: Url,
    document_url: &Url,
    import_map_integrity: Option<String>,
) -> Option<ModulepreloadFetchCandidate> {
    if !modulepreload_media_matches(element) {
        return None;
    }
    let state = modulepreload_as_state(element);
    let attributes = modulepreload_attributes_for_state(state)?;
    let fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
        element.attribute("crossorigin"),
        element.attribute("referrerpolicy"),
        None,
        element.attribute("integrity"),
        element
            .cryptographic_nonce()
            .or_else(|| element.attribute("nonce")),
        element.attribute("fetchpriority"),
    );
    let fetch_metadata = modulepreload_fetch_metadata_with_import_map_integrity(
        fetch_metadata,
        element.attribute("integrity"),
        import_map_integrity,
    );
    let key = if state == ModulepreloadAsState::Text {
        ModuleMapKey::modulepreload_text(request_url.clone())
    } else {
        ModuleMapKey::from_url_and_attributes(&request_url, &attributes).ok()?
    };
    Some(ModulepreloadFetchCandidate {
        key: key.clone(),
        request: NativeModuleSingleFetchRequest::new(
            request_url.clone(),
            request_url,
            document_url.clone(),
            key,
            ModuleFetchMetadata::from_modulepreload_script_fetch_metadata(&fetch_metadata),
        ),
    })
}

pub(crate) fn resolve_parser_network_resource_url(base_url: &Url, raw_url: &str) -> Option<Url> {
    let request_url = base_url.join(raw_url).ok()?;
    match request_url.scheme() {
        "about" | "data" | "javascript" => None,
        _ => Some(request_url),
    }
}

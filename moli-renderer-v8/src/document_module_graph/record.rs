pub(crate) use moli_module_script_tree::ModuleSourceOrigin;
use url::Url;

use super::{ModuleAttributesKey, ModuleKind, ModuleMapKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModuleSource {
    Text(String),
    TextWithOrigin {
        source: String,
        origin: Box<ModuleSourceOrigin>,
    },
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleGraphFetchedSource {
    final_url: Url,
    redirected: bool,
    response_referrer_policy: Option<String>,
    source: ModuleSource,
}

impl ModuleGraphFetchedSource {
    pub(crate) fn new(final_url: Url, redirected: bool, source: ModuleSource) -> Self {
        Self {
            final_url,
            redirected,
            response_referrer_policy: None,
            source,
        }
    }

    pub(crate) fn with_response_referrer_policy(
        mut self,
        response_referrer_policy: Option<String>,
    ) -> Self {
        self.response_referrer_policy = response_referrer_policy;
        self
    }

    pub(crate) fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub(crate) fn redirected(&self) -> bool {
        self.redirected
    }

    pub(crate) fn response_referrer_policy(&self) -> Option<&str> {
        self.response_referrer_policy.as_deref()
    }

    pub(crate) fn effective_key_for_request(&self, request_key: &ModuleMapKey) -> ModuleMapKey {
        match request_key.kind() {
            ModuleKind::JavaScript => ModuleMapKey::java_script_with_attributes(
                self.final_url.clone(),
                request_key.attributes().clone(),
            ),
            ModuleKind::Json => ModuleMapKey::json_with_attributes(
                self.final_url.clone(),
                request_key.attributes().clone(),
            ),
            ModuleKind::Css => ModuleMapKey::css_with_attributes(
                self.final_url.clone(),
                request_key.attributes().clone(),
            ),
            ModuleKind::ModulePreloadText => {
                ModuleMapKey::modulepreload_text(self.final_url.clone())
            }
            ModuleKind::WebAssembly => ModuleMapKey::webassembly(self.final_url.clone()),
        }
    }

    pub(crate) fn into_source(self) -> ModuleSource {
        self.source
    }

    pub(crate) fn source(&self) -> &ModuleSource {
        &self.source
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.source.len()
    }
}

impl ModuleSource {
    pub(crate) fn text(source: String) -> Self {
        Self::Text(source)
    }

    pub(crate) fn text_with_origin(source: String, origin: ModuleSourceOrigin) -> Self {
        Self::TextWithOrigin {
            source,
            origin: Box::new(origin),
        }
    }

    pub(crate) fn origin(&self) -> Option<&ModuleSourceOrigin> {
        match self {
            Self::TextWithOrigin { origin, .. } => Some(origin),
            Self::Text(_) | Self::Binary(_) => None,
        }
    }

    pub(crate) fn binary(bytes: Vec<u8>) -> Self {
        Self::Binary(bytes)
    }

    pub(crate) fn text_source(&self) -> Option<&str> {
        match self {
            Self::Text(source) | Self::TextWithOrigin { source, .. } => Some(source),
            Self::Binary(_) => None,
        }
    }

    pub(crate) fn binary_source(&self) -> Option<&[u8]> {
        match self {
            Self::Text(_) | Self::TextWithOrigin { .. } => None,
            Self::Binary(bytes) => Some(bytes),
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Text(source) | Self::TextWithOrigin { source, .. } => source.len(),
            Self::Binary(bytes) => bytes.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleImportPhase {
    Evaluation,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleRequestRecord {
    specifier: String,
    attributes: ModuleAttributesKey,
    phase: ModuleImportPhase,
}

impl ModuleRequestRecord {
    pub(crate) fn new(
        specifier: impl Into<String>,
        attributes: ModuleAttributesKey,
        phase: ModuleImportPhase,
    ) -> Self {
        Self {
            specifier: specifier.into(),
            attributes,
            phase,
        }
    }

    pub(crate) fn specifier(&self) -> &str {
        &self.specifier
    }

    pub(crate) fn phase(&self) -> ModuleImportPhase {
        self.phase
    }

    pub(crate) fn attributes(&self) -> &ModuleAttributesKey {
        &self.attributes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleResolvedDependency {
    specifier: String,
    attributes: ModuleAttributesKey,
    resolved_key: ModuleMapKey,
}

impl ModuleResolvedDependency {
    pub(crate) fn new(
        specifier: impl Into<String>,
        attributes: ModuleAttributesKey,
        resolved_key: ModuleMapKey,
    ) -> Self {
        Self {
            specifier: specifier.into(),
            attributes,
            resolved_key,
        }
    }

    pub(crate) fn resolved_key(&self) -> &ModuleMapKey {
        &self.resolved_key
    }

    pub(crate) fn matches_request(
        &self,
        specifier: &str,
        attributes: &ModuleAttributesKey,
        referrer_url: &Url,
    ) -> bool {
        if &self.attributes != attributes {
            return false;
        }
        if self.specifier == specifier {
            return true;
        }
        if Url::parse(specifier).is_ok_and(|url| url == *self.resolved_key.url()) {
            return true;
        }
        referrer_url
            .join(&self.specifier)
            .is_ok_and(|url| url == *self.resolved_key.url() && url.as_str() == specifier)
    }
}

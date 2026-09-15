use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ModuleEntryId(u32);

impl ModuleEntryId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("native module map entry index exceeded u32::MAX"))
    }

    pub(crate) fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) fn raw(self) -> u32 {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn for_test(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ModuleAttributesKey {
    attributes: Vec<(String, String)>,
}

impl ModuleAttributesKey {
    pub(crate) fn empty() -> Self {
        Self {
            attributes: Vec::new(),
        }
    }

    pub(crate) fn from_pairs(mut attributes: Vec<(String, String)>) -> Self {
        attributes.sort();
        attributes.dedup();
        Self { attributes }
    }

    pub(crate) fn module_type(&self) -> Option<&str> {
        self.attributes
            .iter()
            .find_map(|(key, value)| (key == "type").then_some(value.as_str()))
    }

    pub(crate) fn invalid_import_attribute_key(&self) -> Option<&str> {
        self.attributes
            .iter()
            .find_map(|(key, _)| (key != "type").then_some(key.as_str()))
    }

    pub(crate) fn pairs(&self) -> &[(String, String)] {
        &self.attributes
    }
}

impl Default for ModuleAttributesKey {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ModuleKind {
    JavaScript,
    Json,
    Css,
    ModulePreloadText,
    WebAssembly,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ModuleMapKey {
    url: Url,
    kind: ModuleKind,
    attributes: ModuleAttributesKey,
}

impl ModuleMapKey {
    pub(crate) fn from_url_and_attributes(
        url: &Url,
        attributes: &ModuleAttributesKey,
    ) -> std::result::Result<Self, String> {
        if let Some(invalid_key) = attributes.invalid_import_attribute_key() {
            return Err(format!("Invalid attribute key \"{invalid_key}\"."));
        }
        let Some(module_type) = attributes.module_type() else {
            return Ok(Self::from_script_url(url.clone()));
        };
        match module_type {
            "json" => Ok(Self::json_with_attributes(url.clone(), attributes.clone())),
            "css" => Ok(Self::css_with_attributes(url.clone(), attributes.clone())),
            other => Err(format!("module type `{other}` is not a valid module type")),
        }
    }

    pub(crate) fn from_script_url(url: Url) -> Self {
        if url.path().to_ascii_lowercase().ends_with(".wasm") {
            Self::webassembly(url)
        } else {
            Self::java_script(url)
        }
    }

    pub(crate) fn java_script(url: Url) -> Self {
        Self::java_script_with_attributes(url, ModuleAttributesKey::empty())
    }

    pub(crate) fn java_script_with_attributes(url: Url, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            kind: ModuleKind::JavaScript,
            attributes,
        }
    }

    pub(crate) fn json_with_attributes(url: Url, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            kind: ModuleKind::Json,
            attributes,
        }
    }

    pub(crate) fn css_with_attributes(url: Url, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            kind: ModuleKind::Css,
            attributes,
        }
    }

    pub(crate) fn modulepreload_text(url: Url) -> Self {
        Self {
            url,
            kind: ModuleKind::ModulePreloadText,
            attributes: ModuleAttributesKey::empty(),
        }
    }

    pub(crate) fn webassembly(url: Url) -> Self {
        Self {
            url,
            kind: ModuleKind::WebAssembly,
            attributes: ModuleAttributesKey::empty(),
        }
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn kind(&self) -> ModuleKind {
        self.kind
    }

    pub(crate) fn attributes(&self) -> &ModuleAttributesKey {
        &self.attributes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleMapEntryState {
    Fetching,
    Fetched,
    Compiled,
    Instantiated,
    Evaluating,
    Evaluated,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleMapFetchDisposition {
    StartedFetch(ModuleEntryId),
    JoinedFetching(ModuleEntryId),
    AlreadyFetched(ModuleEntryId),
    AlreadyCompiled(ModuleEntryId),
    AlreadyFailed(ModuleEntryId),
}

impl ModuleMapFetchDisposition {
    pub(crate) fn entry_id(self) -> ModuleEntryId {
        match self {
            Self::StartedFetch(entry_id)
            | Self::JoinedFetching(entry_id)
            | Self::AlreadyFetched(entry_id)
            | Self::AlreadyCompiled(entry_id)
            | Self::AlreadyFailed(entry_id) => entry_id,
        }
    }
}

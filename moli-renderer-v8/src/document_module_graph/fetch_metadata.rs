use moli_fetch::{RequestCredentialsMode, ScriptFetchRequestMetadata};

use crate::planning::{ScriptFetchMetadata, module_script_credentials_mode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleFetchMetadata {
    pub(crate) credentials_mode: RequestCredentialsMode,
    pub(crate) request_metadata: ScriptFetchRequestMetadata,
    pub(crate) parser_inserted: bool,
}

impl Default for ModuleFetchMetadata {
    fn default() -> Self {
        Self {
            credentials_mode: RequestCredentialsMode::SameOrigin,
            request_metadata: ScriptFetchRequestMetadata::default(),
            parser_inserted: false,
        }
    }
}

impl ModuleFetchMetadata {
    pub(crate) fn from_top_level_script_fetch_metadata(metadata: &ScriptFetchMetadata) -> Self {
        let mut mapped = Self::from_script_fetch_metadata_inner(metadata);
        mapped.request_metadata.charset = metadata.charset.clone();
        mapped.request_metadata.integrity = metadata.integrity.clone();
        mapped.request_metadata.nonce = metadata.nonce.clone();
        mapped
    }

    pub(crate) fn from_modulepreload_script_fetch_metadata(metadata: &ScriptFetchMetadata) -> Self {
        let mut mapped = Self::from_script_fetch_metadata_inner(metadata);
        mapped.request_metadata.integrity = metadata.integrity.clone();
        mapped.request_metadata.nonce = metadata.nonce.clone();
        mapped.request_metadata.fetch_priority = None;
        mapped
    }

    pub(crate) fn from_dynamic_import_referrer_fetch_metadata(
        metadata: &ScriptFetchMetadata,
    ) -> Self {
        let mut mapped = Self::from_script_fetch_metadata_inner(metadata);
        mapped.request_metadata.nonce = metadata.nonce.clone();
        mapped
    }

    pub(crate) fn from_loaded_module_script_fetch_metadata(metadata: &ScriptFetchMetadata) -> Self {
        let mut mapped = Self::from_parser_owned_script_fetch_metadata(metadata);
        mapped.request_metadata.nonce = metadata.nonce.clone();
        mapped
    }

    pub(crate) fn from_parser_owned_script_fetch_metadata(metadata: &ScriptFetchMetadata) -> Self {
        Self::from_script_fetch_metadata_inner(metadata)
    }

    pub(crate) fn nonce(&self) -> Option<&str> {
        self.request_metadata.nonce.as_deref()
    }

    pub(crate) fn integrity(&self) -> Option<&str> {
        self.request_metadata.integrity.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn fetch_priority_for_test(&self) -> Option<moli_fetch::FetchPriorityHint> {
        self.request_metadata.fetch_priority
    }

    #[cfg(test)]
    pub(crate) fn for_descendant_fetches(&self) -> Self {
        let mut metadata = self.clone();
        metadata.request_metadata.charset = None;
        metadata.request_metadata.integrity = None;
        metadata.request_metadata.fetch_priority = Some(moli_fetch::FetchPriorityHint::Auto);
        metadata.request_metadata.scheduler_priority = None;
        metadata
    }

    fn from_script_fetch_metadata_inner(metadata: &ScriptFetchMetadata) -> Self {
        Self {
            credentials_mode: module_script_credentials_mode(metadata.cross_origin.as_deref()),
            request_metadata: ScriptFetchRequestMetadata {
                cross_origin: metadata.cross_origin.clone(),
                referrer_policy: metadata.referrer_policy.clone(),
                document_referrer_policy: None,
                charset: None,
                integrity: None,
                nonce: None,
                fetch_priority: metadata.fetch_priority,
                scheduler_priority: None,
            },
            parser_inserted: metadata.parser_inserted,
        }
    }

    pub(crate) fn with_response_referrer_policy(mut self, policy: Option<&str>) -> Self {
        if let Some(policy) = policy {
            self.request_metadata.referrer_policy = Some(policy.to_owned());
        }
        self
    }

    pub(crate) fn with_import_map_integrity_if_absent(mut self, integrity: Option<String>) -> Self {
        if self.request_metadata.integrity.is_none() {
            self.request_metadata.integrity = integrity;
        }
        self
    }

    pub(crate) fn with_import_map_integrity(mut self, integrity: Option<String>) -> Self {
        self.request_metadata.integrity = integrity;
        self
    }
}

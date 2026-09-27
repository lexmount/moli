use std::collections::HashMap;
use std::fmt;

use url::Url;

use super::{ModuleFetchMetadata, ModuleMapKey, NativeModuleGraphFetchRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeModuleSingleFetchRequest {
    source_url: Url,
    base_url: Url,
    initiator_url: Url,
    // Fetch-client defaults belong to this request, not the module's persistent options.
    document_referrer_policy: Option<String>,
    module_key: ModuleMapKey,
    fetch_metadata: ModuleFetchMetadata,
}

impl NativeModuleSingleFetchRequest {
    pub(crate) fn new(
        source_url: Url,
        base_url: Url,
        initiator_url: Url,
        module_key: ModuleMapKey,
        fetch_metadata: ModuleFetchMetadata,
    ) -> Self {
        Self {
            source_url,
            base_url,
            initiator_url,
            document_referrer_policy: None,
            module_key,
            fetch_metadata,
        }
    }

    pub(crate) fn with_document_referrer_policy(mut self, policy: Option<String>) -> Self {
        self.document_referrer_policy = policy;
        self
    }

    #[cfg(test)]
    pub(crate) fn source_url(&self) -> &Url {
        &self.source_url
    }

    pub(crate) fn fetch_metadata(&self) -> &ModuleFetchMetadata {
        &self.fetch_metadata
    }

    pub(crate) fn module_key(&self) -> &ModuleMapKey {
        &self.module_key
    }

    pub(crate) fn effective_key_for_fetched_source(
        &self,
        fetched_source: &super::ModuleGraphFetchedSource,
    ) -> ModuleMapKey {
        fetched_source.effective_key_for_request(&self.module_key)
    }

    pub(crate) fn effective_fetch_metadata_for_fetched_source(
        &self,
        fetched_source: &super::ModuleGraphFetchedSource,
    ) -> ModuleFetchMetadata {
        self.fetch_metadata
            .clone()
            .with_response_referrer_policy(fetched_source.response_referrer_policy())
    }

    pub(crate) fn fetch_request(&self) -> NativeModuleGraphFetchRequest {
        NativeModuleGraphFetchRequest::new(
            self.source_url.clone(),
            self.initiator_url.clone(),
            self.fetch_metadata.clone(),
            self.module_key.kind(),
        )
        .with_document_referrer_policy(self.document_referrer_policy.clone())
    }
}

#[derive(Default)]
pub(crate) struct NativeModuleSingleFetchQueue {
    inflight_fetches: HashMap<u64, NativeModuleSingleFetchRequest>,
}

impl fmt::Debug for NativeModuleSingleFetchQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeModuleSingleFetchQueue")
            .field("inflight_fetch_count", &self.inflight_fetches.len())
            .finish()
    }
}

impl NativeModuleSingleFetchQueue {
    pub(crate) fn suspend_fetch(&mut self, load_id: u64, request: NativeModuleSingleFetchRequest) {
        self.inflight_fetches.insert(load_id, request);
    }

    pub(crate) fn take_inflight_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<NativeModuleSingleFetchRequest> {
        self.inflight_fetches.remove(&load_id)
    }

    pub(crate) fn has_inflight_fetch_for(&self, load_id: u64) -> bool {
        self.inflight_fetches.contains_key(&load_id)
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_fetch(&self) -> bool {
        !self.inflight_fetches.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str) -> NativeModuleSingleFetchRequest {
        let base_url = Url::parse("https://example.test/base/").expect("base url should parse");
        NativeModuleSingleFetchRequest::new(
            base_url.join(path).expect("preload url should parse"),
            base_url.clone(),
            base_url.clone(),
            ModuleMapKey::java_script(base_url.join(path).expect("preload url should parse")),
            ModuleFetchMetadata::default(),
        )
    }

    #[test]
    fn inflight_fetches_are_keyed_by_load_id() {
        let mut queue = NativeModuleSingleFetchQueue::default();

        assert!(!queue.has_inflight_fetch());
        queue.suspend_fetch(7, request("entry.mjs"));
        assert!(queue.has_inflight_fetch());
        assert!(queue.has_inflight_fetch_for(7));
        assert!(!queue.has_inflight_fetch_for(8));

        assert!(queue.take_inflight_fetch(8).is_none());
        assert_eq!(
            queue
                .take_inflight_fetch(7)
                .expect("matching load id should return fetch")
                .source_url()
                .as_str(),
            "https://example.test/base/entry.mjs"
        );
        assert!(queue.take_inflight_fetch(7).is_none());
        assert!(!queue.has_inflight_fetch_for(7));
        assert!(!queue.has_inflight_fetch());
    }
}

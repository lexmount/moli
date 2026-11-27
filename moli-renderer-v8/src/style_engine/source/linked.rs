use std::collections::{HashMap, HashSet};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::super::source_owner::stylesheet_owner_is_in_style_scope;
use super::store::StyloStylesheetSource;

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkedStylesheetOwnerBinding {
    /// Root request URL captured by the accepted owner-bound load operation.
    /// Imported children belong to the root live stylesheet graph and never
    /// become additional owner bindings.
    request_url: url::Url,
}

#[derive(Debug)]
struct LinkedStylesheetOwnerLiveSource {
    stylesheet: crate::live_stylesheet::LiveStylesheetRef,
    source: StyloStylesheetSource,
}

impl LinkedStylesheetOwnerLiveSource {
    fn new(
        stylesheet: crate::live_stylesheet::LiveStylesheetRef,
        sheet_url: &url::Url,
        origin_clean: bool,
    ) -> Self {
        let source = StyloStylesheetSource::from_live_stylesheet(&stylesheet)
            .with_sheet_url(sheet_url.clone())
            .with_origin_clean(origin_clean);
        Self { stylesheet, source }
    }

    fn refresh(&mut self) -> bool {
        let source = StyloStylesheetSource::from_live_stylesheet(&self.stylesheet)
            .with_sheet_url(self.source.sheet_url().clone())
            .with_origin_clean(self.source.origin_clean());
        if self.source == source {
            return false;
        }
        self.source = source;
        true
    }
}

impl LinkedStylesheetOwnerBinding {
    fn new(request_url: url::Url) -> Self {
        Self { request_url }
    }
}

#[derive(Debug, Default)]
pub(in crate::style_engine) struct LinkedStylesheetSources {
    bindings_by_owner: HashMap<DomHandle, LinkedStylesheetOwnerBinding>,
    live_sources_by_owner: HashMap<DomHandle, LinkedStylesheetOwnerLiveSource>,
    sources_by_url: HashMap<String, StyloStylesheetSource>,
    final_url_by_request_url: HashMap<String, String>,
    direct_request_url_keys: HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::style_engine) enum LinkedStylesheetSourceRecordChange {
    Unchanged,
    Inserted,
    Updated,
}

impl LinkedStylesheetSources {
    pub(in crate::style_engine) fn bind_owner(
        &mut self,
        owner: DomHandle,
        request_url: url::Url,
    ) -> bool {
        let binding = LinkedStylesheetOwnerBinding::new(request_url);
        if self.bindings_by_owner.get(&owner) == Some(&binding) {
            return false;
        }
        self.bindings_by_owner.insert(owner, binding);
        true
    }

    pub(in crate::style_engine) fn remove_owner(&mut self, owner: DomHandle) -> bool {
        let binding_removed = self.bindings_by_owner.remove(&owner).is_some();
        let live_source_removed = self.live_sources_by_owner.remove(&owner).is_some();
        binding_removed || live_source_removed
    }

    pub(in crate::style_engine) fn source_for_owner(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<StyloStylesheetSource> {
        if !stylesheet_owner_is_in_style_scope(host, owner) {
            return None;
        }
        if let Some(source) = self.live_sources_by_owner.get(&owner) {
            return Some(source.source.clone());
        }
        let binding = self.bindings_by_owner.get(&owner)?;
        self.source_for_url(&binding.request_url)
    }

    pub(in crate::style_engine) fn set_owner_live_stylesheet(
        &mut self,
        owner: DomHandle,
        stylesheet: Option<crate::live_stylesheet::LiveStylesheetRef>,
        sheet_url: &url::Url,
        origin_clean: bool,
    ) -> bool {
        let Some(stylesheet) = stylesheet else {
            return self.live_sources_by_owner.remove(&owner).is_some();
        };
        let source = LinkedStylesheetOwnerLiveSource::new(stylesheet, sheet_url, origin_clean);
        if self
            .live_sources_by_owner
            .get(&owner)
            .is_some_and(|current| {
                current.stylesheet.id() == source.stylesheet.id() && current.source == source.source
            })
        {
            return false;
        }
        self.live_sources_by_owner.insert(owner, source);
        true
    }

    pub(in crate::style_engine) fn live_stylesheet(
        &self,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        self.live_sources_by_owner
            .get(&owner)
            .map(|source| source.stylesheet.clone())
    }

    pub(in crate::style_engine) fn refresh_live_stylesheet(
        &mut self,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        self.live_sources_by_owner
            .get_mut(&owner)
            .filter(|source| source.stylesheet.id() == stylesheet_id)
            .is_some_and(LinkedStylesheetOwnerLiveSource::refresh)
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn counts_for_test(&self) -> (usize, usize) {
        (
            usize::from(!self.bindings_by_owner.is_empty()),
            self.bindings_by_owner.len(),
        )
    }
    pub(in crate::style_engine) fn record_source_for_url(
        &mut self,
        url: &url::Url,
        source: StyloStylesheetSource,
    ) -> LinkedStylesheetSourceRecordChange {
        let final_url = source.base_url().clone();
        self.record_direct_request_url_key(url, &final_url);
        let previous_final_url = self.record_final_url_for_request_url(url, &final_url);
        let mut changed = self.record_source_for_url_key(url, source.clone());
        if final_url != *url {
            changed = changed.merge(self.record_source_for_url_key(&final_url, source));
        }
        if let Some(previous_final_url) = previous_final_url {
            changed =
                changed.merge(self.remove_source_for_unreferenced_url_key(previous_final_url));
        }
        changed
    }

    pub(in crate::style_engine) fn source_for_url(
        &self,
        url: &url::Url,
    ) -> Option<StyloStylesheetSource> {
        self.source_for_url_ref(url).cloned()
    }

    fn source_for_url_ref(&self, url: &url::Url) -> Option<&StyloStylesheetSource> {
        self.sources_by_url.get(url.as_str())
    }

    fn record_direct_request_url_key(&mut self, url: &url::Url, final_url: &url::Url) {
        let request_key = url.as_str().to_owned();
        if final_url == url {
            self.direct_request_url_keys.insert(request_key);
        } else {
            self.direct_request_url_keys.remove(&request_key);
        }
    }

    fn record_final_url_for_request_url(
        &mut self,
        url: &url::Url,
        final_url: &url::Url,
    ) -> Option<String> {
        let request_key = url.as_str().to_owned();
        if final_url == url {
            return self
                .final_url_by_request_url
                .remove(&request_key)
                .filter(|previous_final_url| previous_final_url != &request_key);
        }
        let final_key = final_url.as_str().to_owned();
        self.final_url_by_request_url
            .insert(request_key, final_key)
            .filter(|previous_final_url| previous_final_url != final_url.as_str())
    }

    fn record_source_for_url_key(
        &mut self,
        url: &url::Url,
        source: StyloStylesheetSource,
    ) -> LinkedStylesheetSourceRecordChange {
        match self.sources_by_url.get(url.as_str()) {
            Some(existing) if existing == &source => LinkedStylesheetSourceRecordChange::Unchanged,
            Some(_) => {
                self.sources_by_url.insert(url.as_str().to_owned(), source);
                LinkedStylesheetSourceRecordChange::Updated
            }
            None => {
                self.sources_by_url.insert(url.as_str().to_owned(), source);
                LinkedStylesheetSourceRecordChange::Inserted
            }
        }
    }

    fn remove_source_for_unreferenced_url_key(
        &mut self,
        url_key: String,
    ) -> LinkedStylesheetSourceRecordChange {
        if self.url_key_is_referenced(&url_key) {
            return LinkedStylesheetSourceRecordChange::Unchanged;
        }
        if self.sources_by_url.remove(&url_key).is_some() {
            LinkedStylesheetSourceRecordChange::Updated
        } else {
            LinkedStylesheetSourceRecordChange::Unchanged
        }
    }

    fn url_key_is_referenced(&self, url_key: &str) -> bool {
        if self.direct_request_url_keys.contains(url_key) {
            return true;
        }
        self.final_url_by_request_url
            .iter()
            .any(|(request_url, final_url)| request_url == url_key || final_url == url_key)
    }
}

impl LinkedStylesheetSourceRecordChange {
    pub(in crate::style_engine) fn changed(self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Updated, _) | (_, Self::Updated) => Self::Updated,
            (Self::Inserted, _) | (_, Self::Inserted) => Self::Inserted,
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_store_records_only_redirect_aliases() {
        let mut store = LinkedStylesheetSources::default();
        let request_url = url::Url::parse("https://example.test/app.css").unwrap();
        let old_final_url = url::Url::parse("https://cdn.example.test/old.css").unwrap();

        store.record_source_for_url(
            &request_url,
            StyloStylesheetSource::new(
                ".target { color: rgb(1, 2, 3); }".to_owned(),
                old_final_url.clone(),
            ),
        );
        assert_eq!(
            store
                .final_url_by_request_url
                .get(request_url.as_str())
                .map(String::as_str),
            Some(old_final_url.as_str())
        );
        assert!(!store.direct_request_url_keys.contains(request_url.as_str()));
        assert!(store.sources_by_url.contains_key(old_final_url.as_str()));

        store.record_source_for_url(
            &request_url,
            StyloStylesheetSource::new(
                ".target { color: rgb(4, 5, 6); }".to_owned(),
                request_url.clone(),
            ),
        );

        assert!(
            !store
                .final_url_by_request_url
                .contains_key(request_url.as_str())
        );
        assert!(store.direct_request_url_keys.contains(request_url.as_str()));
        assert!(store.sources_by_url.contains_key(request_url.as_str()));
        assert!(!store.sources_by_url.contains_key(old_final_url.as_str()));
    }

    #[test]
    fn source_store_drops_direct_request_key_when_request_redirects() {
        let mut store = LinkedStylesheetSources::default();
        let request_url = url::Url::parse("https://example.test/app.css").unwrap();
        let final_url = url::Url::parse("https://cdn.example.test/app.css").unwrap();

        store.record_source_for_url(
            &request_url,
            StyloStylesheetSource::new(
                ".direct { color: rgb(1, 2, 3); }".to_owned(),
                request_url.clone(),
            ),
        );
        assert!(store.direct_request_url_keys.contains(request_url.as_str()));

        store.record_source_for_url(
            &request_url,
            StyloStylesheetSource::new(
                ".redirect { color: rgb(4, 5, 6); }".to_owned(),
                final_url.clone(),
            ),
        );

        assert!(!store.direct_request_url_keys.contains(request_url.as_str()));
        assert_eq!(
            store
                .final_url_by_request_url
                .get(request_url.as_str())
                .map(String::as_str),
            Some(final_url.as_str())
        );
        assert_eq!(
            store
                .source_for_url_ref(&request_url)
                .map(|source| source.serialized_css_text().to_string()),
            Some(".redirect { color: rgb(4, 5, 6); }".to_owned())
        );
    }

    #[test]
    fn source_store_keeps_final_url_referenced_by_another_request() {
        let mut store = LinkedStylesheetSources::default();
        let first_request_url = url::Url::parse("https://example.test/first.css").unwrap();
        let second_request_url = url::Url::parse("https://example.test/second.css").unwrap();
        let shared_final_url = url::Url::parse("https://cdn.example.test/shared.css").unwrap();
        let new_final_url = url::Url::parse("https://cdn.example.test/new.css").unwrap();

        store.record_source_for_url(
            &first_request_url,
            StyloStylesheetSource::new(
                ".first { color: rgb(1, 2, 3); }".to_owned(),
                shared_final_url.clone(),
            ),
        );
        store.record_source_for_url(
            &second_request_url,
            StyloStylesheetSource::new(
                ".second { color: rgb(4, 5, 6); }".to_owned(),
                shared_final_url.clone(),
            ),
        );

        store.record_source_for_url(
            &first_request_url,
            StyloStylesheetSource::new(
                ".first { color: rgb(7, 8, 9); }".to_owned(),
                new_final_url.clone(),
            ),
        );

        assert_eq!(
            store
                .final_url_by_request_url
                .get(first_request_url.as_str())
                .map(String::as_str),
            Some(new_final_url.as_str())
        );
        assert_eq!(
            store
                .final_url_by_request_url
                .get(second_request_url.as_str())
                .map(String::as_str),
            Some(shared_final_url.as_str())
        );
        assert!(store.sources_by_url.contains_key(shared_final_url.as_str()));
        assert!(store.sources_by_url.contains_key(new_final_url.as_str()));
    }

    #[test]
    fn source_store_keeps_direct_request_url_when_redirect_alias_is_removed() {
        let mut store = LinkedStylesheetSources::default();
        let redirect_request_url = url::Url::parse("https://example.test/redirect.css").unwrap();
        let direct_request_url = url::Url::parse("https://cdn.example.test/shared.css").unwrap();
        let new_final_url = url::Url::parse("https://cdn.example.test/new.css").unwrap();

        store.record_source_for_url(
            &redirect_request_url,
            StyloStylesheetSource::new(
                ".redirect { color: rgb(1, 2, 3); }".to_owned(),
                direct_request_url.clone(),
            ),
        );
        store.record_source_for_url(
            &direct_request_url,
            StyloStylesheetSource::new(
                ".direct { color: rgb(4, 5, 6); }".to_owned(),
                direct_request_url.clone(),
            ),
        );

        store.record_source_for_url(
            &redirect_request_url,
            StyloStylesheetSource::new(
                ".redirect { color: rgb(7, 8, 9); }".to_owned(),
                new_final_url.clone(),
            ),
        );

        assert!(
            store
                .sources_by_url
                .contains_key(direct_request_url.as_str())
        );
        assert_eq!(
            store
                .source_for_url_ref(&direct_request_url)
                .map(|source| source.serialized_css_text().to_string()),
            Some(".direct { color: rgb(4, 5, 6); }".to_owned())
        );
        assert!(store.sources_by_url.contains_key(new_final_url.as_str()));
    }
}

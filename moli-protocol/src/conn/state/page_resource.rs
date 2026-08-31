use std::collections::HashMap;

use url::Url;

use crate::conn::CapturedBody;

#[derive(Debug, Clone)]
pub(crate) struct MainDocumentResourceSnapshot {
    pub(crate) frame_id: String,
    pub(crate) url: Url,
    pub(crate) response_headers: Vec<(String, String)>,
    pub(crate) from_cache: bool,
    pub(crate) body: Option<CapturedBody>,
}

#[derive(Debug, Clone)]
struct MainDocumentResourceEntry {
    frame_id: String,
    url: Url,
    response_headers: Vec<(String, String)>,
    from_cache: bool,
    body: Option<CapturedBody>,
}

impl MainDocumentResourceEntry {
    fn snapshot(&self) -> MainDocumentResourceSnapshot {
        MainDocumentResourceSnapshot {
            frame_id: self.frame_id.clone(),
            url: self.url.clone(),
            response_headers: self.response_headers.clone(),
            from_cache: self.from_cache,
            body: self.body.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TargetPageResourceStore {
    committed_loader_id: Option<String>,
    main_documents: HashMap<String, MainDocumentResourceEntry>,
}

impl TargetPageResourceStore {
    pub(crate) fn record_main_document_body(
        &mut self,
        frame_id: String,
        loader_id: String,
        url: Url,
        response_headers: Vec<(String, String)>,
        from_cache: bool,
        body: CapturedBody,
    ) {
        self.main_documents.retain(|candidate_loader_id, _| {
            self.committed_loader_id.as_deref() == Some(candidate_loader_id.as_str())
                || candidate_loader_id == &loader_id
        });
        let entry = self
            .main_documents
            .entry(loader_id.clone())
            .or_insert_with(|| MainDocumentResourceEntry {
                frame_id: frame_id.clone(),
                url: url.clone(),
                response_headers: response_headers.clone(),
                from_cache,
                body: None,
            });
        entry.frame_id = frame_id;
        entry.url = url;
        entry.response_headers = response_headers;
        entry.from_cache = from_cache;
        entry.body = Some(body);
    }

    pub(crate) fn commit_main_document(
        &mut self,
        frame_id: String,
        loader_id: String,
        url: Url,
        response_headers: Vec<(String, String)>,
        from_cache: bool,
        body: Option<CapturedBody>,
    ) {
        let body = body.or_else(|| {
            self.main_documents
                .remove(&loader_id)
                .and_then(|entry| entry.body)
        });
        self.main_documents.clear();
        self.main_documents.insert(
            loader_id.clone(),
            MainDocumentResourceEntry {
                frame_id,
                url,
                response_headers,
                from_cache,
                body,
            },
        );
        self.committed_loader_id = Some(loader_id);
    }

    pub(crate) fn main_document_for_loader(
        &self,
        loader_id: &str,
    ) -> Option<MainDocumentResourceSnapshot> {
        (self.committed_loader_id.as_deref() == Some(loader_id))
            .then(|| self.main_documents.get(loader_id))
            .flatten()
            .map(MainDocumentResourceEntry::snapshot)
    }

    pub(crate) fn discard_uncommitted_loader(&mut self, loader_id: &str) {
        if self.committed_loader_id.as_deref() != Some(loader_id) {
            self.main_documents.remove(loader_id);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.committed_loader_id.is_none() && self.main_documents.is_empty()
    }

    pub(crate) fn retained_body_bytes(&self) -> usize {
        self.main_documents
            .values()
            .filter_map(|entry| entry.body.as_ref())
            .map(CapturedBody::len)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(value: &str) -> CapturedBody {
        CapturedBody::from_string(value.to_owned())
    }

    #[test]
    fn body_that_finishes_before_commit_is_adopted_by_loader() {
        let mut store = TargetPageResourceStore::default();
        store.record_main_document_body(
            "FRAME".to_owned(),
            "LOADER-2".to_owned(),
            Url::parse("https://example.test/final").unwrap(),
            vec![("content-type".to_owned(), "text/html".to_owned())],
            true,
            body("source"),
        );

        store.commit_main_document(
            "FRAME".to_owned(),
            "LOADER-2".to_owned(),
            Url::parse("https://example.test/final").unwrap(),
            vec![("content-type".to_owned(), "text/html".to_owned())],
            true,
            None,
        );

        let resource = store.main_document_for_loader("LOADER-2").unwrap();
        assert!(resource.from_cache);
        assert_eq!(
            resource.body.unwrap().materialize_bytes().unwrap(),
            b"source"
        );
    }

    #[test]
    fn committing_new_loader_drops_previous_and_unrelated_candidates() {
        let mut store = TargetPageResourceStore::default();
        store.commit_main_document(
            "FRAME".to_owned(),
            "LOADER-1".to_owned(),
            Url::parse("https://example.test/one").unwrap(),
            Vec::new(),
            false,
            Some(body("one")),
        );
        store.record_main_document_body(
            "FRAME".to_owned(),
            "STALE".to_owned(),
            Url::parse("https://example.test/stale").unwrap(),
            Vec::new(),
            false,
            body("stale"),
        );
        store.commit_main_document(
            "FRAME".to_owned(),
            "LOADER-2".to_owned(),
            Url::parse("https://example.test/two").unwrap(),
            Vec::new(),
            false,
            Some(body("two")),
        );

        assert!(store.main_document_for_loader("LOADER-1").is_none());
        assert!(store.main_document_for_loader("STALE").is_none());
        assert_eq!(
            store
                .main_document_for_loader("LOADER-2")
                .unwrap()
                .body
                .unwrap()
                .materialize_bytes()
                .unwrap(),
            b"two"
        );
    }

    #[test]
    fn body_that_finishes_after_commit_fills_the_committed_loader() {
        let mut store = TargetPageResourceStore::default();
        let url = Url::parse("https://example.test/streamed").unwrap();
        let headers = vec![("content-type".to_owned(), "text/html".to_owned())];
        store.commit_main_document(
            "FRAME".to_owned(),
            "LOADER".to_owned(),
            url.clone(),
            headers.clone(),
            true,
            None,
        );

        store.record_main_document_body(
            "FRAME".to_owned(),
            "LOADER".to_owned(),
            url,
            headers,
            true,
            body("late source"),
        );

        let resource = store.main_document_for_loader("LOADER").unwrap();
        assert!(resource.from_cache);
        assert_eq!(
            resource.body.unwrap().materialize_bytes().unwrap(),
            b"late source"
        );
    }
}

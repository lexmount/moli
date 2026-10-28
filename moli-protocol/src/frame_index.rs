//! Frame identity and ownership, independent of protocol subscriptions.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRecord {
    pub id: String,
    pub parent: Option<String>,
    pub page: String,
    pub browser_context_id: Option<String>,
    pub url: String,
    pub loader_id: Option<String>,
    pub document: Option<moli_core::page::RendererDocumentLifecycleIdentity>,
}

impl FrameRecord {
    pub fn new(
        id: impl Into<String>,
        parent: Option<String>,
        page: impl Into<String>,
        browser_context_id: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            parent,
            page: page.into(),
            browser_context_id,
            url: "about:blank".to_owned(),
            loader_id: None,
            document: None,
        }
    }
}

/// The core owns live records; protocol adapters may retain a projection of
/// these records for event filtering. Allocating a frontend session is not a
/// condition for registering a Frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameIndex {
    records: BTreeMap<String, FrameRecord>,
}

impl FrameIndex {
    pub fn get(&self, id: &str) -> Option<&FrameRecord> {
        self.records.get(id)
    }

    pub fn insert(&mut self, record: FrameRecord) -> bool {
        self.records.insert(record.id.clone(), record).is_none()
    }

    pub fn attach(&mut self, mut record: FrameRecord) -> bool {
        if let Some(existing) = self.get(&record.id) {
            record.url.clone_from(&existing.url);
            record.loader_id.clone_from(&existing.loader_id);
            record.document = existing.document;
        }
        self.insert(record)
    }

    pub fn update_document(&mut self, id: &str, url: &str, loader_id: Option<&str>) {
        if let Some(record) = self.records.get_mut(id) {
            record.url = url.to_owned();
            record.loader_id = loader_id.map(str::to_owned);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &FrameRecord> {
        self.records.values()
    }

    pub fn roots(&self) -> impl Iterator<Item = &FrameRecord> {
        self.iter().filter(|record| record.id == record.page)
    }

    pub fn top_level_context<'a>(&'a self, context: &'a str) -> &'a str {
        self.get(context).map_or(context, |record| &record.page)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&FrameRecord) -> bool) {
        self.records.retain(|_, record| keep(record));
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn replace_page(&mut self, page: &str, records: impl IntoIterator<Item = FrameRecord>) {
        self.retain(|record| record.page != page);
        for record in records {
            assert_eq!(
                record.page, page,
                "one Page snapshot cannot change another Page"
            );
            self.insert(record);
        }
    }

    /// Remove descendants before their parent, including partially known
    /// subtrees. This also makes a parent-only detach revoke all child IDs.
    fn subtree_ids(&self, id: &str) -> Vec<String> {
        let mut children = BTreeMap::<&str, Vec<&str>>::new();
        for record in self.iter() {
            if let Some(parent) = record.parent.as_deref() {
                children.entry(parent).or_default().push(&record.id);
            }
        }
        let mut pending = vec![(id, false)];
        let mut visited = BTreeSet::new();
        let mut removed_ids = Vec::new();
        while let Some((current, expanded)) = pending.pop() {
            if expanded {
                removed_ids.push(current.to_owned());
            } else if visited.insert(current) {
                pending.push((current, true));
                if let Some(children) = children.get(current) {
                    pending.extend(children.iter().map(|child| (*child, false)));
                }
            }
        }
        removed_ids
    }

    pub fn subtree(&self, id: &str) -> Vec<FrameRecord> {
        self.subtree_ids(id)
            .into_iter()
            .filter_map(|id| self.get(&id).cloned())
            .collect()
    }

    pub fn remove_subtree(&mut self, id: &str) -> Vec<FrameRecord> {
        self.subtree_ids(id)
            .into_iter()
            .filter_map(|id| self.records.remove(&id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, parent: Option<&str>, page: &str) -> FrameRecord {
        FrameRecord::new(
            id,
            parent.map(str::to_owned),
            page,
            Some(format!("context-{page}")),
        )
    }

    #[test]
    fn removing_parent_revokes_nested_frames_and_preserves_other_pages() {
        let mut index = FrameIndex::default();
        for frame in [
            record("page-a", None, "page-a"),
            record("outer", Some("page-a"), "page-a"),
            record("inner", Some("outer"), "page-a"),
            record("sibling", Some("page-a"), "page-a"),
            record("page-b", None, "page-b"),
            record("other", Some("page-b"), "page-b"),
        ] {
            index.insert(frame);
        }
        let removed = index.remove_subtree("outer");
        assert_eq!(
            removed
                .iter()
                .map(|frame| frame.id.as_str())
                .collect::<Vec<_>>(),
            ["inner", "outer"]
        );
        assert!(index.get("inner").is_none());
        assert_eq!(index.top_level_context("sibling"), "page-a");
        assert_eq!(
            index.get("other").unwrap().browser_context_id.as_deref(),
            Some("context-page-b")
        );
        assert_eq!(index.len(), 4);
    }

    #[test]
    fn page_replacement_retires_children_without_changing_other_owners() {
        let mut index = FrameIndex::default();
        for frame in [
            record("a", None, "a"),
            record("child", Some("a"), "a"),
            record("b", None, "b"),
        ] {
            index.insert(frame);
        }
        index.replace_page("a", [record("a", None, "a")]);
        assert!(index.get("child").is_none());
        assert!(index.get("a").is_some());
        assert!(index.get("b").is_some());
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn frame_identity_survives_updates_and_duplicate_attach_is_not_new() {
        let mut index = FrameIndex::default();
        assert!(index.attach(record("child", Some("a"), "a")));
        index.update_document("child", "https://example.test/", Some("loader-1"));
        assert!(!index.attach(record("child", Some("wrapper"), "a")));
        assert_eq!(index.get("child").unwrap().url, "https://example.test/");
        assert_eq!(
            index.get("child").unwrap().loader_id.as_deref(),
            Some("loader-1")
        );
        assert_eq!(
            index.get("child").unwrap().parent.as_deref(),
            Some("wrapper")
        );
        assert_eq!(index.top_level_context("child"), "a");
        assert_eq!(index.len(), 1);
    }
}

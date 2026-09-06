use moli_core::page::SameDocumentHistoryUpdate;

pub(crate) enum HistoryTraversalDestination {
    Entry(i32),
    Delta(i64),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolvedHistoryTraversal {
    Noop,
    Entry {
        entry_id: i32,
        url: String,
        same_document_delta: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageNavigationHistoryEntry {
    pub id: i32,
    pub url: String,
    pub user_typed_url: String,
    pub title: String,
    pub transition_type: String,
    pub document_sequence_number: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingNavigationHistoryUpdate {
    ReplaceCurrent,
    ReplaceInitialEmptyDocument,
    TraverseToEntry(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NavigationHistoryState {
    entries: Vec<PageNavigationHistoryEntry>,
    current_index: Option<usize>,
    next_entry_id: i32,
    next_document_sequence_number: u64,
    pending_update: Option<PendingNavigationHistoryUpdate>,
}

impl Default for NavigationHistoryState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            current_index: None,
            next_entry_id: 1,
            next_document_sequence_number: 1,
            pending_update: None,
        }
    }
}

impl NavigationHistoryState {
    pub(super) fn resolve_traversal(
        &self,
        destination: HistoryTraversalDestination,
    ) -> Result<ResolvedHistoryTraversal, &'static str> {
        let current_index = self.current_index.ok_or("NoSuchHistoryEntry")?;
        let target_index = match destination {
            HistoryTraversalDestination::Entry(id) => self
                .entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or("NoSuchHistoryEntry")?,
            HistoryTraversalDestination::Delta(delta) => {
                usize::try_from(current_index as i128 + i128::from(delta))
                    .map_err(|_| "NoSuchHistoryEntry")?
            }
        };
        let current = self
            .entries
            .get(current_index)
            .ok_or("NoSuchHistoryEntry")?;
        let target = self.entries.get(target_index).ok_or("NoSuchHistoryEntry")?;
        if current_index == target_index {
            return Ok(ResolvedHistoryTraversal::Noop);
        }
        let same_document_delta = if current.document_sequence_number.is_some()
            && current.document_sequence_number == target.document_sequence_number
        {
            Some(
                i64::try_from(target_index as i128 - current_index as i128)
                    .map_err(|_| "NoSuchHistoryEntry")?,
            )
        } else {
            None
        };
        Ok(ResolvedHistoryTraversal::Entry {
            entry_id: target.id,
            url: target.url.clone(),
            same_document_delta,
        })
    }

    pub(super) fn current_title(&self) -> Option<&str> {
        Some(self.entries.get(self.current_index?)?.title.as_str())
    }

    pub(super) fn current_url(&self) -> Option<&str> {
        Some(self.entries.get(self.current_index?)?.url.as_str())
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn allocate_entry_id(&mut self) -> i32 {
        let id = self.next_entry_id;
        self.next_entry_id = self
            .next_entry_id
            .checked_add(1)
            .expect("Page navigation history entry id overflow");
        id
    }

    fn push_entry(&mut self, entry: PageNavigationHistoryEntry) {
        if let Some(current_index) = self.current_index {
            self.entries.truncate(current_index + 1);
        }
        self.entries.push(entry);
        self.current_index = self.entries.len().checked_sub(1);
    }

    fn allocate_document_sequence_number(&mut self) -> u64 {
        let sequence_number = self.next_document_sequence_number;
        self.next_document_sequence_number = self
            .next_document_sequence_number
            .checked_add(1)
            .expect("Page navigation Document sequence number overflow");
        sequence_number
    }

    fn assign_new_document_sequence_number(&mut self, entry: &mut PageNavigationHistoryEntry) {
        entry.document_sequence_number = Some(self.allocate_document_sequence_number());
    }

    fn assign_current_document_sequence_number(&mut self, entry: &mut PageNavigationHistoryEntry) {
        entry.document_sequence_number = self
            .current_index
            .and_then(|index| self.entries.get(index))
            .and_then(|entry| entry.document_sequence_number)
            .or_else(|| Some(self.allocate_document_sequence_number()));
    }

    fn replace_current_entry(&mut self, mut entry: PageNavigationHistoryEntry) {
        if let Some(current_index) = self.current_index
            && let Some(current_entry) = self.entries.get_mut(current_index)
        {
            entry.id = current_entry.id;
            *current_entry = entry;
            return;
        }
        self.push_entry(entry);
    }

    fn traverse_to_entry(&mut self, entry_id: i32, mut loaded_entry: PageNavigationHistoryEntry) {
        if let Some(index) = self.entries.iter().position(|entry| entry.id == entry_id) {
            loaded_entry.transition_type = self.entries[index].transition_type.clone();
            loaded_entry.user_typed_url = self.entries[index].user_typed_url.clone();
            loaded_entry.document_sequence_number = self.entries[index].document_sequence_number;
            loaded_entry.id = entry_id;
            self.entries[index] = loaded_entry;
            self.current_index = Some(index);
            return;
        }
        self.push_entry(loaded_entry);
    }

    pub(super) fn mark_replace_current(&mut self) {
        self.pending_update = Some(PendingNavigationHistoryUpdate::ReplaceCurrent);
    }

    pub(super) fn mark_replace_initial_empty_document(&mut self) {
        self.pending_update = Some(PendingNavigationHistoryUpdate::ReplaceInitialEmptyDocument);
    }

    pub(super) fn mark_traverse_to_entry(&mut self, entry_id: i32) {
        self.pending_update = Some(PendingNavigationHistoryUpdate::TraverseToEntry(entry_id));
    }

    pub(super) fn clear_pending_update(&mut self) {
        self.pending_update = None;
    }

    pub(super) fn take_pending_update(&mut self) -> Option<PendingNavigationHistoryUpdate> {
        self.pending_update.take()
    }

    pub(super) fn entry_url(&self, entry_id: i32) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| entry.url.clone())
    }

    pub(super) fn refresh_current_entry_title(&mut self, title: String) -> bool {
        let Some(current_entry) = self
            .current_index
            .and_then(|current_index| self.entries.get_mut(current_index))
        else {
            return false;
        };
        if current_entry.title == title {
            return false;
        }
        current_entry.title = title;
        true
    }

    pub(super) fn snapshot(&self) -> (usize, Vec<PageNavigationHistoryEntry>) {
        (self.current_index.unwrap_or(0), self.entries.clone())
    }

    pub(super) fn can_prune_all_but_current(&self) -> bool {
        !matches!(
            self.pending_update,
            Some(PendingNavigationHistoryUpdate::TraverseToEntry(_))
        ) && self
            .current_index
            .is_some_and(|current_index| current_index < self.entries.len())
    }

    pub(super) fn prune_all_but_current(&mut self) -> bool {
        if !self.can_prune_all_but_current() {
            return false;
        }
        let Some(current_index) = self.current_index else {
            return false;
        };
        let Some(current_entry) = self.entries.get(current_index).cloned() else {
            return false;
        };
        self.entries.clear();
        self.entries.push(current_entry);
        self.current_index = Some(0);
        true
    }

    pub(super) fn seed_entry(&mut self, mut entry: PageNavigationHistoryEntry) {
        self.assign_new_document_sequence_number(&mut entry);
        self.push_entry(entry);
    }

    pub(super) fn record_loaded_entry(
        &mut self,
        mut entry: PageNavigationHistoryEntry,
        update: Option<PendingNavigationHistoryUpdate>,
    ) {
        match update {
            Some(PendingNavigationHistoryUpdate::ReplaceCurrent) => {
                entry.transition_type = "reload".to_owned();
                if let Some(current_entry) =
                    self.current_index.and_then(|index| self.entries.get(index))
                {
                    entry.user_typed_url = current_entry.user_typed_url.clone();
                }
                self.assign_new_document_sequence_number(&mut entry);
                self.replace_current_entry(entry);
            }
            Some(PendingNavigationHistoryUpdate::ReplaceInitialEmptyDocument) => {
                entry.transition_type = "auto_toplevel".to_owned();
                self.assign_new_document_sequence_number(&mut entry);
                self.replace_current_entry(entry);
            }
            Some(PendingNavigationHistoryUpdate::TraverseToEntry(entry_id)) => {
                self.traverse_to_entry(entry_id, entry);
            }
            None => {
                self.assign_new_document_sequence_number(&mut entry);
                self.push_entry(entry);
            }
        }
    }

    pub(super) fn record_same_document_update(
        &mut self,
        url: String,
        title: String,
        history_update: SameDocumentHistoryUpdate,
    ) -> bool {
        match history_update {
            SameDocumentHistoryUpdate::Push | SameDocumentHistoryUpdate::Replace => {
                let mut entry = PageNavigationHistoryEntry {
                    id: self.allocate_entry_id(),
                    url,
                    user_typed_url: self
                        .current_index
                        .and_then(|index| self.entries.get(index))
                        .map(|entry| entry.user_typed_url.clone())
                        .unwrap_or_default(),
                    title,
                    transition_type: "link".to_owned(),
                    document_sequence_number: None,
                };
                self.assign_current_document_sequence_number(&mut entry);
                match history_update {
                    SameDocumentHistoryUpdate::Push => self.push_entry(entry),
                    SameDocumentHistoryUpdate::Replace => self.replace_current_entry(entry),
                    SameDocumentHistoryUpdate::Traverse { .. } => unreachable!(),
                }
                true
            }
            SameDocumentHistoryUpdate::Traverse { delta } => {
                let Some(current_index) = self.current_index else {
                    return false;
                };
                let Ok(current_index) = i64::try_from(current_index) else {
                    return false;
                };
                let Some(target_index) = current_index.checked_add(delta) else {
                    return false;
                };
                let Ok(target_index) = usize::try_from(target_index) else {
                    return false;
                };
                let Some(target_entry) = self.entries.get(target_index) else {
                    return false;
                };
                if target_entry.url != url {
                    return false;
                }
                self.current_index = Some(target_index);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_traversal_resolution_is_native_read_only_and_document_aware() {
        let mut history = NavigationHistoryState::default();
        for url in ["https://example.test/a", "https://example.test/a#one"] {
            assert!(history.record_same_document_update(
                url.into(),
                "A".into(),
                SameDocumentHistoryUpdate::Push
            ));
        }
        let before = history.clone();
        for destination in [
            HistoryTraversalDestination::Entry(1),
            HistoryTraversalDestination::Delta(-1),
        ] {
            assert_eq!(
                history.resolve_traversal(destination),
                Ok(ResolvedHistoryTraversal::Entry {
                    entry_id: 1,
                    url: "https://example.test/a".into(),
                    same_document_delta: Some(-1),
                })
            );
        }
        for destination in [
            HistoryTraversalDestination::Entry(2),
            HistoryTraversalDestination::Delta(0),
        ] {
            assert_eq!(
                history.resolve_traversal(destination),
                Ok(ResolvedHistoryTraversal::Noop)
            );
        }
        for destination in [
            HistoryTraversalDestination::Entry(42),
            HistoryTraversalDestination::Delta(1),
            HistoryTraversalDestination::Delta(-2),
            HistoryTraversalDestination::Delta(i64::MAX),
            HistoryTraversalDestination::Delta(i64::MIN),
        ] {
            assert_eq!(
                history.resolve_traversal(destination),
                Err("NoSuchHistoryEntry")
            );
        }
        assert_eq!(history, before);
        let mut next = history.snapshot().1[0].clone();
        next.id = history.allocate_entry_id();
        history.record_loaded_entry(next, None);
        assert_eq!(
            history.resolve_traversal(HistoryTraversalDestination::Entry(1)),
            Ok(ResolvedHistoryTraversal::Entry {
                entry_id: 1,
                url: "https://example.test/a".into(),
                same_document_delta: None,
            }),
            "equal URLs do not make two Browser Documents the same history document"
        );
        assert_eq!(
            NavigationHistoryState::default()
                .resolve_traversal(HistoryTraversalDestination::Delta(0)),
            Err("NoSuchHistoryEntry")
        );
    }

    #[test]
    fn same_document_history_traversal_rejects_mismatched_url_atomically() {
        let mut history = NavigationHistoryState::default();
        for url in ["https://example.test/a", "https://example.test/b"] {
            assert!(history.record_same_document_update(
                url.into(),
                String::new(),
                SameDocumentHistoryUpdate::Push
            ));
        }
        let before = history.clone();
        assert!(!history.record_same_document_update(
            "https://example.test/wrong".into(),
            String::new(),
            SameDocumentHistoryUpdate::Traverse { delta: -1 }
        ));
        assert_eq!(history, before);
    }

    #[test]
    fn same_document_traversal_moves_cursor_without_allocating_or_appending() {
        let mut history = NavigationHistoryState::default();
        assert!(history.record_same_document_update(
            "https://example.test/a".to_owned(),
            "A".to_owned(),
            SameDocumentHistoryUpdate::Push,
        ));
        assert!(history.record_same_document_update(
            "https://example.test/b".to_owned(),
            "B".to_owned(),
            SameDocumentHistoryUpdate::Push,
        ));

        assert!(history.record_same_document_update(
            "https://example.test/a".to_owned(),
            "ignored during traversal".to_owned(),
            SameDocumentHistoryUpdate::Traverse { delta: -1 },
        ));
        let (current_index, entries) = history.snapshot();
        assert_eq!(current_index, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            vec![1, 2]
        );

        assert!(history.record_same_document_update(
            "https://example.test/c".to_owned(),
            "C".to_owned(),
            SameDocumentHistoryUpdate::Push,
        ));
        let (current_index, entries) = history.snapshot();
        assert_eq!(current_index, 1);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.id, entry.url.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "https://example.test/a"), (3, "https://example.test/c")],
            "a traversal must neither allocate an id nor survive as a forward entry after push"
        );
    }

    #[test]
    fn same_document_replace_preserves_entry_id_and_out_of_range_traverse_is_atomic() {
        let mut history = NavigationHistoryState::default();
        assert!(history.record_same_document_update(
            "https://example.test/a".to_owned(),
            "A".to_owned(),
            SameDocumentHistoryUpdate::Push,
        ));
        assert!(history.record_same_document_update(
            "https://example.test/replaced".to_owned(),
            "Replaced".to_owned(),
            SameDocumentHistoryUpdate::Replace,
        ));
        let before = history.snapshot();
        assert_eq!(before.0, 0);
        assert_eq!(before.1[0].id, 1);
        assert_eq!(before.1[0].url, "https://example.test/replaced");

        assert!(!history.record_same_document_update(
            "https://example.test/missing".to_owned(),
            String::new(),
            SameDocumentHistoryUpdate::Traverse { delta: -1 },
        ));
        assert_eq!(history.snapshot(), before);
    }

    #[test]
    fn title_refresh_updates_only_current_entry_metadata() {
        let mut history = NavigationHistoryState::default();
        let first_id = history.allocate_entry_id();
        history.seed_entry(PageNavigationHistoryEntry {
            id: first_id,
            url: "https://example.test/a".to_owned(),
            user_typed_url: "https://example.test/a".to_owned(),
            title: String::new(),
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        });
        let before = history.snapshot().1[0].clone();

        assert!(history.refresh_current_entry_title("A".to_owned()));
        assert!(!history.refresh_current_entry_title("A".to_owned()));

        let refreshed = &history.snapshot().1[0];
        assert_eq!(refreshed.title, "A");
        assert_eq!(refreshed.id, before.id);
        assert_eq!(refreshed.url, before.url);
        assert_eq!(refreshed.user_typed_url, before.user_typed_url);
        assert_eq!(
            refreshed.document_sequence_number,
            before.document_sequence_number
        );
    }
}

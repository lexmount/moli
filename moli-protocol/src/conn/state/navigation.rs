use moli_core::page::SameDocumentHistoryUpdate;

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
pub enum PendingNavigationHistoryUpdate {
    ReplaceCurrent,
    ReplaceInitialEmptyDocument,
    TraverseToEntry(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetNavigationHistoryState {
    entries: Vec<PageNavigationHistoryEntry>,
    current_index: Option<usize>,
    next_entry_id: i32,
    next_document_sequence_number: u64,
    pending_update: Option<PendingNavigationHistoryUpdate>,
}

impl Default for TargetNavigationHistoryState {
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

impl TargetNavigationHistoryState {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn allocate_entry_id(&mut self) -> i32 {
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

    pub(crate) fn mark_replace_current(&mut self) {
        self.pending_update = Some(PendingNavigationHistoryUpdate::ReplaceCurrent);
    }

    pub(crate) fn mark_replace_initial_empty_document(&mut self) {
        self.pending_update = Some(PendingNavigationHistoryUpdate::ReplaceInitialEmptyDocument);
    }

    pub(crate) fn mark_traverse_to_entry(&mut self, entry_id: i32) {
        self.pending_update = Some(PendingNavigationHistoryUpdate::TraverseToEntry(entry_id));
    }

    pub(crate) fn clear_pending_update(&mut self) {
        self.pending_update = None;
    }

    pub(crate) fn entry_url(&self, entry_id: i32) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| entry.url.clone())
    }

    pub(crate) fn refresh_current_entry_title(&mut self, title: String) -> bool {
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

    pub(crate) fn snapshot(&self) -> (usize, Vec<PageNavigationHistoryEntry>) {
        (self.current_index.unwrap_or(0), self.entries.clone())
    }

    pub(crate) fn can_prune_all_but_current(&self) -> bool {
        !matches!(
            self.pending_update,
            Some(PendingNavigationHistoryUpdate::TraverseToEntry(_))
        ) && self
            .current_index
            .is_some_and(|current_index| current_index < self.entries.len())
    }

    pub(crate) fn prune_all_but_current(&mut self) -> bool {
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

    pub(crate) fn seed_entry(&mut self, mut entry: PageNavigationHistoryEntry) {
        self.assign_new_document_sequence_number(&mut entry);
        self.push_entry(entry);
    }

    pub(crate) fn record_loaded_entry(&mut self, mut entry: PageNavigationHistoryEntry) {
        match self.pending_update.take() {
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

    pub(crate) fn record_same_document_update(
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
                debug_assert_eq!(
                    target_entry.url, url,
                    "renderer/browser same-document traversal URL drift: current_index={current_index}, delta={delta}, target_index={target_index}, browser_target_url={}, renderer_target_url={url}",
                    target_entry.url,
                );
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
    fn same_document_traversal_moves_cursor_without_allocating_or_appending() {
        let mut history = TargetNavigationHistoryState::default();
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
        let mut history = TargetNavigationHistoryState::default();
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
        let mut history = TargetNavigationHistoryState::default();
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

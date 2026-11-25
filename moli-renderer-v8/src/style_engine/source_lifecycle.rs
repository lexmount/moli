use indexmap::{IndexMap, IndexSet};
use moli_selector::stylo_shadow_root_host_participates_in_style_scope as shadow_root_host_participates_in_style_scope;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

use super::source_id::{StyleScopeId, StyleSourceId, StyleSourceKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleSourceLifecycleReport {
    records: Vec<StyleSourceLifecycleRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleSourceLifecycleRecord {
    owner: StyleSourceLifecycleOwner,
    document_kind: Option<StyleSourceDocumentKind>,
    availability: StyleSourceLifecycleAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleSourceLifecycleOwnerDetailTrace {
    owner: StyleSourceLifecycleOwner,
    document_kind: Option<StyleSourceDocumentKind>,
    availability: StyleSourceLifecycleAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrackedStyleSourceLifecycleEntry {
    owner: StyleSourceLifecycleOwner,
    state: TrackedStyleSourceLifecycleState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleSourceLifecycleSourceRequests {
    source_ids_by_owner: IndexMap<StyleSourceLifecycleOwner, IndexSet<StyleSourceId>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TrackedStyleSourceLifecycleState {
    Available(TrackedStyleSourceIds),
    Unavailable(StyleSourceLifecycleUnavailableReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TrackedStyleSourceIds {
    Retained(TrackedRetainedStyleSourceIds),
    WithoutSource(StyleSourceLifecycleWithoutSourceReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrackedRetainedStyleSourceIds(Vec<StyleSourceId>);

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum StyleSourceLifecycleOwner {
    OwnerStyleSheet { owner: DomHandle },
    LinkedStyleSheet { owner: DomHandle },
    DocumentAdoptedStyleSheets { document: DomHandle },
    ShadowRootAdoptedStyleSheets { root: DomHandle },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StyleSourceLifecycleAvailability {
    RetainedSources {
        source_ids: Vec<StyleSourceId>,
    },
    AvailableWithoutSource {
        reason: StyleSourceLifecycleWithoutSourceReason,
    },
    Unavailable {
        reason: StyleSourceLifecycleUnavailableReason,
    },
}

pub(super) trait StyleSourceLifecycleSnapshotSink {
    fn record_source_lifecycle_snapshot(&mut self, snapshot: StyleSourceLifecycleSnapshot);
}

pub(super) trait StyleSourceLifecycleOwnerDetailTraceSink {
    fn record_source_lifecycle_owner_detail_trace(
        &mut self,
        trace: StyleSourceLifecycleOwnerDetailTrace,
    );
}

pub(super) trait StyleSourceLifecycleRetainedSourceIdSink {
    fn record_source_lifecycle_retained_source_id(&mut self, source_id: &StyleSourceId);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StyleSourceLifecycleWithoutSourceReason {
    OwnerStyleSheetSourceMissing,
    LinkedStyleSheetOwnerInactive,
    LinkedStyleSheetSourceMissing,
    SourceIdMissing,
    EmptyDocumentAdoptedStyleSheets,
    EmptyShadowRootAdoptedStyleSheets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StyleSourceLifecycleUnavailableReason {
    MissingNode,
    OwnerNotInDocumentTree,
    InactiveShadowRoot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StyleSourceOwnerAvailability {
    Available,
    MissingNode,
    OwnerNotInDocumentTree,
    InactiveShadowRoot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StyleSourceDocumentContext<'a> {
    root_document: DomHandle,
    child_documents: &'a [DomHandle],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedStyleSourceDocumentContext {
    root_document: DomHandle,
    child_documents: Vec<DomHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StyleSourceDocumentKind {
    Root,
    Child,
    Detached,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleSourceLifecycleSnapshot {
    tracked_owner_style_sheet_count: usize,
    retained_owner_style_sheet_source_count: usize,
    available_owner_style_sheet_without_source_count: usize,
    tracked_linked_style_sheet_owner_count: usize,
    retained_linked_style_sheet_source_count: usize,
    available_linked_style_sheet_without_source_count: usize,
    available_linked_style_sheet_without_loaded_source_count: usize,
    tracked_document_adopted_owner_count: usize,
    retained_document_adopted_source_count: usize,
    available_document_adopted_owner_without_source_count: usize,
    available_document_adopted_empty_owner_count: usize,
    tracked_shadow_adopted_owner_count: usize,
    retained_shadow_adopted_source_count: usize,
    available_shadow_adopted_owner_without_source_count: usize,
    available_shadow_adopted_empty_owner_count: usize,
    retained_root_document_source_count: usize,
    retained_child_document_source_count: usize,
    retained_detached_document_source_count: usize,
    available_root_document_owner_without_source_count: usize,
    available_child_document_owner_without_source_count: usize,
    available_detached_document_owner_without_source_count: usize,
    missing_node_owner_count: usize,
    owner_not_in_document_tree_count: usize,
    inactive_shadow_root_owner_count: usize,
}

impl StyleSourceLifecycleReport {
    pub(super) fn from_records(records: Vec<StyleSourceLifecycleRecord>) -> Self {
        Self { records }
    }

    pub(super) fn from_tracked_entries(
        host: &DomHost,
        document_context: StyleSourceDocumentContext<'_>,
        entries: impl IntoIterator<Item = TrackedStyleSourceLifecycleEntry>,
    ) -> Self {
        let mut owners = IndexSet::new();
        let records = entries
            .into_iter()
            .filter_map(|entry| {
                if !owners.insert(entry.owner) {
                    return None;
                }
                Some(entry.into_record(host, document_context))
            })
            .collect();
        Self::from_records(records)
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> StyleSourceLifecycleSnapshot {
        self.snapshot_inner()
    }

    fn snapshot_inner(&self) -> StyleSourceLifecycleSnapshot {
        let mut snapshot = StyleSourceLifecycleSnapshot::default();
        for record in &self.records {
            snapshot.record_lifecycle_record(record);
        }
        snapshot
    }

    pub(super) fn record_snapshot_into(&self, sink: &mut impl StyleSourceLifecycleSnapshotSink) {
        sink.record_source_lifecycle_snapshot(self.snapshot_inner());
    }

    pub(super) fn record_owner_detail_trace_into(
        &self,
        sink: &mut impl StyleSourceLifecycleOwnerDetailTraceSink,
    ) {
        for record in &self.records {
            sink.record_source_lifecycle_owner_detail_trace(record.owner_detail_trace());
        }
    }

    #[cfg(test)]
    pub(super) fn records(&self) -> &[StyleSourceLifecycleRecord] {
        &self.records
    }

    pub(super) fn record_retained_source_ids_into(
        &self,
        sink: &mut impl StyleSourceLifecycleRetainedSourceIdSink,
    ) {
        for record in &self.records {
            record.record_retained_source_ids_into(sink);
        }
    }

    #[cfg(test)]
    fn record_for_source_id(
        &self,
        source_id: &StyleSourceId,
    ) -> Option<StyleSourceLifecycleRecord> {
        let owner = StyleSourceLifecycleOwner::from_source_id(source_id)?;
        self.records
            .iter()
            .find(|record| record.owner == owner)
            .map(|record| record.for_source_id(source_id))
    }

    #[cfg(test)]
    pub(super) fn record_for_source_id_for_test(
        &self,
        source_id: &StyleSourceId,
    ) -> Option<StyleSourceLifecycleRecord> {
        self.record_for_source_id(source_id)
    }
}

impl StyleSourceOwnerAvailability {
    fn stylesheet_owner(host: &DomHost, owner: DomHandle) -> Self {
        if host.node(owner).is_none() {
            return Self::MissingNode;
        }
        if let Some(root) = host.containing_shadow_root(owner) {
            return if shadow_root_host_participates_in_style_scope(host, root) {
                Self::Available
            } else {
                Self::InactiveShadowRoot
            };
        }
        if host
            .owner_document_handle(owner)
            .is_some_and(|document| light_tree_owner_is_in_document_tree(host, owner, document))
        {
            Self::Available
        } else {
            Self::OwnerNotInDocumentTree
        }
    }

    fn document_adopted_owner(host: &DomHost, document: DomHandle) -> Self {
        if host.node(document).is_some() {
            Self::Available
        } else {
            Self::MissingNode
        }
    }

    fn shadow_root_adopted_owner(host: &DomHost, root: DomHandle) -> Self {
        if !host.is_shadow_root(root) {
            return Self::MissingNode;
        }
        if shadow_root_host_participates_in_style_scope(host, root) {
            Self::Available
        } else {
            Self::InactiveShadowRoot
        }
    }

    fn is_available(self) -> bool {
        self == Self::Available
    }

    fn unavailable_reason(self) -> Option<StyleSourceLifecycleUnavailableReason> {
        match self {
            Self::Available => None,
            Self::MissingNode => Some(StyleSourceLifecycleUnavailableReason::MissingNode),
            Self::OwnerNotInDocumentTree => {
                Some(StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree)
            }
            Self::InactiveShadowRoot => {
                Some(StyleSourceLifecycleUnavailableReason::InactiveShadowRoot)
            }
        }
    }
}

impl TrackedStyleSourceLifecycleEntry {
    pub(super) fn new(
        owner: StyleSourceLifecycleOwner,
        state: TrackedStyleSourceLifecycleState,
    ) -> Self {
        Self { owner, state }
    }

    pub(super) fn retaining_only_source_ids(
        mut self,
        requested_source_ids: &IndexSet<StyleSourceId>,
    ) -> Self {
        self.state
            .retain_only_requested_source_ids(requested_source_ids);
        self
    }

    fn into_record(
        self,
        host: &DomHost,
        document_context: StyleSourceDocumentContext<'_>,
    ) -> StyleSourceLifecycleRecord {
        let document_kind = self.owner.document_kind(host, document_context);
        self.state.into_record(self.owner, document_kind)
    }
}

impl StyleSourceLifecycleSourceRequests {
    pub(super) fn from_source_ids(source_ids: impl IntoIterator<Item = StyleSourceId>) -> Self {
        let mut source_ids_by_owner =
            IndexMap::<StyleSourceLifecycleOwner, IndexSet<StyleSourceId>>::new();
        for source_id in source_ids {
            let Some(owner) = StyleSourceLifecycleOwner::from_source_id(&source_id) else {
                continue;
            };
            source_ids_by_owner
                .entry(owner)
                .or_default()
                .insert(source_id);
        }
        Self {
            source_ids_by_owner,
        }
    }

    pub(super) fn into_tracked_entries(
        self,
        mut entry_for_owner: impl FnMut(
            StyleSourceLifecycleOwner,
        ) -> Option<TrackedStyleSourceLifecycleEntry>,
    ) -> impl Iterator<Item = TrackedStyleSourceLifecycleEntry> {
        self.source_ids_by_owner
            .into_iter()
            .filter_map(move |(owner, requested_source_ids)| {
                entry_for_owner(owner)
                    .map(|entry| entry.retaining_only_source_ids(&requested_source_ids))
            })
    }
}

impl StyleSourceLifecycleRecord {
    #[cfg(test)]
    pub(super) fn owner(&self) -> StyleSourceLifecycleOwner {
        self.owner
    }

    #[cfg(test)]
    pub(super) fn document_kind(&self) -> Option<StyleSourceDocumentKind> {
        self.document_kind
    }

    #[cfg(test)]
    pub(super) fn availability(&self) -> &StyleSourceLifecycleAvailability {
        &self.availability
    }

    fn retained_source_ids(&self) -> &[StyleSourceId] {
        match &self.availability {
            StyleSourceLifecycleAvailability::RetainedSources { source_ids } => source_ids,
            StyleSourceLifecycleAvailability::AvailableWithoutSource { .. }
            | StyleSourceLifecycleAvailability::Unavailable { .. } => &[],
        }
    }

    fn record_retained_source_ids_into(
        &self,
        sink: &mut impl StyleSourceLifecycleRetainedSourceIdSink,
    ) {
        for source_id in self.retained_source_ids() {
            sink.record_source_lifecycle_retained_source_id(source_id);
        }
    }

    fn owner_detail_trace(&self) -> StyleSourceLifecycleOwnerDetailTrace {
        StyleSourceLifecycleOwnerDetailTrace {
            owner: self.owner,
            document_kind: self.document_kind,
            availability: self.availability.clone(),
        }
    }

    #[cfg(test)]
    fn for_source_id(&self, source_id: &StyleSourceId) -> Self {
        let availability = match &self.availability {
            StyleSourceLifecycleAvailability::RetainedSources { source_ids } => {
                if source_ids.contains(source_id) {
                    StyleSourceLifecycleAvailability::RetainedSources {
                        source_ids: vec![source_id.clone()],
                    }
                } else {
                    StyleSourceLifecycleAvailability::AvailableWithoutSource {
                        reason: StyleSourceLifecycleWithoutSourceReason::SourceIdMissing,
                    }
                }
            }
            availability => availability.clone(),
        };
        Self {
            owner: self.owner,
            document_kind: self.document_kind,
            availability,
        }
    }
}

impl StyleSourceLifecycleOwnerDetailTrace {
    #[cfg(test)]
    pub(super) fn owner(&self) -> StyleSourceLifecycleOwner {
        self.owner
    }

    #[cfg(test)]
    pub(super) fn document_kind(&self) -> Option<StyleSourceDocumentKind> {
        self.document_kind
    }

    #[cfg(test)]
    pub(super) fn availability(&self) -> &StyleSourceLifecycleAvailability {
        &self.availability
    }
}

impl TrackedStyleSourceLifecycleState {
    fn for_owner_availability(
        availability: StyleSourceOwnerAvailability,
        source_ids: impl FnOnce() -> TrackedStyleSourceIds,
    ) -> Self {
        match availability.unavailable_reason() {
            Some(reason) => Self::Unavailable(reason),
            None => Self::Available(source_ids()),
        }
    }

    pub(super) fn into_record(
        self,
        owner: StyleSourceLifecycleOwner,
        document_kind: Option<StyleSourceDocumentKind>,
    ) -> StyleSourceLifecycleRecord {
        StyleSourceLifecycleRecord {
            owner,
            document_kind,
            availability: self.into_availability(),
        }
    }

    fn retain_only_requested_source_ids(&mut self, requested_source_ids: &IndexSet<StyleSourceId>) {
        let Self::Available(TrackedStyleSourceIds::Retained(source_ids)) = self else {
            return;
        };
        let had_retained_source_ids = !source_ids.is_empty();
        source_ids.retain(|source_id| requested_source_ids.contains(source_id));
        if had_retained_source_ids && source_ids.is_empty() {
            *self = Self::Available(TrackedStyleSourceIds::WithoutSource(
                StyleSourceLifecycleWithoutSourceReason::SourceIdMissing,
            ));
        }
    }

    fn into_availability(self) -> StyleSourceLifecycleAvailability {
        match self {
            Self::Unavailable(reason) => StyleSourceLifecycleAvailability::Unavailable { reason },
            Self::Available(TrackedStyleSourceIds::Retained(source_ids)) => {
                StyleSourceLifecycleAvailability::RetainedSources {
                    source_ids: source_ids.into(),
                }
            }
            Self::Available(TrackedStyleSourceIds::WithoutSource(reason)) => {
                StyleSourceLifecycleAvailability::AvailableWithoutSource { reason }
            }
        }
    }
}

impl TrackedStyleSourceIds {
    pub(super) fn retained_or_without_source(
        source_ids: Vec<StyleSourceId>,
        without_source_reason: StyleSourceLifecycleWithoutSourceReason,
    ) -> Self {
        match TrackedRetainedStyleSourceIds::new(source_ids) {
            Some(source_ids) => Self::Retained(source_ids),
            None => Self::WithoutSource(without_source_reason),
        }
    }
}

impl TrackedRetainedStyleSourceIds {
    fn new(source_ids: Vec<StyleSourceId>) -> Option<Self> {
        if source_ids.is_empty() {
            None
        } else {
            Some(Self(source_ids))
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn retain(&mut self, mut retain_source_id: impl FnMut(&StyleSourceId) -> bool) {
        self.0.retain(|source_id| retain_source_id(source_id));
    }
}

impl From<TrackedRetainedStyleSourceIds> for Vec<StyleSourceId> {
    fn from(source_ids: TrackedRetainedStyleSourceIds) -> Self {
        source_ids.0
    }
}

impl OwnedStyleSourceDocumentContext {
    pub(crate) fn new(root_document: DomHandle) -> Self {
        Self {
            root_document,
            child_documents: Vec::new(),
        }
    }

    pub(crate) fn with_child_documents(
        mut self,
        child_documents: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        self.child_documents.extend(child_documents);
        self
    }

    pub(crate) fn as_ref(&self) -> StyleSourceDocumentContext<'_> {
        StyleSourceDocumentContext {
            root_document: self.root_document,
            child_documents: &self.child_documents,
        }
    }
}

impl<'a> StyleSourceDocumentContext<'a> {
    #[cfg(test)]
    pub(crate) fn for_root_document(root_document: DomHandle) -> Self {
        Self {
            root_document,
            child_documents: &[],
        }
    }

    pub(crate) fn documents(self) -> Vec<DomHandle> {
        let mut documents = Vec::with_capacity(1 + self.child_documents.len());
        push_unique_document(&mut documents, self.root_document);
        for &document in self.child_documents {
            push_unique_document(&mut documents, document);
        }
        documents
    }

    pub(crate) fn documents_with_owner(self, owner_document: DomHandle) -> Vec<DomHandle> {
        let mut documents = Vec::with_capacity(2);
        push_unique_document(&mut documents, self.root_document);
        push_unique_document(&mut documents, owner_document);
        documents
    }

    fn document_kind(self, host: &DomHost, document: DomHandle) -> Option<StyleSourceDocumentKind> {
        if self.root_document == document {
            return Some(StyleSourceDocumentKind::Root);
        }
        if self.child_documents.contains(&document) {
            return Some(StyleSourceDocumentKind::Child);
        }
        host.node(document)
            .is_some_and(Node::is_document)
            .then_some(StyleSourceDocumentKind::Detached)
    }
}

fn push_unique_document(documents: &mut Vec<DomHandle>, document: DomHandle) {
    if !documents.contains(&document) {
        documents.push(document);
    }
}

impl StyleSourceLifecycleOwner {
    pub(super) fn from_source_id(source_id: &StyleSourceId) -> Option<Self> {
        match source_id.kind {
            StyleSourceKind::OwnerStyleSheet { owner } => Some(Self::OwnerStyleSheet { owner }),
            StyleSourceKind::LinkedStyleSheet { owner } => Some(Self::LinkedStyleSheet { owner }),
            StyleSourceKind::DocumentAdoptedStyleSheet { .. } => match source_id.scope_id {
                StyleScopeId::Document(document) => {
                    Some(Self::DocumentAdoptedStyleSheets { document })
                }
                StyleScopeId::ShadowRoot(_) => None,
            },
            StyleSourceKind::ShadowRootAdoptedStyleSheet { .. } => match source_id.scope_id {
                StyleScopeId::ShadowRoot(root) => Some(Self::ShadowRootAdoptedStyleSheets { root }),
                StyleScopeId::Document(_) => None,
            },
        }
    }

    fn availability(self, host: &DomHost) -> StyleSourceOwnerAvailability {
        match self {
            Self::OwnerStyleSheet { owner } | Self::LinkedStyleSheet { owner } => {
                StyleSourceOwnerAvailability::stylesheet_owner(host, owner)
            }
            Self::DocumentAdoptedStyleSheets { document } => {
                StyleSourceOwnerAvailability::document_adopted_owner(host, document)
            }
            Self::ShadowRootAdoptedStyleSheets { root } => {
                StyleSourceOwnerAvailability::shadow_root_adopted_owner(host, root)
            }
        }
    }

    pub(super) fn retained_source_records_are_available(self, host: &DomHost) -> bool {
        self.availability(host).is_available()
    }

    pub(super) fn tracked_entry(
        self,
        host: &DomHost,
        source_ids: impl FnOnce() -> TrackedStyleSourceIds,
    ) -> TrackedStyleSourceLifecycleEntry {
        TrackedStyleSourceLifecycleEntry::new(
            self,
            TrackedStyleSourceLifecycleState::for_owner_availability(
                self.availability(host),
                source_ids,
            ),
        )
    }

    pub(super) fn document_kind(
        self,
        host: &DomHost,
        document_context: StyleSourceDocumentContext<'_>,
    ) -> Option<StyleSourceDocumentKind> {
        let document = match self {
            Self::OwnerStyleSheet { owner } | Self::LinkedStyleSheet { owner } => {
                host.owner_document_handle(owner)?
            }
            Self::DocumentAdoptedStyleSheets { document } => document,
            Self::ShadowRootAdoptedStyleSheets { root } => host.owner_document_handle(root)?,
        };
        document_context.document_kind(host, document)
    }
}

impl StyleSourceLifecycleSnapshot {
    #[cfg(test)]
    pub(super) fn tracked_owner_style_sheet_count(&self) -> usize {
        self.tracked_owner_style_sheet_count
    }

    #[cfg(test)]
    pub(super) fn retained_owner_style_sheet_source_count(&self) -> usize {
        self.retained_owner_style_sheet_source_count
    }

    #[cfg(test)]
    pub(super) fn tracked_linked_style_sheet_owner_count(&self) -> usize {
        self.tracked_linked_style_sheet_owner_count
    }

    #[cfg(test)]
    pub(super) fn retained_linked_style_sheet_source_count(&self) -> usize {
        self.retained_linked_style_sheet_source_count
    }

    #[cfg(test)]
    pub(super) fn retained_document_adopted_source_count(&self) -> usize {
        self.retained_document_adopted_source_count
    }

    #[cfg(test)]
    pub(super) fn retained_shadow_adopted_source_count(&self) -> usize {
        self.retained_shadow_adopted_source_count
    }

    #[cfg(test)]
    pub(super) fn retained_root_document_source_count(&self) -> usize {
        self.retained_root_document_source_count
    }

    #[cfg(test)]
    pub(super) fn retained_detached_document_source_count(&self) -> usize {
        self.retained_detached_document_source_count
    }

    fn record_lifecycle_record(&mut self, record: &StyleSourceLifecycleRecord) {
        match record.owner {
            StyleSourceLifecycleOwner::OwnerStyleSheet { .. } => {
                self.tracked_owner_style_sheet_count += 1;
            }
            StyleSourceLifecycleOwner::LinkedStyleSheet { .. } => {
                self.tracked_linked_style_sheet_owner_count += 1;
            }
            StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { .. } => {
                self.tracked_document_adopted_owner_count += 1;
            }
            StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { .. } => {
                self.tracked_shadow_adopted_owner_count += 1;
            }
        }

        match &record.availability {
            StyleSourceLifecycleAvailability::RetainedSources { source_ids } => {
                for source_id in source_ids {
                    self.record_retained_stylesheet_source_id(source_id, record.document_kind);
                }
            }
            StyleSourceLifecycleAvailability::AvailableWithoutSource { reason } => {
                self.record_available_without_source(record, *reason);
            }
            StyleSourceLifecycleAvailability::Unavailable { reason } => {
                self.record_unavailable_owner(*reason);
            }
        }
    }

    fn record_retained_stylesheet_source_id(
        &mut self,
        id: &StyleSourceId,
        document_kind: Option<StyleSourceDocumentKind>,
    ) {
        match &id.kind {
            StyleSourceKind::OwnerStyleSheet { .. } => {
                self.retained_owner_style_sheet_source_count += 1;
            }
            StyleSourceKind::LinkedStyleSheet { .. } => {
                self.retained_linked_style_sheet_source_count += 1;
            }
            StyleSourceKind::DocumentAdoptedStyleSheet { .. } => {
                self.retained_document_adopted_source_count += 1;
            }
            StyleSourceKind::ShadowRootAdoptedStyleSheet { .. } => {
                self.retained_shadow_adopted_source_count += 1;
            }
        }
        self.record_retained_document_kind_sources(document_kind, 1);
    }

    fn record_available_without_source(
        &mut self,
        record: &StyleSourceLifecycleRecord,
        reason: StyleSourceLifecycleWithoutSourceReason,
    ) {
        match record.owner {
            StyleSourceLifecycleOwner::OwnerStyleSheet { .. } => {
                self.available_owner_style_sheet_without_source_count += 1;
            }
            StyleSourceLifecycleOwner::LinkedStyleSheet { .. } => {
                self.available_linked_style_sheet_without_source_count += 1;
                if reason == StyleSourceLifecycleWithoutSourceReason::LinkedStyleSheetSourceMissing
                {
                    self.available_linked_style_sheet_without_loaded_source_count += 1;
                }
            }
            StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { .. } => {
                self.available_document_adopted_owner_without_source_count += 1;
                if reason
                    == StyleSourceLifecycleWithoutSourceReason::EmptyDocumentAdoptedStyleSheets
                {
                    self.available_document_adopted_empty_owner_count += 1;
                }
            }
            StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { .. } => {
                self.available_shadow_adopted_owner_without_source_count += 1;
                if reason
                    == StyleSourceLifecycleWithoutSourceReason::EmptyShadowRootAdoptedStyleSheets
                {
                    self.available_shadow_adopted_empty_owner_count += 1;
                }
            }
        }
        self.record_available_without_source_document_kind(record.document_kind);
    }

    fn record_unavailable_owner(&mut self, reason: StyleSourceLifecycleUnavailableReason) {
        match reason {
            StyleSourceLifecycleUnavailableReason::MissingNode => {
                self.missing_node_owner_count += 1;
            }
            StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree => {
                self.owner_not_in_document_tree_count += 1;
            }
            StyleSourceLifecycleUnavailableReason::InactiveShadowRoot => {
                self.inactive_shadow_root_owner_count += 1;
            }
        }
    }

    fn record_retained_document_kind_sources(
        &mut self,
        document_kind: Option<StyleSourceDocumentKind>,
        count: usize,
    ) {
        match document_kind {
            Some(StyleSourceDocumentKind::Root) => {
                self.retained_root_document_source_count += count;
            }
            Some(StyleSourceDocumentKind::Child) => {
                self.retained_child_document_source_count += count;
            }
            Some(StyleSourceDocumentKind::Detached) => {
                self.retained_detached_document_source_count += count;
            }
            None => {}
        }
    }

    fn record_available_without_source_document_kind(
        &mut self,
        document_kind: Option<StyleSourceDocumentKind>,
    ) {
        match document_kind {
            Some(StyleSourceDocumentKind::Root) => {
                self.available_root_document_owner_without_source_count += 1;
            }
            Some(StyleSourceDocumentKind::Child) => {
                self.available_child_document_owner_without_source_count += 1;
            }
            Some(StyleSourceDocumentKind::Detached) => {
                self.available_detached_document_owner_without_source_count += 1;
            }
            None => {}
        }
    }
}

fn light_tree_owner_is_in_document_tree(
    host: &DomHost,
    owner: DomHandle,
    document: DomHandle,
) -> bool {
    let mut current = Some(owner);
    while let Some(handle) = current {
        if handle == document {
            return true;
        }
        current = host.node(handle).and_then(Node::parent_node);
    }
    false
}

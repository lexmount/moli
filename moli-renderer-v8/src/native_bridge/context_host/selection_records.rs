use super::range_records::RangeRecordRegistry;
use super::*;
use crate::dom::native::{DomHost, NodeType};
use crate::native_bridge::element::contenteditable_editing_host_in_dom;
use crate::range_boundary::RangeBoundaryPoint;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SelectionRecordHandle(u64);

impl SelectionRecordHandle {
    pub(crate) fn new(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }

    pub(crate) fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionBoundaryRole {
    Anchor,
    Focus,
    ComposedStart,
    ComposedEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectionBoundarySnapshot {
    pub(crate) container: DomHandle,
    pub(crate) offset: u32,
}

impl SelectionBoundarySnapshot {
    fn new(dom: &DomHost, container: DomHandle, offset: u32) -> Option<Self> {
        RangeBoundaryPoint::new(dom, container, offset)?;
        Some(Self { container, offset })
    }

    fn update_text_boundary(
        &mut self,
        dom: &DomHost,
        update: impl FnOnce(&mut RangeBoundaryPoint),
    ) {
        let Some(mut point) = RangeBoundaryPoint::new(dom, self.container, self.offset) else {
            return;
        };
        update(&mut point);
        if let Some(offset) = point.offset(dom) {
            self.container = point.container();
            self.offset = offset;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentSelectionSnapshot {
    pub(crate) start: SelectionBoundarySnapshot,
    pub(crate) end: SelectionBoundarySnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionDirection {
    None,
    Forward,
    Backward,
}

impl SelectionDirection {
    fn from_str(value: &str) -> Self {
        match value {
            "forward" => Self::Forward,
            "backward" => Self::Backward,
            _ => Self::None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }
}

pub(super) struct SelectionRecordRegistry {
    next_id: u64,
    records: HashMap<SelectionRecordHandle, SelectionRecord>,
    // A text control's selection survives focus moving to another document.
    // Explicit DOM Selection changes supersede it in its own document.
    text_controls: HashMap<DomHandle, DomHandle>,
}

struct SelectionRecord {
    owner_document: Option<DomHandle>,
    associated_range: Option<RangeRecordHandle>,
    anchor: Option<RangeBoundaryPoint>,
    focus: Option<RangeBoundaryPoint>,
    direction: SelectionDirection,
    // Raw composed positions preserve their child offsets across insertions;
    // they do not share the associated Range's child-before anchors. Text
    // edits, text splits and removals adjust these positions explicitly.
    composed_start: Option<SelectionBoundarySnapshot>,
    composed_end: Option<SelectionBoundarySnapshot>,
}

pub(super) struct SelectionRangeBoundaryLink {
    selection: SelectionRecordHandle,
    range: RangeRecordHandle,
    role: SelectionBoundaryRole,
    side: RangeBoundarySide,
}

fn same_boundary_position(dom: &DomHost, a: RangeBoundaryPoint, b: RangeBoundaryPoint) -> bool {
    if a.container() != b.container() {
        return false;
    }
    if dom.node(a.container()).is_some_and(|node| {
        matches!(
            node.node_type(),
            NodeType::Text
                | NodeType::CDataSection
                | NodeType::ProcessingInstruction
                | NodeType::Comment
        )
    }) {
        let (mut a, mut b) = (a, b);
        a.offset(dom) == b.offset(dom)
    } else {
        // The child may already have left its parent when a removal hook
        // runs. Compare stable child anchors, not a now-unresolvable offset.
        a.child_before() == b.child_before()
    }
}

fn shadow_including_descendant_or_self(
    dom: &DomHost,
    node: DomHandle,
    ancestor: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(node) = current {
        if node == ancestor {
            return true;
        }
        current = dom.parent_node(node).or_else(|| {
            dom.is_shadow_root(node)
                .then(|| dom.shadow_root_host(node))
                .flatten()
        });
    }
    false
}

impl SelectionRecordRegistry {
    pub(super) fn linked_range_boundaries(
        &self,
        dom: &DomHost,
        ranges: &RangeRecordRegistry,
    ) -> Vec<SelectionRangeBoundaryLink> {
        let mut links = Vec::new();
        for (&selection, record) in &self.records {
            let Some(range) = record.associated_range else {
                continue;
            };
            let start = ranges.boundary_point(range, RangeBoundarySide::Start);
            let end = ranges.boundary_point(range, RangeBoundarySide::End);
            for (role, point) in [
                (SelectionBoundaryRole::Anchor, record.anchor),
                (SelectionBoundaryRole::Focus, record.focus),
            ] {
                let Some(point) = point else { continue };
                let side = if start.is_some_and(|start| same_boundary_position(dom, point, start)) {
                    RangeBoundarySide::Start
                } else if end.is_some_and(|end| same_boundary_position(dom, point, end)) {
                    RangeBoundarySide::End
                } else {
                    continue;
                };
                links.push(SelectionRangeBoundaryLink {
                    selection,
                    range,
                    role,
                    side,
                });
            }
        }
        links
    }

    pub(super) fn sync_linked_range_boundaries(
        &mut self,
        ranges: &RangeRecordRegistry,
        links: Vec<SelectionRangeBoundaryLink>,
    ) {
        for link in links {
            if let Some(point) = ranges.boundary_point(link.range, link.side)
                && let Some(record) = self.records.get_mut(&link.selection)
                && let Some(slot) = record.range_boundary_slot_mut(link.role)
            {
                *slot = Some(point);
            }
        }
    }

    pub(super) fn update_composed_for_character_data_edit(
        &mut self,
        dom: &DomHost,
        target: DomHandle,
        edit_offset: u32,
        removed_count: u32,
        inserted_count: u32,
    ) {
        for record in self.records.values_mut() {
            for point in [&mut record.composed_start, &mut record.composed_end]
                .into_iter()
                .flatten()
            {
                if point.container == target {
                    point.update_text_boundary(dom, |boundary| {
                        boundary.update_for_character_data_edit(
                            dom,
                            target,
                            edit_offset,
                            removed_count,
                            inserted_count,
                        );
                    });
                }
            }
        }
    }

    pub(super) fn update_composed_for_text_split(
        &mut self,
        dom: &DomHost,
        original: DomHandle,
        new_text: DomHandle,
        offset: u32,
    ) {
        for record in self.records.values_mut() {
            for point in [&mut record.composed_start, &mut record.composed_end]
                .into_iter()
                .flatten()
            {
                if point.container == original {
                    point.update_text_boundary(dom, |boundary| {
                        boundary.update_for_text_split(dom, original, new_text, offset);
                    });
                }
            }
        }
    }

    pub(super) fn update_for_child_removal(
        &mut self,
        dom: &DomHost,
        parent: DomHandle,
        removed_child: DomHandle,
        index: u32,
        previous_sibling: Option<DomHandle>,
    ) {
        let removed_offset = previous_sibling
            .and_then(|previous| dom.child_index(parent, previous))
            .and_then(|previous_index| u32::try_from(previous_index + 1).ok())
            .unwrap_or(index);
        for record in self.records.values_mut() {
            // Preserve the editing-host caret projection when its host is
            // removed across a shadow boundary. Ordinary observable endpoints
            // have already followed their associated live Range.
            if let (Some(anchor), Some(focus)) = (record.anchor, record.focus)
                && same_boundary_position(dom, anchor, focus)
                && shadow_including_descendant_or_self(dom, anchor.container(), removed_child)
                && contenteditable_editing_host_in_dom(dom.dom(), anchor.container())
                    == Some(anchor.container())
                && let Some(point) = RangeBoundaryPoint::new(dom, parent, removed_offset)
            {
                record.anchor = Some(point);
                record.focus = Some(point);
            }
            // All document selections share this native registry, including
            // child realms mutated through a borrowed parent-realm method.
            for point in [&mut record.composed_start, &mut record.composed_end]
                .into_iter()
                .flatten()
            {
                if shadow_including_descendant_or_self(dom, point.container, removed_child) {
                    *point = SelectionBoundarySnapshot {
                        container: parent,
                        offset: removed_offset,
                    };
                } else if point.container == parent && point.offset > index {
                    point.offset -= 1;
                }
            }
        }
    }

    pub(super) fn new() -> Self {
        Self {
            next_id: 1,
            records: HashMap::new(),
            text_controls: HashMap::new(),
        }
    }

    fn create_record(&mut self) -> Option<SelectionRecordHandle> {
        let handle = self.allocate_record_id()?;
        self.records.insert(handle, SelectionRecord::empty());
        Some(handle)
    }

    fn allocate_record_id(&mut self) -> Option<SelectionRecordHandle> {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let first_candidate = self.next_id;
        loop {
            let handle = SelectionRecordHandle::new(self.next_id)?;
            self.next_id = if self.next_id == u64::MAX {
                1
            } else {
                self.next_id + 1
            };
            if !self.records.contains_key(&handle) {
                return Some(handle);
            }
            if self.next_id == first_candidate {
                return None;
            }
        }
    }

    fn clear_record(&mut self, handle: SelectionRecordHandle) {
        let owner_document = self
            .records
            .get(&handle)
            .and_then(|record| record.owner_document);
        if let Some(document) = owner_document {
            self.text_controls.remove(&document);
        }
        self.records
            .insert(handle, SelectionRecord::empty_with_owner(owner_document));
    }

    fn set_owner_document(&mut self, handle: SelectionRecordHandle, owner_document: DomHandle) {
        self.record_mut(handle).owner_document = Some(owner_document);
    }

    fn owner_document(&self, handle: SelectionRecordHandle) -> Option<DomHandle> {
        self.records
            .get(&handle)
            .and_then(|record| record.owner_document)
    }

    #[allow(clippy::too_many_arguments)]
    fn store(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: SelectionRecordHandle,
        associated_range: Option<RangeRecordHandle>,
        anchor: (DomHandle, u32),
        focus: (DomHandle, u32),
        direction: &str,
        composed_start: (DomHandle, u32),
        composed_end: (DomHandle, u32),
    ) -> bool {
        let Some(anchor) = RangeBoundaryPoint::new(dom_host, anchor.0, anchor.1) else {
            return false;
        };
        let Some(focus) = RangeBoundaryPoint::new(dom_host, focus.0, focus.1) else {
            return false;
        };
        let Some(composed_start) =
            SelectionBoundarySnapshot::new(dom_host, composed_start.0, composed_start.1)
        else {
            return false;
        };
        let Some(composed_end) =
            SelectionBoundarySnapshot::new(dom_host, composed_end.0, composed_end.1)
        else {
            return false;
        };
        let record = self.record_mut(handle);
        record.associated_range = associated_range;
        record.anchor = Some(anchor);
        record.focus = Some(focus);
        record.direction = SelectionDirection::from_str(direction);
        record.composed_start = Some(composed_start);
        record.composed_end = Some(composed_end);
        if let Some(document) = record.owner_document {
            self.text_controls.remove(&document);
        }
        true
    }

    fn has_range(&self, handle: SelectionRecordHandle) -> bool {
        self.records
            .get(&handle)
            .is_some_and(SelectionRecord::has_range)
    }

    fn direction(&self, handle: SelectionRecordHandle) -> Option<&'static str> {
        let record = self.records.get(&handle)?;
        // getRangeAt() and the observable endpoints can be collapsed while a
        // composed selection still spans shadow trees. Its direction belongs
        // to the composed selection, independently of that DOM projection.
        if let (Some(start), Some(end)) = (record.composed_start, record.composed_end)
            && start == end
        {
            return Some("none");
        }
        Some(record.direction.as_str())
    }

    fn boundary(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: SelectionRecordHandle,
        role: SelectionBoundaryRole,
    ) -> Option<SelectionBoundarySnapshot> {
        let record = self.records.get_mut(&handle)?;
        match role {
            SelectionBoundaryRole::ComposedStart => return record.composed_start,
            SelectionBoundaryRole::ComposedEnd => return record.composed_end,
            _ => {}
        }
        let boundary = record.range_boundary_slot_mut(role)?.as_mut()?;
        Some(SelectionBoundarySnapshot {
            container: boundary.container(),
            offset: boundary.offset(dom_host)?,
        })
    }

    fn is_collapsed(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: SelectionRecordHandle,
    ) -> bool {
        let Some(record) = self.records.get_mut(&handle) else {
            return true;
        };
        let (Some(anchor), Some(focus)) = (&mut record.anchor, &mut record.focus) else {
            return true;
        };
        anchor.container() == focus.container()
            && anchor.offset(dom_host).unwrap_or(0) == focus.offset(dom_host).unwrap_or(0)
    }

    fn document_snapshot(&self, document: DomHandle) -> Option<DocumentSelectionSnapshot> {
        let record = self
            .records
            .iter()
            .filter(|(_, record)| record.owner_document == Some(document) && record.has_range())
            .min_by_key(|(handle, _)| handle.raw())
            .map(|(_, record)| record)?;
        Some(DocumentSelectionSnapshot {
            start: record.composed_start?,
            end: record.composed_end?,
        })
    }

    fn record_mut(&mut self, handle: SelectionRecordHandle) -> &mut SelectionRecord {
        self.records
            .entry(handle)
            .or_insert_with(SelectionRecord::empty)
    }
}

impl SelectionRecord {
    fn empty() -> Self {
        Self::empty_with_owner(None)
    }

    fn empty_with_owner(owner_document: Option<DomHandle>) -> Self {
        Self {
            owner_document,
            associated_range: None,
            anchor: None,
            focus: None,
            direction: SelectionDirection::None,
            composed_start: None,
            composed_end: None,
        }
    }

    fn has_range(&self) -> bool {
        self.associated_range.is_some()
            && self.anchor.is_some()
            && self.focus.is_some()
            && self.composed_start.is_some()
            && self.composed_end.is_some()
    }

    fn range_boundary_slot_mut(
        &mut self,
        role: SelectionBoundaryRole,
    ) -> Option<&mut Option<RangeBoundaryPoint>> {
        match role {
            SelectionBoundaryRole::Anchor => Some(&mut self.anchor),
            SelectionBoundaryRole::Focus => Some(&mut self.focus),
            SelectionBoundaryRole::ComposedStart | SelectionBoundaryRole::ComposedEnd => None,
        }
    }
}

impl JsContextHost {
    pub(crate) fn note_text_control_selection(&mut self, control: DomHandle) {
        if let Some(document) = self.dom_host().owner_document_handle(control) {
            self.selection_record_registry
                .text_controls
                .insert(document, control);
        }
    }

    pub(crate) fn document_selected_text_control(&self, document: DomHandle) -> Option<DomHandle> {
        self.selection_record_registry
            .text_controls
            .get(&document)
            .copied()
    }

    pub(crate) fn create_selection_record(&mut self) -> Option<SelectionRecordHandle> {
        self.selection_record_registry.create_record()
    }

    pub(crate) fn clear_selection_record(&mut self, handle: SelectionRecordHandle) {
        if self.selection_record_registry.has_range(handle)
            && let Some(document) = self.selection_record_registry.owner_document(handle)
            && let Some(control) = self.active_element_handle()
            && self.dom_host().owner_document_handle(control) == Some(document)
            && crate::native_bridge::element::is_text_control(self, control)
        {
            // A focused text control exposes the document selection, including
            // its empty position. An unfocused control retains its cached range.
            let _ = self.set_selection_range(control, 0, 0);
        }
        self.selection_record_registry.clear_record(handle);
    }

    pub(crate) fn set_selection_record_owner_document(
        &mut self,
        handle: SelectionRecordHandle,
        owner_document: DomHandle,
    ) {
        self.selection_record_registry
            .set_owner_document(handle, owner_document);
    }

    pub(crate) fn selection_record_owner_document(
        &self,
        handle: SelectionRecordHandle,
    ) -> Option<DomHandle> {
        self.selection_record_registry.owner_document(handle)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn store_selection_record(
        &mut self,
        handle: SelectionRecordHandle,
        associated_range: Option<RangeRecordHandle>,
        anchor: (DomHandle, u32),
        focus: (DomHandle, u32),
        direction: &str,
        composed_start: (DomHandle, u32),
        composed_end: (DomHandle, u32),
    ) -> bool {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.selection_record_registry.store(
            dom_host,
            handle,
            associated_range,
            anchor,
            focus,
            direction,
            composed_start,
            composed_end,
        )
    }

    pub(crate) fn selection_record_has_range(&self, handle: SelectionRecordHandle) -> bool {
        self.selection_record_registry.has_range(handle)
    }

    pub(crate) fn selection_record_direction(
        &self,
        handle: SelectionRecordHandle,
    ) -> Option<&'static str> {
        self.selection_record_registry.direction(handle)
    }

    pub(crate) fn selection_record_boundary(
        &mut self,
        handle: SelectionRecordHandle,
        role: SelectionBoundaryRole,
    ) -> Option<SelectionBoundarySnapshot> {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.selection_record_registry
            .boundary(dom_host, handle, role)
    }

    pub(crate) fn selection_record_is_collapsed(&mut self, handle: SelectionRecordHandle) -> bool {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.selection_record_registry
            .is_collapsed(dom_host, handle)
    }

    pub(crate) fn selection_document_for_range(
        &self,
        range: RangeRecordHandle,
    ) -> Option<DomHandle> {
        self.selection_record_registry
            .records
            .values()
            .find_map(|record| {
                (record.associated_range == Some(range))
                    .then_some(record.owner_document)
                    .flatten()
            })
    }

    pub(crate) fn document_selection_snapshot(
        &self,
        document: DomHandle,
    ) -> Option<DocumentSelectionSnapshot> {
        self.selection_record_registry.document_snapshot(document)
    }

    pub(crate) fn selection_record_spans_dom_roots(&self, handle: SelectionRecordHandle) -> bool {
        let Some(record) = self.selection_record_registry.records.get(&handle) else {
            return false;
        };
        let (Some(start), Some(end)) = (record.composed_start, record.composed_end) else {
            return false;
        };
        let dom = self.dom_host();
        dom.root_node_handle(start.container)
            .zip(dom.root_node_handle(end.container))
            .is_some_and(|(start, end)| start != end)
    }
}

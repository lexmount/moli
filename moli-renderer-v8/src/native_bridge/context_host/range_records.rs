use super::*;
use crate::range_boundary::RangeBoundaryPoint;

thread_local! {
    static FINALIZED_RANGE_RECORDS: std::cell::RefCell<Vec<(u64, RangeRecordId)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

static NEXT_RANGE_RECORD_LIFETIME_TOKEN: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

fn next_range_record_lifetime_token() -> u64 {
    let token = NEXT_RANGE_RECORD_LIFETIME_TOKEN
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .max(1);
    if token == u64::MAX {
        NEXT_RANGE_RECORD_LIFETIME_TOKEN.store(1, std::sync::atomic::Ordering::Relaxed);
    }
    token
}

fn note_finalized_range_record(lifetime_token: u64, record_id: RangeRecordId) {
    FINALIZED_RANGE_RECORDS.with(|records| {
        records.borrow_mut().push((lifetime_token, record_id));
    });
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RangeRecordId(u64);

impl RangeRecordId {
    fn new(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }

    fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RangeRecordHandle {
    lifetime_token: u64,
    id: RangeRecordId,
}

impl RangeRecordHandle {
    pub(crate) fn new(lifetime_token: u64, raw_id: u64) -> Option<Self> {
        let id = RangeRecordId::new(raw_id)?;
        (lifetime_token != 0).then_some(Self { lifetime_token, id })
    }

    pub(crate) fn lifetime_token(self) -> u64 {
        self.lifetime_token
    }

    pub(crate) fn raw_id(self) -> u64 {
        self.id.raw()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RangeBoundarySide {
    Start,
    End,
}

pub(super) struct RangeRecord {
    start: RangeBoundaryPoint,
    end: RangeBoundaryPoint,
}

// Shared by all realms in this JsContextHost. Creating another Window must not
// invalidate existing wrappers; their weak finalizers retire individual records.
pub(super) struct RangeRecordRegistry {
    lifetime_token: u64,
    active_live_records: HashSet<RangeRecordId>,
    live_record_wrappers: HashMap<RangeRecordId, v8::Weak<v8::Object>>,
    next_id: u64,
    records: HashMap<RangeRecordId, RangeRecord>,
    by_boundary_container: HashMap<DomHandle, HashSet<RangeRecordId>>,
    by_boundary_child_before: HashMap<DomHandle, HashSet<RangeRecordId>>,
}

impl RangeRecordRegistry {
    pub(super) fn new() -> Self {
        Self {
            lifetime_token: next_range_record_lifetime_token(),
            active_live_records: HashSet::new(),
            live_record_wrappers: HashMap::new(),
            next_id: 1,
            records: HashMap::new(),
            by_boundary_container: HashMap::new(),
            by_boundary_child_before: HashMap::new(),
        }
    }

    pub(super) fn active_is_empty(&mut self) -> bool {
        self.drain_finalized_live_records();
        self.active_live_records.is_empty()
    }

    pub(super) fn register_live_record(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: RangeRecordHandle,
        wrapper: v8::Local<'_, v8::Object>,
    ) {
        self.drain_finalized_live_records();
        let Some(id) = self.id_for_handle(handle) else {
            return;
        };
        let lifetime_token = self.lifetime_token;
        let wrapper = v8::Weak::with_guaranteed_finalizer(
            scope,
            wrapper,
            Box::new(move || note_finalized_range_record(lifetime_token, id)),
        );
        self.active_live_records.insert(id);
        self.live_record_wrappers.insert(id, wrapper);
    }

    fn remove_live_record(&mut self, id: RangeRecordId) {
        self.active_live_records.remove(&id);
        self.live_record_wrappers.remove(&id);
    }

    fn boundary_container_candidate_ids(&self, container: DomHandle) -> Vec<RangeRecordId> {
        self.by_boundary_container
            .get(&container)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    fn boundary_child_before_candidate_ids(&self, child_before: DomHandle) -> Vec<RangeRecordId> {
        self.by_boundary_child_before
            .get(&child_before)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    fn live_record_ids_for_boundary_container(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        container: DomHandle,
    ) -> Vec<RangeRecordId> {
        self.drain_finalized_live_records();
        let mut ids = HashSet::new();
        self.collect_live_record_ids_for_boundary_container(scope, container, &mut ids);
        ids.into_iter().collect()
    }

    fn live_record_ids_for_child_removal(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        dom_host: &crate::dom::native::DomHost,
        removed_child: DomHandle,
    ) -> Vec<RangeRecordId> {
        self.drain_finalized_live_records();
        let mut ids = HashSet::new();
        self.collect_live_record_ids_for_boundary_child_before(scope, removed_child, &mut ids);

        let mut stack = vec![removed_child];
        while let Some(handle) = stack.pop() {
            self.collect_live_record_ids_for_boundary_container(scope, handle, &mut ids);
            stack.extend(dom_host.child_handles(handle));
        }

        ids.into_iter().collect()
    }

    fn live_record_ids_for_text_split(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        dom_host: &crate::dom::native::DomHost,
        original: DomHandle,
    ) -> Vec<RangeRecordId> {
        self.drain_finalized_live_records();
        let mut ids = HashSet::new();
        self.collect_live_record_ids_for_boundary_container(scope, original, &mut ids);
        if let Some(parent) = dom_host.node(original).and_then(|node| node.parent_node()) {
            self.collect_live_record_ids_for_boundary_container(scope, parent, &mut ids);
        }
        ids.into_iter().collect()
    }

    fn create_record(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        owner_document: DomHandle,
    ) -> Option<RangeRecordHandle> {
        let record = RangeRecord::new(dom_host, owner_document)?;
        let id = self.allocate_record_id()?;
        self.records.insert(id, record);
        self.index_record_boundaries(id);
        Some(self.handle_for_id(id))
    }

    fn allocate_record_id(&mut self) -> Option<RangeRecordId> {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let first_candidate = self.next_id;
        loop {
            let id = RangeRecordId::new(self.next_id)?;
            self.next_id = if self.next_id == u64::MAX {
                1
            } else {
                self.next_id + 1
            };
            if !self.records.contains_key(&id) {
                return Some(id);
            }
            if self.next_id == first_candidate {
                return None;
            }
        }
    }

    fn reset_record(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: RangeRecordHandle,
        owner_document: DomHandle,
    ) -> bool {
        let Some(id) = self.id_for_handle(handle) else {
            return false;
        };
        let Some(record) = RangeRecord::new(dom_host, owner_document) else {
            return false;
        };
        self.remove_record_boundary_index(id);
        let Some(slot) = self.records.get_mut(&id) else {
            return false;
        };
        *slot = record;
        self.index_record_boundaries(id);
        true
    }

    fn remove_record(&mut self, id: RangeRecordId) {
        self.remove_record_boundary_index(id);
        self.records.remove(&id);
    }

    fn remove_live_record_and_record(&mut self, id: RangeRecordId) {
        self.remove_live_record(id);
        self.remove_record(id);
    }

    fn drain_finalized_live_records(&mut self) {
        let lifetime_token = self.lifetime_token;
        let mut finalized = Vec::new();
        FINALIZED_RANGE_RECORDS.with(|records| {
            let mut records = records.borrow_mut();
            let mut index = 0;
            while index < records.len() {
                if records[index].0 == lifetime_token {
                    finalized.push(records.swap_remove(index).1);
                } else {
                    index += 1;
                }
            }
        });
        for id in finalized {
            self.remove_live_record_and_record(id);
        }
    }

    fn id_for_handle(&self, handle: RangeRecordHandle) -> Option<RangeRecordId> {
        (handle.lifetime_token == self.lifetime_token).then_some(handle.id)
    }

    fn handle_for_id(&self, id: RangeRecordId) -> RangeRecordHandle {
        RangeRecordHandle {
            lifetime_token: self.lifetime_token,
            id,
        }
    }

    fn boundary_container(
        &self,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
    ) -> Option<DomHandle> {
        let id = self.id_for_handle(handle)?;
        self.records
            .get(&id)
            .map(|record| record.boundary(side).container())
    }

    pub(super) fn boundary_point(
        &self,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
    ) -> Option<RangeBoundaryPoint> {
        let id = self.id_for_handle(handle)?;
        self.records.get(&id).map(|record| *record.boundary(side))
    }

    fn boundary_offset(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
    ) -> Option<u32> {
        let id = self.id_for_handle(handle)?;
        self.records
            .get_mut(&id)?
            .boundary_mut(side)
            .offset(dom_host)
    }

    fn boundary_handles(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: RangeRecordHandle,
    ) -> Option<((DomHandle, u32), (DomHandle, u32))> {
        let id = self.id_for_handle(handle)?;
        let record = self.records.get_mut(&id)?;
        let start_container = record.start.container();
        let start_offset = record.start.offset(dom_host)?;
        let end_container = record.end.container();
        let end_offset = record.end.offset(dom_host)?;
        Some(((start_container, start_offset), (end_container, end_offset)))
    }

    fn set_boundary(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
        container: DomHandle,
        offset: u32,
    ) -> bool {
        let Some(id) = self.id_for_handle(handle) else {
            return false;
        };
        let Some(point) = RangeBoundaryPoint::new(dom_host, container, offset) else {
            return false;
        };
        let before = self.records.get(&id).map(RangeRecord::boundary_index_keys);
        let Some(after) = self.records.get_mut(&id).map(|record| {
            *record.boundary_mut(side) = point;
            record.boundary_index_keys()
        }) else {
            return false;
        };
        if let Some(before) = before {
            self.reindex_record_boundaries_if_changed(id, before, after);
        }
        true
    }

    fn update_for_character_data_edit(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        ids: &[RangeRecordId],
        target: DomHandle,
        edit_offset: u32,
        removed_count: u32,
        inserted_count: u32,
    ) {
        for id in ids {
            let Some(record) = self.records.get_mut(id) else {
                continue;
            };
            let before = record.boundary_index_keys();
            record.for_each_boundary_mut(|boundary| {
                if boundary.container() != target {
                    return;
                }
                let Some(current) = boundary.offset(dom_host) else {
                    return;
                };
                let next = update_boundary_offset_for_character_data(
                    current,
                    edit_offset,
                    removed_count,
                    inserted_count,
                );
                if let Some(next_boundary) = RangeBoundaryPoint::new(dom_host, target, next) {
                    *boundary = next_boundary;
                }
            });
            let after = record.boundary_index_keys();
            self.reindex_record_boundaries_if_changed(*id, before, after);
        }
    }

    fn update_for_child_removal(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        ids: &[RangeRecordId],
        parent: DomHandle,
        removed_child: DomHandle,
        index: u32,
        previous_sibling: Option<DomHandle>,
    ) {
        for id in ids {
            let Some(record) = self.records.get_mut(id) else {
                continue;
            };
            let before = record.boundary_index_keys();
            record.for_each_boundary_mut(|boundary| {
                let container = boundary.container();
                if container == parent && boundary.child_before() == Some(removed_child) {
                    boundary.set_child_before_boundary(dom_host, previous_sibling);
                } else if container != parent
                    && is_descendant_of(dom_host, container, removed_child)
                    && let Some(mut next_boundary) =
                        RangeBoundaryPoint::set_to_start_of_node(dom_host, parent)
                {
                    if next_boundary.set_child_before_boundary(dom_host, previous_sibling) {
                        *boundary = next_boundary;
                    } else if let Some(index_boundary) =
                        RangeBoundaryPoint::new(dom_host, parent, index)
                    {
                        *boundary = index_boundary;
                    }
                }
            });
            let after = record.boundary_index_keys();
            self.reindex_record_boundaries_if_changed(*id, before, after);
        }
    }

    fn update_for_text_split(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        ids: &[RangeRecordId],
        original: DomHandle,
        new_text: DomHandle,
        offset: u32,
    ) {
        let parent_and_after_original_index = dom_host
            .node(original)
            .and_then(|node| node.parent_node())
            .and_then(|parent| {
                dom_host
                    .child_index(parent, original)
                    .and_then(|index| u32::try_from(index).ok())
                    .map(|index| (parent, index.saturating_add(1)))
            });
        for id in ids {
            let Some(record) = self.records.get_mut(id) else {
                continue;
            };
            let before = record.boundary_index_keys();
            record.for_each_boundary_mut(|boundary| {
                let container = boundary.container();
                let Some(current) = boundary.offset(dom_host) else {
                    return;
                };
                if container == original && current > offset {
                    if let Some(next_boundary) =
                        RangeBoundaryPoint::new(dom_host, new_text, current - offset)
                    {
                        *boundary = next_boundary;
                    }
                } else if let Some((parent, after_original_index)) = parent_and_after_original_index
                    && container == parent
                    && current == after_original_index
                    && let Some(next_boundary) =
                        RangeBoundaryPoint::new(dom_host, parent, current.saturating_add(1))
                {
                    *boundary = next_boundary;
                }
            });
            let after = record.boundary_index_keys();
            self.reindex_record_boundaries_if_changed(*id, before, after);
        }
    }

    fn is_collapsed(
        &mut self,
        dom_host: &crate::dom::native::DomHost,
        handle: RangeRecordHandle,
    ) -> Option<bool> {
        let id = self.id_for_handle(handle)?;
        let record = self.records.get_mut(&id)?;
        let start_container = record.start.container();
        let end_container = record.end.container();
        let start_offset = record.start.offset(dom_host)?;
        let end_offset = record.end.offset(dom_host)?;
        Some(start_container == end_container && start_offset == end_offset)
    }

    fn index_record_boundaries(&mut self, id: RangeRecordId) {
        let Some(containers) = self.records.get(&id).map(RangeRecord::boundary_index_keys) else {
            return;
        };
        self.add_record_boundary_index(id, containers);
    }

    fn remove_record_boundary_index(&mut self, id: RangeRecordId) {
        let Some(containers) = self.records.get(&id).map(RangeRecord::boundary_index_keys) else {
            return;
        };
        self.remove_record_boundary_index_for_keys(id, containers);
    }

    fn reindex_record_boundaries_if_changed(
        &mut self,
        id: RangeRecordId,
        before: RangeRecordBoundaryIndexKeys,
        after: RangeRecordBoundaryIndexKeys,
    ) {
        if before == after {
            return;
        }
        self.remove_record_boundary_index_for_keys(id, before);
        self.add_record_boundary_index(id, after);
    }

    fn add_record_boundary_index(&mut self, id: RangeRecordId, keys: RangeRecordBoundaryIndexKeys) {
        for container in keys.containers {
            self.by_boundary_container
                .entry(container)
                .or_default()
                .insert(id);
        }
        for child_before in keys.child_befores.into_iter().flatten() {
            self.by_boundary_child_before
                .entry(child_before)
                .or_default()
                .insert(id);
        }
    }

    fn remove_record_boundary_index_for_keys(
        &mut self,
        id: RangeRecordId,
        keys: RangeRecordBoundaryIndexKeys,
    ) {
        for container in keys.containers {
            let Some(ids) = self.by_boundary_container.get_mut(&container) else {
                continue;
            };
            ids.remove(&id);
            if ids.is_empty() {
                self.by_boundary_container.remove(&container);
            }
        }
        for child_before in keys.child_befores.into_iter().flatten() {
            let Some(ids) = self.by_boundary_child_before.get_mut(&child_before) else {
                continue;
            };
            ids.remove(&id);
            if ids.is_empty() {
                self.by_boundary_child_before.remove(&child_before);
            }
        }
    }

    fn collect_live_record_ids_for_boundary_container(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        container: DomHandle,
        out: &mut HashSet<RangeRecordId>,
    ) {
        let candidates = self.boundary_container_candidate_ids(container);
        for id in candidates {
            if self.live_record_is_alive(scope, id) {
                out.insert(id);
            } else {
                self.remove_live_record_and_record(id);
            }
        }
    }

    fn collect_live_record_ids_for_boundary_child_before(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_before: DomHandle,
        out: &mut HashSet<RangeRecordId>,
    ) {
        let candidates = self.boundary_child_before_candidate_ids(child_before);
        for id in candidates {
            if self.live_record_is_alive(scope, id) {
                out.insert(id);
            } else {
                self.remove_live_record_and_record(id);
            }
        }
    }

    fn live_record_is_alive(&self, scope: &mut v8::PinScope<'_, '_>, id: RangeRecordId) -> bool {
        self.active_live_records.contains(&id)
            && self.records.contains_key(&id)
            && self
                .live_record_wrappers
                .get(&id)
                .and_then(|weak| weak.to_local(scope))
                .is_some()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct RangeRecordBoundaryIndexKeys {
    containers: [DomHandle; 2],
    child_befores: [Option<DomHandle>; 2],
}

impl RangeRecord {
    fn new(dom_host: &crate::dom::native::DomHost, owner_document: DomHandle) -> Option<Self> {
        let start = RangeBoundaryPoint::set_to_start_of_node(dom_host, owner_document)?;
        Some(Self { start, end: start })
    }

    fn boundary(&self, side: RangeBoundarySide) -> &RangeBoundaryPoint {
        match side {
            RangeBoundarySide::Start => &self.start,
            RangeBoundarySide::End => &self.end,
        }
    }

    fn boundary_mut(&mut self, side: RangeBoundarySide) -> &mut RangeBoundaryPoint {
        match side {
            RangeBoundarySide::Start => &mut self.start,
            RangeBoundarySide::End => &mut self.end,
        }
    }

    fn for_each_boundary_mut(&mut self, mut callback: impl FnMut(&mut RangeBoundaryPoint)) {
        callback(&mut self.start);
        callback(&mut self.end);
    }

    fn boundary_index_keys(&self) -> RangeRecordBoundaryIndexKeys {
        RangeRecordBoundaryIndexKeys {
            containers: [self.start.container(), self.end.container()],
            child_befores: [self.start.child_before(), self.end.child_before()],
        }
    }
}

fn update_boundary_offset_for_character_data(
    current: u32,
    edit_offset: u32,
    removed_count: u32,
    inserted_count: u32,
) -> u32 {
    let removed_end = edit_offset.saturating_add(removed_count);
    if current <= edit_offset {
        current
    } else if current <= removed_end {
        edit_offset
    } else {
        current
            .saturating_sub(removed_count)
            .saturating_add(inserted_count)
    }
}

fn is_descendant_of(
    dom_host: &crate::dom::native::DomHost,
    node: DomHandle,
    ancestor: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = dom_host.node(handle).and_then(|entry| entry.parent_node());
    }
    false
}

impl JsContextHost {
    pub(crate) fn create_range_record(
        &mut self,
        owner_document: DomHandle,
    ) -> Option<RangeRecordHandle> {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.range_record_registry
            .create_record(dom_host, owner_document)
    }

    pub(crate) fn reset_range_record(
        &mut self,
        handle: RangeRecordHandle,
        owner_document: DomHandle,
    ) -> bool {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.range_record_registry
            .reset_record(dom_host, handle, owner_document)
    }

    pub(crate) fn range_record_boundary_container(
        &self,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
    ) -> Option<DomHandle> {
        self.range_record_registry.boundary_container(handle, side)
    }

    pub(crate) fn range_record_boundary_point(
        &self,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
    ) -> Option<RangeBoundaryPoint> {
        self.range_record_registry.boundary_point(handle, side)
    }

    pub(crate) fn range_record_boundary_offset(
        &mut self,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
    ) -> Option<u32> {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.range_record_registry
            .boundary_offset(dom_host, handle, side)
    }

    pub(crate) fn range_record_boundary_handles(
        &mut self,
        handle: RangeRecordHandle,
    ) -> Option<((DomHandle, u32), (DomHandle, u32))> {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.range_record_registry
            .boundary_handles(dom_host, handle)
    }

    pub(crate) fn set_range_record_boundary(
        &mut self,
        handle: RangeRecordHandle,
        side: RangeBoundarySide,
        container: DomHandle,
        offset: u32,
    ) -> bool {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.range_record_registry
            .set_boundary(dom_host, handle, side, container, offset)
    }

    pub(crate) fn update_live_range_records_for_character_data_edit(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        target: DomHandle,
        edit_offset: u32,
        removed_count: u32,
        inserted_count: u32,
    ) {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        let ids = self
            .range_record_registry
            .live_record_ids_for_boundary_container(scope, target);
        if ids.is_empty() {
            return;
        }
        let selection_links = self.selection_record_registry.linked_range_boundaries(
            dom_host,
            &self.range_record_registry,
            true,
        );
        self.range_record_registry.update_for_character_data_edit(
            dom_host,
            &ids,
            target,
            edit_offset,
            removed_count,
            inserted_count,
        );
        self.selection_record_registry.sync_linked_range_boundaries(
            dom_host,
            &self.range_record_registry,
            selection_links,
        );
    }

    pub(crate) fn update_live_range_records_for_child_removal(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        parent: DomHandle,
        removed_child: DomHandle,
        index: u32,
        previous_sibling: Option<DomHandle>,
    ) {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        let ids = self
            .range_record_registry
            .live_record_ids_for_child_removal(scope, dom_host, removed_child);
        if ids.is_empty() {
            return;
        }
        // Composed removal boundaries have their own shadow-including
        // rescoping step. Synchronize only the observable endpoints here.
        let selection_links = self.selection_record_registry.linked_range_boundaries(
            dom_host,
            &self.range_record_registry,
            false,
        );
        self.range_record_registry.update_for_child_removal(
            dom_host,
            &ids,
            parent,
            removed_child,
            index,
            previous_sibling,
        );
        self.selection_record_registry.sync_linked_range_boundaries(
            dom_host,
            &self.range_record_registry,
            selection_links,
        );
    }

    pub(crate) fn update_live_range_records_for_text_split(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        original: DomHandle,
        new_text: DomHandle,
        offset: u32,
    ) {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        let ids = self
            .range_record_registry
            .live_record_ids_for_text_split(scope, dom_host, original);
        if ids.is_empty() {
            return;
        }
        let selection_links = self.selection_record_registry.linked_range_boundaries(
            dom_host,
            &self.range_record_registry,
            true,
        );
        self.range_record_registry
            .update_for_text_split(dom_host, &ids, original, new_text, offset);
        self.selection_record_registry.sync_linked_range_boundaries(
            dom_host,
            &self.range_record_registry,
            selection_links,
        );
    }

    pub(crate) fn range_record_is_collapsed(&mut self, handle: RangeRecordHandle) -> Option<bool> {
        let runtime = self.runtime;
        let dom_host = unsafe { &*runtime }.dom_host();
        self.range_record_registry.is_collapsed(dom_host, handle)
    }
}

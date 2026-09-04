use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use moli_core::page::{
    NavigationRedirect, ScriptNetworkOutputItem, SubresourceBodyFinished,
    SubresourceBodyFinishedResult, SubresourceDataReceived, SubresourceEventSourceMessageReceived,
    SubresourceNetworkOutcome, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
    SubresourceRequestInitiatorType, SubresourceRequestStarted, SubresourceResourceType,
    SubresourceResponseBody, SubresourceResponseStarted, WebSocketFrameDirection,
    WebSocketFrameOpcode, WebSocketLifecycleEvent, WebSocketLifecycleKind, WebSocketNetworkEvent,
};
use moli_fetch::NegotiatedHttpVersion;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use url::Url;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetNetworkOutputQueue {
    queue_generation: u64,
    subresource_record_count: usize,
    websocket_event_count: usize,
    next_delivery_order_index: usize,
    completed_subresource_handles: HashSet<SubresourceNetworkRequestHandle>,
    websocket_handshake_recorded_socket_ids: HashSet<u64>,
    pending_websocket_lifecycle_events: HashMap<u64, Vec<WebSocketLifecycleEvent>>,
    staged_subresource_requests:
        HashMap<SubresourceNetworkRequestHandle, Arc<TargetSubresourceRequestStartedOutput>>,
    staged_subresource_responses:
        HashMap<SubresourceNetworkRequestHandle, Arc<TargetSubresourceResponseStartedOutput>>,
    delivery_outputs: TargetNetworkDeliveryOutputQueue,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetNetworkBacklogPreparedDelivery {
    delivery: Option<TargetNetworkBacklogDeliveryToken>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetNetworkBacklogDeliveryToken {
    queue_generation: u64,
    families: TargetNetworkBacklogPreparedFamilies,
    entries: Vec<PendingNetworkBacklogDeliveryEntry>,
    cursor_advances: PendingNetworkBacklogCursorAdvances,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TargetNetworkBacklogPreparedDeliveryBatch {
    families: TargetNetworkBacklogPreparedFamilies,
    entries: Vec<PendingNetworkBacklogDeliveryEntry>,
    cursor_advances: PendingNetworkBacklogCursorAdvances,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TargetNetworkBacklogPreparedFamilies {
    subresource: bool,
    websocket: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetNetworkBacklogActivityCursor {
    subresource_record_start_index: Option<usize>,
    websocket_record_start_index: Option<usize>,
    websocket_event_start_index: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TargetNetworkDeliveryOutputQueue {
    outputs: VecDeque<TargetNetworkDeliveryOutputItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetNetworkDeliveryOutputItem {
    subresource_record_tail_after_item: usize,
    websocket_record_tail_after_item: usize,
    websocket_event_tail_after_item: usize,
    output: TargetNetworkDeliveryOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TargetNetworkDeliveryOutput {
    Subresource(Box<TargetSubresourcePlanOutput>),
    WebSocket(TargetWebSocketDeliveryOutput),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetWebSocketDeliveryOutput {
    source: TargetWebSocketDeliveryOutputSource,
    record: TargetWebSocketDeliveryPlanRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetWebSocketHandshakePlanOutput {
    delivery_order_index: usize,
    socket_id: u64,
    handshake: TargetWebSocketHandshakePlanPayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetWebSocketHandshakePlanPayload {
    index: usize,
    url: Url,
    request_headers: Vec<(String, String)>,
    response: Option<TargetWebSocketHandshakeResponseOutput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetWebSocketDeliveryOutputSource {
    Handshake { record_index: usize },
    Frame { event_index: usize },
    Lifecycle { event_index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSubresourceNetworkActivity {
    sessions: Vec<PendingSubresourceNetworkActivitySession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSubresourceNetworkActivitySession {
    session_id: Option<String>,
    start_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingWebSocketNetworkActivity {
    sessions: Vec<PendingWebSocketNetworkActivitySession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingWebSocketNetworkActivitySession {
    session_id: Option<String>,
    record_start_index: usize,
    event_start_index: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PendingNetworkBacklogDeliverySnapshot {
    entries: Vec<PendingNetworkBacklogDeliveryEntry>,
    cursor_advances: PendingNetworkBacklogCursorAdvances,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSubresourceNetworkCursorAdvance {
    session_id: Option<String>,
    start_index: usize,
    record_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingWebSocketNetworkCursorAdvance {
    session_id: Option<String>,
    record_start_index: usize,
    record_count: usize,
    event_start_index: usize,
    event_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PendingNetworkBacklogCursorAdvances {
    subresource: Vec<PendingSubresourceNetworkCursorAdvance>,
    websocket: Vec<PendingWebSocketNetworkCursorAdvance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingNetworkBacklogDeliveryItem {
    Subresource(Box<TargetSubresourceNetworkDeliveryOutput>),
    WebSocket(TargetWebSocketDeliveryRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingNetworkBacklogDeliveryEntry {
    item: PendingNetworkBacklogDeliveryItem,
    session_ids: Vec<Option<String>>,
}

pub(crate) trait TargetNetworkBacklogRequestIdResolver {
    fn request_id_for_subresource_output(&mut self, output: &TargetSubresourcePlanOutput)
    -> String;

    fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String;
}

impl TargetNetworkBacklogPreparedDelivery {
    pub(crate) fn extend(&mut self, other: Self) {
        let Some(other_delivery) = other.delivery else {
            return;
        };
        if let Some(delivery) = self.delivery.as_mut() {
            if delivery.queue_generation == other_delivery.queue_generation {
                delivery.extend(other_delivery);
            } else if other_delivery.queue_generation > delivery.queue_generation {
                self.delivery = Some(other_delivery);
            }
        } else {
            self.delivery = Some(other_delivery);
        }
    }

    pub(crate) fn has_delivery_output(&self) -> bool {
        self.delivery
            .as_ref()
            .is_some_and(TargetNetworkBacklogDeliveryToken::has_items)
    }

    #[cfg(test)]
    pub(crate) fn has_output(&self) -> bool {
        self.has_delivery_output()
    }

    fn take_delivery_token(&mut self) -> Option<TargetNetworkBacklogDeliveryToken> {
        self.delivery.take()
    }

    fn push_batch(
        &mut self,
        queue_generation: u64,
        batch: TargetNetworkBacklogPreparedDeliveryBatch,
    ) {
        if !batch.has_items() {
            return;
        }
        self.delivery_mut(queue_generation).push_batch(batch);
    }

    fn delivery_mut(&mut self, queue_generation: u64) -> &mut TargetNetworkBacklogDeliveryToken {
        self.delivery
            .get_or_insert_with(|| TargetNetworkBacklogDeliveryToken::new(queue_generation))
    }
}

impl TargetNetworkBacklogDeliveryToken {
    fn new(queue_generation: u64) -> Self {
        Self {
            queue_generation,
            families: TargetNetworkBacklogPreparedFamilies::default(),
            entries: Vec::new(),
            cursor_advances: PendingNetworkBacklogCursorAdvances::default(),
        }
    }

    fn extend(&mut self, other: Self) {
        if self.queue_generation != other.queue_generation {
            return;
        }
        let Self {
            queue_generation: _,
            families,
            entries,
            cursor_advances,
        } = other;
        let (subresource_entries, websocket_entries): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .partition(PendingNetworkBacklogDeliveryEntry::is_subresource);
        let PendingNetworkBacklogCursorAdvances {
            subresource,
            websocket,
        } = cursor_advances;

        if families.subresource && self.families.push_subresource_if_absent() {
            self.entries.extend(subresource_entries);
            self.cursor_advances.subresource.extend(subresource);
        }
        if families.websocket && self.families.push_websocket_if_absent() {
            self.entries.extend(websocket_entries);
            self.cursor_advances.websocket.extend(websocket);
        }
        self.sort_entries();
    }

    fn matches_generation(&self, queue_generation: u64) -> bool {
        self.queue_generation == queue_generation
    }

    fn into_parts(
        self,
    ) -> (
        Vec<PendingNetworkBacklogDeliveryEntry>,
        PendingNetworkBacklogCursorAdvances,
    ) {
        (self.entries, self.cursor_advances)
    }

    fn push_batch(&mut self, batch: TargetNetworkBacklogPreparedDeliveryBatch) {
        let TargetNetworkBacklogPreparedDeliveryBatch {
            families,
            mut entries,
            cursor_advances,
        } = batch;
        let accept_subresource = families.subresource && self.families.push_subresource_if_absent();
        let accept_websocket = families.websocket && self.families.push_websocket_if_absent();
        if !accept_subresource && !accept_websocket {
            return;
        }
        let had_entries = !self.entries.is_empty();
        entries.sort_by_key(PendingNetworkBacklogDeliveryEntry::delivery_order_index);
        let PendingNetworkBacklogCursorAdvances {
            subresource,
            websocket,
        } = cursor_advances;
        match (accept_subresource, accept_websocket) {
            (true, true) => {
                self.entries.extend(entries);
                self.cursor_advances.subresource.extend(subresource);
                self.cursor_advances.websocket.extend(websocket);
            }
            (true, false) => {
                self.entries.extend(
                    entries
                        .into_iter()
                        .filter(PendingNetworkBacklogDeliveryEntry::is_subresource),
                );
                self.cursor_advances.subresource.extend(subresource);
            }
            (false, true) => {
                self.entries
                    .extend(entries.into_iter().filter(|entry| !entry.is_subresource()));
                self.cursor_advances.websocket.extend(websocket);
            }
            (false, false) => {}
        }
        if had_entries {
            self.sort_entries();
        }
    }

    fn has_items(&self) -> bool {
        !self.entries.is_empty()
    }

    fn sort_entries(&mut self) {
        self.entries
            .sort_by_key(PendingNetworkBacklogDeliveryEntry::delivery_order_index);
    }
}

impl TargetNetworkBacklogPreparedDeliveryBatch {
    fn push_subresource_entry(&mut self, entry: PendingNetworkBacklogDeliveryEntry) {
        self.families.subresource = true;
        self.entries.push(entry);
    }

    fn push_websocket_entry(&mut self, entry: PendingNetworkBacklogDeliveryEntry) {
        self.families.websocket = true;
        self.entries.push(entry);
    }

    fn extend_subresource_cursor_advances(
        &mut self,
        cursor_advances: Vec<PendingSubresourceNetworkCursorAdvance>,
    ) {
        self.cursor_advances.subresource.extend(cursor_advances);
    }

    fn extend_websocket_cursor_advances(
        &mut self,
        cursor_advances: Vec<PendingWebSocketNetworkCursorAdvance>,
    ) {
        self.cursor_advances.websocket.extend(cursor_advances);
    }

    fn has_subresource_items(&self) -> bool {
        self.families.subresource
    }

    fn has_websocket_items(&self) -> bool {
        self.families.websocket
    }

    fn has_items(&self) -> bool {
        !self.entries.is_empty()
    }
}

impl TargetNetworkBacklogPreparedFamilies {
    fn push_subresource_if_absent(&mut self) -> bool {
        if self.subresource {
            return false;
        }
        self.subresource = true;
        true
    }

    fn push_websocket_if_absent(&mut self) -> bool {
        if self.websocket {
            return false;
        }
        self.websocket = true;
        true
    }
}

#[cfg(test)]
impl TargetNetworkBacklogActivityCursor {
    pub(crate) fn new(
        subresource_record_start_index: Option<usize>,
        websocket_record_start_index: Option<usize>,
        websocket_event_start_index: Option<usize>,
    ) -> Self {
        Self {
            subresource_record_start_index,
            websocket_record_start_index,
            websocket_event_start_index,
        }
    }
}

#[cfg(test)]
struct TargetNetworkBacklogTestRequestIds;

#[cfg(test)]
impl TargetNetworkBacklogRequestIdResolver for TargetNetworkBacklogTestRequestIds {
    fn request_id_for_subresource_output(
        &mut self,
        output: &TargetSubresourcePlanOutput,
    ) -> String {
        output
            .websocket_socket_id()
            .map(|socket_id| format!("REQ-{socket_id}"))
            .unwrap_or_else(|| format!("REQ-{}", output.index() + 1))
    }

    fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
        format!("REQ-{socket_id}")
    }
}

impl TargetNetworkDeliveryOutputQueue {
    fn push_subresource(&mut self, output: TargetSubresourcePlanOutput) {
        self.push_output(TargetNetworkDeliveryOutput::Subresource(Box::new(output)));
    }

    fn push_handshake_output_if_websocket(
        &mut self,
        delivery_order_index: usize,
        index: usize,
        record: &SubresourceNetworkRecord,
    ) -> bool {
        let Some(record) = TargetWebSocketHandshakePlanOutput::from_subresource_record(
            delivery_order_index,
            index,
            record,
        ) else {
            return false;
        };
        self.push_output(TargetNetworkDeliveryOutput::WebSocket(
            TargetWebSocketDeliveryOutput::new(
                TargetWebSocketDeliveryOutputSource::Handshake {
                    record_index: index,
                },
                TargetWebSocketDeliveryPlanRecord::Handshake(record),
            ),
        ));
        true
    }

    fn prepared_delivery_batch_for_activity(
        &self,
        subresource_activity: Option<PendingSubresourceNetworkActivity>,
        websocket_activity: Option<PendingWebSocketNetworkActivity>,
        subresource_record_end_index: usize,
        websocket_event_end_index: usize,
        request_ids: &mut impl TargetNetworkBacklogRequestIdResolver,
    ) -> TargetNetworkBacklogPreparedDeliveryBatch {
        if subresource_activity.is_none() && websocket_activity.is_none() {
            return TargetNetworkBacklogPreparedDeliveryBatch::default();
        }
        let subresource_record_start_index = subresource_activity
            .as_ref()
            .map(PendingSubresourceNetworkActivity::min_start_index);
        let websocket_record_start_index = websocket_activity
            .as_ref()
            .map(PendingWebSocketNetworkActivity::min_record_start_index);
        let websocket_event_start_index = websocket_activity
            .as_ref()
            .map(PendingWebSocketNetworkActivity::min_event_start_index);
        let mut emitted_subresource_record_end =
            subresource_record_start_index.unwrap_or(subresource_record_end_index);
        let mut emitted_websocket_record_end =
            websocket_record_start_index.unwrap_or(subresource_record_end_index);
        let mut emitted_websocket_event_end =
            websocket_event_start_index.unwrap_or(websocket_event_end_index);
        let mut batch = TargetNetworkBacklogPreparedDeliveryBatch::default();
        for item in self
            .outputs
            .iter()
            .skip(self.first_output_position_for_activity(
                subresource_record_start_index,
                websocket_record_start_index,
                websocket_event_start_index,
            ))
        {
            match item.output() {
                TargetNetworkDeliveryOutput::Subresource(output) => {
                    let output = output.as_ref();
                    let Some(activity) = subresource_activity.as_ref() else {
                        continue;
                    };
                    if !is_index_visible_between(
                        output.index(),
                        activity.min_start_index(),
                        subresource_record_end_index,
                    ) {
                        continue;
                    }
                    emitted_subresource_record_end =
                        emitted_subresource_record_end.max(output.index().saturating_add(1));
                    let output = output.clone();
                    let session_ids = activity.session_ids_for_record_index(output.index());
                    let request_id = request_ids.request_id_for_subresource_output(&output);
                    batch.push_subresource_entry(PendingNetworkBacklogDeliveryEntry::new(
                        PendingNetworkBacklogDeliveryItem::Subresource(Box::new(
                            output.into_delivery_output(request_id),
                        )),
                        session_ids,
                    ));
                }
                TargetNetworkDeliveryOutput::WebSocket(output) => {
                    let Some(activity) = websocket_activity.as_ref() else {
                        continue;
                    };
                    let source = output.source();
                    if !source.is_visible_between(
                        activity.min_record_start_index(),
                        subresource_record_end_index,
                        activity.min_event_start_index(),
                        websocket_event_end_index,
                    ) {
                        continue;
                    }
                    emitted_websocket_record_end =
                        emitted_websocket_record_end.max(source.emitted_record_end());
                    emitted_websocket_event_end =
                        emitted_websocket_event_end.max(source.emitted_event_end());
                    let record = output.record();
                    let request_id =
                        request_ids.request_id_for_websocket_socket(record.socket_id());
                    let item = PendingNetworkBacklogDeliveryItem::WebSocket(
                        record.clone().into_delivery_record(request_id),
                    );
                    batch.push_websocket_entry(
                        PendingNetworkBacklogDeliveryEntry::from_websocket_item(item, activity),
                    );
                }
            }
        }
        if batch.has_subresource_items() {
            let activity = subresource_activity
                .as_ref()
                .expect("subresource batch items require subresource activity");
            batch.extend_subresource_cursor_advances(
                activity.cursor_advances_to(emitted_subresource_record_end),
            );
        }
        if batch.has_websocket_items() {
            let activity = websocket_activity
                .as_ref()
                .expect("WebSocket batch items require WebSocket activity");
            batch.extend_websocket_cursor_advances(
                activity
                    .cursor_advances_to(emitted_websocket_record_end, emitted_websocket_event_end),
            );
        }
        batch
    }

    fn push_frame_from_page_event(
        &mut self,
        delivery_order_index: usize,
        index: usize,
        event: &WebSocketNetworkEvent,
        subresource_record_count: usize,
    ) {
        self.push_output(TargetNetworkDeliveryOutput::WebSocket(
            TargetWebSocketDeliveryOutput::new(
                TargetWebSocketDeliveryOutputSource::Frame { event_index: index },
                TargetWebSocketDeliveryPlanRecord::Frame(
                    TargetWebSocketFrameOutput::from_page_event(
                        delivery_order_index,
                        index,
                        event,
                        subresource_record_count,
                    ),
                ),
            ),
        ));
    }

    fn push_lifecycle_from_page_event(
        &mut self,
        delivery_order_index: usize,
        index: usize,
        event: &WebSocketLifecycleEvent,
        subresource_record_count: usize,
    ) -> bool {
        let Some(output) = TargetWebSocketLifecycleOutput::from_page_event(
            delivery_order_index,
            index,
            event,
            subresource_record_count,
        ) else {
            return false;
        };
        self.push_output(TargetNetworkDeliveryOutput::WebSocket(
            TargetWebSocketDeliveryOutput::new(
                TargetWebSocketDeliveryOutputSource::Lifecycle { event_index: index },
                TargetWebSocketDeliveryPlanRecord::Lifecycle(output),
            ),
        ));
        true
    }

    fn push_output(&mut self, output: TargetNetworkDeliveryOutput) {
        let subresource_record_tail_after_item = self
            .subresource_record_tail()
            .max(output.emitted_subresource_record_end());
        let websocket_record_tail_after_item = self
            .websocket_record_tail()
            .max(output.emitted_websocket_record_end());
        let websocket_event_tail_after_item = self
            .websocket_event_tail()
            .max(output.emitted_websocket_event_end());
        self.outputs.push_back(TargetNetworkDeliveryOutputItem::new(
            subresource_record_tail_after_item,
            websocket_record_tail_after_item,
            websocket_event_tail_after_item,
            output,
        ));
    }

    fn discard_observed_outputs(
        &mut self,
        subresource_record_cursor: usize,
        websocket_record_cursor: usize,
        websocket_event_cursor: usize,
    ) {
        while self
            .outputs
            .front()
            .is_some_and(|item| match item.output() {
                TargetNetworkDeliveryOutput::Subresource(output) => {
                    output.index() < subresource_record_cursor
                }
                TargetNetworkDeliveryOutput::WebSocket(output) => match output.source() {
                    TargetWebSocketDeliveryOutputSource::Handshake { record_index } => {
                        record_index < websocket_record_cursor
                    }
                    TargetWebSocketDeliveryOutputSource::Frame { event_index }
                    | TargetWebSocketDeliveryOutputSource::Lifecycle { event_index } => {
                        event_index < websocket_event_cursor
                    }
                },
            })
        {
            self.outputs.pop_front();
        }
    }

    fn first_output_position_for_activity(
        &self,
        subresource_record_start_index: Option<usize>,
        websocket_record_start_index: Option<usize>,
        websocket_event_start_index: Option<usize>,
    ) -> usize {
        self.outputs.partition_point(|item| {
            subresource_record_start_index
                .is_none_or(|start_index| item.subresource_record_tail_after_item() <= start_index)
                && websocket_record_start_index.is_none_or(|start_index| {
                    item.websocket_record_tail_after_item() <= start_index
                })
                && websocket_event_start_index
                    .is_none_or(|start_index| item.websocket_event_tail_after_item() <= start_index)
        })
    }

    fn subresource_record_tail(&self) -> usize {
        self.outputs
            .back()
            .map(TargetNetworkDeliveryOutputItem::subresource_record_tail_after_item)
            .unwrap_or(0)
    }

    fn websocket_record_tail(&self) -> usize {
        self.outputs
            .back()
            .map(TargetNetworkDeliveryOutputItem::websocket_record_tail_after_item)
            .unwrap_or(0)
    }

    fn websocket_event_tail(&self) -> usize {
        self.outputs
            .back()
            .map(TargetNetworkDeliveryOutputItem::websocket_event_tail_after_item)
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn subresource_outputs_from(&self, start_index: usize) -> Vec<TargetSubresourceMetadataOutput> {
        self.outputs
            .iter()
            .skip(self.first_output_position_for_activity(Some(start_index), None, None))
            .filter_map(TargetNetworkDeliveryOutputItem::subresource_output)
            .filter(|output| output.index() >= start_index)
            .cloned()
            .collect()
    }

    #[cfg(test)]
    fn websocket_records(&self) -> impl Iterator<Item = &TargetWebSocketDeliveryPlanRecord> {
        self.outputs.iter().filter_map(|item| match item.output() {
            TargetNetworkDeliveryOutput::Subresource(_) => None,
            TargetNetworkDeliveryOutput::WebSocket(output) => Some(output.record()),
        })
    }

    #[cfg(test)]
    fn websocket_sources(&self) -> impl Iterator<Item = TargetWebSocketDeliveryOutputSource> + '_ {
        self.outputs.iter().filter_map(|item| match item.output() {
            TargetNetworkDeliveryOutput::Subresource(_) => None,
            TargetNetworkDeliveryOutput::WebSocket(output) => Some(output.source()),
        })
    }

    #[cfg(test)]
    fn websocket_frame_outputs_from(&self, start_index: usize) -> Vec<TargetWebSocketFrameOutput> {
        self.websocket_records()
            .filter_map(|record| match record {
                TargetWebSocketDeliveryPlanRecord::Frame(output)
                    if output.index() >= start_index =>
                {
                    Some(output.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    fn subresource_output_mut(
        &mut self,
        record_index: usize,
    ) -> Option<&mut TargetSubresourceMetadataOutput> {
        self.outputs.iter_mut().find_map(|item| {
            let TargetNetworkDeliveryOutput::Subresource(output) = item.output_mut() else {
                return None;
            };
            (output.index() == record_index)
                .then(|| output.as_complete_mut())
                .flatten()
        })
    }

    #[cfg(test)]
    fn websocket_record_mut(
        &mut self,
        position: usize,
    ) -> Option<&mut TargetWebSocketDeliveryPlanRecord> {
        self.outputs
            .iter_mut()
            .filter_map(|item| match item.output_mut() {
                TargetNetworkDeliveryOutput::Subresource(_) => None,
                TargetNetworkDeliveryOutput::WebSocket(output) => Some(output.record_mut()),
            })
            .nth(position)
    }
}

impl TargetNetworkDeliveryOutputItem {
    fn new(
        subresource_record_tail_after_item: usize,
        websocket_record_tail_after_item: usize,
        websocket_event_tail_after_item: usize,
        output: TargetNetworkDeliveryOutput,
    ) -> Self {
        Self {
            subresource_record_tail_after_item,
            websocket_record_tail_after_item,
            websocket_event_tail_after_item,
            output,
        }
    }

    fn subresource_record_tail_after_item(&self) -> usize {
        self.subresource_record_tail_after_item
    }

    fn websocket_record_tail_after_item(&self) -> usize {
        self.websocket_record_tail_after_item
    }

    fn websocket_event_tail_after_item(&self) -> usize {
        self.websocket_event_tail_after_item
    }

    fn output(&self) -> &TargetNetworkDeliveryOutput {
        &self.output
    }

    #[cfg(test)]
    fn output_mut(&mut self) -> &mut TargetNetworkDeliveryOutput {
        &mut self.output
    }

    #[cfg(test)]
    fn subresource_output(&self) -> Option<&TargetSubresourceMetadataOutput> {
        match self.output() {
            TargetNetworkDeliveryOutput::Subresource(output) => output.as_complete(),
            TargetNetworkDeliveryOutput::WebSocket(_) => None,
        }
    }
}

impl TargetNetworkDeliveryOutput {
    fn emitted_subresource_record_end(&self) -> usize {
        match self {
            Self::Subresource(output) => output.index().saturating_add(1),
            Self::WebSocket(_) => 0,
        }
    }

    fn emitted_websocket_record_end(&self) -> usize {
        match self {
            Self::Subresource(_) => 0,
            Self::WebSocket(output) => output.source().emitted_record_end(),
        }
    }

    fn emitted_websocket_event_end(&self) -> usize {
        match self {
            Self::Subresource(_) => 0,
            Self::WebSocket(output) => output.source().emitted_event_end(),
        }
    }
}

impl TargetWebSocketDeliveryOutput {
    fn new(
        source: TargetWebSocketDeliveryOutputSource,
        record: TargetWebSocketDeliveryPlanRecord,
    ) -> Self {
        Self { source, record }
    }

    fn source(&self) -> TargetWebSocketDeliveryOutputSource {
        self.source
    }

    fn record(&self) -> &TargetWebSocketDeliveryPlanRecord {
        &self.record
    }

    #[cfg(test)]
    fn record_mut(&mut self) -> &mut TargetWebSocketDeliveryPlanRecord {
        &mut self.record
    }
}

impl TargetWebSocketDeliveryOutputSource {
    fn is_visible_between(
        self,
        record_start_index: usize,
        record_end_index: usize,
        event_start_index: usize,
        event_end_index: usize,
    ) -> bool {
        match self {
            Self::Handshake { record_index } => {
                record_start_index <= record_index && record_index < record_end_index
            }
            Self::Frame { event_index } | Self::Lifecycle { event_index } => {
                event_start_index <= event_index && event_index < event_end_index
            }
        }
    }

    fn emitted_record_end(self) -> usize {
        match self {
            Self::Handshake { record_index } => record_index.saturating_add(1),
            Self::Frame { .. } | Self::Lifecycle { .. } => 0,
        }
    }

    fn emitted_event_end(self) -> usize {
        match self {
            Self::Handshake { .. } => 0,
            Self::Frame { event_index } | Self::Lifecycle { event_index } => {
                event_index.saturating_add(1)
            }
        }
    }
}

impl TargetWebSocketHandshakePlanOutput {
    fn from_subresource_record(
        delivery_order_index: usize,
        index: usize,
        record: &SubresourceNetworkRecord,
    ) -> Option<Self> {
        if record.resource_type() != SubresourceResourceType::WebSocket {
            return None;
        }
        let socket_id = record.websocket_socket_id()?;
        Some(Self {
            delivery_order_index,
            socket_id,
            handshake: TargetWebSocketHandshakePlanPayload::from_subresource_record(index, record),
        })
    }
}

impl TargetWebSocketHandshakePlanPayload {
    fn from_subresource_record(index: usize, record: &SubresourceNetworkRecord) -> Self {
        let response = match record.outcome() {
            SubresourceNetworkOutcome::Success {
                status,
                response_headers,
                ..
            } => Some(TargetWebSocketHandshakeResponseOutput {
                status: *status,
                response_headers: response_headers.clone(),
            }),
            SubresourceNetworkOutcome::Failure { .. } => None,
        };
        Self {
            index,
            url: record.url().clone(),
            request_headers: record.request_headers().to_vec(),
            response,
        }
    }
}

impl PendingNetworkBacklogDeliverySnapshot {
    fn from_delivery_entries(
        mut entries: Vec<PendingNetworkBacklogDeliveryEntry>,
        cursor_advances: PendingNetworkBacklogCursorAdvances,
    ) -> Option<Self> {
        entries.sort_by_key(PendingNetworkBacklogDeliveryEntry::delivery_order_index);
        (!entries.is_empty()).then_some(Self {
            entries,
            cursor_advances,
        })
    }

    #[cfg(test)]
    pub(crate) fn outputs(&self) -> Vec<&PendingNetworkBacklogDeliveryItem> {
        self.entries.iter().map(|entry| &entry.item).collect()
    }

    pub(crate) fn delivery_entries(
        &self,
    ) -> impl Iterator<Item = (&PendingNetworkBacklogDeliveryItem, &[Option<String>])> {
        self.entries
            .iter()
            .map(|entry| (&entry.item, entry.session_ids.as_slice()))
    }

    #[cfg(test)]
    pub(crate) fn subresource_session_ids_for_record_index(
        &self,
        record_index: usize,
    ) -> Vec<Option<String>> {
        self.entries
            .iter()
            .find_map(|entry| {
                (entry.item.subresource_record_index() == Some(record_index))
                    .then(|| entry.session_ids.clone())
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn websocket_session_ids_for_record_index(
        &self,
        record_index: usize,
    ) -> Vec<Option<String>> {
        self.entries
            .iter()
            .find_map(|entry| {
                (entry.item.websocket_record_index() == Some(record_index))
                    .then(|| entry.session_ids.clone())
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn websocket_session_ids_for_event_index(
        &self,
        event_index: usize,
    ) -> Vec<Option<String>> {
        self.entries
            .iter()
            .find_map(|entry| {
                (entry.item.websocket_event_index() == Some(event_index))
                    .then(|| entry.session_ids.clone())
            })
            .unwrap_or_default()
    }

    pub(crate) fn subresource_cursor_advances(&self) -> &[PendingSubresourceNetworkCursorAdvance] {
        &self.cursor_advances.subresource
    }

    pub(crate) fn websocket_cursor_advances(&self) -> &[PendingWebSocketNetworkCursorAdvance] {
        &self.cursor_advances.websocket
    }
}

impl PendingSubresourceNetworkCursorAdvance {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn start_index(&self) -> usize {
        self.start_index
    }

    pub(crate) fn record_count(&self) -> usize {
        self.record_count
    }
}

impl PendingWebSocketNetworkCursorAdvance {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn record_start_index(&self) -> usize {
        self.record_start_index
    }

    pub(crate) fn record_count(&self) -> usize {
        self.record_count
    }

    pub(crate) fn event_start_index(&self) -> usize {
        self.event_start_index
    }

    pub(crate) fn event_count(&self) -> usize {
        self.event_count
    }
}

impl PendingNetworkBacklogDeliveryEntry {
    fn new(item: PendingNetworkBacklogDeliveryItem, session_ids: Vec<Option<String>>) -> Self {
        Self { item, session_ids }
    }

    fn from_websocket_item(
        item: PendingNetworkBacklogDeliveryItem,
        activity: &PendingWebSocketNetworkActivity,
    ) -> Self {
        let session_ids = match &item {
            PendingNetworkBacklogDeliveryItem::Subresource(_) => Vec::new(),
            PendingNetworkBacklogDeliveryItem::WebSocket(
                TargetWebSocketDeliveryRecord::Handshake(output),
            ) => activity.session_ids_for_record_index(output.index()),
            PendingNetworkBacklogDeliveryItem::WebSocket(TargetWebSocketDeliveryRecord::Frame(
                output,
            )) => activity.session_ids_for_event_index(output.index()),
            PendingNetworkBacklogDeliveryItem::WebSocket(
                TargetWebSocketDeliveryRecord::Lifecycle(output),
            ) => activity.session_ids_for_event_index(output.index()),
        };
        Self { item, session_ids }
    }

    fn delivery_order_index(&self) -> usize {
        self.item.delivery_order_index()
    }

    fn is_subresource(&self) -> bool {
        self.item.is_subresource()
    }
}

impl PendingNetworkBacklogDeliveryItem {
    #[cfg(test)]
    pub(crate) fn as_subresource(&self) -> Option<&TargetSubresourceNetworkDeliveryOutput> {
        match self {
            Self::Subresource(output) => Some(output.as_ref()),
            Self::WebSocket(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_websocket(&self) -> Option<&TargetWebSocketDeliveryRecord> {
        match self {
            Self::Subresource(_) => None,
            Self::WebSocket(output) => Some(output),
        }
    }

    fn delivery_order_index(&self) -> usize {
        match self {
            Self::Subresource(output) => output.delivery_order_index(),
            Self::WebSocket(output) => output.delivery_order_index(),
        }
    }

    fn is_subresource(&self) -> bool {
        matches!(self, Self::Subresource { .. })
    }

    #[cfg(test)]
    fn subresource_record_index(&self) -> Option<usize> {
        self.as_subresource()
            .map(TargetSubresourceNetworkDeliveryOutput::index)
    }

    #[cfg(test)]
    fn websocket_record_index(&self) -> Option<usize> {
        self.as_websocket()
            .and_then(TargetWebSocketDeliveryRecord::record_index)
    }

    #[cfg(test)]
    fn websocket_event_index(&self) -> Option<usize> {
        self.as_websocket()
            .and_then(TargetWebSocketDeliveryRecord::event_index)
    }
}

impl PendingSubresourceNetworkActivity {
    pub(crate) fn from_sessions(
        sessions: Vec<PendingSubresourceNetworkActivitySession>,
    ) -> Option<Self> {
        (!sessions.is_empty()).then_some(Self { sessions })
    }

    pub(crate) fn min_start_index(&self) -> usize {
        self.sessions
            .iter()
            .map(PendingSubresourceNetworkActivitySession::start_index)
            .min()
            .unwrap_or(0)
    }

    pub(crate) fn session_ids_for_record_index(&self, record_index: usize) -> Vec<Option<String>> {
        self.sessions
            .iter()
            .filter(|session| session.start_index <= record_index)
            .map(|session| session.session_id.clone())
            .collect()
    }

    fn cursor_advances_to(
        &self,
        emitted_record_end: usize,
    ) -> Vec<PendingSubresourceNetworkCursorAdvance> {
        self.sessions
            .iter()
            .map(|session| PendingSubresourceNetworkCursorAdvance {
                session_id: session.session_id.clone(),
                start_index: session.start_index(),
                record_count: emitted_record_end.saturating_sub(session.start_index()),
            })
            .collect()
    }
}

impl PendingSubresourceNetworkActivitySession {
    pub(crate) fn new(session_id: Option<String>, start_index: usize) -> Self {
        Self {
            session_id,
            start_index,
        }
    }

    pub(crate) fn start_index(&self) -> usize {
        self.start_index
    }
}

impl PendingWebSocketNetworkActivity {
    pub(crate) fn from_sessions(
        sessions: Vec<PendingWebSocketNetworkActivitySession>,
    ) -> Option<Self> {
        (!sessions.is_empty()).then_some(Self { sessions })
    }

    pub(crate) fn min_record_start_index(&self) -> usize {
        self.sessions
            .iter()
            .map(PendingWebSocketNetworkActivitySession::record_start_index)
            .min()
            .unwrap_or(0)
    }

    pub(crate) fn min_event_start_index(&self) -> usize {
        self.sessions
            .iter()
            .map(PendingWebSocketNetworkActivitySession::event_start_index)
            .min()
            .unwrap_or(0)
    }

    pub(crate) fn session_ids_for_record_index(&self, record_index: usize) -> Vec<Option<String>> {
        self.sessions
            .iter()
            .filter(|session| session.record_start_index <= record_index)
            .map(|session| session.session_id.clone())
            .collect()
    }

    pub(crate) fn session_ids_for_event_index(&self, event_index: usize) -> Vec<Option<String>> {
        self.sessions
            .iter()
            .filter(|session| session.event_start_index <= event_index)
            .map(|session| session.session_id.clone())
            .collect()
    }

    fn cursor_advances_to(
        &self,
        emitted_record_end: usize,
        emitted_event_end: usize,
    ) -> Vec<PendingWebSocketNetworkCursorAdvance> {
        self.sessions
            .iter()
            .map(|session| PendingWebSocketNetworkCursorAdvance {
                session_id: session.session_id.clone(),
                record_start_index: session.record_start_index(),
                record_count: emitted_record_end.saturating_sub(session.record_start_index()),
                event_start_index: session.event_start_index(),
                event_count: emitted_event_end.saturating_sub(session.event_start_index()),
            })
            .collect()
    }
}

impl PendingWebSocketNetworkActivitySession {
    pub(crate) fn new(
        session_id: Option<String>,
        record_start_index: usize,
        event_start_index: usize,
    ) -> Self {
        Self {
            session_id,
            record_start_index,
            event_start_index,
        }
    }

    pub(crate) fn record_start_index(&self) -> usize {
        self.record_start_index
    }

    pub(crate) fn event_start_index(&self) -> usize {
        self.event_start_index
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetSubresourcePlanOutput {
    Complete(Box<TargetSubresourceMetadataOutput>),
    RequestStarted(Arc<TargetSubresourceRequestStartedOutput>),
    RequestExtraInfo(TargetSubresourceRequestExtraInfoOutput),
    ResponseStarted(Arc<TargetSubresourceResponseStartedOutput>),
    DataReceived(TargetSubresourceDataReceivedOutput),
    EventSourceMessageReceived(Box<TargetSubresourceEventSourceMessageReceivedOutput>),
    BodyFinished(Box<TargetSubresourceBodyFinishedOutput>),
}

impl TargetSubresourcePlanOutput {
    pub(crate) fn index(&self) -> usize {
        match self {
            Self::Complete(output) => output.index(),
            Self::RequestStarted(output) => output.index(),
            Self::RequestExtraInfo(output) => output.index(),
            Self::ResponseStarted(output) => output.index(),
            Self::DataReceived(output) => output.index(),
            Self::EventSourceMessageReceived(output) => output.index(),
            Self::BodyFinished(output) => output.index(),
        }
    }

    pub(crate) fn websocket_socket_id(&self) -> Option<u64> {
        match self {
            Self::Complete(output) => output.websocket_socket_id(),
            Self::RequestStarted(_)
            | Self::RequestExtraInfo(_)
            | Self::ResponseStarted(_)
            | Self::DataReceived(_)
            | Self::EventSourceMessageReceived(_)
            | Self::BodyFinished(_) => None,
        }
    }

    pub(crate) fn request_handle(&self) -> Option<SubresourceNetworkRequestHandle> {
        match self {
            Self::Complete(output) => output.request_handle(),
            Self::RequestStarted(output) => Some(output.handle()),
            Self::RequestExtraInfo(output) => Some(output.handle()),
            Self::ResponseStarted(output) => Some(output.handle()),
            Self::DataReceived(output) => Some(output.handle()),
            Self::EventSourceMessageReceived(output) => Some(output.handle()),
            Self::BodyFinished(output) => Some(output.handle()),
        }
    }

    fn into_delivery_output(self, request_id: String) -> TargetSubresourceNetworkDeliveryOutput {
        match self {
            Self::Complete(output) => TargetSubresourceNetworkDeliveryOutput::Complete(Box::new(
                TargetSubresourceCompleteNetworkDeliveryOutput::new(*output, request_id),
            )),
            Self::RequestStarted(output) => TargetSubresourceNetworkDeliveryOutput::RequestStarted(
                TargetSubresourceRequestNetworkDeliveryOutput::new(output, request_id),
            ),
            Self::RequestExtraInfo(output) => {
                TargetSubresourceNetworkDeliveryOutput::RequestExtraInfo(
                    TargetSubresourceRequestExtraInfoNetworkDeliveryOutput::new(output, request_id),
                )
            }
            Self::ResponseStarted(output) => {
                TargetSubresourceNetworkDeliveryOutput::ResponseStarted(
                    TargetSubresourceResponseNetworkDeliveryOutput::new(output, request_id),
                )
            }
            Self::DataReceived(output) => TargetSubresourceNetworkDeliveryOutput::DataReceived(
                TargetSubresourceDataNetworkDeliveryOutput::new(output, request_id),
            ),
            Self::EventSourceMessageReceived(output) => {
                TargetSubresourceNetworkDeliveryOutput::EventSourceMessageReceived(Box::new(
                    TargetSubresourceEventSourceMessageNetworkDeliveryOutput::new(
                        *output, request_id,
                    ),
                ))
            }
            Self::BodyFinished(output) => {
                TargetSubresourceNetworkDeliveryOutput::BodyFinished(Box::new(
                    TargetSubresourceBodyNetworkDeliveryOutput::new(*output, request_id),
                ))
            }
        }
    }

    #[cfg(test)]
    fn as_complete(&self) -> Option<&TargetSubresourceMetadataOutput> {
        match self {
            Self::Complete(output) => Some(output),
            Self::RequestStarted(_)
            | Self::RequestExtraInfo(_)
            | Self::ResponseStarted(_)
            | Self::DataReceived(_)
            | Self::EventSourceMessageReceived(_)
            | Self::BodyFinished(_) => None,
        }
    }

    #[cfg(test)]
    fn as_complete_mut(&mut self) -> Option<&mut TargetSubresourceMetadataOutput> {
        match self {
            Self::Complete(output) => Some(output),
            Self::RequestStarted(_)
            | Self::RequestExtraInfo(_)
            | Self::ResponseStarted(_)
            | Self::DataReceived(_)
            | Self::EventSourceMessageReceived(_)
            | Self::BodyFinished(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceRequestStartedOutput {
    delivery_order_index: usize,
    index: usize,
    loader_id: String,
    handle: SubresourceNetworkRequestHandle,
    frame_id: Option<String>,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    request_body_bytes: Option<Vec<u8>>,
    resource_type: SubresourceResourceType,
    request_initiator_type: SubresourceRequestInitiatorType,
    request_cookie_report: Option<StoredCookieQueryReport>,
}

impl TargetSubresourceRequestStartedOutput {
    fn from_page_request_started(
        delivery_order_index: usize,
        index: usize,
        loader_id: &str,
        request: &SubresourceRequestStarted,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            loader_id: loader_id.to_owned(),
            handle: request.handle(),
            frame_id: request.frame_id().map(str::to_owned),
            document_url: request.document_url().clone(),
            url: request.url().clone(),
            method: request.method().to_owned(),
            request_headers: request.request_headers().to_vec(),
            request_body: request.request_body().map(str::to_owned),
            request_body_bytes: request.request_body_bytes().map(<[u8]>::to_vec),
            resource_type: request.resource_type(),
            request_initiator_type: request.request_initiator_type(),
            request_cookie_report: request.request_cookie_report().cloned(),
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.handle
    }

    pub(crate) fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub(crate) fn frame_id(&self) -> Option<&str> {
        self.frame_id.as_deref()
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn method(&self) -> &str {
        &self.method
    }

    pub(crate) fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    pub(crate) fn request_body(&self) -> Option<&str> {
        self.request_body.as_deref()
    }

    pub(crate) fn request_body_bytes(&self) -> Option<&[u8]> {
        self.request_body_bytes.as_deref()
    }

    pub(crate) fn resource_type(&self) -> SubresourceResourceType {
        self.resource_type
    }

    pub(crate) fn request_initiator_type(&self) -> SubresourceRequestInitiatorType {
        self.request_initiator_type
    }

    pub(crate) fn request_cookie_report(&self) -> Option<&StoredCookieQueryReport> {
        self.request_cookie_report.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceRequestExtraInfoOutput {
    delivery_order_index: usize,
    index: usize,
    request: Arc<TargetSubresourceRequestStartedOutput>,
    request_headers: Vec<(String, String)>,
    request_cookie_report: StoredCookieQueryReport,
}

impl TargetSubresourceRequestExtraInfoOutput {
    fn new(
        delivery_order_index: usize,
        index: usize,
        request: Arc<TargetSubresourceRequestStartedOutput>,
        request_headers: Vec<(String, String)>,
        request_cookie_report: StoredCookieQueryReport,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            request,
            request_headers,
            request_cookie_report,
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.request.handle()
    }

    pub(crate) fn request_cookie_report(&self) -> &StoredCookieQueryReport {
        &self.request_cookie_report
    }

    pub(crate) fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceResponseStartedOutput {
    delivery_order_index: usize,
    index: usize,
    request: Arc<TargetSubresourceRequestStartedOutput>,
    redirect_chain: Vec<TargetSubresourceRedirectOutput>,
    final_url: Url,
    status: u16,
    status_text: Option<String>,
    response_headers: Vec<(String, String)>,
    cookie_set_reports: Vec<StoredCookieSetReport>,
    from_cache: bool,
    network_request_headers: Option<Vec<(String, String)>>,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
}

impl TargetSubresourceResponseStartedOutput {
    fn from_page_response_started(
        delivery_order_index: usize,
        index: usize,
        request: Arc<TargetSubresourceRequestStartedOutput>,
        response: &SubresourceResponseStarted,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            request,
            redirect_chain: response
                .redirect_chain()
                .iter()
                .map(TargetSubresourceRedirectOutput::from_page_redirect)
                .collect(),
            final_url: response.final_url().clone(),
            status: response.status(),
            status_text: response.status_text().map(str::to_owned),
            response_headers: response.response_headers().to_vec(),
            cookie_set_reports: response.cookie_set_reports().to_vec(),
            from_cache: response.from_cache(),
            network_request_headers: response
                .network_request_headers()
                .map(|headers| headers.to_vec()),
            negotiated_http_version: response.negotiated_http_version(),
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.request.handle()
    }

    pub(crate) fn request(&self) -> &TargetSubresourceRequestStartedOutput {
        &self.request
    }

    pub(crate) fn redirect_chain(&self) -> &[TargetSubresourceRedirectOutput] {
        &self.redirect_chain
    }

    pub(crate) fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    pub(crate) fn status_text(&self) -> Option<&str> {
        self.status_text.as_deref()
    }

    pub(crate) fn response_headers(&self) -> &[(String, String)] {
        &self.response_headers
    }

    pub(crate) fn cookie_set_reports(&self) -> &[StoredCookieSetReport] {
        &self.cookie_set_reports
    }

    pub(crate) fn is_from_cache(&self) -> bool {
        self.from_cache
    }

    pub(crate) fn network_request_headers(&self) -> Option<&[(String, String)]> {
        self.network_request_headers.as_deref()
    }

    pub(crate) fn negotiated_http_version(&self) -> Option<NegotiatedHttpVersion> {
        self.negotiated_http_version
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceDataReceivedOutput {
    delivery_order_index: usize,
    index: usize,
    data: SubresourceDataReceived,
}

impl TargetSubresourceDataReceivedOutput {
    fn from_page_data_received(
        delivery_order_index: usize,
        index: usize,
        data: &SubresourceDataReceived,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            data: data.clone(),
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.data.handle()
    }

    pub(crate) fn data_length(&self) -> usize {
        self.data.data_length()
    }

    pub(crate) fn encoded_data_length(&self) -> usize {
        self.data.encoded_data_length()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceEventSourceMessageReceivedOutput {
    delivery_order_index: usize,
    index: usize,
    message: SubresourceEventSourceMessageReceived,
}

impl TargetSubresourceEventSourceMessageReceivedOutput {
    fn from_page_event_source_message_received(
        delivery_order_index: usize,
        index: usize,
        message: &SubresourceEventSourceMessageReceived,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            message: message.clone(),
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.message.handle()
    }

    pub(crate) fn event_name(&self) -> &str {
        self.message.event_name()
    }

    pub(crate) fn event_id(&self) -> &str {
        self.message.event_id()
    }

    pub(crate) fn data(&self) -> &str {
        self.message.data()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceBodyFinishedOutput {
    delivery_order_index: usize,
    index: usize,
    request: Arc<TargetSubresourceRequestStartedOutput>,
    response: Option<Arc<TargetSubresourceResponseStartedOutput>>,
    result: SubresourceBodyFinishedResult,
    data_was_streamed: bool,
}

impl TargetSubresourceBodyFinishedOutput {
    fn from_page_body_finished(
        delivery_order_index: usize,
        index: usize,
        request: Arc<TargetSubresourceRequestStartedOutput>,
        response: Option<Arc<TargetSubresourceResponseStartedOutput>>,
        body: &SubresourceBodyFinished,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            request,
            response,
            result: body.result().clone(),
            data_was_streamed: body.data_was_streamed(),
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.request.handle()
    }

    pub(crate) fn request(&self) -> &TargetSubresourceRequestStartedOutput {
        &self.request
    }

    pub(crate) fn result(&self) -> &SubresourceBodyFinishedResult {
        &self.result
    }

    pub(crate) fn data_was_streamed(&self) -> bool {
        self.data_was_streamed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceMetadataOutput {
    delivery_order_index: usize,
    index: usize,
    loader_id: String,
    response_body: Option<SubresourceResponseBody>,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    websocket_socket_id: Option<u64>,
    frame_id: Option<String>,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    request_body_bytes: Option<Vec<u8>>,
    resource_type: SubresourceResourceType,
    request_initiator_type: SubresourceRequestInitiatorType,
    request_cookie_report: Option<StoredCookieQueryReport>,
    outcome: TargetSubresourceMetadataOutcome,
    cookie_set_reports: Vec<StoredCookieSetReport>,
    from_cache: bool,
    network_request_headers: Option<Vec<(String, String)>>,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
}

impl TargetSubresourceMetadataOutput {
    fn from_page_record(
        delivery_order_index: usize,
        index: usize,
        loader_id: &str,
        record: &SubresourceNetworkRecord,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            loader_id: loader_id.to_owned(),
            response_body: subresource_response_body(record),
            request_handle: record.request_handle(),
            websocket_socket_id: record.websocket_socket_id(),
            frame_id: record.frame_id().map(str::to_owned),
            document_url: record.document_url().clone(),
            url: record.url().clone(),
            method: record.method().to_owned(),
            request_headers: record.request_headers().to_vec(),
            request_body: record.request_body().map(str::to_owned),
            request_body_bytes: record.request_body_bytes().map(|body| body.to_vec()),
            resource_type: record.resource_type(),
            request_initiator_type: record.request_initiator_type(),
            request_cookie_report: record.request_cookie_report().cloned(),
            outcome: TargetSubresourceMetadataOutcome::from_page_outcome(record.outcome()),
            cookie_set_reports: record.cookie_set_reports().to_vec(),
            from_cache: record.from_cache(),
            network_request_headers: record
                .network_request_headers()
                .map(|headers| headers.to_vec()),
            negotiated_http_version: record.negotiated_http_version(),
        }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub(crate) fn response_body(&self) -> Option<&SubresourceResponseBody> {
        self.response_body.as_ref()
    }

    pub(crate) fn request_handle(&self) -> Option<SubresourceNetworkRequestHandle> {
        self.request_handle
    }

    pub(crate) fn websocket_socket_id(&self) -> Option<u64> {
        self.websocket_socket_id
    }

    pub(crate) fn frame_id(&self) -> Option<&str> {
        self.frame_id.as_deref()
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn method(&self) -> &str {
        &self.method
    }

    pub(crate) fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    pub(crate) fn request_body(&self) -> Option<&str> {
        self.request_body.as_deref()
    }

    pub(crate) fn request_body_bytes(&self) -> Option<&[u8]> {
        self.request_body_bytes.as_deref()
    }

    pub(crate) fn resource_type(&self) -> SubresourceResourceType {
        self.resource_type
    }

    pub(crate) fn request_initiator_type(&self) -> SubresourceRequestInitiatorType {
        self.request_initiator_type
    }

    pub(crate) fn request_cookie_report(&self) -> Option<&StoredCookieQueryReport> {
        self.request_cookie_report.as_ref()
    }

    pub(crate) fn outcome(&self) -> &TargetSubresourceMetadataOutcome {
        &self.outcome
    }

    pub(crate) fn cookie_set_reports(&self) -> &[StoredCookieSetReport] {
        &self.cookie_set_reports
    }

    pub(crate) fn is_from_cache(&self) -> bool {
        self.from_cache
    }

    pub(crate) fn network_request_headers(&self) -> Option<&[(String, String)]> {
        self.network_request_headers.as_deref()
    }

    pub(crate) fn negotiated_http_version(&self) -> Option<NegotiatedHttpVersion> {
        self.negotiated_http_version
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetSubresourceNetworkDeliveryOutput {
    Complete(Box<TargetSubresourceCompleteNetworkDeliveryOutput>),
    RequestStarted(TargetSubresourceRequestNetworkDeliveryOutput),
    RequestExtraInfo(TargetSubresourceRequestExtraInfoNetworkDeliveryOutput),
    ResponseStarted(TargetSubresourceResponseNetworkDeliveryOutput),
    DataReceived(TargetSubresourceDataNetworkDeliveryOutput),
    EventSourceMessageReceived(Box<TargetSubresourceEventSourceMessageNetworkDeliveryOutput>),
    BodyFinished(Box<TargetSubresourceBodyNetworkDeliveryOutput>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceCompleteNetworkDeliveryOutput {
    request_id: String,
    output: TargetSubresourceMetadataOutput,
}

impl TargetSubresourceCompleteNetworkDeliveryOutput {
    fn new(output: TargetSubresourceMetadataOutput, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn metadata(&self) -> &TargetSubresourceMetadataOutput {
        &self.output
    }

    #[cfg(test)]
    pub(crate) fn index(&self) -> usize {
        self.output.index()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceRequestNetworkDeliveryOutput {
    request_id: String,
    output: Arc<TargetSubresourceRequestStartedOutput>,
}

impl TargetSubresourceRequestNetworkDeliveryOutput {
    fn new(output: Arc<TargetSubresourceRequestStartedOutput>, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn output(&self) -> &TargetSubresourceRequestStartedOutput {
        &self.output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceRequestExtraInfoNetworkDeliveryOutput {
    request_id: String,
    output: TargetSubresourceRequestExtraInfoOutput,
}

impl TargetSubresourceRequestExtraInfoNetworkDeliveryOutput {
    fn new(output: TargetSubresourceRequestExtraInfoOutput, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn output(&self) -> &TargetSubresourceRequestExtraInfoOutput {
        &self.output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceResponseNetworkDeliveryOutput {
    request_id: String,
    output: Arc<TargetSubresourceResponseStartedOutput>,
}

impl TargetSubresourceResponseNetworkDeliveryOutput {
    fn new(output: Arc<TargetSubresourceResponseStartedOutput>, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn output(&self) -> &TargetSubresourceResponseStartedOutput {
        &self.output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceDataNetworkDeliveryOutput {
    request_id: String,
    output: TargetSubresourceDataReceivedOutput,
}

impl TargetSubresourceDataNetworkDeliveryOutput {
    fn new(output: TargetSubresourceDataReceivedOutput, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn output(&self) -> &TargetSubresourceDataReceivedOutput {
        &self.output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceEventSourceMessageNetworkDeliveryOutput {
    request_id: String,
    output: TargetSubresourceEventSourceMessageReceivedOutput,
}

impl TargetSubresourceEventSourceMessageNetworkDeliveryOutput {
    fn new(output: TargetSubresourceEventSourceMessageReceivedOutput, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn output(&self) -> &TargetSubresourceEventSourceMessageReceivedOutput {
        &self.output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceBodyNetworkDeliveryOutput {
    request_id: String,
    output: TargetSubresourceBodyFinishedOutput,
}

impl TargetSubresourceBodyNetworkDeliveryOutput {
    fn new(output: TargetSubresourceBodyFinishedOutput, request_id: String) -> Self {
        Self { request_id, output }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.output.delivery_order_index()
    }

    pub(crate) fn output(&self) -> &TargetSubresourceBodyFinishedOutput {
        &self.output
    }
}

impl TargetSubresourceNetworkDeliveryOutput {
    #[cfg(test)]
    pub(crate) fn request_id(&self) -> &str {
        match self {
            Self::Complete(output) => output.request_id(),
            Self::RequestStarted(output) => output.request_id(),
            Self::RequestExtraInfo(output) => output.request_id(),
            Self::ResponseStarted(output) => output.request_id(),
            Self::DataReceived(output) => output.request_id(),
            Self::EventSourceMessageReceived(output) => output.request_id(),
            Self::BodyFinished(output) => output.request_id(),
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        match self {
            Self::Complete(output) => output.delivery_order_index(),
            Self::RequestStarted(output) => output.delivery_order_index(),
            Self::RequestExtraInfo(output) => output.delivery_order_index(),
            Self::ResponseStarted(output) => output.delivery_order_index(),
            Self::DataReceived(output) => output.delivery_order_index(),
            Self::EventSourceMessageReceived(output) => output.delivery_order_index(),
            Self::BodyFinished(output) => output.delivery_order_index(),
        }
    }

    #[cfg(test)]
    pub(crate) fn metadata(&self) -> &TargetSubresourceMetadataOutput {
        match self {
            Self::Complete(output) => output.metadata(),
            Self::RequestStarted(_)
            | Self::RequestExtraInfo(_)
            | Self::ResponseStarted(_)
            | Self::DataReceived(_)
            | Self::EventSourceMessageReceived(_)
            | Self::BodyFinished(_) => {
                panic!("staged subresource delivery output has no complete metadata")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn index(&self) -> usize {
        match self {
            Self::Complete(output) => output.index(),
            Self::RequestStarted(output) => output.output().index(),
            Self::RequestExtraInfo(output) => output.output().index(),
            Self::ResponseStarted(output) => output.output().index(),
            Self::DataReceived(output) => output.output().index(),
            Self::EventSourceMessageReceived(output) => output.output().index(),
            Self::BodyFinished(output) => output.output().index(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetSubresourceMetadataOutcome {
    Success {
        redirect_chain: Vec<TargetSubresourceRedirectOutput>,
        final_url: Url,
        status: u16,
        status_text: Option<String>,
        response_headers: Vec<(String, String)>,
        response_body_len: usize,
    },
    Failure {
        error_text: String,
    },
}

impl TargetSubresourceMetadataOutcome {
    fn from_page_outcome(outcome: &SubresourceNetworkOutcome) -> Self {
        match outcome {
            SubresourceNetworkOutcome::Success {
                redirect_chain,
                final_url,
                status,
                status_text,
                response_headers,
                response_body,
            } => Self::Success {
                redirect_chain: redirect_chain
                    .iter()
                    .map(TargetSubresourceRedirectOutput::from_page_redirect)
                    .collect(),
                final_url: final_url.clone(),
                status: *status,
                status_text: status_text.clone(),
                response_headers: response_headers.clone(),
                response_body_len: response_body.len(),
            },
            SubresourceNetworkOutcome::Failure { error_text } => Self::Failure {
                error_text: error_text.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetSubresourceRedirectOutput {
    pub(crate) from_url: Url,
    pub(crate) to_url: Url,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) request_cookie_report: Option<StoredCookieQueryReport>,
    pub(crate) cookie_set_reports: Vec<StoredCookieSetReport>,
    pub(crate) from_cache: bool,
    pub(crate) negotiated_http_version: Option<NegotiatedHttpVersion>,
}

impl TargetSubresourceRedirectOutput {
    fn from_page_redirect(redirect: &NavigationRedirect) -> Self {
        Self {
            from_url: redirect.from_url.clone(),
            to_url: redirect.to_url.clone(),
            status: redirect.status,
            headers: redirect.headers.clone(),
            request_cookie_report: redirect.request_cookie_report.clone(),
            cookie_set_reports: redirect.cookie_set_reports.clone(),
            from_cache: redirect.from_cache,
            negotiated_http_version: redirect.negotiated_http_version,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetWebSocketDeliveryRecord {
    Handshake(TargetWebSocketHandshakeDeliveryOutput),
    Frame(TargetWebSocketFrameDeliveryOutput),
    Lifecycle(TargetWebSocketLifecycleDeliveryOutput),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TargetWebSocketDeliveryPlanRecord {
    Handshake(TargetWebSocketHandshakePlanOutput),
    Frame(TargetWebSocketFrameOutput),
    Lifecycle(TargetWebSocketLifecycleOutput),
}

impl TargetWebSocketDeliveryRecord {
    pub(crate) fn delivery_order_index(&self) -> usize {
        match self {
            Self::Handshake(output) => output.delivery_order_index(),
            Self::Frame(output) => output.delivery_order_index(),
            Self::Lifecycle(output) => output.delivery_order_index(),
        }
    }

    #[cfg(test)]
    pub(crate) fn record_index(&self) -> Option<usize> {
        match self {
            Self::Handshake(output) => Some(output.index()),
            Self::Frame(_) | Self::Lifecycle(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn event_index(&self) -> Option<usize> {
        match self {
            Self::Handshake(_) => None,
            Self::Frame(output) => Some(output.index()),
            Self::Lifecycle(output) => Some(output.index()),
        }
    }

    #[cfg(test)]
    pub(crate) fn as_handshake(&self) -> Option<&TargetWebSocketHandshakeDeliveryOutput> {
        match self {
            Self::Handshake(output) => Some(output),
            Self::Frame(_) | Self::Lifecycle(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_frame(&self) -> Option<&TargetWebSocketFrameDeliveryOutput> {
        match self {
            Self::Handshake(_) => None,
            Self::Frame(output) => Some(output),
            Self::Lifecycle(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_lifecycle(&self) -> Option<&TargetWebSocketLifecycleDeliveryOutput> {
        match self {
            Self::Handshake(_) | Self::Frame(_) => None,
            Self::Lifecycle(output) => Some(output),
        }
    }
}

impl TargetWebSocketDeliveryPlanRecord {
    fn socket_id(&self) -> u64 {
        match self {
            Self::Handshake(output) => output.socket_id,
            Self::Frame(output) => output.socket_id(),
            Self::Lifecycle(output) => output.socket_id(),
        }
    }

    fn into_delivery_record(self, request_id: String) -> TargetWebSocketDeliveryRecord {
        match self {
            Self::Handshake(output) => TargetWebSocketDeliveryRecord::Handshake(
                TargetWebSocketHandshakeDeliveryOutput::from_plan_output(output, request_id),
            ),
            Self::Frame(output) => TargetWebSocketDeliveryRecord::Frame(
                TargetWebSocketFrameDeliveryOutput::from_frame_output(output, request_id),
            ),
            Self::Lifecycle(output) => TargetWebSocketDeliveryRecord::Lifecycle(
                TargetWebSocketLifecycleDeliveryOutput::from_lifecycle_output(output, request_id),
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetWebSocketHandshakeDeliveryOutput {
    delivery_order_index: usize,
    request_id: String,
    index: usize,
    url: Url,
    request_headers: Vec<(String, String)>,
    response: Option<TargetWebSocketHandshakeResponseOutput>,
}

impl TargetWebSocketHandshakeDeliveryOutput {
    fn from_plan_output(record: TargetWebSocketHandshakePlanOutput, request_id: String) -> Self {
        let TargetWebSocketHandshakePlanOutput {
            delivery_order_index,
            socket_id: _,
            handshake:
                TargetWebSocketHandshakePlanPayload {
                    index,
                    url,
                    request_headers,
                    response,
                },
        } = record;
        Self {
            delivery_order_index,
            request_id,
            index,
            url,
            request_headers,
            response,
        }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    pub(crate) fn response(&self) -> Option<&TargetWebSocketHandshakeResponseOutput> {
        self.response.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetWebSocketHandshakeResponseOutput {
    status: u16,
    response_headers: Vec<(String, String)>,
}

impl TargetWebSocketHandshakeResponseOutput {
    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    pub(crate) fn response_headers(&self) -> &[(String, String)] {
        &self.response_headers
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetWebSocketFrameOutput {
    delivery_order_index: usize,
    index: usize,
    timestamp_order_index: usize,
    socket_id: u64,
    direction: WebSocketFrameDirection,
    opcode: WebSocketFrameOpcode,
    payload_length: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetWebSocketFrameDeliveryOutput {
    delivery_order_index: usize,
    request_id: String,
    index: usize,
    timestamp_order_index: usize,
    direction: WebSocketFrameDirection,
    opcode: WebSocketFrameOpcode,
    payload_length: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetWebSocketLifecycleOutput {
    delivery_order_index: usize,
    index: usize,
    timestamp_order_index: usize,
    socket_id: u64,
    kind: TargetWebSocketLifecycleDeliveryKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetWebSocketLifecycleDeliveryOutput {
    delivery_order_index: usize,
    request_id: String,
    index: usize,
    timestamp_order_index: usize,
    kind: TargetWebSocketLifecycleDeliveryKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetWebSocketLifecycleDeliveryKind {
    FrameError { error_text: String },
    Closed,
}

impl TargetWebSocketFrameOutput {
    fn from_page_event(
        delivery_order_index: usize,
        index: usize,
        event: &WebSocketNetworkEvent,
        subresource_record_count: usize,
    ) -> Self {
        Self {
            delivery_order_index,
            index,
            timestamp_order_index: websocket_event_timestamp_order_index(
                subresource_record_count,
                index,
            ),
            socket_id: event.socket_id(),
            direction: event.direction(),
            opcode: event.opcode(),
            payload_length: event.payload_length(),
        }
    }

    #[cfg(test)]
    pub(crate) fn index(&self) -> usize {
        self.index
    }

    fn socket_id(&self) -> u64 {
        self.socket_id
    }
}

impl TargetWebSocketFrameDeliveryOutput {
    fn from_frame_output(output: TargetWebSocketFrameOutput, request_id: String) -> Self {
        let TargetWebSocketFrameOutput {
            delivery_order_index,
            index,
            timestamp_order_index,
            socket_id: _,
            direction,
            opcode,
            payload_length,
        } = output;
        Self {
            delivery_order_index,
            request_id,
            index,
            timestamp_order_index,
            direction,
            opcode,
            payload_length,
        }
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn timestamp_order_index(&self) -> usize {
        self.timestamp_order_index
    }

    pub(crate) fn direction(&self) -> WebSocketFrameDirection {
        self.direction
    }

    pub(crate) fn opcode(&self) -> WebSocketFrameOpcode {
        self.opcode
    }

    pub(crate) fn payload_length(&self) -> usize {
        self.payload_length
    }
}

impl TargetWebSocketLifecycleOutput {
    fn from_page_event(
        delivery_order_index: usize,
        index: usize,
        event: &WebSocketLifecycleEvent,
        subresource_record_count: usize,
    ) -> Option<Self> {
        let kind = match event.kind() {
            WebSocketLifecycleKind::Error => TargetWebSocketLifecycleDeliveryKind::FrameError {
                error_text: event.error_text()?.to_owned(),
            },
            WebSocketLifecycleKind::Close => TargetWebSocketLifecycleDeliveryKind::Closed,
            WebSocketLifecycleKind::Open | WebSocketLifecycleKind::Closing => return None,
        };
        Some(Self {
            delivery_order_index,
            index,
            timestamp_order_index: websocket_event_timestamp_order_index(
                subresource_record_count,
                index,
            ),
            socket_id: event.socket_id(),
            kind,
        })
    }

    fn socket_id(&self) -> u64 {
        self.socket_id
    }
}

impl TargetWebSocketLifecycleDeliveryOutput {
    fn from_lifecycle_output(output: TargetWebSocketLifecycleOutput, request_id: String) -> Self {
        let TargetWebSocketLifecycleOutput {
            delivery_order_index,
            index,
            timestamp_order_index,
            socket_id: _,
            kind,
        } = output;
        Self {
            delivery_order_index,
            request_id,
            index,
            timestamp_order_index,
            kind,
        }
    }

    pub(crate) fn delivery_order_index(&self) -> usize {
        self.delivery_order_index
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn timestamp_order_index(&self) -> usize {
        self.timestamp_order_index
    }

    pub(crate) fn kind(&self) -> &TargetWebSocketLifecycleDeliveryKind {
        &self.kind
    }
}

impl TargetNetworkOutputQueue {
    pub(crate) fn subresource_record_count(&self) -> usize {
        self.subresource_record_count
    }

    pub(crate) fn websocket_event_count(&self) -> usize {
        self.websocket_event_count
    }

    pub(crate) fn reset(&mut self) {
        let queue_generation = self.queue_generation.wrapping_add(1);
        *self = Self {
            queue_generation,
            ..Self::default()
        };
    }

    pub(crate) fn discard_observed_delivery_outputs(
        &mut self,
        subresource_record_cursor: usize,
        websocket_record_cursor: usize,
        websocket_event_cursor: usize,
    ) {
        self.delivery_outputs.discard_observed_outputs(
            subresource_record_cursor,
            websocket_record_cursor,
            websocket_event_cursor,
        );
    }

    #[cfg(test)]
    pub(crate) fn retained_delivery_output_count(&self) -> usize {
        self.delivery_outputs.outputs.len()
    }

    #[cfg(test)]
    fn items(&self) -> Vec<TargetSubresourceMetadataOutput> {
        self.delivery_outputs.subresource_outputs_from(0)
    }

    #[cfg(test)]
    fn websocket_frame_outputs_from(&self, start_index: usize) -> Vec<TargetWebSocketFrameOutput> {
        self.delivery_outputs
            .websocket_frame_outputs_from(start_index)
    }

    fn next_delivery_order_index(&mut self) -> usize {
        let index = self.next_delivery_order_index;
        self.next_delivery_order_index = self.next_delivery_order_index.wrapping_add(1);
        index
    }

    pub(crate) fn pending_network_backlog_delivery_snapshot_from_backlog(
        &self,
        backlog: &mut TargetNetworkBacklogPreparedDelivery,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        let token = backlog.take_delivery_token()?;
        if !token.matches_generation(self.queue_generation) {
            return None;
        }
        let (entries, cursor_advances) = token.into_parts();
        PendingNetworkBacklogDeliverySnapshot::from_delivery_entries(entries, cursor_advances)
    }

    #[cfg(test)]
    pub(crate) fn backlog_prepared_delivery(
        &self,
        cursor: TargetNetworkBacklogActivityCursor,
    ) -> TargetNetworkBacklogPreparedDelivery {
        let subresource_activity = cursor.subresource_record_start_index.map(|start_index| {
            PendingSubresourceNetworkActivity::from_sessions(vec![
                PendingSubresourceNetworkActivitySession::new(None, start_index),
            ])
            .expect("test cursor activity should contain one session")
        });
        let websocket_activity = match (
            cursor.websocket_record_start_index,
            cursor.websocket_event_start_index,
        ) {
            (Some(record_start_index), event_start_index) => {
                PendingWebSocketNetworkActivity::from_sessions(vec![
                    PendingWebSocketNetworkActivitySession::new(
                        None,
                        record_start_index,
                        event_start_index.unwrap_or(usize::MAX),
                    ),
                ])
            }
            (None, Some(event_start_index)) => {
                PendingWebSocketNetworkActivity::from_sessions(vec![
                    PendingWebSocketNetworkActivitySession::new(
                        None,
                        usize::MAX,
                        event_start_index,
                    ),
                ])
            }
            _ => None,
        };
        let mut request_ids = TargetNetworkBacklogTestRequestIds;
        self.backlog_prepared_delivery_for_activity(
            subresource_activity,
            websocket_activity,
            &mut request_ids,
        )
    }

    pub(crate) fn backlog_prepared_delivery_for_activity(
        &self,
        subresource_activity: Option<PendingSubresourceNetworkActivity>,
        websocket_activity: Option<PendingWebSocketNetworkActivity>,
        request_ids: &mut impl TargetNetworkBacklogRequestIdResolver,
    ) -> TargetNetworkBacklogPreparedDelivery {
        let mut outputs = TargetNetworkBacklogPreparedDelivery::default();
        let batch = self.delivery_outputs.prepared_delivery_batch_for_activity(
            subresource_activity,
            websocket_activity,
            self.subresource_record_count,
            self.websocket_event_count,
            request_ids,
        );
        outputs.push_batch(self.queue_generation, batch);
        outputs
    }

    /// Appends one source-bound fact from the concrete renderer output stream.
    ///
    /// The cumulative Network report retained by `Page` is diagnostic state,
    /// not a live protocol source. Every item accepted here is therefore an
    /// incremental FIFO fact; replacement Documents are separated by the
    /// caller's exact lifecycle identity check and queue reset.
    pub(crate) fn append_renderer_output_item_for_loader(
        &mut self,
        item: &ScriptNetworkOutputItem,
        document_loader_id: &str,
    ) {
        let mut subresource_index = self.subresource_record_count;
        self.append_page_output_item_for_loader(item, document_loader_id, &mut subresource_index);
    }

    fn append_page_output_item_for_loader(
        &mut self,
        item: &ScriptNetworkOutputItem,
        document_loader_id: &str,
        subresource_index: &mut usize,
    ) {
        match item {
            ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                self.append_subresource_record(*subresource_index, document_loader_id, record);
                *subresource_index += 1;
            }
            ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                self.append_subresource_request_started(
                    *subresource_index,
                    document_loader_id,
                    request,
                );
                *subresource_index += 1;
            }
            ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
                self.append_missing_subresource_request_extra_info(
                    *subresource_index,
                    response.handle(),
                    response.network_request_headers(),
                    None,
                );
                if self.append_subresource_response_started(*subresource_index, response) {
                    *subresource_index += 1;
                }
            }
            ScriptNetworkOutputItem::SubresourceDataReceived(data) => {
                self.append_subresource_data_received(*subresource_index, data);
                *subresource_index += 1;
            }
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(message) => {
                self.append_subresource_event_source_message_received(*subresource_index, message);
                *subresource_index += 1;
            }
            ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
                if self.append_subresource_body_finished(*subresource_index, body) {
                    *subresource_index += 1;
                }
            }
            ScriptNetworkOutputItem::WebSocketNetworkEvent(event) => {
                let subresource_record_count = self.subresource_record_count;
                let delivery_order_index = self.next_delivery_order_index();
                let websocket_event_index = self.websocket_event_count;
                self.delivery_outputs.push_frame_from_page_event(
                    delivery_order_index,
                    websocket_event_index,
                    event,
                    subresource_record_count,
                );
                self.websocket_event_count += 1;
            }
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(event) => {
                self.append_websocket_lifecycle_event(event);
            }
        }
    }

    fn append_subresource_record(
        &mut self,
        index: usize,
        document_loader_id: &str,
        record: &SubresourceNetworkRecord,
    ) {
        if let Some(handle) = record.request_handle() {
            if !self.completed_subresource_handles.insert(handle) {
                self.subresource_record_count = index + 1;
                return;
            }
            if self.staged_subresource_requests.contains_key(&handle) {
                self.append_staged_subresource_record_completion(index, handle, record);
                self.subresource_record_count = index + 1;
                return;
            }
        }
        let delivery_order_index = self.next_delivery_order_index();
        self.delivery_outputs
            .push_subresource(TargetSubresourcePlanOutput::Complete(Box::new(
                TargetSubresourceMetadataOutput::from_page_record(
                    delivery_order_index,
                    index,
                    document_loader_id,
                    record,
                ),
            )));
        let appended_websocket_handshake = self
            .delivery_outputs
            .push_handshake_output_if_websocket(delivery_order_index, index, record);
        self.subresource_record_count = index + 1;
        if appended_websocket_handshake && let Some(socket_id) = record.websocket_socket_id() {
            self.websocket_handshake_recorded_socket_ids
                .insert(socket_id);
            self.flush_pending_websocket_lifecycle_events(socket_id);
        }
    }

    fn append_websocket_lifecycle_event(&mut self, event: &WebSocketLifecycleEvent) {
        if !matches!(
            event.kind(),
            WebSocketLifecycleKind::Error | WebSocketLifecycleKind::Close
        ) {
            return;
        }
        if !self
            .websocket_handshake_recorded_socket_ids
            .contains(&event.socket_id())
        {
            // Failed handshakes reach the renderer as Error then metadata, while CDP requires
            // webSocketCreated/webSocketWillSendHandshakeRequest before terminal events.
            self.pending_websocket_lifecycle_events
                .entry(event.socket_id())
                .or_default()
                .push(event.clone());
            return;
        }
        self.push_websocket_lifecycle_event(event);
    }

    fn flush_pending_websocket_lifecycle_events(&mut self, socket_id: u64) {
        let Some(events) = self.pending_websocket_lifecycle_events.remove(&socket_id) else {
            return;
        };
        for event in events {
            self.push_websocket_lifecycle_event(&event);
        }
    }

    fn push_websocket_lifecycle_event(&mut self, event: &WebSocketLifecycleEvent) {
        let delivery_order_index = self.next_delivery_order_index();
        let websocket_event_index = self.websocket_event_count;
        if self.delivery_outputs.push_lifecycle_from_page_event(
            delivery_order_index,
            websocket_event_index,
            event,
            self.subresource_record_count,
        ) {
            self.websocket_event_count += 1;
        }
    }

    fn append_staged_subresource_record_completion(
        &mut self,
        index: usize,
        handle: SubresourceNetworkRequestHandle,
        record: &SubresourceNetworkRecord,
    ) {
        self.append_missing_subresource_request_extra_info(
            index,
            handle,
            record.network_request_headers(),
            record.request_cookie_report(),
        );
        match record.outcome() {
            SubresourceNetworkOutcome::Success {
                redirect_chain,
                final_url,
                status,
                status_text,
                response_headers,
                response_body,
            } => {
                let response = SubresourceResponseStarted::new(
                    handle,
                    redirect_chain.clone(),
                    final_url.clone(),
                    *status,
                    response_headers.clone(),
                    record.cookie_set_reports().to_vec(),
                )
                .with_status_text(status_text.clone())
                .with_from_cache(record.from_cache())
                .with_network_request_headers(
                    record
                        .network_request_headers()
                        .map(|headers| headers.to_vec()),
                )
                .with_negotiated_http_version(record.negotiated_http_version());
                self.append_subresource_response_started(index, &response);
                let body = SubresourceBodyFinished::ready(handle, response_body.clone());
                self.append_subresource_body_finished(index, &body);
            }
            SubresourceNetworkOutcome::Failure { error_text } => {
                let body = SubresourceBodyFinished::failed(handle, error_text.clone());
                self.append_subresource_body_finished(index, &body);
            }
        }
    }

    fn append_subresource_request_started(
        &mut self,
        index: usize,
        document_loader_id: &str,
        request: &SubresourceRequestStarted,
    ) {
        let delivery_order_index = self.next_delivery_order_index();
        let output = Arc::new(
            TargetSubresourceRequestStartedOutput::from_page_request_started(
                delivery_order_index,
                index,
                document_loader_id,
                request,
            ),
        );
        self.staged_subresource_requests
            .insert(output.handle(), output.clone());
        self.delivery_outputs
            .push_subresource(TargetSubresourcePlanOutput::RequestStarted(output));
        self.subresource_record_count = index + 1;
    }

    fn append_subresource_response_started(
        &mut self,
        index: usize,
        response: &SubresourceResponseStarted,
    ) -> bool {
        let Some(request) = self
            .staged_subresource_requests
            .get(&response.handle())
            .cloned()
        else {
            return false;
        };
        let delivery_order_index = self.next_delivery_order_index();
        let output = Arc::new(
            TargetSubresourceResponseStartedOutput::from_page_response_started(
                delivery_order_index,
                index,
                request,
                response,
            ),
        );
        self.staged_subresource_responses
            .insert(output.handle(), output.clone());
        self.delivery_outputs
            .push_subresource(TargetSubresourcePlanOutput::ResponseStarted(output));
        self.subresource_record_count = index + 1;
        true
    }

    fn append_subresource_request_extra_info(
        &mut self,
        index: usize,
        handle: SubresourceNetworkRequestHandle,
        request_headers: Vec<(String, String)>,
        request_cookie_report: StoredCookieQueryReport,
    ) -> bool {
        let Some(request) = self.staged_subresource_requests.get(&handle).cloned() else {
            return false;
        };
        let delivery_order_index = self.next_delivery_order_index();
        let output = TargetSubresourceRequestExtraInfoOutput::new(
            delivery_order_index,
            index,
            request,
            request_headers,
            request_cookie_report,
        );
        self.delivery_outputs
            .push_subresource(TargetSubresourcePlanOutput::RequestExtraInfo(output));
        self.subresource_record_count = index + 1;
        true
    }

    fn append_missing_subresource_request_extra_info(
        &mut self,
        index: usize,
        handle: SubresourceNetworkRequestHandle,
        network_request_headers: Option<&[(String, String)]>,
        request_cookie_report: Option<&StoredCookieQueryReport>,
    ) {
        let Some(request) = self.staged_subresource_requests.get(&handle) else {
            return;
        };
        if request.request_cookie_report().is_some() {
            return;
        }
        let request_headers = network_request_headers
            .unwrap_or_else(|| request.request_headers())
            .to_vec();
        let Some(request_cookie_report) = request_cookie_report.cloned().or_else(|| {
            network_request_headers
                .is_some()
                .then(StoredCookieQueryReport::default)
        }) else {
            return;
        };
        self.append_subresource_request_extra_info(
            index,
            handle,
            request_headers,
            request_cookie_report,
        );
    }

    fn append_subresource_data_received(&mut self, index: usize, data: &SubresourceDataReceived) {
        let delivery_order_index = self.next_delivery_order_index();
        let output = TargetSubresourceDataReceivedOutput::from_page_data_received(
            delivery_order_index,
            index,
            data,
        );
        self.delivery_outputs
            .push_subresource(TargetSubresourcePlanOutput::DataReceived(output));
        self.subresource_record_count = index + 1;
    }

    fn append_subresource_event_source_message_received(
        &mut self,
        index: usize,
        message: &SubresourceEventSourceMessageReceived,
    ) {
        let delivery_order_index = self.next_delivery_order_index();
        let output =
            TargetSubresourceEventSourceMessageReceivedOutput::from_page_event_source_message_received(
                delivery_order_index,
                index,
                message,
            );
        self.delivery_outputs.push_subresource(
            TargetSubresourcePlanOutput::EventSourceMessageReceived(Box::new(output)),
        );
        self.subresource_record_count = index + 1;
    }

    fn append_subresource_body_finished(
        &mut self,
        index: usize,
        body: &SubresourceBodyFinished,
    ) -> bool {
        let Some(request) = self
            .staged_subresource_requests
            .get(&body.handle())
            .cloned()
        else {
            return false;
        };
        let response = self
            .staged_subresource_responses
            .get(&body.handle())
            .cloned();
        let delivery_order_index = self.next_delivery_order_index();
        let output = TargetSubresourceBodyFinishedOutput::from_page_body_finished(
            delivery_order_index,
            index,
            request,
            response,
            body,
        );
        self.delivery_outputs
            .push_subresource(TargetSubresourcePlanOutput::BodyFinished(Box::new(output)));
        self.subresource_record_count = index + 1;
        true
    }
}

fn websocket_event_timestamp_order_index(
    subresource_record_count: usize,
    websocket_event_index: usize,
) -> usize {
    subresource_record_count
        .wrapping_add(websocket_event_index)
        .wrapping_add(1)
}

fn is_index_visible_between(index: usize, start_index: usize, end_index: usize) -> bool {
    start_index <= index && index < end_index
}

fn subresource_response_body(record: &SubresourceNetworkRecord) -> Option<SubresourceResponseBody> {
    let SubresourceNetworkOutcome::Success { response_body, .. } = record.outcome() else {
        return None;
    };
    Some(response_body.clone())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Arc;

    use moli_cookie_jar::StoredCookieQueryReport;
    use moli_core::page::{
        NavigationRedirect, ScriptNetworkOutputItem, SubresourceBodyFinished,
        SubresourceNetworkRecord, SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType,
        SubresourceRequestStarted, SubresourceResourceType, SubresourceResponseBody,
        SubresourceResponseStarted, WebSocketFrameDirection, WebSocketFrameOpcode,
        WebSocketLifecycleEvent, WebSocketNetworkEvent,
    };
    use url::Url;

    use super::{
        PendingNetworkBacklogDeliveryItem, PendingNetworkBacklogDeliverySnapshot,
        PendingSubresourceNetworkActivity, PendingSubresourceNetworkActivitySession,
        PendingWebSocketNetworkActivity, PendingWebSocketNetworkActivitySession,
        TargetNetworkBacklogActivityCursor, TargetNetworkBacklogRequestIdResolver,
        TargetNetworkDeliveryOutput, TargetNetworkDeliveryOutputItem, TargetNetworkOutputQueue,
        TargetSubresourceMetadataOutcome, TargetSubresourceMetadataOutput,
        TargetSubresourceNetworkDeliveryOutput, TargetSubresourcePlanOutput,
        TargetWebSocketDeliveryOutputSource, TargetWebSocketDeliveryPlanRecord,
        TargetWebSocketDeliveryRecord, TargetWebSocketFrameOutput,
        TargetWebSocketLifecycleDeliveryKind,
    };

    struct TestBacklogRequestIds;

    impl TargetNetworkBacklogRequestIdResolver for TestBacklogRequestIds {
        fn request_id_for_subresource_output(
            &mut self,
            output: &TargetSubresourcePlanOutput,
        ) -> String {
            output
                .websocket_socket_id()
                .map(|socket_id| format!("REQ-{socket_id}"))
                .unwrap_or_else(|| format!("REQ-{}", output.index() + 1))
        }

        fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
            format!("REQ-{socket_id}")
        }
    }

    #[derive(Default)]
    struct StableSubresourceHandleRequestIds {
        request_ids_by_handle: HashMap<u64, String>,
        next_request_id: usize,
    }

    impl TargetNetworkBacklogRequestIdResolver for StableSubresourceHandleRequestIds {
        fn request_id_for_subresource_output(
            &mut self,
            output: &TargetSubresourcePlanOutput,
        ) -> String {
            if let Some(handle) = output.request_handle() {
                return self
                    .request_ids_by_handle
                    .entry(handle.get())
                    .or_insert_with(|| {
                        self.next_request_id += 1;
                        format!("REQ-H{}", handle.get())
                    })
                    .clone();
            }
            self.next_request_id += 1;
            format!("REQ-{}", self.next_request_id)
        }

        fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
            format!("REQ-{socket_id}")
        }
    }

    fn pending_delivery_snapshot(
        output_queue: &TargetNetworkOutputQueue,
        subresource_activity: Option<PendingSubresourceNetworkActivity>,
        websocket_activity: Option<PendingWebSocketNetworkActivity>,
        request_ids: &mut impl TargetNetworkBacklogRequestIdResolver,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        let mut backlog = output_queue.backlog_prepared_delivery_for_activity(
            subresource_activity,
            websocket_activity,
            request_ids,
        );
        output_queue.pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
    }

    fn subresource_outputs(
        snapshot: &PendingNetworkBacklogDeliverySnapshot,
    ) -> Vec<&TargetSubresourceNetworkDeliveryOutput> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| item.as_subresource())
            .collect()
    }

    fn websocket_outputs(
        snapshot: &PendingNetworkBacklogDeliverySnapshot,
    ) -> Vec<&TargetWebSocketDeliveryRecord> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| item.as_websocket())
            .collect()
    }

    fn producer_network_items_for_test(
        subresource_records: &[SubresourceNetworkRecord],
        websocket_events: &[WebSocketNetworkEvent],
    ) -> Vec<ScriptNetworkOutputItem> {
        subresource_records
            .iter()
            .cloned()
            .map(Box::new)
            .map(ScriptNetworkOutputItem::SubresourceNetworkRecord)
            .chain(
                websocket_events
                    .iter()
                    .cloned()
                    .map(ScriptNetworkOutputItem::WebSocketNetworkEvent),
            )
            .collect()
    }

    fn apply_network_items_for_test(
        output_queue: &mut TargetNetworkOutputQueue,
        subresource_records: &[SubresourceNetworkRecord],
        websocket_events: &[WebSocketNetworkEvent],
    ) {
        append_concrete_items_for_test(
            output_queue,
            &producer_network_items_for_test(subresource_records, websocket_events),
            "LOADER-1",
        );
    }

    fn append_concrete_items_for_test(
        output_queue: &mut TargetNetworkOutputQueue,
        items: &[ScriptNetworkOutputItem],
        document_loader_id: &str,
    ) {
        for item in items {
            output_queue.append_renderer_output_item_for_loader(item, document_loader_id);
        }
    }

    fn subresource_record(
        resource_type: SubresourceResourceType,
        url: &str,
    ) -> SubresourceNetworkRecord {
        let url = Url::parse(url).expect("test URL should parse");
        SubresourceNetworkRecord::success(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
            resource_type,
            None,
            Vec::new(),
            url,
            200,
            Vec::new(),
            String::new(),
            Vec::new(),
        )
    }

    fn websocket_event(socket_id: u64, payload_length: usize) -> WebSocketNetworkEvent {
        WebSocketNetworkEvent::new(
            socket_id,
            Url::parse("https://example.com/").expect("document URL should parse"),
            Url::parse("wss://example.com/socket").expect("websocket URL should parse"),
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Text,
            payload_length,
        )
    }

    fn websocket_record(url: &str, socket_id: u64) -> SubresourceNetworkRecord {
        let url = Url::parse(url).expect("test URL should parse");
        SubresourceNetworkRecord::success(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            url.clone(),
            "GET".to_owned(),
            vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())],
            None,
            SubresourceResourceType::WebSocket,
            None,
            Vec::new(),
            url,
            101,
            vec![("Upgrade".to_owned(), "websocket".to_owned())],
            String::new(),
            Vec::new(),
        )
        .with_websocket_socket_id(socket_id)
    }

    fn failed_websocket_record(
        url: &str,
        socket_id: u64,
        error_text: &str,
    ) -> SubresourceNetworkRecord {
        SubresourceNetworkRecord::failure(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            Url::parse(url).expect("test URL should parse"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::WebSocket,
            error_text.to_owned(),
        )
        .with_websocket_socket_id(socket_id)
    }

    fn subresource_record_with_body(url: &str, body: &str) -> SubresourceNetworkRecord {
        let url = Url::parse(url).expect("test URL should parse");
        SubresourceNetworkRecord::success(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            url,
            200,
            Vec::new(),
            body.to_owned(),
            Vec::new(),
        )
    }

    fn failed_subresource_record(url: &str) -> SubresourceNetworkRecord {
        SubresourceNetworkRecord::failure(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            Url::parse(url).expect("test URL should parse"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "net::ERR_FAILED".to_owned(),
        )
    }

    #[test]
    fn target_network_output_queue_overflow_counters_do_not_panic() {
        let mut queue = TargetNetworkOutputQueue {
            queue_generation: u64::MAX,
            next_delivery_order_index: usize::MAX,
            ..Default::default()
        };

        queue.reset();
        assert_eq!(queue.queue_generation, 0);

        queue.next_delivery_order_index = usize::MAX;
        assert_eq!(queue.next_delivery_order_index(), usize::MAX);
        assert_eq!(queue.next_delivery_order_index, 0);

        assert_eq!(
            super::websocket_event_timestamp_order_index(usize::MAX, 0),
            0
        );
    }

    #[test]
    fn complete_subresource_outputs_skip_duplicate_request_handle() {
        let handle = SubresourceNetworkRequestHandle::new(7);
        let record = subresource_record(SubresourceResourceType::Fetch, "https://example.com/api")
            .with_request_handle(handle);
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
        ];
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");
        assert_eq!(output_queue.subresource_record_count(), 2);

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("deduplicated output should still produce a snapshot");
        let outputs = subresource_outputs(&snapshot);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].request_id(), "REQ-H7");
        assert_eq!(snapshot.subresource_cursor_advances()[0].record_count(), 1);
    }

    #[test]
    fn staged_subresource_outputs_deliver_ordered_events_with_stable_request_id() {
        let handle = SubresourceNetworkRequestHandle::new(7);
        let document_url = Url::parse("https://example.com/").expect("document URL should parse");
        let request_url =
            Url::parse("https://example.com/image.png").expect("request URL should parse");
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME-1".to_owned()),
            document_url,
            request_url.clone(),
            "GET".to_owned(),
            vec![("accept".to_owned(), "image/png".to_owned())],
            None,
            SubresourceResourceType::Image,
            SubresourceRequestInitiatorType::Parser,
            None,
        );
        let response = SubresourceResponseStarted::new(
            handle,
            Vec::new(),
            request_url,
            200,
            vec![("content-type".to_owned(), "image/png".to_owned())],
            Vec::new(),
        )
        .with_from_cache(true);
        let body = SubresourceBodyFinished::ready(
            handle,
            SubresourceResponseBody::from_bytes(vec![1, 2, 3]),
        );
        let items = vec![
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
            ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
            ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(body)),
        ];
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");
        assert_eq!(output_queue.subresource_record_count(), 3);

        let staged_request = output_queue
            .staged_subresource_requests
            .get(&handle)
            .expect("request should remain staged");
        let staged_response = output_queue
            .staged_subresource_responses
            .get(&handle)
            .expect("response should remain staged");
        let request_output = match output_queue.delivery_outputs.outputs[0].output() {
            TargetNetworkDeliveryOutput::Subresource(output) => match output.as_ref() {
                TargetSubresourcePlanOutput::RequestStarted(output) => output,
                _ => panic!("first output should be request-started"),
            },
            TargetNetworkDeliveryOutput::WebSocket(_) => {
                panic!("first output should be a subresource")
            }
        };
        let response_output = match output_queue.delivery_outputs.outputs[1].output() {
            TargetNetworkDeliveryOutput::Subresource(output) => match output.as_ref() {
                TargetSubresourcePlanOutput::ResponseStarted(output) => output,
                _ => panic!("second output should be response-started"),
            },
            TargetNetworkDeliveryOutput::WebSocket(_) => {
                panic!("second output should be a subresource")
            }
        };
        let body_output = match output_queue.delivery_outputs.outputs[2].output() {
            TargetNetworkDeliveryOutput::Subresource(output) => match output.as_ref() {
                TargetSubresourcePlanOutput::BodyFinished(output) => output,
                _ => panic!("third output should be body-finished"),
            },
            TargetNetworkDeliveryOutput::WebSocket(_) => {
                panic!("third output should be a subresource")
            }
        };
        assert!(Arc::ptr_eq(staged_request, request_output));
        assert!(Arc::ptr_eq(staged_request, &response_output.request));
        assert!(Arc::ptr_eq(staged_request, &body_output.request));
        assert!(Arc::ptr_eq(staged_response, response_output));
        assert!(
            body_output
                .response
                .as_ref()
                .is_some_and(|response| Arc::ptr_eq(staged_response, response))
        );

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("staged output should produce a backlog snapshot");
        let outputs = subresource_outputs(&snapshot);
        assert_eq!(outputs.len(), 3);
        assert!(matches!(
            outputs[0],
            TargetSubresourceNetworkDeliveryOutput::RequestStarted(_)
        ));
        assert!(matches!(
            outputs[1],
            TargetSubresourceNetworkDeliveryOutput::ResponseStarted(_)
        ));
        let TargetSubresourceNetworkDeliveryOutput::ResponseStarted(response) = &outputs[1] else {
            unreachable!("checked response-started variant above");
        };
        assert!(response.output().is_from_cache());
        assert!(matches!(
            outputs[2],
            TargetSubresourceNetworkDeliveryOutput::BodyFinished(_)
        ));
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.request_id())
                .collect::<Vec<_>>(),
            vec!["REQ-H7", "REQ-H7", "REQ-H7"]
        );
        assert_eq!(snapshot.subresource_cursor_advances()[0].record_count(), 3);

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 1),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("cursor should be able to resume from staged response");
        let outputs = subresource_outputs(&snapshot);
        assert_eq!(outputs.len(), 2);
        assert!(matches!(
            outputs[0],
            TargetSubresourceNetworkDeliveryOutput::ResponseStarted(_)
        ));
        assert!(matches!(
            outputs[1],
            TargetSubresourceNetworkDeliveryOutput::BodyFinished(_)
        ));
    }

    #[test]
    fn staged_request_followed_by_complete_record_delivers_response_and_body_only() {
        let handle = SubresourceNetworkRequestHandle::new(7);
        let document_url = Url::parse("https://example.com/").expect("document URL should parse");
        let request_url = Url::parse("https://example.com/api").expect("request URL should parse");
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME-1".to_owned()),
            document_url,
            request_url.clone(),
            "PATCH".to_owned(),
            vec![("x-test".to_owned(), "yes".to_owned())],
            Some("payload".to_owned()),
            SubresourceResourceType::Fetch,
            SubresourceRequestInitiatorType::Script,
            None,
        );
        let record = subresource_record(SubresourceResourceType::Fetch, request_url.as_str())
            .with_request_handle(handle);
        let items = vec![
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
        ];
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");
        assert_eq!(output_queue.subresource_record_count(), 3);

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("bridged staged output should produce a backlog snapshot");
        let outputs = subresource_outputs(&snapshot);
        assert_eq!(outputs.len(), 3);
        assert!(matches!(
            outputs[0],
            TargetSubresourceNetworkDeliveryOutput::RequestStarted(_)
        ));
        assert!(matches!(
            outputs[1],
            TargetSubresourceNetworkDeliveryOutput::ResponseStarted(_)
        ));
        assert!(matches!(
            outputs[2],
            TargetSubresourceNetworkDeliveryOutput::BodyFinished(_)
        ));
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.request_id())
                .collect::<Vec<_>>(),
            vec!["REQ-H7", "REQ-H7", "REQ-H7"]
        );
        assert_eq!(snapshot.subresource_cursor_advances()[0].record_count(), 2);
    }

    #[test]
    fn concrete_renderer_items_append_without_cumulative_snapshot_recovery() {
        let handle = SubresourceNetworkRequestHandle::new(17);
        let document_url =
            Url::parse("https://example.com/page").expect("document URL should parse");
        let request_url =
            Url::parse("https://example.com/incremental").expect("request URL should parse");
        let request = ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(
            SubresourceRequestStarted::new(
                handle,
                Some("FRAME-1".to_owned()),
                document_url,
                request_url.clone(),
                "GET".to_owned(),
                Vec::new(),
                None,
                SubresourceResourceType::Xhr,
                SubresourceRequestInitiatorType::Script,
                None,
            ),
        ));
        let completion = ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            subresource_record(SubresourceResourceType::Xhr, request_url.as_str())
                .with_request_handle(handle),
        ));
        let mut output_queue = TargetNetworkOutputQueue::default();

        output_queue.append_renderer_output_item_for_loader(&request, "LOADER-concrete");
        output_queue.append_renderer_output_item_for_loader(&completion, "LOADER-concrete");

        assert_eq!(output_queue.subresource_record_count(), 2);
        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("incremental renderer records should retain their staged request");
        let outputs = subresource_outputs(&snapshot);
        assert!(matches!(
            outputs.as_slice(),
            [
                TargetSubresourceNetworkDeliveryOutput::RequestStarted(_),
                TargetSubresourceNetworkDeliveryOutput::ResponseStarted(_),
                TargetSubresourceNetworkDeliveryOutput::BodyFinished(_)
            ]
        ));
        assert!(
            outputs
                .iter()
                .all(|output| output.request_id() == "REQ-H17")
        );
    }

    #[test]
    fn staged_compact_network_completion_emits_one_transport_extra_info() {
        let handle = SubresourceNetworkRequestHandle::new(8);
        let document_url = Url::parse("https://example.com/").expect("document URL should parse");
        let request_url = Url::parse("https://example.com/api").expect("request URL should parse");
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME-1".to_owned()),
            document_url,
            request_url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            SubresourceRequestInitiatorType::Script,
            None,
        );
        let record = subresource_record(SubresourceResourceType::Fetch, request_url.as_str())
            .with_request_handle(handle)
            .with_network_request_headers(Some(vec![(
                "User-Agent".to_owned(),
                "Moli/Test".to_owned(),
            )]));
        let items = vec![
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
        ];
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("network completion should produce a backlog snapshot");
        let outputs = subresource_outputs(&snapshot);
        assert_eq!(outputs.len(), 4);
        let extras = outputs
            .iter()
            .filter_map(|output| match output {
                TargetSubresourceNetworkDeliveryOutput::RequestExtraInfo(output) => Some(output),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(extras.len(), 1);
        assert_eq!(
            extras[0].output().request_headers(),
            &[("User-Agent".to_owned(), "Moli/Test".to_owned())]
        );
        assert!(matches!(
            outputs[2],
            TargetSubresourceNetworkDeliveryOutput::ResponseStarted(_)
        ));
    }

    #[test]
    fn staged_request_followed_by_redirect_complete_record_preserves_redirect_chain() {
        let handle = SubresourceNetworkRequestHandle::new(7);
        let document_url = Url::parse("https://example.com/").expect("document URL should parse");
        let start_url =
            Url::parse("https://example.com/api-start").expect("start URL should parse");
        let final_url =
            Url::parse("https://other.example/api-final").expect("final URL should parse");
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME-1".to_owned()),
            document_url.clone(),
            start_url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            SubresourceRequestInitiatorType::Script,
            None,
        );
        let record = SubresourceNetworkRecord::success(
            Some("FRAME-1".to_owned()),
            document_url,
            start_url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            Some(StoredCookieQueryReport::default()),
            vec![NavigationRedirect {
                from_url: start_url.clone(),
                to_url: final_url.clone(),
                status: 307,
                headers: vec![("location".to_owned(), final_url.to_string())],
                network_extra_info_available: false,
                request_extra_info: None,
                response_extra_info: None,
                redirect_has_extra_info: false,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                from_cache: true,
                negotiated_http_version: None,
            }],
            final_url.clone(),
            200,
            Vec::new(),
            String::new(),
            Vec::new(),
        )
        .with_request_handle(handle)
        .with_from_cache(true);
        let items = vec![
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
        ];
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("redirect completion should produce staged output");
        let outputs = subresource_outputs(&snapshot);
        assert_eq!(outputs.len(), 4);
        assert!(matches!(
            outputs[0],
            TargetSubresourceNetworkDeliveryOutput::RequestStarted(_)
        ));
        assert!(matches!(
            outputs[1],
            TargetSubresourceNetworkDeliveryOutput::RequestExtraInfo(_)
        ));
        let TargetSubresourceNetworkDeliveryOutput::ResponseStarted(response) = outputs[2] else {
            panic!("redirect completion should deliver response-start output");
        };
        assert_eq!(response.request_id(), "REQ-H7");
        assert_eq!(response.output().redirect_chain().len(), 1);
        assert_eq!(response.output().redirect_chain()[0].from_url, start_url);
        assert_eq!(response.output().redirect_chain()[0].to_url, final_url);
        assert!(response.output().redirect_chain()[0].from_cache);
        assert!(
            response.output().is_from_cache(),
            "staged response synthesized from a compact completion record should preserve cache provenance"
        );
        assert!(matches!(
            outputs[3],
            TargetSubresourceNetworkDeliveryOutput::BodyFinished(_)
        ));
    }

    #[test]
    fn complete_subresource_record_delivery_preserves_cache_state() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let record = subresource_record(
            SubresourceResourceType::Script,
            "https://example.com/app.js",
        )
        .with_from_cache(true);
        append_concrete_items_for_test(
            &mut output_queue,
            &[ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                record,
            ))],
            "LOADER-1",
        );

        let metadata = output_queue
            .items()
            .into_iter()
            .next()
            .expect("complete compact record should produce metadata output");
        assert!(
            metadata.is_from_cache(),
            "metadata output should retain the compact record cache provenance"
        );

        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = StableSubresourceHandleRequestIds::default();
        let snapshot = pending_delivery_snapshot(&output_queue, activity, None, &mut request_ids)
            .expect("complete compact record should produce a backlog snapshot");
        let outputs = subresource_outputs(&snapshot);
        let TargetSubresourceNetworkDeliveryOutput::Complete(output) = outputs[0] else {
            panic!("complete compact record should stay a complete delivery output");
        };
        assert!(
            output.metadata().is_from_cache(),
            "delivery output should keep cache provenance for CDP/BiDi responseStarted synthesis"
        );
    }

    fn expected_subresource_metadata(
        index: usize,
        resource_type: SubresourceResourceType,
        url: &str,
        request_headers: Vec<(String, String)>,
        status: u16,
        response_headers: Vec<(String, String)>,
    ) -> TargetSubresourceMetadataOutput {
        expected_subresource_metadata_with_delivery_order(
            index,
            index,
            resource_type,
            url,
            request_headers,
            status,
            response_headers,
        )
    }

    fn expected_subresource_metadata_with_delivery_order(
        index: usize,
        delivery_order_index: usize,
        resource_type: SubresourceResourceType,
        url: &str,
        request_headers: Vec<(String, String)>,
        status: u16,
        response_headers: Vec<(String, String)>,
    ) -> TargetSubresourceMetadataOutput {
        let url = Url::parse(url).expect("test URL should parse");
        TargetSubresourceMetadataOutput {
            delivery_order_index,
            index,
            loader_id: "LOADER-1".to_owned(),
            response_body: Some(SubresourceResponseBody::from_bytes(Vec::new())),
            request_handle: None,
            websocket_socket_id: (resource_type == SubresourceResourceType::WebSocket).then_some(7),
            frame_id: None,
            document_url: Url::parse("https://example.com/").expect("document URL should parse"),
            url: url.clone(),
            method: "GET".to_owned(),
            request_headers,
            request_body: None,
            request_body_bytes: None,
            resource_type,
            request_initiator_type: SubresourceRequestInitiatorType::Script,
            request_cookie_report: None,
            outcome: TargetSubresourceMetadataOutcome::Success {
                redirect_chain: Vec::new(),
                final_url: url,
                status,
                status_text: None,
                response_headers,
                response_body_len: 0,
            },
            cookie_set_reports: Vec::new(),
            from_cache: false,
            network_request_headers: None,
            negotiated_http_version: None,
        }
    }

    #[test]
    fn target_network_output_queue_snapshot_items_preserve_producer_order() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let script_record = subresource_record(
            SubresourceResourceType::Script,
            "https://example.com/app.js",
        );
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let fetch_record =
            subresource_record(SubresourceResourceType::Fetch, "https://example.com/api");
        let websocket_event = websocket_event(7, 4);
        let all_items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(script_record)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record)),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(fetch_record)),
        ];

        append_concrete_items_for_test(&mut output_queue, &all_items, "LOADER-1");

        assert_eq!(output_queue.subresource_record_count, 3);
        assert_eq!(output_queue.websocket_event_count, 1);
        assert_eq!(
            output_queue.items().to_vec(),
            vec![
                expected_subresource_metadata(
                    0,
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                    Vec::new(),
                    200,
                    Vec::new(),
                ),
                expected_subresource_metadata(
                    1,
                    SubresourceResourceType::WebSocket,
                    "wss://example.com/socket",
                    vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())],
                    101,
                    vec![("Upgrade".to_owned(), "websocket".to_owned())],
                ),
                expected_subresource_metadata_with_delivery_order(
                    2,
                    3,
                    SubresourceResourceType::Fetch,
                    "https://example.com/api",
                    Vec::new(),
                    200,
                    Vec::new(),
                ),
            ],
            "producer item ingestion should keep subresource metadata items stable without mixing WebSocket frame delivery outputs"
        );
        assert_eq!(
            output_queue.websocket_frame_outputs_from(0),
            vec![TargetWebSocketFrameOutput {
                delivery_order_index: 2,
                index: 0,
                timestamp_order_index: 3,
                socket_id: 7,
                direction: WebSocketFrameDirection::Received,
                opcode: WebSocketFrameOpcode::Text,
                payload_length: 4,
            }],
            "WebSocket frame payload should be owned by the WebSocket delivery output queue"
        );
    }

    #[test]
    fn target_network_output_queue_preserves_concrete_producer_item_order() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let script_record = subresource_record(
            SubresourceResourceType::Script,
            "https://example.com/app.js",
        );
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let fetch_record =
            subresource_record(SubresourceResourceType::Fetch, "https://example.com/api");
        let websocket_event = websocket_event(7, 4);
        let events = [websocket_event];
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(script_record)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record)),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(events[0].clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(fetch_record)),
        ];

        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");

        assert_eq!(output_queue.subresource_record_count, 3);
        assert_eq!(output_queue.websocket_event_count, 1);
        assert_eq!(
            output_queue.items().to_vec(),
            vec![
                expected_subresource_metadata(
                    0,
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                    Vec::new(),
                    200,
                    Vec::new(),
                ),
                expected_subresource_metadata(
                    1,
                    SubresourceResourceType::WebSocket,
                    "wss://example.com/socket",
                    vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())],
                    101,
                    vec![("Upgrade".to_owned(), "websocket".to_owned())],
                ),
                expected_subresource_metadata_with_delivery_order(
                    2,
                    3,
                    SubresourceResourceType::Fetch,
                    "https://example.com/api",
                    Vec::new(),
                    200,
                    Vec::new(),
                ),
            ],
            "page output append update should preserve script producer order inside the queue"
        );
        assert_eq!(
            output_queue.websocket_frame_outputs_from(0),
            vec![TargetWebSocketFrameOutput {
                delivery_order_index: 2,
                index: 0,
                timestamp_order_index: 3,
                socket_id: 7,
                direction: WebSocketFrameDirection::Received,
                opcode: WebSocketFrameOpcode::Text,
                payload_length: 4,
            }]
        );
    }

    #[test]
    fn target_network_output_queue_processes_concrete_lifecycle_items_once() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let lifecycle = WebSocketLifecycleEvent::open(
            7,
            Url::parse("https://example.com/").expect("document URL should parse"),
            Url::parse("wss://example.com/socket").expect("websocket URL should parse"),
        );
        let websocket_event = websocket_event(7, 4);

        append_concrete_items_for_test(
            &mut output_queue,
            &[
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                    websocket_record.clone(),
                )),
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event),
            ],
            "LOADER-1",
        );

        assert_eq!(output_queue.subresource_record_count, 1);
        assert_eq!(output_queue.websocket_event_count, 1);
        assert_eq!(
            output_queue.websocket_frame_outputs_from(0),
            vec![TargetWebSocketFrameOutput {
                delivery_order_index: 1,
                index: 0,
                timestamp_order_index: 2,
                socket_id: 7,
                direction: WebSocketFrameDirection::Received,
                opcode: WebSocketFrameOpcode::Text,
                payload_length: 4,
            }],
            "each concrete lifecycle item should be applied once without duplicating Network/WebSocket output"
        );
    }

    #[test]
    fn failed_websocket_lifecycle_waits_for_handshake_and_preserves_terminal_event_order() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let document_url = Url::parse("https://example.com/").expect("document URL should parse");
        let socket_url =
            Url::parse("wss://example.com/socket").expect("websocket URL should parse");
        let error = WebSocketLifecycleEvent::error(
            7,
            document_url.clone(),
            socket_url.clone(),
            "WebSocket handshake failed with HTTP status 404".to_owned(),
        );
        let close =
            WebSocketLifecycleEvent::close(7, document_url, socket_url, 1006, String::new(), false);

        append_concrete_items_for_test(
            &mut output_queue,
            &[ScriptNetworkOutputItem::WebSocketLifecycleEvent(error)],
            "LOADER-1",
        );
        assert_eq!(
            output_queue.websocket_event_count, 0,
            "an error observed before its handshake record must remain pending and unobservable"
        );
        assert_eq!(output_queue.delivery_outputs.websocket_records().count(), 0);

        append_concrete_items_for_test(
            &mut output_queue,
            &[
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                    failed_websocket_record(
                        "wss://example.com/socket",
                        7,
                        "WebSocket handshake failed with HTTP status 404",
                    ),
                )),
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(close),
            ],
            "LOADER-1",
        );

        assert_eq!(output_queue.websocket_event_count, 2);
        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(None, Some(0), Some(0)),
        );
        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("failed WebSocket lifecycle should create protocol backlog");
        let records = websocket_outputs(&snapshot);
        assert_eq!(records.len(), 3);
        let handshake = records[0]
            .as_handshake()
            .expect("handshake must precede terminal lifecycle events");
        assert!(
            handshake.response().is_none(),
            "failed handshake must not synthesize a handshake response event"
        );
        let handshake_request_id = handshake.request_id().to_owned();
        let frame_error = records[1].as_lifecycle().expect("error lifecycle delivery");
        assert_eq!(
            frame_error.kind(),
            &TargetWebSocketLifecycleDeliveryKind::FrameError {
                error_text: "WebSocket handshake failed with HTTP status 404".to_owned(),
            }
        );
        let closed = records[2].as_lifecycle().expect("close lifecycle delivery");
        assert_eq!(closed.kind(), &TargetWebSocketLifecycleDeliveryKind::Closed);
        for record in records {
            let request_id = match record {
                TargetWebSocketDeliveryRecord::Handshake(output) => output.request_id(),
                TargetWebSocketDeliveryRecord::Frame(output) => output.request_id(),
                TargetWebSocketDeliveryRecord::Lifecycle(output) => output.request_id(),
            };
            assert_eq!(
                request_id, handshake_request_id,
                "all events for one WebSocket must share a request id"
            );
        }
    }

    #[test]
    fn target_network_output_queue_tracks_incremental_websocket_records() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let mut records = vec![subresource_record(
            SubresourceResourceType::Script,
            "https://example.com/app.js",
        )];

        apply_network_items_for_test(&mut output_queue, &records, &[]);
        assert_eq!(output_queue.subresource_record_count, 1);
        assert_eq!(output_queue.websocket_event_count, 0);
        assert_eq!(
            output_queue.items().to_vec(),
            vec![expected_subresource_metadata(
                0,
                SubresourceResourceType::Script,
                "https://example.com/app.js",
                Vec::new(),
                200,
                Vec::new(),
            )]
        );

        records.push(websocket_record("wss://example.com/socket", 7));
        apply_network_items_for_test(
            &mut output_queue,
            &records[1..],
            &[websocket_event(7, 4), websocket_event(7, 9)],
        );
        assert_eq!(output_queue.subresource_record_count, 2);
        assert_eq!(output_queue.websocket_event_count, 2);
        assert_eq!(
            output_queue.items().to_vec(),
            vec![
                expected_subresource_metadata(
                    0,
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                    Vec::new(),
                    200,
                    Vec::new(),
                ),
                expected_subresource_metadata(
                    1,
                    SubresourceResourceType::WebSocket,
                    "wss://example.com/socket",
                    vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())],
                    101,
                    vec![("Upgrade".to_owned(), "websocket".to_owned())],
                ),
            ]
        );
        assert_eq!(
            output_queue.websocket_frame_outputs_from(0),
            vec![
                TargetWebSocketFrameOutput {
                    delivery_order_index: 2,
                    index: 0,
                    timestamp_order_index: 3,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 4,
                },
                TargetWebSocketFrameOutput {
                    delivery_order_index: 3,
                    index: 1,
                    timestamp_order_index: 4,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 9,
                },
            ],
            "WebSocket frame payload should be retained in lifecycle state"
        );

        apply_network_items_for_test(&mut output_queue, &[], &[websocket_event(7, 16)]);
        assert_eq!(
            output_queue.items().to_vec(),
            vec![
                expected_subresource_metadata(
                    0,
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                    Vec::new(),
                    200,
                    Vec::new(),
                ),
                expected_subresource_metadata(
                    1,
                    SubresourceResourceType::WebSocket,
                    "wss://example.com/socket",
                    vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())],
                    101,
                    vec![("Upgrade".to_owned(), "websocket".to_owned())],
                ),
            ],
            "syncing only new websocket events should not rescan records or duplicate handshake state"
        );
        assert_eq!(
            output_queue.websocket_frame_outputs_from(0),
            vec![
                TargetWebSocketFrameOutput {
                    delivery_order_index: 2,
                    index: 0,
                    timestamp_order_index: 3,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 4,
                },
                TargetWebSocketFrameOutput {
                    delivery_order_index: 3,
                    index: 1,
                    timestamp_order_index: 4,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 9,
                },
                TargetWebSocketFrameOutput {
                    delivery_order_index: 4,
                    index: 2,
                    timestamp_order_index: 5,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 16,
                },
            ],
            "syncing only new websocket events should append lifecycle frame records"
        );

        let handshake_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(None, 1, 3),
        ])
        .expect("session should create WebSocket activity");
        let mut request_ids = TestBacklogRequestIds;
        let handshake_snapshot = pending_delivery_snapshot(
            &output_queue,
            None,
            Some(handshake_activity),
            &mut request_ids,
        )
        .expect("handshake delivery snapshot should contain pending output");
        let websocket_records = websocket_outputs(&handshake_snapshot);
        assert_eq!(websocket_records.len(), 1);
        let handshake = websocket_records[0]
            .as_handshake()
            .expect("WebSocket delivery output should be handshake");
        assert_eq!(handshake.request_id(), "REQ-7");
        assert_eq!(
            handshake.url().as_str(),
            "wss://example.com/socket",
            "handshake delivery outputs should carry stable payload without rereading page records"
        );

        assert_eq!(
            output_queue.websocket_frame_outputs_from(1),
            vec![
                TargetWebSocketFrameOutput {
                    delivery_order_index: 3,
                    index: 1,
                    timestamp_order_index: 4,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 9,
                },
                TargetWebSocketFrameOutput {
                    delivery_order_index: 4,
                    index: 2,
                    timestamp_order_index: 5,
                    socket_id: 7,
                    direction: WebSocketFrameDirection::Received,
                    opcode: WebSocketFrameOpcode::Text,
                    payload_length: 16,
                },
            ],
            "frame outputs should carry stable payload without rereading page events"
        );

        let replacement_records = vec![subresource_record(
            SubresourceResourceType::Fetch,
            "https://example.com/api",
        )];
        output_queue.reset();
        apply_network_items_for_test(&mut output_queue, &replacement_records, &[]);
        assert_eq!(output_queue.subresource_record_count, 1);
        assert_eq!(
            output_queue.items().to_vec(),
            vec![expected_subresource_metadata(
                0,
                SubresourceResourceType::Fetch,
                "https://example.com/api",
                Vec::new(),
                200,
                Vec::new(),
            )],
            "a replacement Document queue must not retain stale items"
        );
        assert_eq!(output_queue.websocket_event_count, 0);
    }

    #[test]
    fn target_network_output_queue_keeps_body_sources_inside_metadata_items() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = [
            failed_subresource_record("https://example.com/fail"),
            subresource_record_with_body("https://example.com/api", "api-body"),
        ];

        apply_network_items_for_test(&mut output_queue, &records, &[]);

        assert_eq!(output_queue.subresource_record_count, 2);
        let items = output_queue.items();
        assert!(matches!(
            &items[0],
            TargetSubresourceMetadataOutput {
                index: 0,
                response_body: None,
                outcome: TargetSubresourceMetadataOutcome::Failure { .. },
                ..
            }
        ));
        let success_output = &items[1];
        assert!(matches!(
            success_output,
            TargetSubresourceMetadataOutput {
                index: 1,
                response_body: Some(_),
                outcome: TargetSubresourceMetadataOutcome::Success {
                    response_body_len: 8,
                    ..
                },
                ..
            }
        ));
        assert_eq!(
            success_output
                .response_body()
                .expect("success response should retain a body source")
                .diagnostic_bytes()
                .as_ref(),
            b"api-body"
        );
    }

    #[test]
    fn target_network_output_queue_records_websocket_handshake_payload_in_delivery_outputs() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
            websocket_record("wss://example.com/socket", 7),
        ];

        apply_network_items_for_test(&mut output_queue, &records, &[]);

        let handshake_records = output_queue
            .delivery_outputs
            .websocket_records()
            .filter_map(|record| match record {
                TargetWebSocketDeliveryPlanRecord::Handshake(record) => Some(record),
                TargetWebSocketDeliveryPlanRecord::Frame(_)
                | TargetWebSocketDeliveryPlanRecord::Lifecycle(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handshake_records.len(),
            1,
            "only WebSocket subresource records should enter delivery outputs"
        );
        let plan_output = handshake_records[0];
        assert_eq!(plan_output.socket_id, 7);
        assert_eq!(plan_output.handshake.index, 1);
        assert_eq!(
            plan_output.handshake.url.as_str(),
            "wss://example.com/socket"
        );
        assert_eq!(
            plan_output.handshake.request_headers,
            vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())]
        );
        let response = plan_output
            .handshake
            .response
            .as_ref()
            .expect("successful WebSocket handshake should carry response metadata");
        assert_eq!(response.status(), 101);
        assert_eq!(
            response.response_headers(),
            &[("Upgrade".to_owned(), "websocket".to_owned())]
        );
    }

    #[test]
    fn target_network_output_queue_uses_single_websocket_delivery_output_log() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = vec![websocket_event(7, 4), websocket_event(7, 9)];

        apply_network_items_for_test(&mut output_queue, &records, &events);

        assert_eq!(
            output_queue.delivery_outputs.websocket_records().count(),
            3,
            "handshake and frame payloads should share one WebSocket delivery output log"
        );
        assert!(matches!(
            output_queue.delivery_outputs.websocket_record_mut(0).unwrap(),
            TargetWebSocketDeliveryPlanRecord::Handshake(record)
                if record.socket_id == 7 && record.handshake.index == 0
        ));
        assert!(matches!(
            output_queue.delivery_outputs.websocket_record_mut(1).unwrap(),
            TargetWebSocketDeliveryPlanRecord::Frame(output)
                if output.socket_id() == 7 && output.index() == 0 && output.payload_length == 4
        ));
        assert!(matches!(
            output_queue.delivery_outputs.websocket_record_mut(2).unwrap(),
            TargetWebSocketDeliveryPlanRecord::Frame(output)
                if output.socket_id() == 7 && output.index() == 1 && output.payload_length == 9
        ));
    }

    #[test]
    fn target_network_output_queue_maintains_append_time_network_source_items() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = [
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
            websocket_record("wss://example.com/socket-a", 7),
            subresource_record(SubresourceResourceType::Fetch, "https://example.com/api"),
            websocket_record("wss://example.com/socket-b", 8),
        ];
        let events = [websocket_event(7, 4), websocket_event(8, 9)];

        apply_network_items_for_test(&mut output_queue, &records[..2], &events[..1]);
        apply_network_items_for_test(&mut output_queue, &records[2..], &events[1..]);

        assert_eq!(
            output_queue
                .delivery_outputs
                .outputs
                .iter()
                .filter_map(TargetNetworkDeliveryOutputItem::subresource_output)
                .map(TargetSubresourceMetadataOutput::index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "combined delivery output items should track subresource source indexes in append order"
        );
        assert_eq!(
            output_queue
                .delivery_outputs
                .first_output_position_for_activity(Some(2), None, None),
            4,
            "subresource prepare should seek past the combined append-time prefix whose subresource cursor is clean"
        );
        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 2),
        ])
        .expect("test session should create subresource activity");
        let mut request_ids = TestBacklogRequestIds;
        let subresource_snapshot = pending_delivery_snapshot(
            &output_queue,
            Some(subresource_activity),
            None,
            &mut request_ids,
        )
        .expect("partial subresource source scan should include records after the cursor");
        assert_eq!(
            subresource_outputs(&subresource_snapshot)
                .iter()
                .map(|output| output.metadata().url().as_str())
                .collect::<Vec<_>>(),
            vec!["https://example.com/api", "wss://example.com/socket-b"],
            "combined source scan should filter subresource items by source cursor without a visible range token"
        );
        assert_eq!(
            output_queue
                .delivery_outputs
                .websocket_sources()
                .collect::<Vec<_>>(),
            vec![
                TargetWebSocketDeliveryOutputSource::Handshake { record_index: 1 },
                TargetWebSocketDeliveryOutputSource::Frame { event_index: 0 },
                TargetWebSocketDeliveryOutputSource::Handshake { record_index: 3 },
                TargetWebSocketDeliveryOutputSource::Frame { event_index: 1 },
            ],
            "WebSocket source index should track every output in append order"
        );
        assert_eq!(
            output_queue
                .delivery_outputs
                .outputs
                .iter()
                .map(|item| {
                    (
                        item.subresource_record_tail_after_item(),
                        item.websocket_record_tail_after_item(),
                        item.websocket_event_tail_after_item(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (1, 0, 0),
                (2, 0, 0),
                (2, 2, 0),
                (2, 2, 1),
                (3, 2, 1),
                (4, 2, 1),
                (4, 4, 1),
                (4, 4, 2),
            ],
            "combined output items should cache per-family source tails needed to seek dirty cursors"
        );
        assert_eq!(
            output_queue
                .delivery_outputs
                .first_output_position_for_activity(None, Some(3), Some(0)),
            3,
            "WebSocket prepare should seek past combined outputs whose record and event tails are both clean"
        );
        assert_eq!(
            output_queue
                .delivery_outputs
                .first_output_position_for_activity(None, Some(3), Some(1)),
            6,
            "combined WebSocket seek should honor both record and event cursor tails"
        );
        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(None, 3, 0),
        ])
        .expect("test session should create websocket activity");
        let mut request_ids = TestBacklogRequestIds;
        let partial_snapshot = pending_delivery_snapshot(
            &output_queue,
            None,
            Some(websocket_activity),
            &mut request_ids,
        )
        .expect("partial WebSocket source scan should include late handshake and all frames");
        assert_eq!(
            partial_snapshot
                .outputs()
                .iter()
                .map(|item| match item {
                    PendingNetworkBacklogDeliveryItem::Subresource(_) => "subresource",
                    PendingNetworkBacklogDeliveryItem::WebSocket(
                        TargetWebSocketDeliveryRecord::Handshake(_),
                    ) => "handshake",
                    PendingNetworkBacklogDeliveryItem::WebSocket(
                        TargetWebSocketDeliveryRecord::Frame(_),
                    ) => "frame",
                    PendingNetworkBacklogDeliveryItem::WebSocket(
                        TargetWebSocketDeliveryRecord::Lifecycle(_),
                    ) => "lifecycle",
                })
                .collect::<Vec<_>>(),
            vec!["frame", "handshake", "frame"],
            "WebSocket source scan should filter by source cursor without regrouping append order"
        );

        output_queue.reset();
        assert!(output_queue.delivery_outputs.outputs.is_empty());
    }

    #[test]
    fn backlog_outputs_use_subresource_and_websocket_delivery_output_queues() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
            websocket_record("wss://example.com/socket", 7),
        ];
        let events = vec![websocket_event(7, 4)];

        apply_network_items_for_test(&mut output_queue, &records, &events);

        assert!(
            !output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::default())
                .has_output(),
            "backlog presence should stay empty when no Network listener cursor is visible"
        );
        assert!(
            output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::new(
                    Some(0),
                    Some(0),
                    Some(0),
                ))
                .has_output(),
            "queue should project typed pending backlog items as one Network-owned output"
        );
        assert!(
            output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::new(
                    None,
                    Some(0),
                    Some(0),
                ))
                .has_output(),
            "subresource and WebSocket backlog cursors should be independent"
        );
        assert!(
            output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::new(
                    Some(records.len()),
                    Some(records.len()),
                    Some(0),
                ))
                .has_output(),
            "frame backlog should be independently visible after record cursors catch up"
        );
    }

    #[test]
    fn subresource_backlog_outputs_keep_loader_from_ingest_document() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let items = producer_network_items_for_test(
            &[subresource_record(
                SubresourceResourceType::Fetch,
                "https://example.com/api",
            )],
            &[],
        );

        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-DOC");

        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );
        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("subresource backlog should materialize");
        let outputs = subresource_outputs(&snapshot);
        let TargetSubresourceNetworkDeliveryOutput::Complete(output) = outputs[0] else {
            panic!("test record should produce a complete subresource output");
        };

        assert_eq!(output.metadata().loader_id(), "LOADER-DOC");
    }

    #[test]
    fn target_network_output_queue_builds_pending_delivery_snapshots() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
            websocket_record("wss://example.com/socket", 7),
        ];
        let events = vec![websocket_event(7, 4)];
        apply_network_items_for_test(&mut output_queue, &records, &events);

        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(Some("SID-1".to_owned()), 0),
        ])
        .expect("session should create subresource activity");
        let mut request_ids = TestBacklogRequestIds;
        let snapshot = pending_delivery_snapshot(
            &output_queue,
            Some(subresource_activity),
            None,
            &mut request_ids,
        )
        .expect("subresource snapshot should contain pending records");
        assert_eq!(
            snapshot.subresource_session_ids_for_record_index(0),
            vec![Some("SID-1".to_owned())],
            "combined snapshot should carry subresource fanout"
        );
        let subresource_records = subresource_outputs(&snapshot);
        assert_eq!(subresource_records.len(), 2);
        assert_eq!(subresource_records[0].request_id(), "REQ-1");
        assert_eq!(
            subresource_records[1].request_id(),
            "REQ-7",
            "WebSocket subresource delivery should prefer the request id already bound to its socket"
        );
        assert_eq!(
            subresource_records[1].metadata().websocket_socket_id(),
            Some(7)
        );
        assert_eq!(snapshot.subresource_cursor_advances()[0].record_count(), 2);

        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(Some("SID-1".to_owned()), 0, 0),
        ])
        .expect("session should create WebSocket activity");
        let mut request_ids = TestBacklogRequestIds;
        let snapshot = pending_delivery_snapshot(
            &output_queue,
            None,
            Some(websocket_activity),
            &mut request_ids,
        )
        .expect("websocket delivery snapshot should contain pending output");
        assert_eq!(
            snapshot.websocket_session_ids_for_record_index(1),
            vec![Some("SID-1".to_owned())]
        );
        let websocket_records = websocket_outputs(&snapshot);
        assert_eq!(websocket_records.len(), 2);
        let handshake = websocket_records[0]
            .as_handshake()
            .expect("first WebSocket delivery output should be handshake");
        assert_eq!(handshake.request_id(), "REQ-7");
        assert_eq!(handshake.url().as_str(), "wss://example.com/socket");
        assert_eq!(snapshot.websocket_cursor_advances()[0].record_count(), 2);
        assert_eq!(
            snapshot.websocket_session_ids_for_event_index(0),
            vec![Some("SID-1".to_owned())]
        );
        let frame = websocket_records[1]
            .as_frame()
            .expect("second WebSocket delivery output should be frame");
        assert_eq!(frame.request_id(), "REQ-7");
        assert_eq!(frame.payload_length(), 4);
        assert_eq!(snapshot.websocket_cursor_advances()[0].event_count(), 1);
    }

    #[test]
    fn combined_delivery_snapshot_owns_session_fanout_and_cursor_advances() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
            websocket_record("wss://example.com/socket", 7),
            subresource_record(SubresourceResourceType::Fetch, "https://example.com/api"),
        ];
        let events = vec![websocket_event(7, 4)];
        apply_network_items_for_test(&mut output_queue, &records, &events);

        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(Some("SID-1".to_owned()), 1),
        ])
        .expect("subresource session should create activity");
        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(Some("SID-1".to_owned()), 1, 0),
        ])
        .expect("WebSocket session should create activity");
        let mut request_ids = TestBacklogRequestIds;

        let snapshot = pending_delivery_snapshot(
            &output_queue,
            Some(subresource_activity),
            Some(websocket_activity),
            &mut request_ids,
        )
        .expect("combined snapshot should include pending outputs");

        assert_eq!(
            snapshot
                .delivery_entries()
                .map(|(item, session_ids)| match item {
                    PendingNetworkBacklogDeliveryItem::Subresource(_) =>
                        format!("subresource:{}", session_ids.len()),
                    PendingNetworkBacklogDeliveryItem::WebSocket(_) =>
                        format!("websocket:{}", session_ids.len()),
                })
                .collect::<Vec<_>>(),
            vec![
                "subresource:1",
                "websocket:1",
                "subresource:1",
                "websocket:1",
            ],
            "snapshot should retain concrete delivery entries with prepared session fanout"
        );
        assert_eq!(
            snapshot.subresource_session_ids_for_record_index(0),
            Vec::<Option<String>>::new(),
            "records before the session cursor must not fan out"
        );
        assert_eq!(
            snapshot.subresource_session_ids_for_record_index(1),
            vec![Some("SID-1".to_owned())],
            "snapshot should own subresource session fanout"
        );
        assert_eq!(
            snapshot.websocket_session_ids_for_event_index(0),
            vec![Some("SID-1".to_owned())],
            "snapshot should own WebSocket event fanout"
        );

        let subresource_advances = snapshot.subresource_cursor_advances();
        assert_eq!(subresource_advances.len(), 1);
        assert_eq!(subresource_advances[0].session_id(), Some("SID-1"));
        assert_eq!(subresource_advances[0].start_index(), 1);
        assert_eq!(
            subresource_advances[0].record_count(),
            2,
            "subresource cursor should advance from record 1 through the fetch at record 2"
        );

        let websocket_advances = snapshot.websocket_cursor_advances();
        assert_eq!(websocket_advances.len(), 1);
        assert_eq!(websocket_advances[0].session_id(), Some("SID-1"));
        assert_eq!(websocket_advances[0].record_start_index(), 1);
        assert_eq!(websocket_advances[0].record_count(), 1);
        assert_eq!(websocket_advances[0].event_start_index(), 0);
        assert_eq!(websocket_advances[0].event_count(), 1);
    }

    #[test]
    fn prepared_backlog_visible_ranges_are_bounded_by_prepare_time_watermarks() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let initial_records = vec![
            subresource_record_with_body("https://example.com/app.js", "prepared-body"),
            websocket_record("wss://example.com/socket-a", 7),
        ];
        let initial_events = vec![websocket_event(7, 4)];
        apply_network_items_for_test(&mut output_queue, &initial_records, &initial_events);

        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), Some(0), Some(0)),
        );

        let subresource_output = output_queue
            .delivery_outputs
            .subresource_output_mut(0)
            .expect("first subresource output should exist");
        subresource_output.url =
            Url::parse("https://example.com/mutated.js").expect("test URL should parse");
        subresource_output.response_body = Some(SubresourceResponseBody::from_bytes(
            b"mutated-body".to_vec(),
        ));
        let TargetWebSocketDeliveryPlanRecord::Handshake(handshake) = output_queue
            .delivery_outputs
            .websocket_record_mut(0)
            .expect("first WebSocket delivery output should exist")
        else {
            panic!("first WebSocket delivery output should be handshake");
        };
        handshake.handshake.url =
            Url::parse("wss://example.com/mutated-socket").expect("test URL should parse");

        let later_records = [
            initial_records[0].clone(),
            initial_records[1].clone(),
            subresource_record(
                SubresourceResourceType::Fetch,
                "https://example.com/later.png",
            ),
            websocket_record("wss://example.com/socket-b", 8),
        ];
        let later_events = [
            initial_events[0].clone(),
            websocket_event(7, 9),
            websocket_event(8, 11),
        ];
        apply_network_items_for_test(
            &mut output_queue,
            &later_records[initial_records.len()..],
            &later_events[initial_events.len()..],
        );

        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("prepared delivery token should still materialize initial outputs");
        assert_eq!(
            subresource_outputs(&snapshot)
                .iter()
                .map(|output| output.metadata().url().as_str())
                .collect::<Vec<_>>(),
            vec!["https://example.com/app.js", "wss://example.com/socket-a"],
            "prepared subresource token must own prepare-time payload instead of rereading queue slots"
        );
        assert_eq!(
            subresource_outputs(&snapshot)[0]
                .metadata()
                .response_body()
                .expect("prepared success output should own its response body")
                .diagnostic_bytes()
                .as_ref(),
            b"prepared-body",
            "prepared subresource token must own prepare-time body source instead of rereading queue slots"
        );

        assert_eq!(
            websocket_outputs(&snapshot)
                .iter()
                .map(|record| match record {
                    TargetWebSocketDeliveryRecord::Handshake(output) =>
                        format!("handshake:{}", output.url()),
                    TargetWebSocketDeliveryRecord::Frame(output) =>
                        format!("frame:{}", output.payload_length),
                    TargetWebSocketDeliveryRecord::Lifecycle(_) => "lifecycle".to_owned(),
                })
                .collect::<Vec<_>>(),
            vec!["handshake:wss://example.com/socket-a", "frame:4"],
            "prepared WebSocket token must own prepare-time payload instead of rereading queue slots"
        );
        assert_eq!(
            snapshot.subresource_cursor_advances()[0].record_count(),
            2,
            "prepared subresource range should own its prepare-time emitted tail"
        );
        assert_eq!(
            snapshot.websocket_cursor_advances()[0].record_count(),
            2,
            "prepared WebSocket range should own its prepare-time handshake tail"
        );
        assert_eq!(
            snapshot.websocket_cursor_advances()[0].event_count(),
            1,
            "prepared WebSocket range should own its prepare-time frame tail"
        );
    }

    #[test]
    fn prepared_backlog_tokens_are_single_use_slots() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[
                subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                ),
                websocket_record("wss://example.com/socket", 7),
            ],
            &[websocket_event(7, 4)],
        );
        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), Some(0), Some(0)),
        );
        assert!(
            backlog.has_output(),
            "prepared backlog should expose one activity output while retaining typed token items internally"
        );

        assert!(
            output_queue
                .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
                .is_some(),
            "combined delivery token should materialize once"
        );
        assert!(
            !backlog.has_output(),
            "consuming the combined prepared slot should drain all prepared families"
        );
        assert!(
            output_queue
                .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
                .is_none(),
            "combined delivery token should not materialize twice"
        );
    }

    #[test]
    fn prepared_backlog_extend_merges_delivery_token_families() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[
                subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                ),
                websocket_record("wss://example.com/socket", 7),
            ],
            &[websocket_event(7, 4)],
        );

        let mut subresource_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );
        let websocket_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(None, Some(0), Some(0)),
        );
        subresource_backlog.extend(websocket_backlog);

        assert!(
            subresource_backlog.has_output(),
            "separate family preparations should merge into one prepared delivery token"
        );
        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut subresource_backlog)
            .expect("merged delivery token should still materialize once");
        assert_eq!(
            snapshot
                .outputs()
                .into_iter()
                .map(|output| match output {
                    PendingNetworkBacklogDeliveryItem::Subresource(_) => "subresource",
                    PendingNetworkBacklogDeliveryItem::WebSocket(_) => "websocket",
                })
                .collect::<Vec<_>>(),
            vec!["subresource", "subresource", "websocket", "websocket"],
            "merged prepared token should keep the concrete item order owned by the token"
        );
        assert_eq!(subresource_outputs(&snapshot).len(), 2);
        assert_eq!(websocket_outputs(&snapshot).len(), 2);
        assert!(
            !subresource_backlog.has_output(),
            "consuming the merged delivery token should clear all families"
        );
    }

    #[test]
    fn prepared_backlog_visible_ranges_do_not_materialize_after_queue_reset() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[
                subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/old.js",
                ),
                websocket_record("wss://example.com/old-socket", 7),
            ],
            &[websocket_event(7, 4)],
        );
        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), Some(0), Some(0)),
        );

        output_queue.reset();
        apply_network_items_for_test(
            &mut output_queue,
            &[
                subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/new.js",
                ),
                websocket_record("wss://example.com/new-socket", 8),
            ],
            &[websocket_event(8, 9)],
        );

        assert!(
            output_queue
                .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
                .is_none(),
            "a prepared combined token from an old queue generation must not materialize new-page output with matching indexes"
        );
    }

    #[test]
    fn stale_prepared_backlog_token_is_consumed_without_current_queue_recovery() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/old.js",
            )],
            &[],
        );
        let mut stale_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );
        assert!(
            stale_backlog.has_output(),
            "old page should prepare one Network backlog token before reset"
        );

        output_queue.reset();
        apply_network_items_for_test(
            &mut output_queue,
            &[subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/new.js",
            )],
            &[],
        );

        assert!(
            output_queue
                .pending_network_backlog_delivery_snapshot_from_backlog(&mut stale_backlog)
                .is_none(),
            "stale prepared tokens must fail closed instead of recovering from the owner queue generation"
        );
        assert!(
            !stale_backlog.has_output(),
            "a stale prepared token should still be consumed once so it cannot busy-loop"
        );

        let fresh_activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(Some("SID-fresh".to_owned()), 0),
        ])
        .expect("fresh session should create subresource activity");
        let mut request_ids = TestBacklogRequestIds;
        let fresh_snapshot =
            pending_delivery_snapshot(&output_queue, Some(fresh_activity), None, &mut request_ids)
                .expect("fresh durable owner activity should still deliver owner queue output");
        assert_eq!(
            subresource_outputs(&fresh_snapshot)
                .iter()
                .map(|output| output.metadata().url().as_str())
                .collect::<Vec<_>>(),
            vec!["https://example.com/new.js"],
            "stale prepared-token rejection must not poison the owner queue"
        );
    }

    #[test]
    fn prepared_backlog_extend_keeps_fresh_generation_when_stale_token_is_added_later() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/old.js",
            )],
            &[],
        );
        let stale_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );

        output_queue.reset();
        apply_network_items_for_test(
            &mut output_queue,
            &[subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/new.js",
            )],
            &[],
        );
        let mut fresh_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );
        fresh_backlog.extend(stale_backlog);

        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut fresh_backlog)
            .expect("fresh token should survive extending with a stale generation token");
        assert_eq!(
            subresource_outputs(&snapshot)
                .iter()
                .map(|output| output.metadata().url().as_str())
                .collect::<Vec<_>>(),
            vec!["https://example.com/new.js"],
            "extending prepared outputs must not mix old-generation payload with current-generation payload"
        );
    }

    #[test]
    fn prepared_backlog_extend_replaces_stale_generation_when_fresh_token_is_added_later() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/old.js",
            )],
            &[],
        );
        let mut stale_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );

        output_queue.reset();
        apply_network_items_for_test(
            &mut output_queue,
            &[subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/new.js",
            )],
            &[],
        );
        let fresh_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );
        stale_backlog.extend(fresh_backlog);

        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut stale_backlog)
            .expect("fresh token should replace an older generation token during merge");
        assert_eq!(
            subresource_outputs(&snapshot)
                .iter()
                .map(|output| output.metadata().url().as_str())
                .collect::<Vec<_>>(),
            vec!["https://example.com/new.js"],
            "prepared-token merge should not depend on stale/fresh operand order"
        );
    }

    #[test]
    fn prepared_backlog_extend_does_not_duplicate_same_family_token_items() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        apply_network_items_for_test(
            &mut output_queue,
            &[
                subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                ),
                websocket_record("wss://example.com/socket", 7),
            ],
            &[websocket_event(7, 4)],
        );

        let mut first_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), Some(0), Some(0)),
        );
        let duplicate_backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), Some(0), Some(0)),
        );
        first_backlog.extend(duplicate_backlog);

        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut first_backlog)
            .expect("deduplicated prepared token should materialize once");
        assert_eq!(
            snapshot.outputs().len(),
            4,
            "merging the same prepared backlog twice must not duplicate delivery items"
        );
        assert_eq!(
            subresource_outputs(&snapshot).len(),
            2,
            "duplicate subresource token families should be ignored"
        );
        assert_eq!(
            websocket_outputs(&snapshot).len(),
            2,
            "duplicate WebSocket token families should be ignored"
        );
    }

    #[test]
    fn prepared_backlog_token_preserves_cross_family_producer_order() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ))),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record(
                "wss://example.com/socket",
                7,
            ))),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event(7, 4)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(subresource_record(
                SubresourceResourceType::Fetch,
                "https://example.com/api",
            ))),
        ];
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");

        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), Some(0), Some(0)),
        );
        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("prepared token should materialize mixed Network/WebSocket output");

        assert_eq!(
            snapshot
                .outputs()
                .into_iter()
                .map(|output| match output {
                    PendingNetworkBacklogDeliveryItem::Subresource(output) =>
                        format!("subresource:{}", output.metadata().url()),
                    PendingNetworkBacklogDeliveryItem::WebSocket(
                        TargetWebSocketDeliveryRecord::Handshake(output),
                    ) => format!("websocket-handshake:{}", output.url()),
                    PendingNetworkBacklogDeliveryItem::WebSocket(
                        TargetWebSocketDeliveryRecord::Frame(output),
                    ) => format!("websocket-frame:{}", output.payload_length()),
                    PendingNetworkBacklogDeliveryItem::WebSocket(
                        TargetWebSocketDeliveryRecord::Lifecycle(_),
                    ) => "websocket-lifecycle".to_owned(),
                })
                .collect::<Vec<_>>(),
            vec![
                "subresource:https://example.com/app.js",
                "subresource:wss://example.com/socket",
                "websocket-handshake:wss://example.com/socket",
                "websocket-frame:4",
                "subresource:https://example.com/api",
            ],
            "prepared Network backlog must preserve producer append order across subresource and WebSocket families"
        );
    }

    #[test]
    fn websocket_only_prepared_backlog_does_not_materialize_plain_subresources() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(
            &mut output_queue,
            &[
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                ))),
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record(
                    "wss://example.com/socket",
                    7,
                ))),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event(7, 4)),
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(subresource_record(
                    SubresourceResourceType::Fetch,
                    "https://example.com/api",
                ))),
            ],
            "LOADER-1",
        );

        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(None, Some(0), Some(0)),
        );
        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("websocket-only prepared token should materialize websocket output");

        assert!(
            subresource_outputs(&snapshot).is_empty(),
            "a WebSocket-only cursor must not smuggle ordinary subresource records through the prepared token"
        );
        assert_eq!(
            websocket_outputs(&snapshot)
                .iter()
                .map(|output| match output {
                    TargetWebSocketDeliveryRecord::Handshake(output) =>
                        format!("handshake:{}", output.url()),
                    TargetWebSocketDeliveryRecord::Frame(output) =>
                        format!("frame:{}", output.payload_length()),
                    TargetWebSocketDeliveryRecord::Lifecycle(_) => "lifecycle".to_owned(),
                })
                .collect::<Vec<_>>(),
            vec!["handshake:wss://example.com/socket", "frame:4"],
            "the WebSocket family should still carry both handshake and frame output"
        );
    }

    #[test]
    fn websocket_frame_only_prepared_backlog_advances_event_cursor() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        append_concrete_items_for_test(
            &mut output_queue,
            &[
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(subresource_record(
                    SubresourceResourceType::Script,
                    "https://example.com/app.js",
                ))),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event(7, 4)),
            ],
            "LOADER-1",
        );

        let mut backlog = output_queue.backlog_prepared_delivery(
            TargetNetworkBacklogActivityCursor::new(Some(0), None, None),
        );
        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(Some("SID-ws".to_owned()), 0, 0),
        ])
        .expect("session should create WebSocket activity");
        let mut request_ids = TestBacklogRequestIds;
        backlog.extend(output_queue.backlog_prepared_delivery_for_activity(
            None,
            Some(websocket_activity),
            &mut request_ids,
        ));
        let snapshot = output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("merged frame-only prepared backlog should materialize pending output");

        assert_eq!(
            websocket_outputs(&snapshot)
                .iter()
                .map(|output| match output {
                    TargetWebSocketDeliveryRecord::Handshake(_) => "handshake",
                    TargetWebSocketDeliveryRecord::Frame(_) => "frame",
                    TargetWebSocketDeliveryRecord::Lifecycle(_) => "lifecycle",
                })
                .collect::<Vec<_>>(),
            vec!["frame"],
            "test fixture should cover a merged WebSocket token with no handshake record output"
        );
        assert_eq!(
            snapshot
                .websocket_cursor_advances()
                .iter()
                .map(|advance| {
                    (
                        advance.session_id().map(str::to_owned),
                        advance.record_start_index(),
                        advance.record_count(),
                        advance.event_start_index(),
                        advance.event_count(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![(Some("SID-ws".to_owned()), 0, 0, 0, 1)],
            "prepared bounds should advance frame-only WebSocket cursors without rescanning delivery payload at emit time"
        );
    }

    #[test]
    fn target_network_output_queue_uses_append_order_for_websocket_delivery() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record(
                "wss://example.com/socket-a",
                7,
            ))),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event(7, 4)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record(
                "wss://example.com/socket-b",
                8,
            ))),
        ];
        append_concrete_items_for_test(&mut output_queue, &items, "LOADER-1");

        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(None, 0, 0),
        ])
        .expect("session should create WebSocket activity");
        let mut request_ids = TestBacklogRequestIds;
        let snapshot = pending_delivery_snapshot(
            &output_queue,
            None,
            Some(websocket_activity),
            &mut request_ids,
        )
        .expect("websocket delivery snapshot should contain pending output");

        assert_eq!(
            websocket_outputs(&snapshot)
                .iter()
                .map(|output| match output {
                    TargetWebSocketDeliveryRecord::Handshake(output) =>
                        format!("handshake:{}", output.request_id()),
                    TargetWebSocketDeliveryRecord::Frame(output) =>
                        format!("frame:{}", output.request_id()),
                    TargetWebSocketDeliveryRecord::Lifecycle(output) =>
                        format!("lifecycle:{}", output.request_id()),
                })
                .collect::<Vec<_>>(),
            vec!["handshake:REQ-7", "frame:REQ-7", "handshake:REQ-8"],
            "WebSocket delivery should follow append-time lifecycle order instead of regrouping all handshakes before frames"
        );
    }

    #[test]
    fn target_network_output_queue_delegates_websocket_request_id_binding_to_owner() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = vec![websocket_event(7, 4)];
        apply_network_items_for_test(&mut output_queue, &records, &events);

        let bindings = RefCell::new(HashMap::<u64, String>::new());
        let lookup_calls = Cell::new(0);
        let allocation_calls = Cell::new(0);
        struct RecordingBacklogRequestIds<'a> {
            bindings: &'a RefCell<HashMap<u64, String>>,
            lookup_calls: &'a Cell<usize>,
            allocation_calls: &'a Cell<usize>,
        }
        impl TargetNetworkBacklogRequestIdResolver for RecordingBacklogRequestIds<'_> {
            fn request_id_for_subresource_output(
                &mut self,
                output: &TargetSubresourcePlanOutput,
            ) -> String {
                let socket_id = output
                    .websocket_socket_id()
                    .expect("test output should be a websocket subresource");
                self.request_id_for_websocket_socket(socket_id)
            }

            fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
                self.lookup_calls.set(self.lookup_calls.get() + 1);
                self.bindings
                    .borrow_mut()
                    .entry(socket_id)
                    .or_insert_with(|| {
                        self.allocation_calls.set(self.allocation_calls.get() + 1);
                        "REQ-from-websocket".to_owned()
                    })
                    .clone()
            }
        }
        let mut request_ids = RecordingBacklogRequestIds {
            bindings: &bindings,
            lookup_calls: &lookup_calls,
            allocation_calls: &allocation_calls,
        };

        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(None, 0, 0),
        ])
        .expect("session should create WebSocket activity");
        let websocket_snapshot = pending_delivery_snapshot(
            &output_queue,
            None,
            Some(websocket_activity),
            &mut request_ids,
        )
        .expect("websocket delivery snapshot should contain pending output");
        assert_eq!(
            lookup_calls.get(),
            2,
            "queue should delegate both handshake and frame request-id resolution to the owner"
        );
        assert_eq!(
            allocation_calls.get(),
            1,
            "owner should allocate only one request id for one WebSocket socket"
        );
        assert!(
            websocket_outputs(&websocket_snapshot)
                .iter()
                .all(|output| match output {
                    TargetWebSocketDeliveryRecord::Handshake(output) =>
                        output.request_id() == "REQ-from-websocket",
                    TargetWebSocketDeliveryRecord::Frame(output) =>
                        output.request_id() == "REQ-from-websocket",
                    TargetWebSocketDeliveryRecord::Lifecycle(output) =>
                        output.request_id() == "REQ-from-websocket",
                })
        );

        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ])
        .expect("session should create subresource activity");
        let subresource_snapshot = pending_delivery_snapshot(
            &output_queue,
            Some(subresource_activity),
            None,
            &mut request_ids,
        )
        .expect("subresource delivery snapshot should contain pending output");
        assert_eq!(
            subresource_outputs(&subresource_snapshot)[0].request_id(),
            "REQ-from-websocket",
            "subresource delivery should reuse the owner-level WebSocket request id binding"
        );
        assert_eq!(
            lookup_calls.get(),
            3,
            "subresource delivery should also resolve WebSocket request id through the owner"
        );
        assert_eq!(
            allocation_calls.get(),
            1,
            "cross-family request id reuse should come from the owner binding, not queue-local lifecycle cache"
        );

        let late_websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(None, 0, 0),
        ])
        .expect("session should create WebSocket activity");
        let late_snapshot = pending_delivery_snapshot(
            &output_queue,
            None,
            Some(late_websocket_activity),
            &mut request_ids,
        )
        .expect("late websocket delivery snapshot should contain pending output");
        assert!(
            websocket_outputs(&late_snapshot)
                .iter()
                .all(|output| match output {
                    TargetWebSocketDeliveryRecord::Handshake(output) =>
                        output.request_id() == "REQ-from-websocket",
                    TargetWebSocketDeliveryRecord::Frame(output) =>
                        output.request_id() == "REQ-from-websocket",
                    TargetWebSocketDeliveryRecord::Lifecycle(output) =>
                        output.request_id() == "REQ-from-websocket",
                })
        );
        assert_eq!(
            lookup_calls.get(),
            5,
            "late replay should delegate request-id resolution again instead of using queue-local cache"
        );
        assert_eq!(
            allocation_calls.get(),
            1,
            "late replay should still reuse owner binding without allocating a second id"
        );
    }

    #[test]
    fn websocket_handshake_backlog_presence_tracks_sparse_delivery_outputs() {
        let mut output_queue = TargetNetworkOutputQueue::default();
        let records = vec![
            websocket_record("wss://example.com/socket-a", 7),
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
        ];
        apply_network_items_for_test(&mut output_queue, &records, &[]);

        assert!(
            output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::new(
                    None,
                    Some(0),
                    None,
                ))
                .has_output(),
            "a listener cursor before the latest handshake record should see handshake backlog"
        );
        assert!(
            !output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::new(
                    None,
                    Some(1),
                    None,
                ))
                .has_output(),
            "plain subresource records after the last WebSocket handshake must not keep the handshake family dirty"
        );

        let records = vec![
            websocket_record("wss://example.com/socket-a", 7),
            subresource_record(
                SubresourceResourceType::Script,
                "https://example.com/app.js",
            ),
            websocket_record("wss://example.com/socket-b", 8),
        ];
        apply_network_items_for_test(&mut output_queue, &records, &[]);
        assert!(
            output_queue
                .backlog_prepared_delivery(TargetNetworkBacklogActivityCursor::new(
                    None,
                    Some(1),
                    None,
                ))
                .has_output(),
            "a later sparse WebSocket handshake should reopen the handshake family for caught-up listeners"
        );
    }
}

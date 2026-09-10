use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

use moli_core::page::{
    ScriptNetworkOutputItem, SubresourceNetworkOutcome, SubresourceNetworkRequestHandle,
};

use super::network::http_status_text;
use crate::conn::{DevToolsConsoleOutputSessionState, DevToolsLogViolationThreshold};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetNetworkLogEntry {
    url: String,
    text: String,
    timestamp_micros: u64,
    request_handle: Option<SubresourceNetworkRequestHandle>,
}

impl TargetNetworkLogEntry {
    fn http_error(
        url: String,
        status: u16,
        status_text: Option<&str>,
        request_handle: Option<SubresourceNetworkRequestHandle>,
    ) -> Self {
        let status_text = status_text.unwrap_or_else(|| http_status_text(status));
        Self {
            url,
            text: format!(
                "Failed to load resource: the server responded with a status of {status} ({status_text})"
            ),
            timestamp_micros: unix_epoch_micros(),
            request_handle,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn timestamp_millis(&self) -> f64 {
        self.timestamp_micros as f64 / 1_000.0
    }

    pub(crate) fn timestamp_micros(&self) -> u64 {
        self.timestamp_micros
    }

    pub(crate) fn request_handle(&self) -> Option<SubresourceNetworkRequestHandle> {
        self.request_handle
    }
}

fn unix_epoch_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetLogOutputQueueState {
    network_entries: Vec<TargetNetworkLogEntry>,
    logged_request_handles: HashSet<SubresourceNetworkRequestHandle>,
}

impl TargetLogOutputQueueState {
    pub(crate) fn reset(&mut self) {
        // This is a cross-Document retirement boundary, not a reusable
        // scratch buffer. `clear()` would keep the largest Document's Vec and
        // HashSet allocations attached to the target after navigation.
        *self = Self::default();
    }

    pub(crate) fn ingest_renderer_network_output_item(&mut self, item: &ScriptNetworkOutputItem) {
        self.append_network_output_item(item);
    }

    fn append_network_output_item(&mut self, item: &ScriptNetworkOutputItem) {
        match item {
            ScriptNetworkOutputItem::SubresourceResponseStarted(response)
                if response.status() >= 400 =>
            {
                self.push_http_error(
                    response.final_url().as_str().to_owned(),
                    response.status(),
                    response.status_text(),
                    Some(response.handle()),
                );
            }
            ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                let SubresourceNetworkOutcome::Success {
                    final_url,
                    status,
                    status_text,
                    ..
                } = record.outcome()
                else {
                    return;
                };
                if *status >= 400 {
                    self.push_http_error(
                        final_url.as_str().to_owned(),
                        *status,
                        status_text.as_deref(),
                        record.request_handle(),
                    );
                }
            }
            ScriptNetworkOutputItem::SubresourceRequestStarted(_)
            | ScriptNetworkOutputItem::SubresourceResponseStarted(_)
            | ScriptNetworkOutputItem::SubresourceDataReceived(_)
            | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
            | ScriptNetworkOutputItem::SubresourceBodyFinished(_)
            | ScriptNetworkOutputItem::WebSocketNetworkEvent(_)
            | ScriptNetworkOutputItem::WebSocketLifecycleEvent(_) => {}
        }
    }

    fn push_http_error(
        &mut self,
        url: String,
        status: u16,
        status_text: Option<&str>,
        request_handle: Option<SubresourceNetworkRequestHandle>,
    ) {
        if request_handle.is_some_and(|handle| !self.logged_request_handles.insert(handle)) {
            return;
        }
        self.network_entries.push(TargetNetworkLogEntry::http_error(
            url,
            status,
            status_text,
            request_handle,
        ));
    }

    pub(crate) fn network_entries(&self) -> &[TargetNetworkLogEntry] {
        &self.network_entries
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetLogStorageState {
    generation: u64,
    lifecycle_start: usize,
    network_start: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TargetLogOutputCursor {
    generation: u64,
    lifecycle_start: usize,
    network_start: usize,
}

impl TargetLogOutputCursor {
    pub(crate) fn new(generation: u64, lifecycle_start: usize, network_start: usize) -> Self {
        Self {
            generation,
            lifecycle_start,
            network_start,
        }
    }

    pub(crate) fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) fn lifecycle_start(self) -> usize {
        self.lifecycle_start
    }

    pub(crate) fn network_start(self) -> usize {
        self.network_start
    }
}

impl TargetLogStorageState {
    pub(crate) fn reset_for_new_document(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.lifecycle_start = 0;
        self.network_start = 0;
    }

    pub(crate) fn clear_at(&mut self, lifecycle_end: usize, network_end: usize) {
        self.generation = self.generation.wrapping_add(1);
        self.lifecycle_start = lifecycle_end;
        self.network_start = network_end;
    }

    pub(crate) fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) fn lifecycle_start(self) -> usize {
        self.lifecycle_start
    }

    pub(crate) fn network_start(self) -> usize {
        self.network_start
    }

    pub(crate) fn is_empty(self) -> bool {
        self.lifecycle_start == 0 && self.network_start == 0
    }
}

impl DevToolsConsoleOutputSessionState {
    pub(crate) fn reset_log_delivery_for_enable(&mut self, storage: TargetLogStorageState) {
        self.log_output_generation = storage.generation();
        self.log_lifecycle_entries = storage.lifecycle_start();
        self.log_network_entries = storage.network_start();
    }

    pub(crate) fn pending_log_cursor(
        &self,
        storage: TargetLogStorageState,
        lifecycle_end: usize,
        network_end: usize,
    ) -> Option<TargetLogOutputCursor> {
        let (lifecycle_start, network_start) = if self.log_output_generation == storage.generation()
        {
            (
                self.log_lifecycle_entries.max(storage.lifecycle_start()),
                self.log_network_entries.max(storage.network_start()),
            )
        } else {
            (storage.lifecycle_start(), storage.network_start())
        };
        if lifecycle_start > lifecycle_end || network_start > network_end {
            return None;
        }
        (lifecycle_start < lifecycle_end || network_start < network_end).then_some(
            TargetLogOutputCursor::new(storage.generation(), lifecycle_start, network_start),
        )
    }

    pub(crate) fn mark_log_entries_emitted(
        &mut self,
        generation: u64,
        lifecycle_end: usize,
        network_end: usize,
    ) {
        self.log_output_generation = generation;
        self.log_lifecycle_entries = lifecycle_end;
        self.log_network_entries = network_end;
    }

    pub(crate) fn set_log_violation_thresholds(
        &mut self,
        thresholds: Vec<DevToolsLogViolationThreshold>,
    ) {
        self.log_violation_thresholds = thresholds;
    }

    pub(crate) fn clear_log_violation_thresholds(&mut self) {
        self.log_violation_thresholds.clear();
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        ScriptNetworkOutputItem, SubresourceNetworkRequestHandle, SubresourceResponseStarted,
    };
    use url::Url;

    use super::{TargetLogOutputQueueState, TargetLogStorageState};
    use crate::conn::DevToolsConsoleOutputSessionState;

    #[test]
    fn network_log_queue_records_each_http_error_response_once() {
        let response = SubresourceResponseStarted::new(
            SubresourceNetworkRequestHandle::new(7),
            Vec::new(),
            Url::parse("https://example.test/missing").unwrap(),
            404,
            Vec::new(),
            Vec::new(),
        )
        .with_status_text(Some("Not Found".to_owned()));
        let items = [ScriptNetworkOutputItem::SubresourceResponseStarted(
            std::sync::Arc::new(response),
        )];
        let mut queue = TargetLogOutputQueueState::default();

        queue.ingest_renderer_network_output_item(&items[0]);
        queue.ingest_renderer_network_output_item(&items[0]);

        assert_eq!(queue.network_entries().len(), 1);
        let entry = &queue.network_entries()[0];
        assert_eq!(entry.url(), "https://example.test/missing");
        assert_eq!(
            entry.text(),
            "Failed to load resource: the server responded with a status of 404 (Not Found)"
        );
        assert_eq!(
            entry.request_handle(),
            Some(SubresourceNetworkRequestHandle::new(7))
        );
        assert!(entry.timestamp_millis() > 1_000_000_000_000.0);
    }

    #[test]
    fn reset_releases_current_document_network_log_storage() {
        let mut queue = TargetLogOutputQueueState::default();
        for request_id in 1..=64 {
            queue.push_http_error(
                format!("https://example.test/missing-{request_id}"),
                404,
                Some("Not Found"),
                Some(SubresourceNetworkRequestHandle::new(request_id)),
            );
        }
        assert!(queue.network_entries.capacity() >= 64);
        assert!(queue.logged_request_handles.capacity() >= 64);

        queue.reset();

        assert_eq!(queue, TargetLogOutputQueueState::default());
        assert_eq!(queue.network_entries.capacity(), 0);
        assert_eq!(queue.logged_request_handles.capacity(), 0);
    }

    #[test]
    fn log_clear_invalidates_prepared_session_cursor_even_at_same_offsets() {
        let mut storage = TargetLogStorageState::default();
        let mut session = DevToolsConsoleOutputSessionState::default();
        session.reset_log_delivery_for_enable(storage);
        let captured = session
            .pending_log_cursor(storage, 1, 0)
            .expect("one lifecycle entry should be pending");

        storage.clear_at(0, 0);
        let current = session
            .pending_log_cursor(storage, 1, 0)
            .expect("the retained entry is pending in the new generation");

        assert_ne!(captured, current);
        assert_eq!(captured.lifecycle_start(), current.lifecycle_start());
        assert_eq!(captured.network_start(), current.network_start());
        assert_ne!(captured.generation(), current.generation());
    }
}

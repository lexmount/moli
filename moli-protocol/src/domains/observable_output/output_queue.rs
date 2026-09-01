#[cfg(test)]
use moli_core::page::RendererPageDiagnosticsSnapshot;
use moli_core::page::ScriptObservableOutputItem;

use crate::conn::DevToolsConsoleOutputSessionState;
use crate::conn::TargetRuntimeSlot;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, TargetOwnerState, TargetPageAttachmentId,
    TargetPageProtocolAttachmentIdentity,
};
use crate::domains::activity::ProtocolOutputSink;
use crate::domains::audits_output_state::TargetAuditsOutputCursor;
use crate::domains::console_output_state::TargetConsoleOutputDomain;
use crate::domains::log_output_state::{TargetLogOutputCursor, TargetNetworkLogEntry};

#[cfg(test)]
use super::TargetRuntimeObservableQueueSnapshot;
use super::TargetRuntimeObservableSourceOutput;
use super::emission::{
    ObservableOutputProjectionContext, ObservableOutputProjectionStep, SLOT_AUDITS, SLOT_CONSOLE,
    SLOT_LOG, SLOT_RUNTIME_OBSERVABLE, emit_pending_observable_activity_background_events_async,
};
use super::items::{
    ObservableOutputItem, ObservableRuntimePreparedItems, console_domain_items, log_domain_items,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ObservablePreparedOutputs {
    audits: Vec<ObservableSessionAuditsPreparedRange>,
    console: Option<ObservableConsoleLogPreparedRange>,
    log: Vec<ObservableSessionLogPreparedRange>,
    runtime_observable_items: Vec<ObservableSessionRuntimePreparedItems>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ObservablePreparedOutputSlot {
    outputs: ObservablePreparedOutputs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableConsoleLogPreparedRange {
    domain: ObservableConsoleLogDomain,
    url: String,
    page_attachment_id: TargetPageAttachmentId,
    items: Vec<ObservableOutputItem>,
    console_end: usize,
    lifecycle_end: usize,
    network_end: usize,
    log_cursor: Option<TargetLogOutputCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableSessionLogPreparedRange {
    session_id: Option<String>,
    range: ObservableConsoleLogPreparedRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableSessionAuditsPreparedRange {
    session_id: Option<String>,
    source_document: moli_core::RendererDocumentLifecycleIdentity,
    frame_id: String,
    loader_id: String,
    page_attachment_id: TargetPageAttachmentId,
    issues: Vec<moli_core::page::InspectorIssueSnapshot>,
    cursor: TargetAuditsOutputCursor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableSessionRuntimePreparedItems {
    attachment: TargetPageProtocolAttachmentIdentity,
    items: ObservableRuntimePreparedItems,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableConsoleLogEmissionCursor {
    domain: ObservableConsoleLogDomain,
    console_end: usize,
    lifecycle_end: usize,
    network_end: usize,
    log_cursor: Option<TargetLogOutputCursor>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct TargetObservableOutputQueue {
    observable_output_items: Vec<ScriptObservableOutputItem>,
    network_log_entries: Vec<TargetNetworkLogEntry>,
    #[cfg(test)]
    runtime_source_output: Option<TargetRuntimeObservableSourceOutput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) enum ObservableConsoleLogDomain {
    Console,
    Log,
}

impl ObservablePreparedOutputs {
    pub(crate) fn projection_context<'a>(
        step: ObservableOutputProjectionStep,
        session_id: Option<&'a str>,
        prepared_outputs: Option<&'a mut Self>,
    ) -> ObservableOutputProjectionContext<'a> {
        ObservableOutputProjectionContext::new(step, session_id)
            .with_prepared_outputs(prepared_outputs)
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for range in other.audits {
            self.push_audits(range);
        }
        if let Some(range) = other.console {
            self.push_console(range);
        }
        for range in other.log {
            self.push_log(range.session_id.as_deref(), range.range);
        }
        for prepared in other.runtime_observable_items {
            self.push_runtime_observable_items(prepared.attachment, prepared.items);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.audits.is_empty()
            && self.console.is_none()
            && self.log.is_empty()
            && self.runtime_observable_items.is_empty()
    }

    pub(in crate::domains) fn append_to_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.has_audits() {
            sink.push_produced_slot(SLOT_AUDITS);
        }
        if self.has_console() {
            sink.push_produced_slot(SLOT_CONSOLE);
        }
        if self.has_log() {
            sink.push_produced_slot(SLOT_LOG);
        }
        if self.has_runtime_observable() {
            sink.push_produced_slot(SLOT_RUNTIME_OBSERVABLE);
        }
        if !self.is_empty() {
            sink.push_prepared_payload(ObservablePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains::observable_output) fn take_console_range(
        &mut self,
    ) -> Option<ObservableConsoleLogPreparedRange> {
        self.console.take()
    }

    pub(in crate::domains::observable_output) fn take_audits_ranges(
        &mut self,
    ) -> Vec<ObservableSessionAuditsPreparedRange> {
        std::mem::take(&mut self.audits)
    }

    pub(in crate::domains::observable_output) fn take_log_ranges(
        &mut self,
    ) -> Vec<ObservableSessionLogPreparedRange> {
        std::mem::take(&mut self.log)
    }

    #[cfg(test)]
    pub(in crate::domains::observable_output) fn take_log_range(
        &mut self,
    ) -> Option<ObservableConsoleLogPreparedRange> {
        (!self.log.is_empty()).then(|| self.log.remove(0).into_range())
    }

    pub(in crate::domains::observable_output) fn take_console_log_range(
        &mut self,
        domain: ObservableConsoleLogDomain,
        session_id: Option<&str>,
    ) -> Option<ObservableConsoleLogPreparedRange> {
        match domain {
            ObservableConsoleLogDomain::Console => self.take_console_range(),
            ObservableConsoleLogDomain::Log => {
                let index = self
                    .log
                    .iter()
                    .position(|prepared| prepared.session_id() == session_id)?;
                Some(self.log.remove(index).into_range())
            }
        }
    }

    pub(in crate::domains::observable_output) fn take_runtime_observable_items(
        &mut self,
    ) -> Vec<ObservableSessionRuntimePreparedItems> {
        std::mem::take(&mut self.runtime_observable_items)
    }

    pub(in crate::domains::observable_output) fn take_runtime_observable_items_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<ObservableSessionRuntimePreparedItems> {
        let index = self
            .runtime_observable_items
            .iter()
            .position(|prepared| prepared.session_id() == session_id)?;
        Some(self.runtime_observable_items.remove(index))
    }

    pub(in crate::domains::observable_output) fn has_console(&self) -> bool {
        self.console.is_some()
    }

    pub(in crate::domains) fn has_audits(&self) -> bool {
        !self.audits.is_empty()
    }

    pub(in crate::domains::observable_output) fn has_log(&self) -> bool {
        !self.log.is_empty()
    }

    pub(in crate::domains::observable_output) fn has_runtime_observable(&self) -> bool {
        !self.runtime_observable_items.is_empty()
    }

    pub(in crate::domains::observable_output) fn push_console(
        &mut self,
        range: ObservableConsoleLogPreparedRange,
    ) {
        debug_assert_eq!(range.domain(), ObservableConsoleLogDomain::Console);
        debug_assert!(!range.is_empty());
        self.console.get_or_insert(range);
    }

    pub(in crate::domains::observable_output) fn push_audits(
        &mut self,
        range: ObservableSessionAuditsPreparedRange,
    ) {
        if self.audits.iter().any(|prepared| {
            prepared.session_id() == range.session_id()
                && prepared.page_attachment_id == range.page_attachment_id
        }) {
            return;
        }
        self.audits.push(range);
    }

    pub(in crate::domains::observable_output) fn push_log(
        &mut self,
        session_id: Option<&str>,
        range: ObservableConsoleLogPreparedRange,
    ) {
        debug_assert_eq!(range.domain(), ObservableConsoleLogDomain::Log);
        debug_assert!(!range.is_empty());
        if self
            .log
            .iter()
            .any(|prepared| prepared.session_id() == session_id)
        {
            return;
        }
        self.log.push(ObservableSessionLogPreparedRange {
            session_id: session_id.map(str::to_owned),
            range,
        });
    }

    pub(in crate::domains::observable_output) fn push_runtime_observable_items(
        &mut self,
        attachment: TargetPageProtocolAttachmentIdentity,
        items: ObservableRuntimePreparedItems,
    ) {
        if self
            .runtime_observable_items
            .iter()
            .any(|prepared| prepared.attachment == attachment)
        {
            return;
        }
        self.runtime_observable_items
            .push(ObservableSessionRuntimePreparedItems { attachment, items });
    }
}

impl ObservableSessionLogPreparedRange {
    pub(in crate::domains::observable_output) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(in crate::domains::observable_output) fn into_range(
        self,
    ) -> ObservableConsoleLogPreparedRange {
        self.range
    }
}

impl ObservableSessionAuditsPreparedRange {
    pub(in crate::domains::observable_output) fn new(
        session_id: Option<&str>,
        source_document: moli_core::RendererDocumentLifecycleIdentity,
        frame_id: String,
        loader_id: String,
        page_attachment_id: TargetPageAttachmentId,
        issues: Vec<moli_core::page::InspectorIssueSnapshot>,
        cursor: TargetAuditsOutputCursor,
    ) -> Self {
        Self {
            session_id: session_id.map(str::to_owned),
            source_document,
            frame_id,
            loader_id,
            page_attachment_id,
            issues,
            cursor,
        }
    }

    pub(in crate::domains::observable_output) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(in crate::domains::observable_output) fn materialize_for_owner(
        self,
        conn: &CdpConnection,
    ) -> Option<Self> {
        let session_id = self.session_id();
        let runtime_slot = conn.runtime_session_owner_slot(session_id).ok()?;
        let document_binding = runtime_slot.committed_renderer_document_binding()?;
        let owner_state = conn.target_owner_state_for_session(session_id)?;
        let session_state = conn.target_page_session_state_for_session(session_id)?;
        if document_binding.renderer_document_identity() != self.source_document
            || document_binding.page_attachment_id != self.page_attachment_id
            || document_binding.frame_id != self.frame_id
            || document_binding.loader_id != self.loader_id
            || session_state
                .audits
                .pending_cursor(&owner_state.audits_storage_state)
                != Some(self.cursor)
        {
            return None;
        }
        Some(self)
    }

    pub(in crate::domains::observable_output) fn into_emission_parts(
        self,
    ) -> (Vec<ObservableOutputItem>, TargetAuditsOutputCursor) {
        let items = self
            .issues
            .into_iter()
            .map(|issue| ObservableOutputItem::AuditsIssueAdded {
                issue,
                frame_id: self.frame_id.clone(),
                loader_id: self.loader_id.clone(),
            })
            .collect();
        (items, self.cursor)
    }
}

impl ObservableSessionRuntimePreparedItems {
    pub(in crate::domains::observable_output) fn session_id(&self) -> Option<&str> {
        self.attachment.session_id()
    }

    pub(in crate::domains::observable_output) fn materialize_for_owner(
        self,
        conn: &CdpConnection,
    ) -> Option<(Option<String>, ObservableRuntimePreparedItems)> {
        if !conn.target_page_protocol_attachment_identity_is_current(&self.attachment) {
            return None;
        }
        let session_id = self.attachment.session_id().map(str::to_owned);
        let url = conn.runtime_session_owner_target_url(session_id.as_deref())?;
        let runtime_slot = conn
            .runtime_session_owner_slot(session_id.as_deref())
            .ok()?;
        runtime_slot
            .page_attachment_id()
            .is_some_and(|attachment_id| self.items.matches_source_identity(&url, attachment_id))
            .then_some((session_id, self.items))
    }
}

impl ObservablePreparedOutputSlot {
    pub(crate) fn from_outputs(outputs: ObservablePreparedOutputs) -> Self {
        Self { outputs }
    }

    #[cfg(test)]
    pub(crate) fn outputs_mut_for_test(&mut self) -> &mut ObservablePreparedOutputs {
        &mut self.outputs
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    pub(crate) fn projection_context<'a>(
        &'a mut self,
        step: ObservableOutputProjectionStep,
        session_id: Option<&'a str>,
    ) -> ObservableOutputProjectionContext<'a> {
        ObservablePreparedOutputs::projection_context(step, session_id, Some(&mut self.outputs))
    }

    pub(crate) async fn emit_activity_background_events_async(
        &mut self,
        step: ObservableOutputProjectionStep,
        conn: &mut CdpConnection,
        out: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
    ) {
        emit_pending_observable_activity_background_events_async(
            conn,
            out,
            self.projection_context(step, session_id),
        )
        .await;
    }
}

impl ObservableConsoleLogPreparedRange {
    pub(in crate::domains::observable_output) fn new(
        domain: ObservableConsoleLogDomain,
        url: String,
        page_attachment_id: TargetPageAttachmentId,
        items: Vec<ObservableOutputItem>,
        console_end: usize,
        lifecycle_end: usize,
        network_end: usize,
        log_cursor: Option<TargetLogOutputCursor>,
    ) -> Self {
        Self {
            domain,
            url,
            page_attachment_id,
            items,
            console_end,
            lifecycle_end,
            network_end,
            log_cursor,
        }
    }

    fn for_domain(
        domain: ObservableConsoleLogDomain,
        url: String,
        page_attachment_id: TargetPageAttachmentId,
        input: ConsoleLogPreparedRangeInput<'_>,
        log_cursor: Option<TargetLogOutputCursor>,
    ) -> Option<Self> {
        let items = build_console_log_items(domain, &url, &input)?;
        Some(Self::new(
            domain,
            url,
            page_attachment_id,
            items,
            input.console_end,
            input.lifecycle_end,
            input.network_end,
            log_cursor,
        ))
    }

    pub(in crate::domains::observable_output) fn domain(&self) -> ObservableConsoleLogDomain {
        self.domain
    }

    pub(in crate::domains::observable_output) fn url(&self) -> &str {
        &self.url
    }

    pub(in crate::domains::observable_output) fn page_attachment_id(
        &self,
    ) -> TargetPageAttachmentId {
        self.page_attachment_id
    }

    pub(in crate::domains::observable_output) fn items(&self) -> &[ObservableOutputItem] {
        &self.items
    }

    #[cfg(test)]
    pub(in crate::domains::observable_output) fn console_end(&self) -> usize {
        self.console_end
    }

    #[cfg(test)]
    pub(in crate::domains::observable_output) fn lifecycle_end(&self) -> usize {
        self.lifecycle_end
    }

    pub(in crate::domains::observable_output) fn into_emission_parts(
        self,
    ) -> (
        Vec<ObservableOutputItem>,
        ObservableConsoleLogEmissionCursor,
    ) {
        let cursor = ObservableConsoleLogEmissionCursor {
            domain: self.domain,
            console_end: self.console_end,
            lifecycle_end: self.lifecycle_end,
            network_end: self.network_end,
            log_cursor: self.log_cursor,
        };
        (self.items, cursor)
    }

    pub(in crate::domains::observable_output) fn materialize_for_owner(
        self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
    ) -> Option<Self> {
        let runtime_slot = conn.runtime_session_owner_slot(session_id).ok()?;
        let url = conn.runtime_session_owner_target_url(session_id)?;
        if !self.domain.enabled_for_session(conn, session_id)?
            || url != self.url()
            || runtime_slot.page_attachment_id() != Some(self.page_attachment_id())
        {
            return None;
        }
        if self.domain == ObservableConsoleLogDomain::Log {
            let captured_cursor = self.log_cursor?;
            let owner_state = conn.target_owner_state_for_session(session_id)?;
            let session_state = conn.target_devtools_session_state_for_session(session_id)?;
            let current_cursor = session_state
                .console_output_session_state
                .pending_log_cursor(
                    owner_state.log_storage_state,
                    self.lifecycle_end,
                    self.network_end,
                );
            if current_cursor != Some(captured_cursor) {
                return None;
            }
        }
        let mut range = self;
        for item in &mut range.items {
            item.materialize_network_request_id(conn, session_id);
        }
        (!range.items().is_empty()).then_some(range)
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl ObservableConsoleLogDomain {
    #[cfg(test)]
    fn projection_step(self) -> ObservableOutputProjectionStep {
        match self {
            Self::Console => ObservableOutputProjectionStep::Console,
            Self::Log => ObservableOutputProjectionStep::Log,
        }
    }

    fn enabled_for_session(self, conn: &CdpConnection, session_id: Option<&str>) -> Option<bool> {
        let devtools_session_state = conn.target_devtools_session_state_for_session(session_id)?;
        match self {
            Self::Console => Some(
                devtools_session_state
                    .console_output_session_state
                    .console_enabled,
            ),
            Self::Log => Some(devtools_session_state.page_session_state.log_enabled),
        }
    }
}

impl ObservableConsoleLogEmissionCursor {
    #[cfg(test)]
    pub(in crate::domains::observable_output) fn projection_step(
        &self,
    ) -> ObservableOutputProjectionStep {
        self.domain.projection_step()
    }

    pub(in crate::domains::observable_output) fn mark_emitted_for_owner(
        self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
    ) {
        match self.domain {
            ObservableConsoleLogDomain::Console => {
                let _ = conn.with_target_owner_state_for_session_mut(session_id, |owner_state| {
                    owner_state.console_output_state.advance_to_current(
                        TargetConsoleOutputDomain::Console,
                        self.console_end,
                        self.lifecycle_end,
                    );
                });
            }
            ObservableConsoleLogDomain::Log => {
                let Some(log_cursor) = self.log_cursor else {
                    return;
                };
                let _ =
                    conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
                        state.console_output_session_state.mark_log_entries_emitted(
                            log_cursor.generation(),
                            self.lifecycle_end,
                            self.network_end,
                        );
                    });
            }
        }
    }
}

impl TargetObservableOutputQueue {
    #[cfg(test)]
    pub(in crate::domains::observable_output) fn for_test(
        observable_output_items: Vec<ScriptObservableOutputItem>,
    ) -> Self {
        Self {
            observable_output_items,
            network_log_entries: Vec::new(),
            runtime_source_output: None,
        }
    }

    #[cfg(test)]
    pub(super) fn from_runtime_source_output(
        runtime_source_output: Option<TargetRuntimeObservableSourceOutput>,
    ) -> Self {
        Self::from_runtime_source_output_ref(runtime_source_output.as_ref())
    }

    pub(super) fn from_runtime_source_output_ref(
        runtime_source_output: Option<&TargetRuntimeObservableSourceOutput>,
    ) -> Self {
        let observable_output_items = runtime_source_output
            .map(TargetRuntimeObservableSourceOutput::observable_output_items)
            .unwrap_or_default();
        Self {
            observable_output_items,
            network_log_entries: Vec::new(),
            #[cfg(test)]
            runtime_source_output: runtime_source_output.cloned(),
        }
    }

    pub(super) fn from_log_storage(runtime_slot: &TargetRuntimeSlot) -> Option<Self> {
        let observable_output_items = runtime_slot
            .observable_output_latest_source_tail()
            .map(|source| source.observable_output_items())
            .unwrap_or_default();
        Some(Self {
            observable_output_items,
            network_log_entries: runtime_slot.network_log_entries()?.to_vec(),
            #[cfg(test)]
            runtime_source_output: None,
        })
    }

    #[cfg(test)]
    fn from_runtime_snapshot(
        snapshot: TargetRuntimeObservableQueueSnapshot,
        network_log_entries: Vec<TargetNetworkLogEntry>,
    ) -> Self {
        Self {
            observable_output_items: snapshot.observable_output_items,
            network_log_entries,
            runtime_source_output: None,
        }
    }

    #[cfg(test)]
    pub(super) fn from_runtime_slot(runtime_slot: &TargetRuntimeSlot) -> Option<Self> {
        let snapshot = runtime_slot.observable_output_queue_snapshot()?;
        let network_log_entries = runtime_slot.network_log_entries()?.to_vec();
        Some(Self::from_runtime_snapshot(snapshot, network_log_entries))
    }

    #[cfg(test)]
    pub(super) fn from_runtime_slot_source_outputs(runtime_slot: &TargetRuntimeSlot) -> Self {
        Self::from_runtime_source_output(runtime_slot.observable_output_latest_source_tail())
    }

    #[cfg(test)]
    pub(in crate::domains::observable_output) fn from_runtime_slot_source_snapshot(
        runtime_slot: &mut TargetRuntimeSlot,
        url: String,
        snapshot: &RendererPageDiagnosticsSnapshot,
    ) -> Self {
        Self::from_runtime_source_output(
            runtime_slot.sync_observable_output_source_from_renderer_snapshot(url, snapshot),
        )
    }

    pub(super) fn console_message_count(&self) -> usize {
        count_console_observable_output_items(&self.observable_output_items)
    }

    pub(super) fn lifecycle_error_count(&self) -> usize {
        count_lifecycle_error_observable_output_items(&self.observable_output_items)
    }

    pub(super) fn network_log_count(&self) -> usize {
        self.network_log_entries.len()
    }

    #[cfg(test)]
    pub(super) fn runtime_source_output(&self) -> Option<TargetRuntimeObservableSourceOutput> {
        self.runtime_source_output.clone()
    }

    #[cfg(test)]
    pub(super) fn runtime_source_prepared_items(
        &self,
        runtime_frontend_enabled: bool,
        include_console_api_messages: bool,
        owner_state: &TargetOwnerState,
    ) -> Option<ObservableRuntimePreparedItems> {
        if !runtime_frontend_enabled {
            return None;
        }
        runtime_source_prepared_items(
            self.runtime_source_output.as_ref()?,
            include_console_api_messages,
            owner_state,
        )
    }

    pub(super) fn console_log_backlog_ranges(
        &self,
        url: &str,
        page_attachment_id: TargetPageAttachmentId,
        console_enabled: bool,
        log_enabled: bool,
        include_console_api_messages: bool,
        owner_state: &TargetOwnerState,
        log_session_state: &DevToolsConsoleOutputSessionState,
        log_event_session_id: Option<&str>,
    ) -> ObservablePreparedOutputs {
        let mut prepared = ObservablePreparedOutputs::default();
        let console_end = self.console_message_count();
        let lifecycle_end = self.lifecycle_error_count();
        let network_end = self.network_log_count();
        if console_enabled {
            self.push_prepared_console_log_range(
                &mut prepared,
                ObservableConsoleLogDomain::Console,
                url,
                page_attachment_id,
                console_end,
                lifecycle_end,
                network_end,
                include_console_api_messages,
                owner_state,
                log_session_state,
                None,
            );
        }
        if log_enabled {
            self.push_prepared_console_log_range(
                &mut prepared,
                ObservableConsoleLogDomain::Log,
                url,
                page_attachment_id,
                console_end,
                lifecycle_end,
                network_end,
                true,
                owner_state,
                log_session_state,
                log_event_session_id,
            );
        }
        prepared
    }

    fn push_prepared_console_log_range(
        &self,
        prepared: &mut ObservablePreparedOutputs,
        domain: ObservableConsoleLogDomain,
        url: &str,
        page_attachment_id: TargetPageAttachmentId,
        console_end: usize,
        lifecycle_end: usize,
        network_end: usize,
        include_console_api_messages: bool,
        owner_state: &TargetOwnerState,
        log_session_state: &DevToolsConsoleOutputSessionState,
        log_event_session_id: Option<&str>,
    ) {
        let (console_start, lifecycle_start, network_start, log_cursor) = match domain {
            ObservableConsoleLogDomain::Console => {
                let Some(cursor) = owner_state.console_output_state.pending_cursor(
                    TargetConsoleOutputDomain::Console,
                    console_end,
                    lifecycle_end,
                ) else {
                    return;
                };
                (
                    cursor.console_start(),
                    cursor.lifecycle_start(),
                    network_end,
                    None,
                )
            }
            ObservableConsoleLogDomain::Log => {
                let Some(cursor) = log_session_state.pending_log_cursor(
                    owner_state.log_storage_state,
                    lifecycle_end,
                    network_end,
                ) else {
                    return;
                };
                (
                    console_end,
                    cursor.lifecycle_start(),
                    cursor.network_start(),
                    Some(cursor),
                )
            }
        };
        let range = ObservableConsoleLogPreparedRange::for_domain(
            domain,
            url.to_owned(),
            page_attachment_id,
            ConsoleLogPreparedRangeInput {
                console_start,
                console_end,
                lifecycle_start,
                lifecycle_end,
                network_start,
                network_end,
                include_console_api_messages,
                observable_output_items: &self.observable_output_items,
                network_log_entries: &self.network_log_entries,
            },
            log_cursor,
        );
        match (domain, range) {
            (ObservableConsoleLogDomain::Console, Some(range)) => prepared.push_console(range),
            (ObservableConsoleLogDomain::Log, Some(range)) => {
                prepared.push_log(log_event_session_id, range)
            }
            (_, None) => {}
        }
    }
}

#[cfg(test)]
fn runtime_source_prepared_items(
    source: &TargetRuntimeObservableSourceOutput,
    include_console_api_messages: bool,
    owner_state: &TargetOwnerState,
) -> Option<ObservableRuntimePreparedItems> {
    source.source_items_prepared_for_state(
        &owner_state.runtime_observable_state,
        include_console_api_messages,
    )
}

struct ConsoleLogPreparedRangeInput<'a> {
    console_start: usize,
    console_end: usize,
    lifecycle_start: usize,
    lifecycle_end: usize,
    network_start: usize,
    network_end: usize,
    include_console_api_messages: bool,
    observable_output_items: &'a [ScriptObservableOutputItem],
    network_log_entries: &'a [TargetNetworkLogEntry],
}

impl ConsoleLogPreparedRangeInput<'_> {
    fn is_valid(&self) -> bool {
        self.console_start <= self.console_end
            && self.lifecycle_start <= self.lifecycle_end
            && self.network_start <= self.network_end
            && self.console_end
                <= count_console_observable_output_items(self.observable_output_items)
            && self.lifecycle_end
                <= count_lifecycle_error_observable_output_items(self.observable_output_items)
            && self.network_end <= self.network_log_entries.len()
    }
}

fn build_console_items(
    url: &str,
    input: &ConsoleLogPreparedRangeInput<'_>,
) -> Option<Vec<ObservableOutputItem>> {
    if !input.is_valid() {
        return None;
    }
    let console_messages = console_observable_output_texts(
        input.observable_output_items,
        input.console_start,
        input.console_end,
    )
    .collect::<Vec<_>>();
    let lifecycle_errors = lifecycle_error_observable_output_texts(
        input.observable_output_items,
        input.lifecycle_start,
        input.lifecycle_end,
    )
    .collect::<Vec<_>>();
    let console_messages = if input.include_console_api_messages {
        console_messages.as_slice()
    } else {
        &[]
    };
    let items = console_domain_items(
        url,
        console_messages.iter().copied(),
        lifecycle_errors.iter().copied(),
    );
    (!items.is_empty()).then_some(items)
}

fn build_console_log_items(
    domain: ObservableConsoleLogDomain,
    url: &str,
    input: &ConsoleLogPreparedRangeInput<'_>,
) -> Option<Vec<ObservableOutputItem>> {
    match domain {
        ObservableConsoleLogDomain::Console => build_console_items(url, input),
        ObservableConsoleLogDomain::Log => build_log_items(url, input),
    }
}

fn build_log_items(
    url: &str,
    input: &ConsoleLogPreparedRangeInput<'_>,
) -> Option<Vec<ObservableOutputItem>> {
    if !input.is_valid() {
        return None;
    }
    let items = log_domain_items(
        url,
        console_observable_output_texts(
            input.observable_output_items,
            input.console_start,
            input.console_end,
        ),
        lifecycle_error_observable_output_texts(
            input.observable_output_items,
            input.lifecycle_start,
            input.lifecycle_end,
        ),
        input
            .network_log_entries
            .iter()
            .skip(input.network_start)
            .take(input.network_end.saturating_sub(input.network_start)),
    );
    (!items.is_empty()).then_some(items)
}

fn console_observable_output_texts(
    observable_output_items: &[ScriptObservableOutputItem],
    start: usize,
    end: usize,
) -> impl Iterator<Item = &str> {
    observable_output_items
        .iter()
        .filter_map(|item| match item {
            ScriptObservableOutputItem::ConsoleMessage(message) => Some(message.as_str()),
            ScriptObservableOutputItem::LifecycleError(_)
            | ScriptObservableOutputItem::InspectorIssue(_) => None,
        })
        .skip(start)
        .take(end.saturating_sub(start))
}

fn lifecycle_error_observable_output_texts(
    observable_output_items: &[ScriptObservableOutputItem],
    start: usize,
    end: usize,
) -> impl Iterator<Item = &str> {
    observable_output_items
        .iter()
        .filter_map(|item| match item {
            ScriptObservableOutputItem::ConsoleMessage(_)
            | ScriptObservableOutputItem::InspectorIssue(_) => None,
            ScriptObservableOutputItem::LifecycleError(error) => Some(error.as_str()),
        })
        .skip(start)
        .take(end.saturating_sub(start))
}

fn count_console_observable_output_items(
    observable_output_items: &[ScriptObservableOutputItem],
) -> usize {
    observable_output_items
        .iter()
        .filter(|item| matches!(item, ScriptObservableOutputItem::ConsoleMessage(_)))
        .count()
}

fn count_lifecycle_error_observable_output_items(
    observable_output_items: &[ScriptObservableOutputItem],
) -> usize {
    observable_output_items
        .iter()
        .filter(|item| matches!(item, ScriptObservableOutputItem::LifecycleError(_)))
        .count()
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        RendererPageDiagnosticsSnapshot, RendererRuntimeObservableSourceItem,
        RendererRuntimeObservableSourceSummary, RuntimeConsoleMessageSnapshot,
        ScriptObservableOutputItem,
    };

    use crate::testing::TestContext;

    use super::{
        super::TargetRuntimeObservableSourceSummary,
        super::items::{ObservableOutputItem, ObservableRuntimePreparedItems},
        ObservableConsoleLogDomain, ObservableConsoleLogPreparedRange, ObservablePreparedOutputs,
        TargetObservableOutputQueue,
    };
    use crate::conn::{
        BrowserContext, TargetPageAttachmentId, TargetPageProtocolAttachmentIdentity,
        TargetPageResidenceIdentity, TargetRuntimeSlot,
    };
    use crate::domains::log_output_state::TargetLogOutputCursor;

    fn page_attachment_id(raw: u64) -> TargetPageAttachmentId {
        TargetPageAttachmentId::from_raw_for_test(raw)
    }

    fn protocol_attachment(
        session_id: Option<&str>,
        raw: u64,
    ) -> TargetPageProtocolAttachmentIdentity {
        TargetPageProtocolAttachmentIdentity::new(
            TargetPageResidenceIdentity::new(
                "BID-observable-output".to_owned(),
                Some("TID-observable-output".to_owned()),
                page_attachment_id(raw),
            ),
            session_id.map(str::to_owned),
        )
    }

    fn renderer_source_snapshot(
        source: RendererRuntimeObservableSourceSummary,
    ) -> RendererPageDiagnosticsSnapshot {
        RendererPageDiagnosticsSnapshot::from_runtime_observable_source(source)
    }

    #[test]
    fn observable_source_queue_captures_runtime_observable_output() {
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_page_attachment_id_for_test(42);
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(5),
                vec![RuntimeConsoleMessageSnapshot {
                    execution_context_id: 5,
                    message: "log: source output".to_owned(),
                    args: Vec::new(),
                    stack: None,
                }],
                vec!["source lifecycle".to_owned()],
            ),
        );
        let queue = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/source-output".to_owned(),
            &source_snapshot,
        );
        let source = queue
            .runtime_source_output()
            .expect("source queue should expose a runtime observable source output");
        assert_eq!(
            queue.observable_output_items,
            vec![
                ScriptObservableOutputItem::ConsoleMessage("log: source output".to_owned()),
                ScriptObservableOutputItem::LifecycleError("source lifecycle".to_owned()),
            ],
        );
        assert_eq!(queue.console_message_count(), 1);
        assert_eq!(queue.lifecycle_error_count(), 1);
        assert_eq!(
            source.summary(),
            TargetRuntimeObservableSourceSummary::from_renderer_snapshot(&source_snapshot),
            "observable source queue should capture the RuntimeObservable source output through the runtime slot snapshot"
        );
        assert!(
            source.has_source_items(),
            "RuntimeObservable source output should carry concrete source items instead of a diagnostics-only token"
        );
        assert_eq!(
            source.url(),
            "http://example.test/source-output",
            "runtime observable source URL should come from the owner snapshot boundary, not be patched from the current BrowserContext later"
        );
        assert_eq!(
            source.page_attachment_id().get(),
            42,
            "runtime observable Page attachment should come from the owner snapshot boundary, not be recomputed by the observable domain"
        );
    }

    #[test]
    fn observable_source_queue_materializes_runtime_items_from_owner_cursor() {
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_page_attachment_id_for_test(43);
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(5),
                vec![RuntimeConsoleMessageSnapshot {
                    execution_context_id: 5,
                    message: "warn: queue owned runtime".to_owned(),
                    args: Vec::new(),
                    stack: None,
                }],
                vec!["queue owned lifecycle".to_owned()],
            ),
        );
        let queue = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/source-items".to_owned(),
            &source_snapshot,
        );

        let prepared = queue
            .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
            .expect("queue should produce RuntimeObservable prepared items");
        let (items, cursor) = prepared.into_output_emission_parts_for_test();

        assert_eq!(
            items.len(),
            2,
            "runtime source queue should materialize concrete wire items at the queue boundary"
        );
        assert_eq!(cursor.context_console_counts().get(&5), Some(&1));
        assert_eq!(cursor.exception_end(), 1);
        assert!(matches!(
            &items[0],
            ObservableOutputItem::RuntimeConsoleApiCalled {
                console_type,
                text,
                execution_context_id,
                ..
            } if console_type == "warning"
                && text == "queue owned runtime"
                && *execution_context_id == 5
        ));
        assert!(matches!(
            &items[1],
            ObservableOutputItem::RuntimeExceptionThrown {
                text,
                url,
                execution_context_id,
                exception_index,
            } if text == "queue owned lifecycle"
                && url == "http://example.test/source-items"
                && *execution_context_id == 5
                && *exception_index == 0
        ));

        bc.active_page_target_mut()
            .owner_state
            .runtime_observable_state
            .mark_emitted_console_counts(std::collections::HashMap::from([(5, 1)]));
        bc.active_page_target_mut()
            .owner_state
            .runtime_observable_state
            .mark_emitted_exception_entries(1);
        assert!(
            queue
                .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
                .is_none(),
            "owner cursor should suppress source output already emitted from this queue item"
        );
        assert!(
            queue
                .runtime_source_prepared_items(false, true, &bc.active_page_target().owner_state)
                .is_none(),
            "disabled Runtime should not produce RuntimeObservable prepared items"
        );
    }

    #[test]
    fn observable_source_queue_advances_contextless_lifecycle_source_items() {
        let bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_page_attachment_id_for_test(44);
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                None,
                Vec::new(),
                vec!["contextless lifecycle".to_owned()],
            ),
        );
        let queue = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/contextless-lifecycle".to_owned(),
            &source_snapshot,
        );

        let prepared = queue
            .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
            .expect(
                "contextless lifecycle source should still advance the RuntimeObservable cursor",
            );
        let (items, cursor) = prepared.into_output_emission_parts_for_test();

        assert!(
            items.is_empty(),
            "Runtime.exceptionThrown cannot be emitted without an execution context"
        );
        assert_eq!(
            cursor.context_console_counts(),
            &std::collections::HashMap::new()
        );
        assert_eq!(
            cursor.exception_end(),
            1,
            "queue-owned source item should carry enough cursor data to mark the lifecycle source emitted"
        );
    }

    #[test]
    fn observable_source_queue_materializes_renderer_producer_source_items() {
        let bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_page_attachment_id_for_test(46);
        let source = RendererRuntimeObservableSourceSummary::from_source_items(
            Some(7),
            vec![RendererRuntimeObservableSourceItem::LifecycleError {
                text: "renderer item context".to_owned(),
                execution_context_id: Some(99),
                exception_index: 0,
            }],
        );
        let source_snapshot = renderer_source_snapshot(source);
        let queue = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/renderer-source-item".to_owned(),
            &source_snapshot,
        );

        let prepared = queue
            .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
            .expect("renderer producer source item should materialize RuntimeObservable output");
        let (items, cursor) = prepared.into_output_emission_parts_for_test();

        assert_eq!(
            cursor.context_console_counts(),
            &std::collections::HashMap::from([(7, 0)])
        );
        assert_eq!(cursor.exception_end(), 1);
        assert!(
            matches!(
                items.as_slice(),
                [ObservableOutputItem::RuntimeExceptionThrown {
                    text,
                    execution_context_id: 99,
                    exception_index: 0,
                    ..
                }] if text == "renderer item context"
            ),
            "CDP source queue should consume renderer producer source item context instead of recomputing it from the summary"
        );
    }

    #[test]
    fn observable_source_queue_materializes_latest_appended_runtime_source_item() {
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_page_attachment_id_for_test(47);
        let first_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(5),
                vec![RuntimeConsoleMessageSnapshot {
                    execution_context_id: 5,
                    message: "log: first queue source".to_owned(),
                    args: Vec::new(),
                    stack: None,
                }],
                Vec::new(),
            ),
        );
        let second_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(5),
                vec![
                    RuntimeConsoleMessageSnapshot {
                        execution_context_id: 5,
                        message: "log: first queue source".to_owned(),
                        args: Vec::new(),
                        stack: None,
                    },
                    RuntimeConsoleMessageSnapshot {
                        execution_context_id: 5,
                        message: "log: second queue source".to_owned(),
                        args: Vec::new(),
                        stack: None,
                    },
                ],
                Vec::new(),
            ),
        );

        let _ = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/source-items".to_owned(),
            &first_snapshot,
        );
        let queue = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/source-items".to_owned(),
            &second_snapshot,
        );
        let prepared = queue
            .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
            .expect("combined source deltas should produce the full RuntimeObservable tail");
        let (items, cursor) = prepared.into_output_emission_parts_for_test();

        assert_eq!(cursor.context_console_counts().get(&5), Some(&2));
        assert_eq!(cursor.exception_end(), 0);
        assert!(
            matches!(
                items.as_slice(),
                [
                    ObservableOutputItem::RuntimeConsoleApiCalled { text: first, .. },
                    ObservableOutputItem::RuntimeConsoleApiCalled { text: second, .. },
                ] if first == "first queue source" && second == "second queue source"
            ),
            "source materialization should combine append-time source deltas for an owner cursor before the first delta"
        );

        bc.active_page_target_mut()
            .owner_state
            .runtime_observable_state
            .mark_emitted_console_counts(std::collections::HashMap::from([(5, 1)]));

        let prepared = queue
            .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
            .expect("latest appended source item should produce the new RuntimeObservable tail");
        let (items, cursor) = prepared.into_output_emission_parts_for_test();

        assert_eq!(cursor.context_console_counts().get(&5), Some(&2));
        assert_eq!(cursor.exception_end(), 0);
        assert!(matches!(
            items.as_slice(),
            [ObservableOutputItem::RuntimeConsoleApiCalled { text, .. }] if text == "second queue source"
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observable_source_queue_snapshot_can_be_read_without_resyncing_renderer_snapshot() {
        let mut ctx = TestContext::new();
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("test page should load");
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_loaded_page_for_test(page);
        runtime_slot.set_page_attachment_id_for_test(45);
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(5),
                vec![RuntimeConsoleMessageSnapshot {
                    execution_context_id: 5,
                    message: "log: stored source".to_owned(),
                    args: Vec::new(),
                    stack: None,
                }],
                Vec::new(),
            ),
        );
        let _ = TargetObservableOutputQueue::from_runtime_slot_source_snapshot(
            &mut runtime_slot,
            "http://example.test/stored-source".to_owned(),
            &source_snapshot,
        );

        let queue = TargetObservableOutputQueue::from_runtime_slot_source_outputs(&runtime_slot);
        let prepared = queue
            .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
            .expect("stored queue source should materialize RuntimeObservable items");
        let (items, cursor) = prepared.into_output_emission_parts_for_test();

        assert_eq!(cursor.context_console_counts().get(&5), Some(&1));
        assert_eq!(cursor.exception_end(), 0);
        assert!(matches!(
            items.as_slice(),
            [ObservableOutputItem::RuntimeConsoleApiCalled { text, .. }]
                if text == "stored source"
        ));

        bc.active_page_target_mut()
            .owner_state
            .runtime_observable_state
            .mark_emitted_console_counts(std::collections::HashMap::from([(5, 1)]));
        assert!(
            queue
                .runtime_source_prepared_items(true, true, &bc.active_page_target().owner_state)
                .is_none(),
            "stored source output must still respect owner RuntimeObservable cursor state"
        );
    }

    #[test]
    fn observable_prepared_outputs_keep_first_console_range() {
        let first = ObservableConsoleLogPreparedRange::new(
            ObservableConsoleLogDomain::Console,
            "http://example.test/first".to_owned(),
            page_attachment_id(1),
            vec![ObservableOutputItem::ConsoleMessageAdded {
                source: "console-api".to_owned(),
                level: "warning".to_owned(),
                text: "first".to_owned(),
                url: "http://example.test/first".to_owned(),
            }],
            1,
            0,
            0,
            None,
        );
        let second = ObservableConsoleLogPreparedRange::new(
            ObservableConsoleLogDomain::Console,
            "http://example.test/second".to_owned(),
            page_attachment_id(2),
            vec![ObservableOutputItem::ConsoleMessageAdded {
                source: "console-api".to_owned(),
                level: "warning".to_owned(),
                text: "second".to_owned(),
                url: "http://example.test/second".to_owned(),
            }],
            2,
            0,
            0,
            None,
        );
        let mut prepared = ObservablePreparedOutputs::default();
        prepared.push_console(first.clone());
        prepared.push_console(second);

        assert_eq!(
            prepared.take_console_range(),
            Some(first),
            "prepared Console slot should preserve the first prepared range"
        );
        assert!(
            prepared.take_console_range().is_none(),
            "taking a prepared Console slot should clear it"
        );
    }

    #[test]
    fn observable_prepared_outputs_keep_first_log_range_per_session() {
        let log_range = |text: &str, lifecycle_end: usize| {
            ObservableConsoleLogPreparedRange::new(
                ObservableConsoleLogDomain::Log,
                "http://example.test/log".to_owned(),
                page_attachment_id(1),
                vec![ObservableOutputItem::LogEntryAdded {
                    source: "javascript".to_owned(),
                    level: "error".to_owned(),
                    text: text.to_owned(),
                    url: "http://example.test/log".to_owned(),
                    timestamp_micros: None,
                    network_request_handle: None,
                    network_request_id: None,
                }],
                0,
                lifecycle_end,
                0,
                Some(TargetLogOutputCursor::new(0, 0, 0)),
            )
        };
        let first = log_range("first", 1);
        let duplicate = log_range("duplicate", 2);
        let peer = log_range("peer", 1);
        let mut prepared = ObservablePreparedOutputs::default();

        prepared.push_log(Some("SID-1"), first.clone());
        prepared.push_log(Some("SID-1"), duplicate);
        prepared.push_log(Some("SID-2"), peer.clone());

        let ranges = prepared.take_log_ranges();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].session_id(), Some("SID-1"));
        assert_eq!(ranges[0].clone().into_range(), first);
        assert_eq!(ranges[1].session_id(), Some("SID-2"));
        assert_eq!(ranges[1].clone().into_range(), peer);
    }

    #[test]
    fn observable_prepared_outputs_own_only_prepared_slots() {
        let mut prepared = ObservablePreparedOutputs::default();

        assert!(
            prepared.take_console_range().is_none()
                && prepared.take_log_range().is_none()
                && prepared.take_runtime_observable_items().is_empty(),
            "empty captured outputs should not expose projection payload slots"
        );

        let console_range = ObservableConsoleLogPreparedRange::new(
            ObservableConsoleLogDomain::Console,
            "http://example.test/console".to_owned(),
            page_attachment_id(1),
            vec![ObservableOutputItem::ConsoleMessageAdded {
                source: "console-api".to_owned(),
                level: "warning".to_owned(),
                text: "console".to_owned(),
                url: "http://example.test/console".to_owned(),
            }],
            1,
            0,
            0,
            None,
        );
        let log_range = ObservableConsoleLogPreparedRange::new(
            ObservableConsoleLogDomain::Log,
            "http://example.test/log".to_owned(),
            page_attachment_id(1),
            vec![ObservableOutputItem::LogEntryAdded {
                source: "javascript".to_owned(),
                level: "warning".to_owned(),
                text: "log".to_owned(),
                url: "http://example.test/log".to_owned(),
                timestamp_micros: None,
                network_request_handle: None,
                network_request_id: None,
            }],
            1,
            0,
            0,
            Some(TargetLogOutputCursor::new(0, 0, 0)),
        );
        let runtime_items = ObservableRuntimePreparedItems::for_test(
            vec![ObservableOutputItem::RuntimeConsoleApiCalled {
                console_type: "log".to_owned(),
                text: "runtime".to_owned(),
                args: Vec::new(),
                stack: None,
                execution_context_id: 1,
            }],
            std::collections::HashMap::from([(1, 1)]),
            0,
        );
        prepared.push_console(console_range.clone());
        prepared.push_log(None, log_range.clone());
        prepared.push_runtime_observable_items(
            protocol_attachment(Some("SID-runtime"), 1),
            runtime_items.clone(),
        );

        assert_eq!(
            prepared.take_console_range(),
            Some(console_range),
            "captured Console slot should expose only its projection payload"
        );
        assert_eq!(
            prepared.take_log_range(),
            Some(log_range),
            "captured Log slot should expose only its projection payload"
        );
        assert_eq!(
            prepared
                .take_runtime_observable_items_for_session(Some("SID-runtime"))
                .map(|prepared| prepared.items),
            Some(runtime_items),
            "captured RuntimeObservable slot should expose only concrete projection items"
        );

        assert!(
            prepared.take_console_range().is_none()
                && prepared.take_log_range().is_none()
                && prepared.take_runtime_observable_items().is_empty(),
            "taking prepared slots should clear the payload container"
        );
    }

    #[test]
    fn observable_captured_outputs_build_projection_context() {
        let mut prepared = ObservablePreparedOutputs::default();
        let context = ObservablePreparedOutputs::projection_context(
            super::super::ObservableOutputProjectionStep::Console,
            Some("SID-1"),
            Some(&mut prepared),
        );

        assert_eq!(
            context.step,
            super::super::ObservableOutputProjectionStep::Console
        );
        assert_eq!(context.session_id, Some("SID-1"));
        assert!(
            context.prepared_outputs.is_some(),
            "Observable captured outputs should own Observable projection-context construction"
        );
    }

    #[test]
    fn observable_backlog_queue_reports_console_and_log_ranges_from_owner_cursors() {
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.set_target_url("http://example.test/observable".to_owned());
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        let queue = TargetObservableOutputQueue {
            observable_output_items: vec![
                ScriptObservableOutputItem::ConsoleMessage("warn: observable".to_owned()),
                ScriptObservableOutputItem::LifecycleError("observable failure".to_owned()),
            ],
            network_log_entries: Vec::new(),
            runtime_source_output: None,
        };

        let mut prepared = queue.console_log_backlog_ranges(
            bc.target_url(),
            page_attachment_id(1),
            bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled,
            true,
            &bc.active_page_target().owner_state,
            &bc.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state,
            None,
        );
        let console = prepared
            .take_console_range()
            .expect("console cursor should produce a range");
        let log = prepared
            .take_log_range()
            .expect("log cursor should produce a range");

        assert_eq!(console.domain(), ObservableConsoleLogDomain::Console);
        assert_eq!(log.domain(), ObservableConsoleLogDomain::Log);
        assert_eq!(console.console_end(), 1);
        assert_eq!(console.lifecycle_end(), 1);
        assert_eq!(log.console_end(), 1);
        assert_eq!(log.lifecycle_end(), 1);
        assert!(matches!(
            console.items(),
            [
                ObservableOutputItem::ConsoleMessageAdded {
                    source,
                    text,
                    ..
                },
                ObservableOutputItem::ConsoleMessageAdded {
                    source: error_source,
                    text: error_text,
                    ..
                },
            ] if source == "console-api"
                && text == "observable"
                && error_source == "javascript"
                && error_text == "observable failure"
        ));
        assert!(matches!(
            log.items(),
            [ObservableOutputItem::LogEntryAdded { text, .. }] if text == "observable failure"
        ));
    }

    #[test]
    fn observable_backlog_queue_filters_single_raw_queue_by_kind_cursor() {
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.set_target_url("http://example.test/observable".to_owned());
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.active_page_target_mut()
            .owner_state
            .console_output_state
            .advance_console_domain_to_current(1, 0);
        let queue = TargetObservableOutputQueue {
            observable_output_items: vec![
                ScriptObservableOutputItem::ConsoleMessage("log: old".to_owned()),
                ScriptObservableOutputItem::LifecycleError("boom".to_owned()),
                ScriptObservableOutputItem::ConsoleMessage("warn: new".to_owned()),
            ],
            network_log_entries: Vec::new(),
            runtime_source_output: None,
        };

        let mut prepared = queue.console_log_backlog_ranges(
            bc.target_url(),
            page_attachment_id(1),
            bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled,
            true,
            &bc.active_page_target().owner_state,
            &bc.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state,
            None,
        );
        let console = prepared
            .take_console_range()
            .expect("console cursor should filter a mixed raw queue");

        assert_eq!(console.console_end(), 2);
        assert_eq!(console.lifecycle_end(), 1);
        assert!(matches!(
            console.items(),
            [
                ObservableOutputItem::ConsoleMessageAdded {
                    source,
                    level,
                    text,
                    ..
                },
                ObservableOutputItem::ConsoleMessageAdded {
                    source: error_source,
                    level: error_level,
                    text: error_text,
                    ..
                },
            ]
            if source == "console-api"
                && level == "warning"
                && text == "new"
                && error_source == "javascript"
                && error_level == "error"
                && error_text == "boom"
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observable_queue_snapshot_is_owned_by_target_runtime_slot() {
        let mut ctx = TestContext::new();
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('slot queue')</script>",
            )
            .await
            .expect("test page should load");
        let mut runtime_slot = TargetRuntimeSlot::default();
        runtime_slot.set_loaded_page_for_test(page);

        let snapshot = runtime_slot
            .observable_output_queue_snapshot()
            .expect("runtime slot should expose an observable source snapshot");

        assert_eq!(
            snapshot
                .observable_output_items
                .iter()
                .filter(|item| matches!(item, ScriptObservableOutputItem::ConsoleMessage(_)))
                .count(),
            1,
            "runtime slot DTO should capture loaded-page console output"
        );
        assert_eq!(
            snapshot
                .observable_output_items
                .iter()
                .filter(|item| matches!(item, ScriptObservableOutputItem::LifecycleError(_)))
                .count(),
            0,
            "runtime slot DTO should capture loaded-page lifecycle output"
        );

        let queue = TargetObservableOutputQueue::from_runtime_slot(&runtime_slot)
            .expect("observable queue should be built from the runtime slot snapshot");
        assert_eq!(
            queue.console_message_count(),
            1,
            "observable queue should consume the runtime slot DTO"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observable_queue_snapshot_tracks_runtime_slot_page_replacement() {
        let mut ctx = TestContext::new();
        let first_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('first slot queue')</script>",
            )
            .await
            .expect("first test page should load");
        let second_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('second slot queue')</script>",
            )
            .await
            .expect("second test page should load");
        let mut runtime_slot = TargetRuntimeSlot::default();

        let _ = runtime_slot.replace_loaded_page(Some(first_page));
        assert_eq!(
            runtime_slot
                .observable_output_queue_snapshot()
                .expect("first page should seed observable snapshot")
                .observable_output_items
                .iter()
                .filter_map(|item| match item {
                    ScriptObservableOutputItem::ConsoleMessage(message) => Some(message.as_str()),
                    ScriptObservableOutputItem::LifecycleError(_)
                    | ScriptObservableOutputItem::InspectorIssue(_) => None,
                })
                .collect::<Vec<_>>(),
            ["warn: first slot queue"],
            "runtime slot observable snapshot should track the current loaded page"
        );

        let _ = runtime_slot.replace_loaded_page(Some(second_page));
        assert_eq!(
            runtime_slot
                .observable_output_queue_snapshot()
                .expect("second page should refresh observable snapshot")
                .observable_output_items
                .iter()
                .filter_map(|item| match item {
                    ScriptObservableOutputItem::ConsoleMessage(message) => Some(message.as_str()),
                    ScriptObservableOutputItem::LifecycleError(_)
                    | ScriptObservableOutputItem::InspectorIssue(_) => None,
                })
                .collect::<Vec<_>>(),
            ["warn: second slot queue"],
            "runtime slot observable snapshot should not retain the previous page output"
        );

        let _ = runtime_slot.clear_loaded_page_for_test_fixture();
        assert!(
            runtime_slot.observable_output_queue_snapshot().is_none(),
            "clearing the loaded page should clear the runtime slot observable snapshot"
        );
    }
}

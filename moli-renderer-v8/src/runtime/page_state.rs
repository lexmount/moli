use super::*;

#[derive(Debug, Clone)]
pub struct RendererPageRecord {
    pub requested_url: Url,
    pub final_url: Url,
    pub status: u16,
}

#[derive(Debug, Clone)]
pub struct RendererPageState {
    pub requested_url: Url,
    pub navigation_initiator_url: Option<Url>,
    pub navigation_redirected: bool,
    pub navigation_redirect_count: usize,
    pub final_url: Url,
    pub document_title: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub script_execution: Arc<ScriptExecutionReport>,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub service_worker_client_id: u64,
    pub dedicated_worker_running_worker_isolate_count: usize,
    pub performance_metric_snapshot: RendererPerformanceMetricSnapshot,
}

impl RendererPageState {
    pub(crate) fn from_vm_state_capture(
        mut requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        mut status: u16,
        mut headers: Vec<(String, String)>,
        state_capture: PageVmStateCapture,
    ) -> Arc<Self> {
        if let Some(navigation) = state_capture.navigation_response.as_ref() {
            requested_url = navigation.requested_url.clone();
            status = navigation.status;
            headers = navigation.headers.clone();
        }

        Arc::new(Self {
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            final_url: state_capture.final_url,
            document_title: state_capture.document_title,
            status,
            headers,
            script_execution: state_capture.report,
            idle_override: state_capture.idle_override,
            service_worker_client_id: state_capture.service_worker_client_id,
            dedicated_worker_running_worker_isolate_count: state_capture
                .dedicated_worker_running_worker_isolate_count,
            performance_metric_snapshot: state_capture.performance_metric_snapshot,
        })
    }

    pub fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub fn document_title(&self) -> &str {
        &self.document_title
    }

    pub fn idle_override(&self) -> Option<crate::protocol_types::EmulatedIdleOverride> {
        self.idle_override
    }

    fn to_record(&self) -> RendererPageRecord {
        RendererPageRecord {
            requested_url: self.requested_url.clone(),
            final_url: self.final_url().clone(),
            status: self.status,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RendererPageEntry {
    page_id: PageId,
    vm_creation_id: u64,
    pub(crate) view_generation: u64,
    command_epoch: u64,
    pub(crate) in_flight_command_epoch: Option<u64>,
    pub(super) state: RendererPageEntryState,
}

#[derive(Debug, Clone)]
pub(crate) enum RendererPageEntryState {
    Active(Arc<RendererPageState>),
    // Keep the tombstone so stale commands remain distinguishable from an
    // unknown PageId, but never retain the closed page's report/page state.
    Removed,
}

impl RendererPageEntry {
    pub(crate) fn active(
        page_id: PageId,
        vm_creation_id: u64,
        view_generation: u64,
        command_epoch: u64,
        page_state: Arc<RendererPageState>,
    ) -> Self {
        Self {
            page_id,
            vm_creation_id,
            view_generation,
            command_epoch,
            in_flight_command_epoch: None,
            state: RendererPageEntryState::Active(page_state),
        }
    }

    pub(crate) fn removed(page_id: PageId) -> Self {
        Self {
            page_id,
            vm_creation_id: 0,
            view_generation: 0,
            command_epoch: 0,
            in_flight_command_epoch: None,
            state: RendererPageEntryState::Removed,
        }
    }

    pub(crate) fn active_record(&self) -> Option<RendererPageRecord> {
        match &self.state {
            RendererPageEntryState::Active(page_state) => Some(page_state.to_record()),
            RendererPageEntryState::Removed => None,
        }
    }

    fn into_removed(self) -> Self {
        Self::removed(self.page_id)
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(self.state, RendererPageEntryState::Active(_))
    }

    pub(crate) fn command_epoch(&self) -> u64 {
        self.command_epoch
    }

    pub(crate) fn vm_creation_id(&self) -> u64 {
        self.vm_creation_id
    }

    fn active_page_state(&self) -> Option<Arc<RendererPageState>> {
        match &self.state {
            RendererPageEntryState::Active(page_state) => Some(page_state.clone()),
            RendererPageEntryState::Removed => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RendererPageSlotHandle {
    owner: std::sync::Weak<RendererOwnerState>,
    inner: Arc<Mutex<RendererPageEntry>>,
    page_context_cancel_tx: RendererPageContextCancelSender,
    script_execution_control: crate::script_execution_control::RendererScriptExecutionControl,
}

impl RendererPageSlotHandle {
    pub(super) fn new(
        owner: std::sync::Weak<RendererOwnerState>,
        entry: RendererPageEntry,
        page_context_cancel_tx: RendererPageContextCancelSender,
        script_execution_control: crate::script_execution_control::RendererScriptExecutionControl,
    ) -> Self {
        Self {
            owner,
            inner: Arc::new(Mutex::new(entry)),
            page_context_cancel_tx,
            script_execution_control,
        }
    }

    pub(super) fn entry(&self) -> RendererPageEntry {
        self.inner.lock().clone()
    }

    pub(super) fn active_page_state(&self) -> Result<Arc<RendererPageState>> {
        let entry = self.entry();
        ensure!(
            entry.is_active(),
            "renderer owner no longer tracks active page {}",
            entry.page_id.as_u64()
        );
        entry.active_page_state().ok_or_else(|| {
            anyhow!(
                "renderer owner no longer tracks active page {}",
                entry.page_id.as_u64()
            )
        })
    }

    pub(crate) fn refresh(&self, view: RendererPageView) -> Result<()> {
        let mut entry = self.inner.lock();
        ensure!(
            entry.page_id.as_u64() == view.page_id.as_u64(),
            "renderer owner routed refresh for mismatched page {}",
            view.page_id.as_u64()
        );
        ensure!(
            entry.is_active(),
            "renderer owner no longer tracks active page {} for refresh",
            view.page_id.as_u64()
        );
        ensure!(
            view.view_generation >= entry.view_generation,
            "renderer owner received stale page view refresh for page {}",
            view.page_id.as_u64()
        );
        entry.vm_creation_id = view.vm_creation_id;
        entry.view_generation = view.view_generation;
        entry.state = RendererPageEntryState::Active(view.page_state);
        Ok(())
    }

    pub(crate) fn remove(&self) {
        self.cancel_page_context(RendererPageContextCancelReason::PageClosed);
        let mut entry = self.inner.lock();
        *entry = entry.clone().into_removed();
    }

    pub(crate) fn cancel_page_context(&self, reason: RendererPageContextCancelReason) {
        self.page_context_cancel_tx.cancel(reason);
    }

    pub(crate) fn page_context_cancel_sender(&self) -> RendererPageContextCancelSender {
        self.page_context_cancel_tx.clone()
    }

    pub(crate) fn script_execution_control(
        &self,
    ) -> crate::script_execution_control::RendererScriptExecutionControl {
        self.script_execution_control.clone()
    }

    pub(crate) fn page_id(&self) -> PageId {
        self.entry().page_id
    }

    pub(super) fn same_slot(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn owner_state(&self) -> Result<Arc<RendererOwnerState>> {
        self.owner.upgrade().ok_or_else(|| {
            anyhow!(
                "renderer owner no longer exists for page {}",
                self.page_id().as_u64()
            )
        })
    }

    fn ensure_owned(&self) -> Result<()> {
        let owner = self.owner_state()?;
        ensure!(
            owner.page_table.owns_slot(self),
            "renderer owner does not own slot for page {}",
            self.page_id().as_u64()
        );
        Ok(())
    }

    fn update_command_epoch(&self, page_id: PageId, command_epoch: u64) -> Result<()> {
        let mut entry = self.inner.lock();
        ensure!(
            entry.page_id.as_u64() == page_id.as_u64(),
            "renderer owner routed command epoch update for mismatched page {}",
            page_id.as_u64()
        );
        ensure!(
            entry.is_active(),
            "renderer owner no longer tracks active page {} for command epoch update",
            page_id.as_u64()
        );
        ensure!(
            command_epoch >= entry.command_epoch,
            "renderer owner received stale page command epoch update for page {}",
            page_id.as_u64()
        );
        entry.command_epoch = command_epoch;
        Ok(())
    }

    fn begin_command(&self, page_id: PageId, command_epoch: u64) -> Result<()> {
        let mut entry = self.inner.lock();
        ensure!(
            entry.page_id.as_u64() == page_id.as_u64(),
            "renderer owner routed command begin for mismatched page {}",
            page_id.as_u64()
        );
        ensure!(
            entry.is_active(),
            "renderer owner no longer tracks active page {} for command begin",
            page_id.as_u64()
        );
        ensure!(
            command_epoch >= entry.command_epoch,
            "renderer owner received stale page command epoch begin for page {}",
            page_id.as_u64()
        );
        ensure!(
            entry.in_flight_command_epoch.is_none(),
            "renderer owner already has in-flight page command for page {}",
            page_id.as_u64()
        );
        entry.in_flight_command_epoch = Some(command_epoch);
        Ok(())
    }

    fn finish_command(&self, page_id: PageId, command_epoch: u64) -> Result<()> {
        let mut entry = self.inner.lock();
        ensure!(
            entry.page_id.as_u64() == page_id.as_u64(),
            "renderer owner routed command finish for mismatched page {}",
            page_id.as_u64()
        );
        ensure!(
            entry.is_active(),
            "renderer owner no longer tracks active page {} for command finish",
            page_id.as_u64()
        );
        ensure!(
            entry.in_flight_command_epoch == Some(command_epoch),
            "renderer owner finished mismatched in-flight page command for page {}",
            page_id.as_u64()
        );
        entry.in_flight_command_epoch = None;
        Ok(())
    }

    pub(crate) fn cancel_in_flight_command(&self, page_id: PageId) -> Result<()> {
        let mut entry = self.inner.lock();
        ensure!(
            entry.page_id.as_u64() == page_id.as_u64(),
            "renderer owner routed command cancel for mismatched page {}",
            page_id.as_u64()
        );
        if entry.is_active() {
            entry.in_flight_command_epoch = None;
        }
        Ok(())
    }

    async fn dispatch_async(
        &self,
        command_epoch: u64,
        vm: &mut PageVm,
        command: RendererPageCommand,
    ) -> Result<RendererPageReply> {
        let page_id = self.page_id();
        debug_assert_eq!(page_id.as_u64(), vm.page_id.as_u64());
        let entry = self.entry();
        ensure!(
            entry.is_active(),
            "renderer owner no longer tracks active page {}",
            page_id.as_u64()
        );
        ensure!(
            entry.vm_creation_id == vm.creation_id,
            "renderer owner routed command for stale page vm {}",
            page_id.as_u64()
        );
        ensure!(
            command_epoch >= entry.command_epoch(),
            "renderer owner received stale page command epoch for page {}",
            page_id.as_u64()
        );
        self.begin_command(page_id, command_epoch)?;
        let result = vm.dispatch_renderer_page_command_async(command).await;
        self.finish_command(page_id, command_epoch)?;
        let reply = result?;
        self.update_command_epoch(page_id, command_epoch)?;
        Ok(reply)
    }

    pub(super) fn refresh_owned_view(&self, view: RendererPageView) -> Result<()> {
        self.ensure_owned()?;
        self.refresh(view)
    }

    pub(super) fn remove_from_owner(&self) {
        if self.ensure_owned().is_ok() {
            self.remove();
        }
    }

    pub(super) async fn dispatch_async_owned(
        &self,
        command_epoch: u64,
        vm: &mut PageVm,
        command: RendererPageCommand,
    ) -> Result<RendererPageReply> {
        self.ensure_owned()?;
        self.dispatch_async(command_epoch, vm, command).await
    }
}

use super::BrowserContext;
use crate::conn::state::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;
use moli_core::browser::DocumentHandle;
use moli_core::page::{
    CompletedPageCommand, EmulatedIdleOverride, EmulatedMediaOverrides, PendingPageCommand,
    ViewportSurface,
};

pub(crate) enum DocumentPolicyUpdate {
    NetworkRequestPolicy {
        extra_headers: Vec<(String, String)>,
        bypass_service_worker: bool,
        cache_disabled: bool,
        blocked_url_patterns: Vec<String>,
    },
    ExtraHttpHeaders(Vec<(String, String)>),
    BlockedUrls(Vec<String>),
    BypassServiceWorker(bool),
    LocaleOverride(Option<String>),
    NetworkOffline(bool),
    CpuThrottlingRate(f64),
    IdleOverride(Option<EmulatedIdleOverride>),
    TimezoneOverride(Option<String>),
    EmulatedMedia(EmulatedMediaOverrides),
    ViewportSurface(Option<ViewportSurface>),
    ScriptExecutionDisabled(bool),
    ClearDeviceMetricsSurface,
}

#[derive(Clone, Copy)]
enum DocumentPolicyUpdateKind {
    SetNetworkRequestPolicy,
    SetExtraHttpHeaders,
    SetBlockedUrls,
    SetBypassServiceWorker,
    SetLocaleOverride,
    SetNetworkConditions,
    SetCpuThrottlingRate,
    SetIdleOverride,
    SetTimezoneOverride,
    SetEmulatedMedia,
    SetViewportSurface,
    SetScriptExecutionDisabled,
    PageSurfaceOverride,
}

pub(crate) struct PendingDocumentPolicyUpdate {
    document: DocumentHandle,
    kind: DocumentPolicyUpdateKind,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedDocumentPolicyUpdate {
    document: DocumentHandle,
    kind: DocumentPolicyUpdateKind,
    completed: Result<CompletedPageCommand, String>,
}

pub(crate) struct PendingDocumentPolicyBatch {
    context: moli_core::browser::BrowserContextId,
    document: DocumentHandle,
    admission_error: Option<String>,
    updates: Vec<PendingDocumentPolicyUpdate>,
}

pub(crate) struct CompletedDocumentPolicyBatch {
    context: moli_core::browser::BrowserContextId,
    document: DocumentHandle,
    admission_error: Option<String>,
    updates: Vec<CompletedDocumentPolicyUpdate>,
}

pub(crate) struct DocumentRuntimePolicyReconciliation {
    pub(crate) script_execution_disabled: bool,
    pub(crate) emulated_media: EmulatedMediaOverrides,
    pub(crate) cpu_throttling_rate: f64,
    pub(crate) network_offline: bool,
    pub(crate) viewport_surface: Option<ViewportSurface>,
    pub(crate) clear_device_metrics_surface: bool,
}

impl PendingDocumentPolicyUpdate {
    pub(crate) async fn wait(self) -> CompletedDocumentPolicyUpdate {
        CompletedDocumentPolicyUpdate {
            document: self.document,
            kind: self.kind,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedDocumentPolicyUpdate {
    pub(crate) fn document(&self) -> DocumentHandle {
        self.document
    }
}

impl PendingDocumentPolicyBatch {
    pub(crate) async fn wait(self) -> CompletedDocumentPolicyBatch {
        let mut updates = Vec::with_capacity(self.updates.len());
        for update in self.updates {
            updates.push(update.wait().await);
        }
        CompletedDocumentPolicyBatch {
            context: self.context,
            document: self.document,
            admission_error: self.admission_error,
            updates,
        }
    }
}

impl CompletedDocumentPolicyBatch {
    pub(crate) fn context(&self) -> moli_core::browser::BrowserContextId {
        self.context
    }

    pub(crate) fn document(&self) -> DocumentHandle {
        self.document
    }
}

impl BrowserContext {
    pub(crate) fn start_document_runtime_policy_reconciliation(
        &mut self,
        document: DocumentHandle,
        policy: DocumentRuntimePolicyReconciliation,
    ) -> PendingDocumentPolicyBatch {
        let mut updates = vec![
            DocumentPolicyUpdate::ScriptExecutionDisabled(policy.script_execution_disabled),
            DocumentPolicyUpdate::EmulatedMedia(policy.emulated_media),
            DocumentPolicyUpdate::CpuThrottlingRate(policy.cpu_throttling_rate),
            DocumentPolicyUpdate::NetworkOffline(policy.network_offline),
            DocumentPolicyUpdate::ViewportSurface(policy.viewport_surface),
        ];
        if policy.clear_device_metrics_surface {
            updates.push(DocumentPolicyUpdate::ClearDeviceMetricsSurface);
        }
        self.start_document_policy_batch(document, updates)
    }

    pub(crate) fn start_document_policy_batch(
        &mut self,
        document: DocumentHandle,
        updates: Vec<DocumentPolicyUpdate>,
    ) -> PendingDocumentPolicyBatch {
        let mut pending = Vec::with_capacity(updates.len());
        let mut admission_error = None;
        for update in updates {
            match self.start_document_policy_update(document, update) {
                Ok(update) => pending.push(update),
                Err(error) => {
                    admission_error.get_or_insert(error);
                }
            }
        }
        PendingDocumentPolicyBatch {
            context: document.web_contents().context(),
            document,
            admission_error,
            updates: pending,
        }
    }

    pub(crate) fn start_document_policy_batch_with_surface(
        &mut self,
        document: DocumentHandle,
        updates: Vec<DocumentPolicyUpdate>,
        foreground: bool,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> PendingDocumentPolicyBatch {
        let mut batch = self.start_document_policy_batch(document, updates);
        match self.start_document_page_surface_update(document, foreground, browser_globals) {
            Ok(update) => batch.updates.push(update),
            Err(error) => {
                batch.admission_error.get_or_insert(error);
            }
        }
        batch
    }

    pub(crate) fn start_document_policy_update(
        &mut self,
        document: DocumentHandle,
        update: DocumentPolicyUpdate,
    ) -> Result<PendingDocumentPolicyUpdate, String> {
        let page = &mut self.physical.document_mut(document)?.page;
        let (kind, pending) = match update {
            DocumentPolicyUpdate::NetworkRequestPolicy {
                extra_headers,
                bypass_service_worker,
                cache_disabled,
                blocked_url_patterns,
            } => (
                DocumentPolicyUpdateKind::SetNetworkRequestPolicy,
                page.start_set_network_request_policy(
                    &extra_headers,
                    bypass_service_worker,
                    cache_disabled,
                    &blocked_url_patterns,
                ),
            ),
            DocumentPolicyUpdate::ExtraHttpHeaders(headers) => (
                DocumentPolicyUpdateKind::SetExtraHttpHeaders,
                page.start_set_extra_http_headers(&headers),
            ),
            DocumentPolicyUpdate::BlockedUrls(patterns) => (
                DocumentPolicyUpdateKind::SetBlockedUrls,
                page.start_set_blocked_url_patterns(&patterns),
            ),
            DocumentPolicyUpdate::BypassServiceWorker(bypass) => (
                DocumentPolicyUpdateKind::SetBypassServiceWorker,
                page.start_set_bypass_service_worker(bypass),
            ),
            DocumentPolicyUpdate::LocaleOverride(locale) => (
                DocumentPolicyUpdateKind::SetLocaleOverride,
                page.start_set_locale_override(locale.as_deref()),
            ),
            DocumentPolicyUpdate::NetworkOffline(offline) => (
                DocumentPolicyUpdateKind::SetNetworkConditions,
                page.start_set_network_offline(offline),
            ),
            DocumentPolicyUpdate::CpuThrottlingRate(rate) => (
                DocumentPolicyUpdateKind::SetCpuThrottlingRate,
                page.start_set_cpu_throttling_rate(rate),
            ),
            DocumentPolicyUpdate::IdleOverride(override_) => (
                DocumentPolicyUpdateKind::SetIdleOverride,
                page.start_set_idle_override(override_),
            ),
            DocumentPolicyUpdate::TimezoneOverride(timezone) => (
                DocumentPolicyUpdateKind::SetTimezoneOverride,
                page.start_set_timezone_override(timezone.as_deref()),
            ),
            DocumentPolicyUpdate::EmulatedMedia(overrides) => (
                DocumentPolicyUpdateKind::SetEmulatedMedia,
                page.start_set_emulated_media(&overrides),
            ),
            DocumentPolicyUpdate::ViewportSurface(surface) => (
                DocumentPolicyUpdateKind::SetViewportSurface,
                page.start_set_viewport_surface(surface),
            ),
            DocumentPolicyUpdate::ScriptExecutionDisabled(disabled) => (
                DocumentPolicyUpdateKind::SetScriptExecutionDisabled,
                page.start_set_script_execution_disabled(disabled),
            ),
            DocumentPolicyUpdate::ClearDeviceMetricsSurface => (
                DocumentPolicyUpdateKind::PageSurfaceOverride,
                page.start_page_surface_override_script(LIVE_DEVICE_METRICS_CLEAR_SCRIPT),
            ),
        };
        Ok(PendingDocumentPolicyUpdate {
            document,
            kind,
            pending: pending.map_err(|error| error.to_string())?,
        })
    }

    pub(crate) fn finish_document_policy_update(
        &mut self,
        completed: CompletedDocumentPolicyUpdate,
    ) -> Result<(), String> {
        let completion = completed.completed?;
        let page = match self.physical.document_mut(completed.document) {
            Ok(document) => &mut document.page,
            Err(error) => {
                completion
                    .into_unit_page_command_turn()
                    .map(drop)
                    .map_err(|unexpected| {
                        format!(
                            "retired Document policy command returned an unexpected reply: {unexpected}"
                        )
                    })?;
                return Err(error);
            }
        };
        match completed.kind {
            DocumentPolicyUpdateKind::SetNetworkRequestPolicy => {
                page.finish_set_network_request_policy(completion)
            }
            DocumentPolicyUpdateKind::SetExtraHttpHeaders => {
                page.finish_set_extra_http_headers(completion)
            }
            DocumentPolicyUpdateKind::SetBlockedUrls => {
                page.finish_set_blocked_url_patterns(completion)
            }
            DocumentPolicyUpdateKind::SetBypassServiceWorker => {
                page.finish_set_bypass_service_worker(completion)
            }
            DocumentPolicyUpdateKind::SetLocaleOverride => {
                page.finish_set_locale_override(completion)
            }
            DocumentPolicyUpdateKind::SetNetworkConditions => {
                page.finish_set_network_offline(completion)
            }
            DocumentPolicyUpdateKind::SetCpuThrottlingRate => {
                page.finish_set_cpu_throttling_rate(completion)
            }
            DocumentPolicyUpdateKind::SetIdleOverride => page.finish_set_idle_override(completion),
            DocumentPolicyUpdateKind::SetTimezoneOverride => {
                page.finish_set_timezone_override(completion)
            }
            DocumentPolicyUpdateKind::SetEmulatedMedia => {
                page.finish_set_emulated_media(completion)
            }
            DocumentPolicyUpdateKind::SetViewportSurface => {
                page.finish_set_viewport_surface(completion)
            }
            DocumentPolicyUpdateKind::SetScriptExecutionDisabled => {
                page.finish_set_script_execution_disabled(completion)
            }
            DocumentPolicyUpdateKind::PageSurfaceOverride => {
                page.finish_page_surface_override_script(completion)
            }
        }
        .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_document_policy_batch(
        &mut self,
        completed: CompletedDocumentPolicyBatch,
    ) -> Result<(), String> {
        let mut first_error = completed.admission_error;
        for update in completed.updates {
            if let Err(error) = self.finish_document_policy_update(update) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn start_document_page_surface_update(
        &mut self,
        document: DocumentHandle,
        foreground: bool,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<PendingDocumentPolicyUpdate, String> {
        let source = self
            .page_surface_for_web_contents(
                document.web_contents(),
                foreground,
                browser_globals.network_conditions,
                browser_globals.geolocation.as_ref(),
            )?
            .script();
        let pending = self
            .physical
            .document(document)?
            .page
            .start_page_surface_override_script(&source)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentPolicyUpdate {
            document,
            kind: DocumentPolicyUpdateKind::PageSurfaceOverride,
            pending,
        })
    }
}

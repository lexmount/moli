use super::BrowserContext;
use crate::browser::DocumentHandle;
use crate::browser::web_contents::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;
use crate::page::{
    CompletedPageCommand, EmulatedIdleOverride, EmulatedMediaOverrides, PendingPageCommand,
    ViewportSurface,
};

pub enum DocumentPolicyUpdate {
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
    NavigatorOverrides(moli_page_types::NavigatorOverrides),
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
    SetNavigatorOverrides,
    SetTimezoneOverride,
    SetEmulatedMedia,
    SetViewportSurface,
    SetScriptExecutionDisabled,
    PageSurfaceOverride,
}

pub struct PendingDocumentPolicyUpdate {
    document: DocumentHandle,
    kind: DocumentPolicyUpdateKind,
    pending: PendingPageCommand,
}

pub struct CompletedDocumentPolicyUpdate {
    document: DocumentHandle,
    kind: DocumentPolicyUpdateKind,
    completed: Result<CompletedPageCommand, String>,
}

pub struct PendingDocumentPolicyBatch {
    context: crate::browser::BrowserContextId,
    document: DocumentHandle,
    admission_error: Option<String>,
    updates: Vec<PendingDocumentPolicyUpdate>,
}

pub struct CompletedDocumentPolicyBatch {
    context: crate::browser::BrowserContextId,
    document: DocumentHandle,
    admission_error: Option<String>,
    updates: Vec<CompletedDocumentPolicyUpdate>,
}

pub struct DocumentRuntimePolicyReconciliation {
    pub script_execution_disabled: bool,
    pub emulated_media: EmulatedMediaOverrides,
    pub cpu_throttling_rate: f64,
    pub network_offline: bool,
    pub viewport_surface: Option<ViewportSurface>,
    pub clear_device_metrics_surface: bool,
}

impl PendingDocumentPolicyUpdate {
    pub async fn wait(self) -> CompletedDocumentPolicyUpdate {
        CompletedDocumentPolicyUpdate {
            document: self.document,
            kind: self.kind,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedDocumentPolicyUpdate {
    pub fn document(&self) -> DocumentHandle {
        self.document
    }
}

impl PendingDocumentPolicyBatch {
    pub async fn wait(self) -> CompletedDocumentPolicyBatch {
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
    pub fn context(&self) -> crate::browser::BrowserContextId {
        self.context
    }

    pub fn document(&self) -> DocumentHandle {
        self.document
    }
}

impl BrowserContext {
    pub fn start_document_runtime_policy_reconciliation(
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

    pub fn start_document_policy_batch(
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

    pub fn start_document_policy_batch_with_surface(
        &mut self,
        document: DocumentHandle,
        updates: Vec<DocumentPolicyUpdate>,
        foreground: bool,
        global_network_conditions: Option<crate::browser::EmulatedNetworkConditions>,
        global_geolocation: Option<&crate::browser::EmulatedGeolocationOverrideState>,
    ) -> PendingDocumentPolicyBatch {
        let mut batch = self.start_document_policy_batch(document, updates);
        match self.start_document_page_surface_update(
            document,
            foreground,
            global_network_conditions,
            global_geolocation,
        ) {
            Ok(surface) => {
                batch.updates.extend(surface.updates);
                if let Some(error) = surface.admission_error {
                    batch.admission_error.get_or_insert(error);
                }
            }
            Err(error) => {
                batch.admission_error.get_or_insert(error);
            }
        }
        batch
    }

    pub fn start_document_policy_update(
        &mut self,
        document: DocumentHandle,
        update: DocumentPolicyUpdate,
    ) -> Result<PendingDocumentPolicyUpdate, String> {
        let page = &mut self.document_mut(document)?.page;
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
            DocumentPolicyUpdate::NavigatorOverrides(overrides) => (
                DocumentPolicyUpdateKind::SetNavigatorOverrides,
                page.start_set_navigator_overrides(&overrides),
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

    pub fn finish_document_policy_update(
        &mut self,
        completed: CompletedDocumentPolicyUpdate,
    ) -> Result<(), String> {
        let completion = completed.completed?;
        let page = match self.document_mut(completed.document) {
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
            DocumentPolicyUpdateKind::SetNavigatorOverrides => {
                page.finish_set_navigator_overrides(completion)
            }
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

    pub fn finish_document_policy_batch(
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

    pub fn start_document_page_surface_update(
        &mut self,
        document: DocumentHandle,
        foreground: bool,
        global_network_conditions: Option<crate::browser::EmulatedNetworkConditions>,
        global_geolocation: Option<&crate::browser::EmulatedGeolocationOverrideState>,
    ) -> Result<PendingDocumentPolicyBatch, String> {
        let surface = self.page_surface_for_web_contents(
            document.web_contents(),
            foreground,
            global_network_conditions,
            global_geolocation,
        )?;
        let mut batch = self.start_document_policy_batch(
            document,
            vec![DocumentPolicyUpdate::NavigatorOverrides(
                surface.navigator_overrides(),
            )],
        );
        let pending = self.document(document).and_then(|document| {
            document
                .page
                .start_page_surface_override_script(&surface.script())
                .map_err(|error| error.to_string())
        });
        match pending {
            Ok(pending) => batch.updates.push(PendingDocumentPolicyUpdate {
                document,
                kind: DocumentPolicyUpdateKind::PageSurfaceOverride,
                pending,
            }),
            Err(error) => {
                batch.admission_error.get_or_insert(error);
            }
        }
        Ok(batch)
    }

    pub fn page_surface_for_web_contents(
        &self,
        handle: crate::browser::WebContentsHandle,
        foreground: bool,
        global_network_conditions: Option<crate::browser::EmulatedNetworkConditions>,
        global_geolocation: Option<&crate::browser::EmulatedGeolocationOverrideState>,
    ) -> Result<crate::browser::web_contents::PageSurface, String> {
        Ok(self.web_contents(handle)?.page_surface(
            foreground,
            self.emulation_defaults
                .network_conditions
                .or(global_network_conditions),
            self.emulation_defaults
                .geolocation
                .as_ref()
                .or(global_geolocation),
            self.emulation_defaults.device_metrics.as_ref(),
        ))
    }
}

use std::collections::{BTreeMap, BTreeSet};

use moli_core::page::{
    RendererServiceWorkerExceptionMessage, RendererServiceWorkerFetchDiagnostic,
    RendererServiceWorkerRunIdentity, RendererServiceWorkerVersionStatus,
    RuntimeConsoleMessageSnapshot,
};
use serde_json::Value;

use crate::devtools_runtime::RuntimeExecutionContextEvent;

use super::parking::PendingInspectorAwait;
use super::service_worker_lifetime::{
    TargetServiceWorkerProtocolAttachmentIdentity, TargetServiceWorkerProtocolAttachmentRetirement,
    TargetServiceWorkerProtocolAttachmentScope, TargetServiceWorkerRunIdentity,
    TargetServiceWorkerRunRetirement, TargetServiceWorkerRunScope,
    TargetServiceWorkerRuntimeAttachmentIdentity, TargetServiceWorkerVersionIdentity,
    TargetServiceWorkerVersionRetirement, TargetServiceWorkerVersionScope,
};
use super::{
    DevToolsSessionState, DuplicatePendingRendererCommand, PreparedRendererCallDispatch,
    RegisterRendererCallError, RendererCommandCorrelation, RendererCommandDescriptor,
};

// Chromium does not reserve negative Runtime.ExecutionContextId values for
// worker target types. It gets the integer id from V8 inspector, scopes
// inspector sessions by contextGroupId, and exposes uniqueId for cross-process
// context identity. This Moli-only fallback bridges target/log/realm
// projections that can exist before Runtime.executionContextCreated is recorded.
// Keep it in a negative worker-typed range so it cannot be confused with a real
// renderer/V8 context id, then rebind snapshots once the real id arrives.
const SERVICE_WORKER_SYNTHETIC_EXECUTION_CONTEXT_ID_BASE: i64 = -20_000_000;

/// CDP-side projection for a renderer-owned `service_worker` target.
///
/// The renderer service worker runtime owns version lifecycle. This state keeps
/// the protocol target id/session projection plus Runtime/Console cursors for
/// a live renderer version. Runtime execution is still renderer-owned; this
/// state only records the CDP-observable projection for target-scoped events.
#[derive(Debug)]
pub(crate) struct ServiceWorkerTargetState {
    pub(crate) renderer_registration_id: u64,
    pub(crate) renderer_version_id: u64,
    pub(crate) target_id: String,
    sessions: BTreeMap<String, ServiceWorkerTargetSessionState>,
    pub(crate) script_url: String,
    pub(crate) scope_url: String,
    version_status: RendererServiceWorkerVersionStatus,
    version_scope: Option<TargetServiceWorkerVersionScope>,
    run_state: ServiceWorkerTargetRunState,
    inspector_target_crashed_session_ids: BTreeSet<String>,
    runtime_execution_context_id: Option<i64>,
    console_messages: Vec<RuntimeConsoleMessageSnapshot>,
    exception_messages: Vec<ServiceWorkerRuntimeExceptionSnapshot>,
    fetch_diagnostics: Vec<RendererServiceWorkerFetchDiagnostic>,
    classic_log_cursors: BTreeMap<String, usize>,
}

/// Exact state of the short-lived V8 worker beneath one stable version target.
///
/// A live run cannot exist without both the renderer-created identity and its
/// protocol projection scope. A stopped target remembers only the exact most
/// recently retired renderer identity, preventing an already-captured stop
/// from being reopened before its ordered retirement output drains.
#[derive(Debug)]
enum ServiceWorkerTargetRunState {
    Live {
        phase: ServiceWorkerTargetLiveRunPhase,
        run: ServiceWorkerTargetLiveRun,
    },
    Stopped {
        last_retired_renderer_run: Option<RendererServiceWorkerRunIdentity>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServiceWorkerTargetLiveRunPhase {
    Starting,
    Running,
}

#[derive(Debug)]
struct ServiceWorkerTargetLiveRun {
    scope: TargetServiceWorkerRunScope,
}

/// Session-local protocol state plus the exact attachment lifetime.
///
/// `DevToolsSessionState` contains mutable domain state. The sibling scope is
/// deliberately not cloneable: removing this map entry is the only ordinary
/// way to expire output captured for this exact target attachment.
#[derive(Debug)]
struct ServiceWorkerTargetSessionState {
    devtools: DevToolsSessionState,
    attachment_scope: TargetServiceWorkerProtocolAttachmentScope,
}

impl Default for ServiceWorkerTargetSessionState {
    fn default() -> Self {
        Self {
            devtools: DevToolsSessionState::default(),
            attachment_scope: TargetServiceWorkerProtocolAttachmentScope::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceWorkerTargetRunningStatus {
    Starting,
    Running,
    Stopped,
}

impl ServiceWorkerTargetRunningStatus {
    pub(crate) fn as_cdp_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceWorkerRuntimeExceptionSnapshot {
    pub(crate) execution_context_id: i64,
    pub(crate) message: RendererServiceWorkerExceptionMessage,
}

impl ServiceWorkerTargetState {
    pub(crate) fn new(
        renderer_registration_id: u64,
        renderer_version_id: u64,
        target_id: String,
        script_url: String,
        scope_url: String,
        version_status: RendererServiceWorkerVersionStatus,
        active_renderer_run: Option<RendererServiceWorkerRunIdentity>,
    ) -> Self {
        let run_state = match active_renderer_run {
            Some(renderer_run) => ServiceWorkerTargetRunState::Live {
                phase: ServiceWorkerTargetLiveRunPhase::Starting,
                run: ServiceWorkerTargetLiveRun {
                    scope: TargetServiceWorkerRunScope::new(renderer_run),
                },
            },
            None => ServiceWorkerTargetRunState::Stopped {
                last_retired_renderer_run: None,
            },
        };
        Self {
            renderer_registration_id,
            renderer_version_id,
            target_id,
            sessions: BTreeMap::new(),
            script_url,
            scope_url,
            version_status,
            version_scope: Some(TargetServiceWorkerVersionScope::new()),
            run_state,
            inspector_target_crashed_session_ids: BTreeSet::new(),
            runtime_execution_context_id: None,
            console_messages: Vec::new(),
            exception_messages: Vec::new(),
            fetch_diagnostics: Vec::new(),
            classic_log_cursors: BTreeMap::new(),
        }
    }

    pub(crate) fn execution_context_id(&self) -> i64 {
        if let Some(id) = self.runtime_execution_context_id {
            return id;
        }
        let version_id = i64::try_from(self.renderer_version_id).unwrap_or(i64::MAX);
        SERVICE_WORKER_SYNTHETIC_EXECUTION_CONTEXT_ID_BASE.saturating_sub(version_id)
    }

    pub(crate) fn real_runtime_execution_context_id(&self) -> Option<i64> {
        self.runtime_execution_context_id
    }

    fn rebind_synthetic_runtime_snapshots(&mut self, synthetic_id: i64, real_id: i64) {
        for message in &mut self.console_messages {
            if message.execution_context_id == synthetic_id {
                message.execution_context_id = real_id;
            }
        }
        for message in &mut self.exception_messages {
            if message.execution_context_id == synthetic_id {
                message.execution_context_id = real_id;
            }
        }
    }

    pub(crate) fn record_runtime_execution_context_created_event(
        &mut self,
        event: &RuntimeExecutionContextEvent,
    ) {
        if matches!(
            event.context_type.as_deref(),
            Some("worker" | "service-worker")
        ) && let Some(id) = event.context_id
        {
            let previous_id = self.execution_context_id();
            self.runtime_execution_context_id = Some(id);
            if previous_id < 0 {
                self.rebind_synthetic_runtime_snapshots(previous_id, id);
            }
        }
    }

    pub(crate) fn record_runtime_execution_context_destroyed_event(
        &mut self,
        event: &RuntimeExecutionContextEvent,
    ) {
        if event.context_id == self.runtime_execution_context_id {
            self.runtime_execution_context_id = None;
        }
    }

    pub(crate) fn record_runtime_execution_contexts_cleared_event(&mut self) {
        self.runtime_execution_context_id = None;
    }

    pub(crate) fn version_identity(
        &self,
        browser_context_id: &str,
    ) -> Option<TargetServiceWorkerVersionIdentity> {
        Some(self.version_scope.as_ref()?.bind(
            browser_context_id,
            self.renderer_registration_id,
            self.renderer_version_id,
            self.target_id.clone(),
        ))
    }

    pub(crate) fn protocol_attachment_identity(
        &self,
        browser_context_id: &str,
        session_id: &str,
    ) -> Option<TargetServiceWorkerProtocolAttachmentIdentity> {
        let session = self.sessions.get(session_id)?;
        Some(
            session
                .attachment_scope
                .bind(self.version_identity(browser_context_id)?, session_id),
        )
    }

    #[cfg(test)]
    pub(crate) fn runtime_attachment_identity_for_current_run(
        &self,
        browser_context_id: &str,
        session_id: &str,
    ) -> Option<TargetServiceWorkerRuntimeAttachmentIdentity> {
        Some(TargetServiceWorkerRuntimeAttachmentIdentity::new(
            self.protocol_attachment_identity(browser_context_id, session_id)?,
            self.current_run_identity(browser_context_id)?,
        ))
    }

    pub(crate) fn runtime_attachment_identity_for_run(
        &self,
        browser_context_id: &str,
        session_id: &str,
        run: &TargetServiceWorkerRunIdentity,
    ) -> Option<TargetServiceWorkerRuntimeAttachmentIdentity> {
        if self.current_run_identity(browser_context_id)?.ne(run) {
            return None;
        }
        Some(TargetServiceWorkerRuntimeAttachmentIdentity::new(
            self.protocol_attachment_identity(browser_context_id, session_id)?,
            run.clone(),
        ))
    }

    pub(crate) fn observes_runtime_identity(
        &self,
        browser_context_id: &str,
        runtime: &TargetServiceWorkerRuntimeAttachmentIdentity,
    ) -> bool {
        if !runtime.is_current() {
            return false;
        }
        self.current_run_identity(browser_context_id).as_ref() == Some(runtime.run())
            && self
                .protocol_attachment_identity(browser_context_id, runtime.session_id())
                .as_ref()
                == Some(runtime.attachment())
    }

    /// Projects one exact renderer run into this protocol target.
    ///
    /// Runtime inspector output can arrive before the public `Started` event,
    /// so the first run-specific fact may establish a `Starting` projection.
    /// A different identity can be admitted only after the prior run moved to
    /// an ordered retirement output.
    pub(crate) fn observe_worker_run(
        &mut self,
        browser_context_id: &str,
        renderer_run: RendererServiceWorkerRunIdentity,
    ) -> Option<TargetServiceWorkerRunIdentity> {
        match &self.run_state {
            ServiceWorkerTargetRunState::Live { run, .. } => {
                assert!(
                    run.scope.renderer_run() == &renderer_run,
                    "a different renderer ServiceWorker run must not replace a live protocol run"
                );
                return self.current_run_identity(browser_context_id);
            }
            ServiceWorkerTargetRunState::Stopped {
                last_retired_renderer_run: Some(retired),
            } if retired == &renderer_run => return None,
            ServiceWorkerTargetRunState::Stopped { .. } => {}
        }
        self.runtime_execution_context_id = None;
        self.run_state = ServiceWorkerTargetRunState::Live {
            phase: ServiceWorkerTargetLiveRunPhase::Starting,
            run: ServiceWorkerTargetLiveRun {
                scope: TargetServiceWorkerRunScope::new(renderer_run),
            },
        };
        self.current_run_identity(browser_context_id)
    }

    pub(crate) fn mark_worker_started(
        &mut self,
        browser_context_id: &str,
        renderer_run: RendererServiceWorkerRunIdentity,
    ) -> Option<TargetServiceWorkerRunIdentity> {
        let run = self.observe_worker_run(browser_context_id, renderer_run)?;
        match &mut self.run_state {
            ServiceWorkerTargetRunState::Live { phase, .. }
                if *phase == ServiceWorkerTargetLiveRunPhase::Starting =>
            {
                *phase = ServiceWorkerTargetLiveRunPhase::Running;
                Some(run)
            }
            ServiceWorkerTargetRunState::Live { .. } => None,
            ServiceWorkerTargetRunState::Stopped { .. } => {
                unreachable!("observed service-worker run must remain live")
            }
        }
    }

    pub(crate) fn mark_worker_stopped(
        &mut self,
        browser_context_id: &str,
        renderer_run: RendererServiceWorkerRunIdentity,
        _reason: &str,
    ) -> Option<TargetServiceWorkerRunRetirement> {
        let identity = self.observe_worker_run(browser_context_id, renderer_run.clone())?;
        let live_state = std::mem::replace(
            &mut self.run_state,
            ServiceWorkerTargetRunState::Stopped {
                last_retired_renderer_run: Some(renderer_run.clone()),
            },
        );
        let ServiceWorkerTargetRunState::Live { run, .. } = live_state else {
            unreachable!("observed service-worker run must retain its unique scope")
        };
        assert_eq!(
            run.scope.renderer_run(),
            &renderer_run,
            "service-worker stop must retire the exact observed run"
        );
        self.runtime_execution_context_id = None;
        Some(run.scope.into_retirement(identity))
    }

    pub(crate) fn worker_running(&self) -> bool {
        matches!(
            self.run_state,
            ServiceWorkerTargetRunState::Live {
                phase: ServiceWorkerTargetLiveRunPhase::Running,
                ..
            }
        )
    }

    /// Verifies the renderer's version-destruction snapshot without deriving a
    /// run from the stable version target.
    pub(crate) fn observes_destroyed_active_run(
        &self,
        renderer_run: Option<&RendererServiceWorkerRunIdentity>,
    ) -> bool {
        match (&self.run_state, renderer_run) {
            (ServiceWorkerTargetRunState::Live { run, .. }, Some(renderer_run)) => {
                run.scope.renderer_run() == renderer_run
            }
            (ServiceWorkerTargetRunState::Stopped { .. }, None) => true,
            _ => false,
        }
    }

    pub(crate) fn running_status_cdp_str(&self) -> &'static str {
        self.running_status().as_cdp_str()
    }

    pub(crate) fn version_status_cdp_str(&self) -> &'static str {
        self.version_status.as_cdp_str()
    }

    pub(crate) fn update_version_status(
        &mut self,
        browser_context_id: &str,
        status: RendererServiceWorkerVersionStatus,
    ) -> Option<TargetServiceWorkerVersionIdentity> {
        self.version_status = status;
        self.version_identity(browser_context_id)
    }

    pub(crate) fn register_runtime_remote_object_ids_for_session<I>(
        &mut self,
        session_id: &str,
        object_ids: I,
    ) where
        I: IntoIterator<Item = String>,
    {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_ids(object_ids);
    }

    pub(crate) fn register_runtime_remote_object_ids_with_realm<I>(
        &mut self,
        session_id: &str,
        object_ids: I,
        realm_id: &str,
    ) where
        I: IntoIterator<Item = String>,
    {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_ids_with_realm(object_ids, realm_id);
    }

    pub(crate) fn register_runtime_remote_object_ids_with_group<I>(
        &mut self,
        session_id: &str,
        object_ids: I,
        object_group: &str,
    ) where
        I: IntoIterator<Item = String>,
    {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_ids_with_group(object_ids, object_group);
    }

    pub(crate) fn register_runtime_remote_object_alias_with_realm(
        &mut self,
        session_id: &str,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_alias_with_realm(alias_id, object_id, realm_id);
    }

    pub(crate) fn unregister_runtime_remote_object_ids(
        &mut self,
        session_id: &str,
        object_ids: &[String],
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.unregister_runtime_remote_object_ids(object_ids);
    }

    pub(crate) fn unregister_runtime_remote_object_group(
        &mut self,
        session_id: &str,
        object_group: &str,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.unregister_runtime_remote_object_group(object_group);
    }

    pub(crate) fn clear_runtime_remote_object_tracking(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.clear_runtime_remote_object_tracking();
    }

    pub(crate) fn record_runtime_contexts_reported_to_frontend(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.record_runtime_contexts_reported_to_frontend();
    }

    pub(crate) fn record_runtime_contexts_cleared_for_frontend(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.record_runtime_contexts_cleared_for_frontend();
    }

    pub(crate) fn runtime_context_reported_session_ids(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, state)| {
                state
                    .devtools
                    .runtime_session_state
                    .runtime_contexts_reported_to_frontend
            })
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }

    pub(crate) fn clear_runtime_remote_objects_for_realm(
        &mut self,
        session_id: &str,
        realm_id: &str,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.clear_runtime_remote_objects_for_realm(realm_id);
    }

    pub(crate) fn runtime_remote_object_group(
        &self,
        session_id: &str,
        object_id: &str,
    ) -> Option<&str> {
        self.session_state(session_id)?
            .runtime_remote_object_group(object_id)
    }

    pub(crate) fn has_runtime_remote_object_id(&self, session_id: &str, object_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| state.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn any_session_has_runtime_remote_object_id(&self, object_id: &str) -> bool {
        self.sessions
            .values()
            .any(|state| state.devtools.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn runtime_remote_object_realm(
        &self,
        session_id: &str,
        object_id: &str,
    ) -> Option<&str> {
        self.session_state(session_id)?
            .runtime_remote_object_realm(object_id)
    }

    pub(crate) fn runtime_remote_object_alias(
        &self,
        session_id: &str,
        object_id: &str,
    ) -> Option<&str> {
        self.session_state(session_id)?
            .runtime_remote_object_alias(object_id)
    }

    pub(crate) fn attach_session(&mut self, session_id: String) {
        self.sessions.entry(session_id).or_default();
    }

    pub(crate) fn detach_session(&mut self, session_id: &str) -> bool {
        self.inspector_target_crashed_session_ids.remove(session_id);
        self.sessions.remove(session_id).is_some()
    }

    pub(crate) fn take_protocol_attachment_retirement(
        &mut self,
        browser_context_id: &str,
        session_id: &str,
    ) -> Option<TargetServiceWorkerProtocolAttachmentRetirement> {
        self.inspector_target_crashed_session_ids.remove(session_id);
        let session = self.sessions.remove(session_id)?;
        let identity = session
            .attachment_scope
            .bind(self.version_identity(browser_context_id)?, session_id);
        Some(session.attachment_scope.into_retirement(identity))
    }

    pub(crate) fn take_current_run_retirement(
        &mut self,
        browser_context_id: &str,
    ) -> Option<TargetServiceWorkerRunRetirement> {
        let identity = self.current_run_identity(browser_context_id)?;
        let renderer_run = identity.renderer_run().clone();
        let live_state = std::mem::replace(
            &mut self.run_state,
            ServiceWorkerTargetRunState::Stopped {
                last_retired_renderer_run: Some(renderer_run),
            },
        );
        let ServiceWorkerTargetRunState::Live { run, .. } = live_state else {
            unreachable!("current service-worker run identity must retain its unique scope")
        };
        self.runtime_execution_context_id = None;
        Some(run.scope.into_retirement(identity))
    }

    pub(crate) fn take_version_retirement(
        &mut self,
        browser_context_id: &str,
    ) -> Option<TargetServiceWorkerVersionRetirement> {
        assert!(
            self.sessions.is_empty(),
            "service-worker version retirement must follow attachment retirement"
        );
        assert!(
            matches!(self.run_state, ServiceWorkerTargetRunState::Stopped { .. }),
            "service-worker version retirement must follow run retirement"
        );
        let identity = self.version_identity(browser_context_id)?;
        Some(
            self.version_scope
                .take()
                .expect("live service-worker version must retain its unique scope")
                .into_retirement(identity),
        )
    }

    pub(crate) fn is_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    pub(crate) fn has_session(&self) -> bool {
        !self.sessions.is_empty()
    }

    pub(crate) fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    pub(crate) fn set_runtime_frontend_enabled(&mut self, session_id: &str, enabled: bool) {
        let console_len = self.console_messages.len();
        let exception_len = self.exception_messages.len();
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.runtime_session_state.runtime_frontend_enabled = enabled;
        if !enabled {
            state
                .runtime_session_state
                .runtime_contexts_reported_to_frontend = false;
            state.console_output_session_state.runtime_console_entries = console_len;
            state.console_output_session_state.runtime_exception_entries = exception_len;
        }
    }

    pub(crate) fn set_inspector_enabled(&mut self, session_id: &str, enabled: bool) -> bool {
        let Some(state) = self.session_state_mut(session_id) else {
            return false;
        };
        state.runtime_session_state.inspector_enabled = enabled;
        if !enabled {
            self.inspector_target_crashed_session_ids.remove(session_id);
        }
        true
    }

    pub(crate) fn inspector_enabled(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| state.runtime_session_state.inspector_enabled)
    }

    pub(crate) fn inspector_enabled_session_ids(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, state)| state.devtools.runtime_session_state.inspector_enabled)
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }

    pub(crate) fn record_inspector_target_crashed_for_session(&mut self, session_id: &str) -> bool {
        if !self.inspector_enabled(session_id) {
            return false;
        }
        self.inspector_target_crashed_session_ids
            .insert(session_id.to_owned())
    }

    pub(crate) fn take_inspector_target_reloaded_after_crash_session_ids(&mut self) -> Vec<String> {
        let session_ids = self
            .inspector_target_crashed_session_ids
            .iter()
            .filter(|session_id| self.inspector_enabled(session_id))
            .cloned()
            .collect();
        self.inspector_target_crashed_session_ids.clear();
        session_ids
    }

    pub(crate) fn set_console_enabled(&mut self, session_id: &str, enabled: bool) {
        let console_len = self.console_messages.len();
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.console_output_session_state.console_enabled = enabled;
        state.console_output_session_state.console_domain_entries = console_len;
    }

    pub(crate) fn set_network_enabled(&mut self, session_id: &str, enabled: bool) -> bool {
        let diagnostic_len = self.fetch_diagnostics.len();
        let Some(state) = self.session_state_mut(session_id) else {
            return false;
        };
        let was_enabled = state.network_session_state.network_enabled;
        if enabled {
            state.network_session_state.network_enabled = true;
            if !was_enabled {
                state
                    .network_session_state
                    .service_worker_fetch_diagnostic_entries = diagnostic_len;
            }
        } else {
            state.network_session_state = Default::default();
            state
                .network_session_state
                .service_worker_fetch_diagnostic_entries = diagnostic_len;
        }
        true
    }

    pub(crate) fn network_enabled(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| state.network_session_state.network_enabled)
    }

    pub(crate) fn clear_console_messages(&mut self, session_id: &str) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.console_domain_entries = console_len;
        }
    }

    pub(crate) fn discard_runtime_console_entries(&mut self, session_id: &str) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.runtime_console_entries = console_len;
        }
    }

    pub(crate) fn record_console_message(
        &mut self,
        message: String,
        args: Vec<Value>,
        stack: Option<String>,
    ) {
        self.console_messages.push(RuntimeConsoleMessageSnapshot {
            execution_context_id: self.execution_context_id(),
            message,
            args,
            stack,
        });
    }

    pub(crate) fn record_exception_message(
        &mut self,
        message: RendererServiceWorkerExceptionMessage,
    ) {
        self.exception_messages
            .push(ServiceWorkerRuntimeExceptionSnapshot {
                execution_context_id: self.execution_context_id(),
                message,
            });
    }

    pub(crate) fn record_fetch_diagnostic(
        &mut self,
        diagnostic: RendererServiceWorkerFetchDiagnostic,
    ) {
        self.fetch_diagnostics.push(diagnostic);
    }

    pub(crate) fn mark_console_domain_emitted(&mut self, session_id: &str, console_end: usize) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.console_domain_entries =
                console_end.min(console_len);
        }
    }

    pub(crate) fn pending_console_domain_messages(
        &self,
        session_id: &str,
    ) -> &[RuntimeConsoleMessageSnapshot] {
        let Some(state) = self.session_state(session_id) else {
            return &[];
        };
        if !state.console_output_session_state.console_enabled {
            return &[];
        }
        &self.console_messages[state
            .console_output_session_state
            .console_domain_entries
            .min(self.console_messages.len())..]
    }

    pub(crate) fn pending_runtime_console_messages(
        &self,
        session_id: &str,
    ) -> &[RuntimeConsoleMessageSnapshot] {
        let Some(state) = self.session_state(session_id) else {
            return &[];
        };
        if !state.runtime_session_state.runtime_frontend_enabled {
            return &[];
        }
        if self.runtime_execution_context_id.is_none() {
            return &[];
        }
        &self.console_messages[state
            .console_output_session_state
            .runtime_console_entries
            .min(self.console_messages.len())..]
    }

    pub(crate) fn pending_classic_log_messages(
        &self,
        cursor_id: &str,
    ) -> &[RuntimeConsoleMessageSnapshot] {
        let start = self
            .classic_log_cursors
            .get(cursor_id)
            .copied()
            .unwrap_or_default()
            .min(self.console_messages.len());
        &self.console_messages[start..]
    }

    pub(crate) fn pending_runtime_exception_messages(
        &self,
        session_id: &str,
    ) -> &[ServiceWorkerRuntimeExceptionSnapshot] {
        let Some(state) = self.session_state(session_id) else {
            return &[];
        };
        if !state.runtime_session_state.runtime_frontend_enabled {
            return &[];
        }
        if self.runtime_execution_context_id.is_none() {
            return &[];
        }
        &self.exception_messages[state
            .console_output_session_state
            .runtime_exception_entries
            .min(self.exception_messages.len())..]
    }

    pub(crate) fn pending_fetch_diagnostics(
        &self,
        session_id: &str,
    ) -> &[RendererServiceWorkerFetchDiagnostic] {
        let Some(state) = self.session_state(session_id) else {
            return &[];
        };
        if !state.network_session_state.network_enabled {
            return &[];
        }
        &self.fetch_diagnostics[state
            .network_session_state
            .service_worker_fetch_diagnostic_entries
            .min(self.fetch_diagnostics.len())..]
    }

    pub(crate) fn mark_runtime_console_emitted(&mut self, session_id: &str, console_end: usize) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.runtime_console_entries =
                console_end.min(console_len);
        }
    }

    pub(crate) fn mark_classic_log_emitted(&mut self, cursor_id: String, console_end: usize) {
        self.classic_log_cursors
            .insert(cursor_id, console_end.min(self.console_messages.len()));
    }

    pub(crate) fn mark_runtime_exception_emitted(
        &mut self,
        session_id: &str,
        exception_end: usize,
    ) {
        let exception_len = self.exception_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.runtime_exception_entries =
                exception_end.min(exception_len);
        }
    }

    pub(crate) fn mark_fetch_diagnostics_emitted(
        &mut self,
        session_id: &str,
        diagnostic_end: usize,
    ) {
        let diagnostic_len = self.fetch_diagnostics.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state
                .network_session_state
                .service_worker_fetch_diagnostic_entries = diagnostic_end.min(diagnostic_len);
        }
    }

    pub(crate) fn console_message_count(&self) -> usize {
        self.console_messages.len()
    }

    pub(crate) fn exception_message_count(&self) -> usize {
        self.exception_messages.len()
    }

    pub(crate) fn fetch_diagnostic_count(&self) -> usize {
        self.fetch_diagnostics.len()
    }

    #[cfg(test)]
    pub(crate) fn register_pending_inspector_await(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) {
        self.try_register_pending_inspector_await(
            owner_session_id,
            cdp_request_id,
            session_id,
            object_group,
        )
        .expect("pending Inspector await frontend command id must be unique per session");
    }

    pub(crate) fn try_register_pending_inspector_await(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let Some(state) = self.session_state_mut(owner_session_id) else {
            return Ok(());
        };
        state
            .pending_inspector_awaits
            .try_insert(cdp_request_id, session_id, object_group)
    }

    pub(crate) fn try_register_renderer_call(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Option<Result<PreparedRendererCallDispatch, RegisterRendererCallError>> {
        Some(
            self.session_state_mut(owner_session_id)?
                .try_register_renderer_call(cdp_request_id, dispatched_attachment_id, descriptor),
        )
    }

    pub(crate) fn take_renderer_call_for_frontend(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state_mut(owner_session_id)?
            .take_renderer_call_for_frontend(cdp_request_id)
    }

    pub(crate) fn renderer_call_for_frontend(
        &self,
        owner_session_id: &str,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state(owner_session_id)?
            .renderer_call_for_frontend(cdp_request_id)
    }

    pub(crate) fn take_renderer_call_for_frontend_if_matches(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state_mut(owner_session_id)?
            .take_renderer_call_for_frontend_if_matches(
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn take_frontend_command_for_renderer_if_attachment_matches(
        &mut self,
        owner_session_id: &str,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state_mut(owner_session_id)?
            .take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn has_pending_inspector_awaits(&self) -> bool {
        self.sessions
            .values()
            .any(|state| !state.devtools.pending_inspector_awaits.is_empty())
    }

    pub(crate) fn has_pending_inspector_awaits_for_session(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| !state.pending_inspector_awaits.is_empty())
    }

    pub(crate) fn pending_inspector_await_count_all_sessions(&self) -> usize {
        self.sessions
            .values()
            .map(|state| state.devtools.pending_inspector_awaits.len())
            .sum()
    }

    pub(crate) fn drain_pending_inspector_awaits_for_session(
        &mut self,
        session_id: &str,
    ) -> Vec<(u64, PendingInspectorAwait)> {
        self.session_state_mut(session_id)
            .map(|state| state.pending_inspector_awaits.drain_all())
            .unwrap_or_default()
    }

    pub(crate) fn drain_pending_inspector_awaits(&mut self) -> Vec<(u64, PendingInspectorAwait)> {
        self.sessions
            .values_mut()
            .flat_map(|state| state.devtools.pending_inspector_awaits.drain_all())
            .collect()
    }

    pub(crate) fn terminate_renderer_calls_for_session(
        &mut self,
        session_id: &str,
        reason: &str,
    ) -> Vec<RendererCommandCorrelation> {
        self.session_state_mut(session_id)
            .map(|state| state.terminate_all_renderer_calls(reason))
            .unwrap_or_default()
    }

    pub(crate) fn terminate_renderer_calls(
        &mut self,
        reason: &str,
    ) -> Vec<(String, RendererCommandCorrelation)> {
        self.sessions
            .iter_mut()
            .flat_map(|(session_id, state)| {
                state
                    .devtools
                    .terminate_all_renderer_calls(reason)
                    .into_iter()
                    .map(|correlation| (session_id.clone(), correlation))
            })
            .collect()
    }

    pub(crate) fn remove_pending_inspector_await(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
    ) -> Option<PendingInspectorAwait> {
        self.session_state_mut(owner_session_id)?
            .pending_inspector_awaits
            .remove(cdp_request_id)
    }

    fn current_run_identity(
        &self,
        browser_context_id: &str,
    ) -> Option<TargetServiceWorkerRunIdentity> {
        let ServiceWorkerTargetRunState::Live { run, .. } = &self.run_state else {
            return None;
        };
        Some(run.scope.bind(self.version_identity(browser_context_id)?))
    }

    fn running_status(&self) -> ServiceWorkerTargetRunningStatus {
        match self.run_state {
            ServiceWorkerTargetRunState::Live {
                phase: ServiceWorkerTargetLiveRunPhase::Starting,
                ..
            } => ServiceWorkerTargetRunningStatus::Starting,
            ServiceWorkerTargetRunState::Live {
                phase: ServiceWorkerTargetLiveRunPhase::Running,
                ..
            } => ServiceWorkerTargetRunningStatus::Running,
            ServiceWorkerTargetRunState::Stopped { .. } => {
                ServiceWorkerTargetRunningStatus::Stopped
            }
        }
    }

    fn session_state(&self, session_id: &str) -> Option<&DevToolsSessionState> {
        self.sessions.get(session_id).map(|state| &state.devtools)
    }

    fn session_state_mut(&mut self, session_id: &str) -> Option<&mut DevToolsSessionState> {
        self.sessions
            .get_mut(session_id)
            .map(|state| &mut state.devtools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ServiceWorkerTargetState {
        ServiceWorkerTargetState::new(
            41,
            7,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        )
    }

    fn target_with_started_run() -> (ServiceWorkerTargetState, RendererServiceWorkerRunIdentity) {
        let mut target = target();
        let renderer_run = RendererServiceWorkerRunIdentity::fresh();
        target
            .mark_worker_started("BID-1", renderer_run.clone())
            .expect("test renderer run should start");
        (target, renderer_run)
    }

    fn service_worker_context_created_event(
        context_id: i64,
        context_type: &str,
    ) -> RuntimeExecutionContextEvent {
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: None,
            frame_id: None,
            origin: None,
            name: None,
            is_default: None,
            context_type: Some(context_type.to_owned()),
            grant_universal_access: None,
        }
    }

    fn context_destroyed_event(context_id: i64) -> RuntimeExecutionContextEvent {
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: None,
            frame_id: None,
            origin: None,
            name: None,
            is_default: None,
            context_type: None,
            grant_universal_access: None,
        }
    }

    #[test]
    fn records_runtime_execution_context_from_worker_aux_types() {
        let mut target = target();

        target.record_runtime_execution_context_created_event(
            &service_worker_context_created_event(9101, "service-worker"),
        );
        assert_eq!(target.execution_context_id(), 9101);

        target.record_runtime_execution_context_destroyed_event(&context_destroyed_event(9101));
        assert_eq!(target.execution_context_id(), -20_000_007);

        target.record_runtime_execution_context_created_event(
            &service_worker_context_created_event(9102, "worker"),
        );
        assert_eq!(target.execution_context_id(), 9102);
    }

    #[test]
    fn real_runtime_context_rebinds_synthetic_console_and_exception_snapshots() {
        let mut target = target();
        target.attach_session("SID-service-worker".to_owned());
        target.set_runtime_frontend_enabled("SID-service-worker", true);
        target.record_console_message("log: before context".to_owned(), Vec::new(), None);
        target.record_exception_message(RendererServiceWorkerExceptionMessage {
            message: "Uncaught Error: before context".to_owned(),
            filename: "https://example.test/service-worker.js".to_owned(),
            lineno: 1,
            colno: 1,
            event_kind: "error_event".to_owned(),
            phase: "runtime".to_owned(),
            source: "runtime".to_owned(),
        });

        assert!(
            target
                .pending_runtime_console_messages("SID-service-worker")
                .is_empty(),
            "Runtime console messages must wait for a real renderer context id"
        );
        assert!(
            target
                .pending_runtime_exception_messages("SID-service-worker")
                .is_empty(),
            "Runtime exception messages must wait for a real renderer context id"
        );

        target.record_runtime_execution_context_created_event(
            &service_worker_context_created_event(9101, "service-worker"),
        );

        assert_eq!(
            target.pending_runtime_console_messages("SID-service-worker")[0].execution_context_id,
            9101
        );
        assert_eq!(
            target.pending_runtime_exception_messages("SID-service-worker")[0].execution_context_id,
            9101
        );
    }

    #[test]
    fn stopping_a_run_preserves_the_version_and_protocol_attachment() {
        let (mut target, renderer_run) = target_with_started_run();
        target.attach_session("SID-service-worker".to_owned());
        let version = target
            .version_identity("BID-1")
            .expect("target should own a live version");
        let attachment = target
            .protocol_attachment_identity("BID-1", "SID-service-worker")
            .expect("session should own a live attachment");
        let runtime = target
            .runtime_attachment_identity_for_current_run("BID-1", "SID-service-worker")
            .expect("initial worker run should be live");

        let retirement = target
            .mark_worker_stopped("BID-1", renderer_run.clone(), "idle_timeout")
            .expect("initial run should retire");

        assert!(version.is_current());
        assert!(attachment.is_current());
        assert!(
            runtime.is_current(),
            "accepted output remains valid until its ordered run retirement drains"
        );
        assert!(
            target
                .runtime_attachment_identity_for_current_run("BID-1", "SID-service-worker",)
                .is_none(),
            "the registry must stop authorizing new output for a retired run"
        );
        assert!(
            target.observe_worker_run("BID-1", renderer_run).is_none(),
            "a late event must not recreate the retired exact run"
        );

        retirement.retire();

        assert!(!runtime.is_current());
        assert!(version.is_current());
        assert!(attachment.is_current());
    }

    #[test]
    fn restart_rotates_only_the_worker_run_identity() {
        let (mut target, old_renderer_run) = target_with_started_run();
        target.attach_session("SID-service-worker".to_owned());
        let version = target
            .version_identity("BID-1")
            .expect("target should own a live version");
        let attachment = target
            .protocol_attachment_identity("BID-1", "SID-service-worker")
            .expect("session should own a live attachment");
        let old_runtime = target
            .runtime_attachment_identity_for_current_run("BID-1", "SID-service-worker")
            .expect("initial worker run should be live");
        let old_retirement = target
            .mark_worker_stopped("BID-1", old_renderer_run, "idle_timeout")
            .expect("initial run should retire");

        let new_renderer_run = RendererServiceWorkerRunIdentity::fresh();
        let new_run = target
            .mark_worker_started("BID-1", new_renderer_run)
            .expect("the next exact renderer identity should open a new run");
        let new_runtime = target
            .runtime_attachment_identity_for_run("BID-1", "SID-service-worker", &new_run)
            .expect("the existing session should observe the new run");

        assert_ne!(old_runtime, new_runtime);
        assert!(
            old_runtime.is_current(),
            "the old run remains drainable until its retirement output"
        );
        assert!(new_runtime.is_current());
        assert!(version.is_current());
        assert!(attachment.is_current());

        old_retirement.retire();

        assert!(!old_runtime.is_current());
        assert!(new_runtime.is_current());
        assert!(version.is_current());
        assert!(attachment.is_current());
    }

    #[test]
    fn a_failed_restart_can_stop_a_new_run_without_started() {
        let (mut target, first_run) = target_with_started_run();
        let first_retirement = target
            .mark_worker_stopped("BID-1", first_run, "idle_timeout")
            .expect("initial run should retire");

        let failed_run = RendererServiceWorkerRunIdentity::fresh();
        let failed_restart = target
            .mark_worker_stopped("BID-1", failed_run, "script_load_failed")
            .expect("a new exact run stop should represent a failed unseen restart");

        assert_eq!(target.running_status_cdp_str(), "stopped");
        first_retirement.retire();
        failed_restart.retire();
    }

    #[test]
    #[should_panic(expected = "must not replace a live protocol run")]
    fn a_different_renderer_identity_cannot_replace_a_live_protocol_run() {
        let (mut target, _) = target_with_started_run();

        let _ = target.observe_worker_run("BID-1", RendererServiceWorkerRunIdentity::fresh());
    }

    #[test]
    fn version_update_does_not_manufacture_a_run() {
        let mut target = target();
        target.attach_session("SID-service-worker".to_owned());

        assert!(
            target
                .update_version_status("BID-1", RendererServiceWorkerVersionStatus::Activating,)
                .is_some()
        );
        assert_eq!(target.running_status_cdp_str(), "stopped");
        assert!(
            target
                .runtime_attachment_identity_for_current_run("BID-1", "SID-service-worker",)
                .is_none(),
            "a version-level fact must not manufacture a V8 worker run"
        );

        let renderer_run = RendererServiceWorkerRunIdentity::fresh();
        assert!(
            target.mark_worker_started("BID-1", renderer_run).is_some(),
            "only a concrete exact-run fact should create the run"
        );
        assert_eq!(target.running_status_cdp_str(), "running");
        assert!(
            target
                .runtime_attachment_identity_for_current_run("BID-1", "SID-service-worker",)
                .is_some()
        );
    }
}

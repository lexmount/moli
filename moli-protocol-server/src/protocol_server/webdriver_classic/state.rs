use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::extract::ws::WebSocket;
use moli_cookie_jar::StoredCookie;
use moli_core::{page::RendererDocumentLifecycleMilestone, runtime::NavigationRuntimeConfig};
use moli_protocol::{
    CdpInitialStoragePartition, DevToolsPageResidenceIdentity,
    devtools_runtime::{
        DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsDomNodeReference,
        DevToolsError, DevToolsErrorKind, DevToolsFrameId, DevToolsGetFrameOwnerCommand,
        DevToolsGetFrameOwnerResult, DevToolsGetFrameTreeCommand, DevToolsProtocol,
        DevToolsSessionId, DevToolsTargetId, DevToolsTerminateExecutionCommand,
    },
};
use moli_protocol_webdriver_classic::{
    ClassicActionTick, ClassicDevToolsCommandContext, ClassicElementOriginViewportPoints,
    ClassicError, ClassicErrorCode, ClassicPageLoadStrategy, ClassicSessionRegistry,
    ClassicTimeouts, ClassicUnhandledPromptBehavior, ClassicViewportBounds, ClassicWindowPosition,
    cdp_node_id_from_classic_element_id, cdp_node_id_from_classic_shadow_root_id,
    perform_actions_ticks_with_state_and_viewport,
    release_actions_commands as build_release_actions_commands,
};
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use crate::cdp_scheduler::{
    CdpScheduler, CdpSchedulerEventReceivers, DevToolsCommandExecution,
    DevToolsPageCommandExecution, ProtocolAdapterScheduler,
};

use super::super::webdriver_bidi::{
    BidiSocketActor, BidiSocketActorInput, SharedBidiSessionRegistry,
};
use super::super::{CookieProfileCommit, protocol_local_executor::spawn_protocol_local_task};

const CLASSIC_SCRIPT_TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub(in crate::protocol_server) struct SharedClassicSessionRegistry {
    inner: Arc<Mutex<ClassicSessionManager>>,
}

impl SharedClassicSessionRegistry {
    pub(super) fn lock(&self) -> parking_lot::MutexGuard<'_, ClassicSessionManager> {
        self.inner.lock()
    }

    pub(in crate::protocol_server) fn has_session(&self, session_id: &str) -> bool {
        self.inner.lock().has_session(session_id)
    }

    pub(in crate::protocol_server) fn file_prompt_handler_for_bidi_script_commands(
        &self,
        session_id: &str,
    ) -> Option<&'static str> {
        self.inner
            .lock()
            .file_prompt_handler_for_bidi_script_commands(session_id)
    }

    pub(in crate::protocol_server) fn runtime_handle(
        &self,
        session_id: &str,
    ) -> Option<ClassicSessionRuntimeHandle> {
        self.inner.lock().runtime_handle(session_id)
    }
}

#[derive(Debug, Default)]
pub(super) struct ClassicSessionManager {
    registry: ClassicSessionRegistry,
    runtimes: BTreeMap<String, ClassicSessionRuntimeHandle>,
    next_element_id: u64,
    next_shadow_root_id: u64,
    element_owners: BTreeMap<(String, String), ClassicElementOwner>,
    element_ids_by_owner: BTreeMap<(String, ClassicElementOwner), String>,
    shadow_root_owners: BTreeMap<(String, String), ClassicShadowRootOwner>,
    window_positions: BTreeMap<(String, String), ClassicWindowPosition>,
    uploaded_files: BTreeMap<String, Vec<PathBuf>>,
    download_directories: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClassicElementOwner {
    node_id: u32,
    reference: ClassicPageBoundDomReference,
    target_id: String,
    browsing_context_target_id: String,
}

#[derive(Debug, Clone)]
struct ClassicShadowRootOwner {
    node_id: u32,
    reference: ClassicPageBoundDomReference,
    target_id: String,
    browsing_context_target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ClassicPageBoundDomReference {
    pub(super) page_residence: DevToolsPageResidenceIdentity,
    pub(super) reference: DevToolsDomNodeReference,
}

impl ClassicSessionManager {
    pub(super) fn create_session(
        &mut self,
        page_load_strategy: ClassicPageLoadStrategy,
        unhandled_prompt_behavior: ClassicUnhandledPromptBehavior,
    ) -> moli_protocol_webdriver_classic::ClassicSessionState {
        self.registry
            .create_session_with_capabilities(page_load_strategy, unhandled_prompt_behavior)
    }

    pub(super) fn has_session(&self, session_id: &str) -> bool {
        self.registry.has_session(session_id)
    }

    pub(super) fn session_count(&self) -> usize {
        self.registry.session_count()
    }

    pub(super) fn file_prompt_handler_for_bidi_script_commands(
        &self,
        session_id: &str,
    ) -> Option<&'static str> {
        self.registry
            .unhandled_prompt_behavior(session_id)?
            .file_prompt_handler_for_bidi_script_commands()
    }

    pub(super) fn bind_runtime(
        &mut self,
        session_id: &str,
        target_id: String,
        runtime: ClassicSessionRuntimeHandle,
    ) {
        self.registry
            .set_current_target_id(session_id, target_id.clone());
        self.runtimes.insert(session_id.to_owned(), runtime);
    }

    pub(super) fn runtime_handle(&self, session_id: &str) -> Option<ClassicSessionRuntimeHandle> {
        self.runtimes.get(session_id).cloned()
    }

    pub(super) fn set_current_target_id(
        &mut self,
        session_id: &str,
        target_id: impl Into<String>,
    ) -> bool {
        self.registry.set_current_target_id(session_id, target_id)
    }

    pub(super) fn set_current_frame_id(
        &mut self,
        session_id: &str,
        frame_id: Option<String>,
    ) -> bool {
        self.registry.set_current_frame_id(session_id, frame_id)
    }

    pub(super) fn window_position(
        &self,
        session_id: &str,
        target_id: &str,
    ) -> ClassicWindowPosition {
        self.window_positions
            .get(&(session_id.to_owned(), target_id.to_owned()))
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn set_window_position(
        &mut self,
        session_id: &str,
        target_id: &str,
        position: ClassicWindowPosition,
    ) -> bool {
        if !self.registry.has_session(session_id) {
            return false;
        }
        self.window_positions
            .insert((session_id.to_owned(), target_id.to_owned()), position);
        true
    }

    pub(super) fn remove_window_position(&mut self, session_id: &str, target_id: &str) {
        self.window_positions
            .remove(&(session_id.to_owned(), target_id.to_owned()));
    }

    pub(super) fn register_uploaded_file(&mut self, session_id: &str, path: PathBuf) -> bool {
        if !self.registry.has_session(session_id) {
            return false;
        }
        self.uploaded_files
            .entry(session_id.to_owned())
            .or_default()
            .push(path);
        true
    }

    pub(super) fn register_download_directory(&mut self, session_id: &str, path: PathBuf) -> bool {
        if !self.registry.has_session(session_id) {
            return false;
        }
        self.download_directories
            .insert(session_id.to_owned(), path);
        true
    }

    pub(super) fn download_directory(&self, session_id: &str) -> Option<PathBuf> {
        if !self.registry.has_session(session_id) {
            return None;
        }
        self.download_directories.get(session_id).cloned()
    }

    pub(super) fn register_element_reference(
        &mut self,
        binding: &ClassicSessionBinding,
        node_id: u32,
        reference: ClassicPageBoundDomReference,
    ) -> String {
        let owner = ClassicElementOwner {
            node_id,
            reference,
            target_id: binding.target_id.clone(),
            browsing_context_target_id: binding.browsing_context_target_id().to_owned(),
        };
        let owner_key = (binding.session_id.clone(), owner.clone());
        if let Some(element_id) = self.element_ids_by_owner.get(&owner_key) {
            return element_id.clone();
        }

        self.next_element_id += 1;
        let element_id = format!("moli-node-{node_id}-element-{}", self.next_element_id);
        self.element_owners
            .insert((binding.session_id.clone(), element_id.clone()), owner);
        self.element_ids_by_owner
            .insert(owner_key, element_id.clone());
        element_id
    }

    pub(super) fn register_shadow_root_reference(
        &mut self,
        binding: &ClassicSessionBinding,
        node_id: u32,
        reference: ClassicPageBoundDomReference,
    ) -> String {
        if let Some(((_, shadow_root_id), _)) =
            self.shadow_root_owners
                .iter()
                .find(|((session_id, _), owner)| {
                    session_id == &binding.session_id
                        && owner.node_id == node_id
                        && owner.reference == reference
                        && owner.target_id == binding.target_id
                        && owner.browsing_context_target_id == binding.browsing_context_target_id()
                })
        {
            return shadow_root_id.clone();
        }

        self.next_shadow_root_id += 1;
        let shadow_root_id = format!("moli-shadow-{node_id}-shadow-{}", self.next_shadow_root_id);
        self.shadow_root_owners.insert(
            (binding.session_id.clone(), shadow_root_id.clone()),
            ClassicShadowRootOwner {
                node_id,
                reference,
                target_id: binding.target_id.clone(),
                browsing_context_target_id: binding.browsing_context_target_id().to_owned(),
            },
        );
        shadow_root_id
    }

    pub(super) fn resolve_element_reference(
        &self,
        binding: &ClassicSessionBinding,
        element_id: &str,
    ) -> Result<ClassicPageBoundDomReference, ClassicError> {
        Ok(self
            .resolve_element_owner(binding, element_id)?
            .reference
            .clone())
    }

    fn resolve_element_owner(
        &self,
        binding: &ClassicSessionBinding,
        element_id: &str,
    ) -> Result<&ClassicElementOwner, ClassicError> {
        cdp_node_id_from_classic_element_id(element_id)?;
        let Some(owner) = self
            .element_owners
            .get(&(binding.session_id.clone(), element_id.to_owned()))
        else {
            return Err(ClassicError::new(
                ClassicErrorCode::NoSuchElement,
                "element not found",
            ));
        };
        if owner.target_id != binding.target_id
            || owner.browsing_context_target_id != binding.browsing_context_target_id()
        {
            return Err(ClassicError::new(
                ClassicErrorCode::NoSuchElement,
                "element not found in the current browsing context",
            ));
        }
        Ok(owner)
    }

    pub(super) fn resolve_shadow_root_reference(
        &self,
        binding: &ClassicSessionBinding,
        shadow_root_id: &str,
    ) -> Result<ClassicPageBoundDomReference, ClassicError> {
        Ok(self
            .resolve_shadow_root_owner(binding, shadow_root_id)?
            .reference
            .clone())
    }

    fn resolve_shadow_root_owner(
        &self,
        binding: &ClassicSessionBinding,
        shadow_root_id: &str,
    ) -> Result<&ClassicShadowRootOwner, ClassicError> {
        cdp_node_id_from_classic_shadow_root_id(shadow_root_id)?;
        let Some(owner) = self
            .shadow_root_owners
            .get(&(binding.session_id.clone(), shadow_root_id.to_owned()))
        else {
            return Err(ClassicError::new(
                ClassicErrorCode::NoSuchShadowRoot,
                "shadow root not found",
            ));
        };
        if owner.target_id != binding.target_id
            || owner.browsing_context_target_id != binding.browsing_context_target_id()
        {
            return Err(ClassicError::new(
                ClassicErrorCode::NoSuchShadowRoot,
                "shadow root not found",
            ));
        }
        Ok(owner)
    }

    pub(super) fn timeouts(&self, session_id: &str) -> Option<ClassicTimeouts> {
        self.registry.timeouts(session_id)
    }

    pub(super) fn set_timeouts(&mut self, session_id: &str, timeouts: ClassicTimeouts) -> bool {
        self.registry.set_timeouts(session_id, timeouts)
    }

    pub(super) fn perform_actions_ticks(
        &mut self,
        session_id: &str,
        context: &ClassicDevToolsCommandContext,
        params: &serde_json::Value,
        element_origins: &ClassicElementOriginViewportPoints,
        viewport_bounds: ClassicViewportBounds,
    ) -> Result<Vec<ClassicActionTick>, ClassicError> {
        let Some(action_state) = self.registry.action_state_mut(session_id) else {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        };
        perform_actions_ticks_with_state_and_viewport(
            context,
            params,
            element_origins,
            Some(viewport_bounds),
            action_state,
        )
    }

    pub(super) fn release_actions_commands(
        &mut self,
        session_id: &str,
        context: &ClassicDevToolsCommandContext,
    ) -> Result<Vec<DevToolsCommand>, ClassicError> {
        let Some(action_state) = self.registry.action_state_mut(session_id) else {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        };
        Ok(build_release_actions_commands(context, action_state))
    }

    pub(super) fn session_binding(&self, session_id: &str) -> Option<ClassicSessionBinding> {
        let target_id = self.registry.current_target_id(session_id)?.to_owned();
        let current_frame_id = self
            .registry
            .current_frame_id(session_id)?
            .map(str::to_owned);
        let timeouts = self.registry.timeouts(session_id)?;
        let page_load_strategy = self.registry.page_load_strategy(session_id)?;
        let unhandled_prompt_behavior = self.registry.unhandled_prompt_behavior(session_id)?;
        let runtime = self.runtimes.get(session_id)?.clone();
        Some(ClassicSessionBinding {
            session_id: session_id.to_owned(),
            target_id,
            current_frame_id,
            timeouts,
            page_load_strategy,
            unhandled_prompt_behavior,
            runtime,
        })
    }

    pub(super) fn release_session(
        &mut self,
        session_id: &str,
    ) -> Option<ClassicSessionRuntimeHandle> {
        if !self.registry.release_session(session_id) {
            return None;
        }
        self.element_owners
            .retain(|(owner_session_id, _), _| owner_session_id != session_id);
        self.element_ids_by_owner
            .retain(|(owner_session_id, _), _| owner_session_id != session_id);
        self.shadow_root_owners
            .retain(|(owner_session_id, _), _| owner_session_id != session_id);
        self.window_positions
            .retain(|(owner_session_id, _), _| owner_session_id != session_id);
        if let Some(paths) = self.uploaded_files.remove(session_id) {
            for path in paths {
                let _ = fs::remove_file(&path);
                if let Some(parent) = path.parent() {
                    let _ = fs::remove_dir(parent);
                }
            }
        }
        if let Some(path) = self.download_directories.remove(session_id) {
            let _ = fs::remove_dir_all(path);
        }
        self.runtimes.remove(session_id)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ClassicSessionBinding {
    pub(super) session_id: String,
    pub(super) target_id: String,
    pub(super) current_frame_id: Option<String>,
    pub(super) timeouts: ClassicTimeouts,
    pub(super) page_load_strategy: ClassicPageLoadStrategy,
    pub(super) unhandled_prompt_behavior: ClassicUnhandledPromptBehavior,
    pub(super) runtime: ClassicSessionRuntimeHandle,
}

impl ClassicSessionBinding {
    pub(super) fn browsing_context_target_id(&self) -> &str {
        self.current_frame_id.as_deref().unwrap_or(&self.target_id)
    }
}

#[derive(Debug, Clone)]
pub(in crate::protocol_server) struct ClassicSessionRuntimeHandle {
    tx: mpsc::UnboundedSender<ClassicSessionRuntimeRequest>,
}

impl ClassicSessionRuntimeHandle {
    pub(super) fn spawn(
        initial_cookie_snapshot: Vec<StoredCookie>,
        initial_storage_partition: CdpInitialStoragePartition,
        navigation_runtime_config: NavigationRuntimeConfig,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _runtime_finished_rx = spawn_protocol_local_task("classic-session", move || {
            classic_session_runtime_loop(
                rx,
                initial_cookie_snapshot,
                initial_storage_partition,
                navigation_runtime_config,
            )
        });
        Self { tx }
    }

    pub(super) async fn execute(
        &self,
        command: DevToolsCommand,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_inner(command, None).await
    }

    pub(super) async fn execute_inner(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_with_options(command, timeout, None, false)
            .await
    }

    pub(super) async fn execute_with_pending_navigation_wait(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
        pending_navigation_timeout: Option<Duration>,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_with_options(command, timeout, pending_navigation_timeout, false)
            .await
    }

    pub(super) async fn execute_with_pending_navigation_wait_on_page(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
        pending_navigation_timeout: Option<Duration>,
        expected_page: DevToolsPageResidenceIdentity,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_request(
            command,
            timeout,
            pending_navigation_timeout,
            false,
            Some(expected_page),
        )
        .await
        .result
    }

    pub(super) async fn wait_for_document_lifecycle(
        &self,
        context: DevToolsCommandContext,
        milestone: RendererDocumentLifecycleMilestone,
        timeout: Option<Duration>,
    ) -> Result<(), DevToolsError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ClassicSessionRuntimeRequest::WaitForDocumentLifecycle {
                context,
                milestone,
                timeout,
                response_tx,
            })
            .map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchSession,
                    "Classic session runtime is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before document lifecycle wait completed",
            )
        })?
    }

    async fn execute_with_options(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
        pending_navigation_timeout: Option<Duration>,
        terminate_execution_on_timeout: bool,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_request(
            command,
            timeout,
            pending_navigation_timeout,
            terminate_execution_on_timeout,
            None,
        )
        .await
        .result
    }

    pub(super) async fn execute_with_page_residence(
        &self,
        command: DevToolsCommand,
    ) -> Result<(DevToolsCommandResult, DevToolsPageResidenceIdentity), DevToolsError> {
        let execution = self.execute_request(command, None, None, false, None).await;
        let result = execution.result?;
        let page_residence = execution.page_residence.ok_or_else(|| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "Classic command did not address a live Page",
            )
        })?;
        Ok((result, page_residence))
    }

    pub(super) async fn execute_script_with_page_residence(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
    ) -> Result<(DevToolsCommandResult, DevToolsPageResidenceIdentity), DevToolsError> {
        let execution = self
            .execute_request(command, timeout, None, true, None)
            .await;
        let result = execution.result?;
        let page_residence = execution.page_residence.ok_or_else(|| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "Classic script did not address a live Page",
            )
        })?;
        Ok((result, page_residence))
    }

    pub(super) async fn execute_on_page(
        &self,
        command: DevToolsCommand,
        expected_page: DevToolsPageResidenceIdentity,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_request(command, None, None, false, Some(expected_page))
            .await
            .result
    }

    pub(super) async fn execute_script_on_page(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
        expected_page: DevToolsPageResidenceIdentity,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_request(command, timeout, None, true, Some(expected_page))
            .await
            .result
    }

    async fn execute_request(
        &self,
        command: DevToolsCommand,
        timeout: Option<Duration>,
        pending_navigation_timeout: Option<Duration>,
        terminate_execution_on_timeout: bool,
        expected_page: Option<DevToolsPageResidenceIdentity>,
    ) -> ClassicSessionRuntimeCommandExecution {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ClassicSessionRuntimeRequest::Execute {
                command: Box::new(command),
                timeout,
                pending_navigation_timeout,
                terminate_execution_on_timeout,
                expected_page,
                response_tx,
            })
            .ok();
        response_rx.await.unwrap_or_else(|_| {
            ClassicSessionRuntimeCommandExecution::error(DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before command completion",
            ))
        })
    }

    pub(super) async fn frame_id_for_index(
        &self,
        session_id: String,
        target_id: String,
        current_frame_id: Option<String>,
        index: usize,
    ) -> Result<String, DevToolsError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ClassicSessionRuntimeRequest::FrameIdForIndex {
                session_id,
                target_id,
                current_frame_id,
                index,
                response_tx,
            })
            .map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchSession,
                    "Classic session runtime is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before frame resolution",
            )
        })?
    }

    pub(super) async fn frame_id_for_element(
        &self,
        session_id: String,
        target_id: String,
        current_frame_id: Option<String>,
        element_reference: ClassicPageBoundDomReference,
    ) -> Result<String, DevToolsError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ClassicSessionRuntimeRequest::FrameIdForElement {
                session_id,
                target_id,
                current_frame_id,
                expected_page: element_reference.page_residence,
                element_reference: element_reference.reference,
                response_tx,
            })
            .map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchSession,
                    "Classic session runtime is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before frame resolution",
            )
        })?
    }

    pub(super) async fn parent_frame_id(
        &self,
        session_id: String,
        target_id: String,
        current_frame_id: String,
    ) -> Result<Option<String>, DevToolsError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ClassicSessionRuntimeRequest::ParentFrameId {
                session_id,
                target_id,
                current_frame_id,
                response_tx,
            })
            .map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchSession,
                    "Classic session runtime is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before frame resolution",
            )
        })?
    }

    pub(super) async fn browsing_context_exists(
        &self,
        session_id: String,
        target_id: String,
        frame_id: Option<String>,
    ) -> Result<bool, DevToolsError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ClassicSessionRuntimeRequest::BrowsingContextExists {
                session_id,
                target_id,
                frame_id,
                response_tx,
            })
            .map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchSession,
                    "Classic session runtime is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before browsing context lookup",
            )
        })?
    }

    pub(super) async fn set_javascript_dialog_handler_enabled(
        &self,
        enabled: bool,
    ) -> Result<(), DevToolsError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(
                ClassicSessionRuntimeRequest::SetJavaScriptDialogHandlerEnabled {
                    enabled,
                    response_tx,
                },
            )
            .map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchSession,
                    "Classic session runtime is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped before dialog handler configuration",
            )
        })?
    }

    pub(super) async fn shutdown(self) -> CookieProfileCommit {
        let (response_tx, response_rx) = oneshot::channel();
        if self
            .tx
            .send(ClassicSessionRuntimeRequest::Shutdown { response_tx })
            .is_err()
        {
            return CookieProfileCommit::unchanged();
        }
        response_rx
            .await
            .unwrap_or_else(|_| CookieProfileCommit::unchanged())
    }

    pub(in crate::protocol_server) async fn attach_bidi_socket(
        &self,
        socket: WebSocket,
        web_socket_url: String,
        session_id: String,
        file_prompt_handler: Option<String>,
        session_registry: SharedBidiSessionRegistry,
    ) -> bool {
        let (response_tx, response_rx) = oneshot::channel();
        if self
            .tx
            .send(ClassicSessionRuntimeRequest::AttachBidiSocket {
                socket: Box::new(socket),
                web_socket_url,
                session_id,
                file_prompt_handler,
                session_registry,
                response_tx,
            })
            .is_err()
        {
            return false;
        }
        response_rx.await.unwrap_or(false)
    }
}

struct ClassicSessionRuntimeCommandExecution {
    result: Result<DevToolsCommandResult, DevToolsError>,
    page_residence: Option<DevToolsPageResidenceIdentity>,
}

impl ClassicSessionRuntimeCommandExecution {
    fn error(error: DevToolsError) -> Self {
        Self {
            result: Err(error),
            page_residence: None,
        }
    }
}

enum ClassicSessionRuntimeRequest {
    Execute {
        command: Box<DevToolsCommand>,
        timeout: Option<Duration>,
        pending_navigation_timeout: Option<Duration>,
        terminate_execution_on_timeout: bool,
        expected_page: Option<DevToolsPageResidenceIdentity>,
        response_tx: oneshot::Sender<ClassicSessionRuntimeCommandExecution>,
    },
    WaitForDocumentLifecycle {
        context: DevToolsCommandContext,
        milestone: RendererDocumentLifecycleMilestone,
        timeout: Option<Duration>,
        response_tx: oneshot::Sender<Result<(), DevToolsError>>,
    },
    AttachBidiSocket {
        socket: Box<WebSocket>,
        web_socket_url: String,
        session_id: String,
        file_prompt_handler: Option<String>,
        session_registry: SharedBidiSessionRegistry,
        response_tx: oneshot::Sender<bool>,
    },
    FrameIdForIndex {
        session_id: String,
        target_id: String,
        current_frame_id: Option<String>,
        index: usize,
        response_tx: oneshot::Sender<Result<String, DevToolsError>>,
    },
    FrameIdForElement {
        session_id: String,
        target_id: String,
        current_frame_id: Option<String>,
        expected_page: DevToolsPageResidenceIdentity,
        element_reference: DevToolsDomNodeReference,
        response_tx: oneshot::Sender<Result<String, DevToolsError>>,
    },
    ParentFrameId {
        session_id: String,
        target_id: String,
        current_frame_id: String,
        response_tx: oneshot::Sender<Result<Option<String>, DevToolsError>>,
    },
    BrowsingContextExists {
        session_id: String,
        target_id: String,
        frame_id: Option<String>,
        response_tx: oneshot::Sender<Result<bool, DevToolsError>>,
    },
    SetJavaScriptDialogHandlerEnabled {
        enabled: bool,
        response_tx: oneshot::Sender<Result<(), DevToolsError>>,
    },
    Shutdown {
        response_tx: oneshot::Sender<CookieProfileCommit>,
    },
}

struct ClassicAttachedBidiSocket {
    actor: BidiSocketActor,
    session_registry: SharedBidiSessionRegistry,
}

impl ClassicAttachedBidiSocket {
    async fn release_session(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) {
        self.actor.release_event_sources(scheduler, receivers).await;
        self.actor
            .release_session(&mut self.session_registry.lock());
    }
}

enum ClassicSessionRuntimeRequestOutcome {
    Continue,
    AttachedBidi(Box<ClassicAttachedBidiSocket>),
    DetachBidi,
    Shutdown(CookieProfileCommit),
}

async fn handle_classic_session_runtime_request(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    initial_cookie_snapshot: &[StoredCookie],
    request: ClassicSessionRuntimeRequest,
    mut attached_bidi: Option<&mut ClassicAttachedBidiSocket>,
) -> ClassicSessionRuntimeRequestOutcome {
    match request {
        ClassicSessionRuntimeRequest::Execute {
            command,
            timeout,
            pending_navigation_timeout,
            terminate_execution_on_timeout,
            expected_page,
            response_tx,
        } => {
            if expected_page.as_ref().is_some_and(|expected| {
                scheduler.page_residence_identity_for_devtools_context(command.context())
                    != Some(expected.clone())
            }) {
                let _ = response_tx.send(ClassicSessionRuntimeCommandExecution::error(
                    DevToolsError::new(
                        DevToolsErrorKind::NoSuchNode,
                        "DOM reference belongs to a replaced Page",
                    ),
                ));
                return ClassicSessionRuntimeRequestOutcome::Continue;
            }
            let termination_context = command.context().clone();
            let mut execution = execute_classic_devtools_command_with_pending_navigation_retry(
                scheduler,
                receivers,
                *command,
                timeout,
                pending_navigation_timeout,
                expected_page.as_ref(),
            )
            .await;
            if terminate_execution_on_timeout
                && matches!(
                    execution.execution.result,
                    Err(ref error) if error.kind == DevToolsErrorKind::Timeout
                )
            {
                // Finish the IO-side termination before the HTTP handler
                // releases argument handles or admits the next Classic
                // command on this session.
                let termination = execute_classic_devtools_command_once(
                    scheduler,
                    receivers,
                    DevToolsCommand::TerminateExecution(DevToolsTerminateExecutionCommand {
                        context: termination_context,
                    }),
                    Some(CLASSIC_SCRIPT_TERMINATION_TIMEOUT),
                    execution.page_residence.as_ref(),
                )
                .await;
                if let Err(error) = &termination.execution.result {
                    tracing::warn!(
                        ?error,
                        "failed to terminate timed-out WebDriver Classic script execution"
                    );
                }
                execution
                    .execution
                    .protocol_output
                    .append(termination.execution.protocol_output);
            }
            let result = execution.execution.result;
            let keep_attached = if let Some(attached) = attached_bidi.as_mut() {
                attached
                    .actor
                    .send_or_route_protocol_output(
                        scheduler,
                        receivers,
                        execution.execution.protocol_output,
                        None,
                    )
                    .await
            } else {
                true
            };
            let _ = response_tx.send(ClassicSessionRuntimeCommandExecution {
                result,
                page_residence: execution.page_residence,
            });
            if keep_attached {
                ClassicSessionRuntimeRequestOutcome::Continue
            } else {
                ClassicSessionRuntimeRequestOutcome::DetachBidi
            }
        }
        ClassicSessionRuntimeRequest::WaitForDocumentLifecycle {
            context,
            milestone,
            timeout,
            response_tx,
        } => {
            let execution = scheduler
                .wait_for_devtools_context_document_lifecycle(
                    receivers, &context, milestone, timeout,
                )
                .await;
            let result = execution.result.map(|_| ());
            let keep_attached = if let Some(attached) = attached_bidi.as_mut() {
                attached
                    .actor
                    .send_or_route_protocol_output(
                        scheduler,
                        receivers,
                        execution.protocol_output,
                        None,
                    )
                    .await
            } else {
                true
            };
            let _ = response_tx.send(result);
            if keep_attached {
                ClassicSessionRuntimeRequestOutcome::Continue
            } else {
                ClassicSessionRuntimeRequestOutcome::DetachBidi
            }
        }
        ClassicSessionRuntimeRequest::AttachBidiSocket {
            socket,
            web_socket_url,
            session_id,
            file_prompt_handler,
            session_registry,
            response_tx,
        } => {
            if attached_bidi.is_some() {
                let _ = response_tx.send(false);
                return ClassicSessionRuntimeRequestOutcome::Continue;
            }
            let mut actor = BidiSocketActor::new(*socket, web_socket_url);
            let attached = {
                let mut registry = session_registry.lock();
                actor.attach_existing_session(session_id, &mut registry)
            };
            if !attached {
                let _ = response_tx.send(false);
                return ClassicSessionRuntimeRequestOutcome::Continue;
            }
            actor.install_runtime_response_ready_sender(scheduler);
            actor.set_file_prompt_handler_for_script_commands(file_prompt_handler.as_deref());
            let _ = response_tx.send(true);
            ClassicSessionRuntimeRequestOutcome::AttachedBidi(Box::new(ClassicAttachedBidiSocket {
                actor,
                session_registry,
            }))
        }
        ClassicSessionRuntimeRequest::FrameIdForIndex {
            session_id,
            target_id,
            current_frame_id,
            index,
            response_tx,
        } => {
            let result = resolve_classic_frame_id_for_index(
                scheduler,
                receivers,
                &session_id,
                &target_id,
                current_frame_id.as_deref(),
                index,
            )
            .await;
            let _ = response_tx.send(result);
            ClassicSessionRuntimeRequestOutcome::Continue
        }
        ClassicSessionRuntimeRequest::FrameIdForElement {
            session_id,
            target_id,
            current_frame_id,
            expected_page,
            element_reference,
            response_tx,
        } => {
            let result = resolve_classic_frame_id_for_element(
                scheduler,
                receivers,
                &session_id,
                &target_id,
                current_frame_id.as_deref(),
                &expected_page,
                element_reference,
            )
            .await;
            let _ = response_tx.send(result);
            ClassicSessionRuntimeRequestOutcome::Continue
        }
        ClassicSessionRuntimeRequest::ParentFrameId {
            session_id,
            target_id,
            current_frame_id,
            response_tx,
        } => {
            let result = resolve_classic_parent_frame_id(
                scheduler,
                receivers,
                &session_id,
                &target_id,
                &current_frame_id,
            )
            .await;
            let _ = response_tx.send(result);
            ClassicSessionRuntimeRequestOutcome::Continue
        }
        ClassicSessionRuntimeRequest::BrowsingContextExists {
            session_id,
            target_id,
            frame_id,
            response_tx,
        } => {
            let result = resolve_classic_browsing_context_exists(
                scheduler,
                receivers,
                &session_id,
                &target_id,
                frame_id.as_deref(),
            )
            .await;
            let _ = response_tx.send(result);
            ClassicSessionRuntimeRequestOutcome::Continue
        }
        ClassicSessionRuntimeRequest::SetJavaScriptDialogHandlerEnabled {
            enabled,
            response_tx,
        } => {
            let result = scheduler
                .set_automation_javascript_dialog_handler_enabled(enabled)
                .then_some(())
                .ok_or_else(|| {
                    DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        "Classic session has no browser context",
                    )
                });
            let _ = response_tx.send(result);
            ClassicSessionRuntimeRequestOutcome::Continue
        }
        ClassicSessionRuntimeRequest::Shutdown { response_tx } => {
            let cookie_commit = CookieProfileCommit::from_optional_profile_backed_snapshot(
                initial_cookie_snapshot.to_vec(),
                scheduler.snapshot_profile_backed_cookies(),
            );
            let _ = response_tx.send(cookie_commit.clone());
            ClassicSessionRuntimeRequestOutcome::Shutdown(cookie_commit)
        }
    }
}

async fn execute_classic_devtools_command_with_pending_navigation_retry(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    command: DevToolsCommand,
    timeout: Option<Duration>,
    pending_navigation_timeout: Option<Duration>,
    expected_page: Option<&DevToolsPageResidenceIdentity>,
) -> ClassicDevToolsCommandExecution {
    let mut execution = execute_classic_devtools_command_once(
        scheduler,
        receivers,
        command.clone(),
        timeout,
        expected_page,
    )
    .await;

    let Some(pending_navigation_timeout) = pending_navigation_timeout else {
        return execution;
    };
    let started = Instant::now();
    loop {
        if !classic_runtime_result_is_navigation_changing_document(&execution.execution.result) {
            return execution;
        }
        let Some(remaining) = pending_navigation_timeout.checked_sub(started.elapsed()) else {
            execution.execution.result = Err(classic_pending_navigation_timeout_error());
            return execution;
        };
        let mut progress = scheduler
            .complete_ready_protocol_residences_for_external_load_wait()
            .await;
        if progress.is_empty() {
            let input = match tokio::time::timeout(remaining, receivers.recv_interleaved_input())
                .await
            {
                Ok(Some(input)) => input,
                Ok(None) => {
                    execution.execution.result = Err(DevToolsError::new(
                        DevToolsErrorKind::NoSuchSession,
                        "Classic session runtime stopped while waiting for navigation",
                    ));
                    return execution;
                }
                Err(_) => {
                    execution.execution.result = Err(classic_pending_navigation_timeout_error());
                    return execution;
                }
            };
            // Once selected, finish the move-owned input outside the timeout
            // race. In particular, an admitted renderer publication must not
            // disappear merely because the navigation deadline expires while
            // its protocol projection is awaiting an owner action.
            progress = match scheduler
                .complete_interleaved_scheduler_input(receivers, input)
                .await
            {
                Ok(progress) => progress,
                Err(failure) => {
                    let (progress, error) = failure.into_parts();
                    execution.execution.protocol_output.append(progress);
                    execution.execution.result = Err(error);
                    return execution;
                }
            };
        }
        execution.execution.protocol_output.append(progress);

        let retry_timeout = match timeout {
            Some(timeout) => {
                let Some(remaining) = pending_navigation_timeout.checked_sub(started.elapsed())
                else {
                    execution.execution.result = Err(classic_pending_navigation_timeout_error());
                    return execution;
                };
                Some(timeout.min(remaining))
            }
            None => None,
        };
        let retry = execute_classic_devtools_command_once(
            scheduler,
            receivers,
            command.clone(),
            retry_timeout,
            expected_page,
        )
        .await;
        execution
            .execution
            .protocol_output
            .append(retry.execution.protocol_output);
        execution.execution.result = retry.execution.result;
        execution.page_residence = retry.page_residence;
    }
}

async fn execute_classic_devtools_command_once(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    command: DevToolsCommand,
    timeout: Option<Duration>,
    expected_page: Option<&DevToolsPageResidenceIdentity>,
) -> ClassicDevToolsCommandExecution {
    let DevToolsPageCommandExecution {
        execution,
        page_residence,
    } = scheduler
        .execute_devtools_command_with_external_load_wait_and_page_residence(
            receivers,
            command,
            timeout,
            expected_page,
        )
        .await;
    ClassicDevToolsCommandExecution {
        execution,
        page_residence,
    }
}

struct ClassicDevToolsCommandExecution {
    execution: DevToolsCommandExecution,
    page_residence: Option<DevToolsPageResidenceIdentity>,
}

fn classic_runtime_result_is_navigation_changing_document(
    result: &Result<DevToolsCommandResult, DevToolsError>,
) -> bool {
    matches!(
        result,
        Err(error) if error.kind == DevToolsErrorKind::NavigationChangingDocument
    )
}

fn classic_pending_navigation_timeout_error() -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Timeout, "navigation wait timed out")
}

async fn classic_session_runtime_loop(
    mut rx: mpsc::UnboundedReceiver<ClassicSessionRuntimeRequest>,
    initial_cookie_snapshot: Vec<StoredCookie>,
    initial_storage_partition: CdpInitialStoragePartition,
    navigation_runtime_config: NavigationRuntimeConfig,
) -> CookieProfileCommit {
    let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
        initial_storage_partition,
        navigation_runtime_config,
    );
    let mut attached_bidi: Option<ClassicAttachedBidiSocket> = None;
    let mut adapter_scheduler = ProtocolAdapterScheduler::<()>::default();
    loop {
        if receivers.renderer_publication_rx.is_closed() {
            break;
        }
        if attached_bidi.is_some() {
            let mut detach_bidi = false;
            let mut shutdown_cookies = None;
            {
                let attached = attached_bidi.as_mut().expect("attached BiDi socket");
                let page_javascript_blocked = scheduler.has_pending_javascript_dialog();
                adapter_scheduler.schedule_turn_if_needed(&scheduler, page_javascript_blocked);
                tokio::select! {
                    biased;
                    completion = receivers.background_navigation_completion_rx.recv() => {
                        let Some(completion) = completion else {
                            break;
                        };
                        if !attached.actor.handle_background_navigation_completion(
                                &mut scheduler,
                                &mut receivers,
                                completion,
                            ).await
                        {
                            detach_bidi = true;
                        }
                    }
                    event = receivers.background_event_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        let output = scheduler.route_background_event_around_inflight_navigation(event);
                        if !attached.actor.send_or_route_protocol_output(
                                &mut scheduler,
                                &mut receivers,
                                output,
                                None,
                            ).await
                        {
                            detach_bidi = true;
                        }
                    }
                    publication = receivers.renderer_publication_rx.recv(), if !page_javascript_blocked => {
                        let Some(publication) = publication else {
                            break;
                        };
                        if !attached.actor.handle_renderer_publication(
                                &mut adapter_scheduler,
                                &mut scheduler,
                                &mut receivers,
                                publication,
                            ).await
                        {
                            detach_bidi = true;
                        }
                    }
                    actor_input = attached.actor.recv_attached_input(
                        &mut adapter_scheduler,
                        page_javascript_blocked,
                    ) => {
                        match actor_input {
                            BidiSocketActorInput::Socket(Some(message)) => {
                                if !attached.actor.handle_socket_message(
                                        &mut scheduler,
                                        &mut receivers,
                                        &attached.session_registry,
                                        message,
                                    ).await
                                {
                                    detach_bidi = true;
                                }
                            }
                            BidiSocketActorInput::Socket(None) => {
                                detach_bidi = true;
                            }
                            BidiSocketActorInput::AdapterScheduler(input) => {
                                if !attached.actor.handle_adapter_scheduler_input(
                                        &mut adapter_scheduler,
                                        &mut scheduler,
                                        &mut receivers,
                                        input,
                                    ).await
                                {
                                    detach_bidi = true;
                                }
                            }
                            BidiSocketActorInput::RuntimeResponseReady(Some(response)) => {
                                if !attached
                                    .actor
                                    .handle_runtime_response_ready(
                                        &mut scheduler,
                                        &mut receivers,
                                        *response,
                                    )
                                    .await
                                {
                                    detach_bidi = true;
                                }
                            }
                            BidiSocketActorInput::RuntimeResponseReady(None) => {
                                detach_bidi = true;
                            }
                        }
                    }
                    request = rx.recv() => {
                        let Some(request) = request else {
                            break;
                        };
                        match handle_classic_session_runtime_request(
                            &mut scheduler,
                            &mut receivers,
                            &initial_cookie_snapshot,
                            request,
                            Some(attached),
                        )
                        .await
                        {
                            ClassicSessionRuntimeRequestOutcome::Continue => {}
                            ClassicSessionRuntimeRequestOutcome::AttachedBidi(mut duplicate) => {
                                duplicate
                                    .release_session(&mut scheduler, &mut receivers)
                                    .await;
                            }
                            ClassicSessionRuntimeRequestOutcome::DetachBidi => {
                                detach_bidi = true;
                            }
                            ClassicSessionRuntimeRequestOutcome::Shutdown(cookies) => {
                                attached
                                    .release_session(&mut scheduler, &mut receivers)
                                    .await;
                                shutdown_cookies = Some(cookies);
                            }
                        }
                    }
                }
            }
            if let Some(cookies) = shutdown_cookies {
                return cookies;
            }
            if detach_bidi && let Some(mut attached) = attached_bidi.take() {
                attached
                    .release_session(&mut scheduler, &mut receivers)
                    .await;
            }
        } else {
            let page_javascript_blocked = scheduler.has_pending_javascript_dialog();
            adapter_scheduler.schedule_turn_if_needed(&scheduler, page_javascript_blocked);
            tokio::select! {
                biased;
                completion = receivers.background_navigation_completion_rx.recv() => {
                    let Some(completion) = completion else {
                        break;
                    };
                    if scheduler
                        .drain_background_navigation_completion_with_progress_barrier(
                            completion,
                            &mut receivers,
                        )
                        .await
                        .is_err()
                    {
                        // No frontend is attached in this branch, but a
                        // renderer-output terminal still retires the shared
                        // protocol owner. Continuing the loop would let later
                        // Classic commands observe a half-delivered runtime.
                        break;
                    }
                }
                event = receivers.background_event_rx.recv() => {
                    let Some(event) = event else {
                        break;
                    };
                    let _ = scheduler.route_background_event_around_inflight_navigation(event);
                }
                publication = receivers.renderer_publication_rx.recv(), if !page_javascript_blocked => {
                    let Some(publication) = publication else {
                        break;
                    };
                    let _ = adapter_scheduler
                        .ingest_renderer_publication(&mut scheduler, publication)
                        .await;
                }
                input = adapter_scheduler.recv_input(), if !page_javascript_blocked => {
                    let _ = adapter_scheduler
                        .advance_input(&mut scheduler, input, || ())
                        .await;
                }
                request = rx.recv() => {
                    let Some(request) = request else {
                        break;
                    };
                    match handle_classic_session_runtime_request(
                        &mut scheduler,
                        &mut receivers,
                        &initial_cookie_snapshot,
                        request,
                        None,
                    )
                    .await
                    {
                        ClassicSessionRuntimeRequestOutcome::Continue => {}
                        ClassicSessionRuntimeRequestOutcome::AttachedBidi(attached) => {
                            attached_bidi = Some(*attached);
                        }
                        ClassicSessionRuntimeRequestOutcome::DetachBidi => {}
                        ClassicSessionRuntimeRequestOutcome::Shutdown(cookies) => return cookies,
                    }
                    if attached_bidi.is_none() {
                        classic_session_ingest_ready_renderer_publications(
                            &mut adapter_scheduler,
                            &mut scheduler,
                            &mut receivers,
                        )
                        .await;
                    }
                }
            }
        }
    }
    CookieProfileCommit::from_optional_profile_backed_snapshot(
        initial_cookie_snapshot,
        scheduler.snapshot_profile_backed_cookies(),
    )
}

async fn resolve_classic_frame_id_for_index(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
    current_frame_id: Option<&str>,
    index: usize,
) -> Result<String, DevToolsError> {
    let frame_tree = classic_frame_tree(scheduler, receivers, session_id, target_id).await?;
    let siblings = match current_frame_id {
        Some(frame_id) => {
            classic_child_frames_for_frame_id(&frame_tree, frame_id).ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "frame not found")
            })?
        }
        None => frame_tree
            .get("childFrames")
            .and_then(ValueExt::as_array_slice)
            .unwrap_or(&[]),
    };
    siblings
        .get(index)
        .and_then(classic_frame_tree_item_frame_id)
        .map(str::to_owned)
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "frame not found"))
}

async fn resolve_classic_frame_id_for_element(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
    current_frame_id: Option<&str>,
    expected_page: &DevToolsPageResidenceIdentity,
    element_reference: DevToolsDomNodeReference,
) -> Result<String, DevToolsError> {
    let frame_tree =
        classic_frame_tree_on_page(scheduler, receivers, session_id, target_id, expected_page)
            .await?;
    let candidate_frames = match current_frame_id {
        Some(frame_id) => {
            classic_child_frames_for_frame_id(&frame_tree, frame_id).ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "frame not found")
            })?
        }
        None => frame_tree
            .get("childFrames")
            .and_then(ValueExt::as_array_slice)
            .unwrap_or(&[]),
    };
    for candidate in candidate_frames {
        let Some(frame_id) = classic_frame_tree_item_frame_id(candidate) else {
            continue;
        };
        let owner = classic_frame_owner_reference_on_page(
            scheduler,
            receivers,
            session_id,
            target_id,
            frame_id,
            expected_page,
        )
        .await?;
        if classic_frame_owner_matches_reference(&owner, &element_reference) {
            return Ok(frame_id.to_owned());
        }
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "frame not found",
    ))
}

async fn classic_frame_owner_reference_on_page(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
    frame_id: &str,
    expected_page: &DevToolsPageResidenceIdentity,
) -> Result<DevToolsGetFrameOwnerResult, DevToolsError> {
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from(session_id)),
        target_id: Some(DevToolsTargetId::from(target_id)),
        browser_context_id: None,
    };
    ensure_classic_command_page_is_current(scheduler, &context, expected_page)?;
    match scheduler
        .execute_devtools_command_with_external_load_wait(
            receivers,
            DevToolsCommand::GetFrameOwner(DevToolsGetFrameOwnerCommand {
                context,
                frame_id: DevToolsFrameId::new(frame_id),
            }),
        )
        .await
    {
        Ok(DevToolsCommandResult::GetFrameOwner(owner)) => Ok(owner),
        Ok(_) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedFrameOwnerResult",
        )),
        Err(error) => Err(error),
    }
}

fn classic_frame_owner_matches_reference(
    owner: &DevToolsGetFrameOwnerResult,
    reference: &DevToolsDomNodeReference,
) -> bool {
    match reference {
        DevToolsDomNodeReference::FrontendNodeId(node_id) => owner.node_id == *node_id,
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => {
            owner.backend_node_id == *backend_node_id
        }
    }
}

async fn resolve_classic_parent_frame_id(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
    current_frame_id: &str,
) -> Result<Option<String>, DevToolsError> {
    let frame_tree = classic_frame_tree(scheduler, receivers, session_id, target_id).await?;
    if classic_frame_exists(&frame_tree, current_frame_id) {
        let parent_frame_id = classic_parent_frame_id_for_frame_id(&frame_tree, current_frame_id);
        Ok(parent_frame_id.filter(|parent_frame_id| parent_frame_id != target_id))
    } else {
        Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "frame not found",
        ))
    }
}

async fn resolve_classic_browsing_context_exists(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
    frame_id: Option<&str>,
) -> Result<bool, DevToolsError> {
    let frame_tree = classic_frame_tree(scheduler, receivers, session_id, target_id).await?;
    Ok(frame_id.is_none_or(|frame_id| classic_frame_exists(&frame_tree, frame_id)))
}

async fn classic_frame_tree(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
) -> Result<serde_json::Value, DevToolsError> {
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from(session_id)),
        target_id: Some(DevToolsTargetId::from(target_id)),
        browser_context_id: None,
    };
    match scheduler
        .execute_devtools_command_with_external_load_wait(
            receivers,
            DevToolsCommand::GetFrameTree(DevToolsGetFrameTreeCommand {
                context,
                max_depth: None,
            }),
        )
        .await
    {
        Ok(DevToolsCommandResult::GetFrameTree(result)) => Ok(result.frame_tree),
        Ok(_) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedFrameTreeResult",
        )),
        Err(error) => Err(error),
    }
}

async fn classic_frame_tree_on_page(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    target_id: &str,
    expected_page: &DevToolsPageResidenceIdentity,
) -> Result<serde_json::Value, DevToolsError> {
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from(session_id)),
        target_id: Some(DevToolsTargetId::from(target_id)),
        browser_context_id: None,
    };
    ensure_classic_command_page_is_current(scheduler, &context, expected_page)?;
    match scheduler
        .execute_devtools_command_with_external_load_wait(
            receivers,
            DevToolsCommand::GetFrameTree(DevToolsGetFrameTreeCommand {
                context,
                max_depth: None,
            }),
        )
        .await
    {
        Ok(DevToolsCommandResult::GetFrameTree(result)) => Ok(result.frame_tree),
        Ok(_) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedFrameTreeResult",
        )),
        Err(error) => Err(error),
    }
}

fn ensure_classic_command_page_is_current(
    scheduler: &mut CdpScheduler,
    context: &DevToolsCommandContext,
    expected_page: &DevToolsPageResidenceIdentity,
) -> Result<(), DevToolsError> {
    if scheduler.page_residence_identity_for_devtools_context(context)
        == Some(expected_page.clone())
    {
        Ok(())
    } else {
        Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "DOM reference belongs to a replaced Page",
        ))
    }
}

fn classic_child_frames_for_frame_id<'a>(
    frame_tree: &'a serde_json::Value,
    frame_id: &str,
) -> Option<&'a [serde_json::Value]> {
    if classic_frame_tree_item_frame_id(frame_tree) == Some(frame_id) {
        return Some(
            frame_tree
                .get("childFrames")
                .and_then(ValueExt::as_array_slice)
                .unwrap_or(&[]),
        );
    }
    for child in frame_tree
        .get("childFrames")
        .and_then(ValueExt::as_array_slice)
        .unwrap_or(&[])
    {
        if let Some(children) = classic_child_frames_for_frame_id(child, frame_id) {
            return Some(children);
        }
    }
    None
}

fn classic_frame_exists(frame_tree: &serde_json::Value, frame_id: &str) -> bool {
    classic_frame_tree_item_frame_id(frame_tree) == Some(frame_id)
        || frame_tree
            .get("childFrames")
            .and_then(ValueExt::as_array_slice)
            .unwrap_or(&[])
            .iter()
            .any(|child| classic_frame_exists(child, frame_id))
}

fn classic_parent_frame_id_for_frame_id(
    frame_tree: &serde_json::Value,
    frame_id: &str,
) -> Option<String> {
    for child in frame_tree
        .get("childFrames")
        .and_then(ValueExt::as_array_slice)
        .unwrap_or(&[])
    {
        if classic_frame_tree_item_frame_id(child) == Some(frame_id) {
            return classic_frame_tree_item_frame_id(frame_tree).map(str::to_owned);
        }
        if let Some(parent) = classic_parent_frame_id_for_frame_id(child, frame_id) {
            return Some(parent);
        }
    }
    None
}

fn classic_frame_tree_item_frame_id(frame_tree_item: &serde_json::Value) -> Option<&str> {
    frame_tree_item
        .get("frame")
        .and_then(|frame| frame.get("id"))
        .and_then(serde_json::Value::as_str)
}

trait ValueExt {
    fn as_array_slice(&self) -> Option<&[serde_json::Value]>;
}

impl ValueExt for serde_json::Value {
    fn as_array_slice(&self) -> Option<&[serde_json::Value]> {
        self.as_array().map(Vec::as_slice)
    }
}

async fn classic_session_ingest_ready_renderer_publications(
    adapter_scheduler: &mut ProtocolAdapterScheduler<()>,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) {
    while let Ok(publication) = receivers.renderer_publication_rx.try_recv() {
        let _ = adapter_scheduler
            .ingest_renderer_publication(scheduler, publication)
            .await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_owner_matching_keeps_frontend_and_backend_ids_disjoint() {
        let owner = DevToolsGetFrameOwnerResult {
            node_id: 42,
            backend_node_id: 2_000_000_042,
        };

        assert!(classic_frame_owner_matches_reference(
            &owner,
            &DevToolsDomNodeReference::FrontendNodeId(42)
        ));
        assert!(!classic_frame_owner_matches_reference(
            &owner,
            &DevToolsDomNodeReference::FrontendNodeId(2_000_000_042)
        ));
        assert!(classic_frame_owner_matches_reference(
            &owner,
            &DevToolsDomNodeReference::BackendNodeId(2_000_000_042)
        ));
        assert!(!classic_frame_owner_matches_reference(
            &owner,
            &DevToolsDomNodeReference::BackendNodeId(42)
        ));
    }
}

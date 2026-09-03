use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionStateMut};
use super::*;
use crate::CdpRendererOwnerTurnOutcome;
use moli_core::page::V8InspectorSessionState;
use serde_json::json;

pub(crate) enum SessionOwnerInspectorEnableResult {
    Handled,
    TargetCrashed { event_session_id: Option<String> },
    UnknownSession,
}

pub(crate) enum SessionOwnerRuntimeFrontendEnableResult {
    Handled,
    UnknownSession,
}

impl TargetSessionStateMut<'_> {
    fn set_runtime_frontend_enabled(mut self, enabled: bool) {
        let state = self.runtime_session_state_mut();
        state.runtime_frontend_enabled = enabled;
        if !enabled {
            state.runtime_contexts_reported_to_frontend = false;
        }
    }

    fn set_inspector_enabled(mut self, enabled: bool) {
        self.runtime_session_state_mut().inspector_enabled = enabled;
    }
}

impl TargetSessionOwnerMut<'_> {
    fn set_inspector_enabled(mut self, enabled: bool) {
        self.mutate_session_state_ref(|state| state.set_inspector_enabled(enabled));
    }

    fn set_runtime_frontend_enabled(
        mut self,
        enabled: bool,
    ) -> SessionOwnerRuntimeFrontendEnableResult {
        self.mutate_session_state_ref(|state| state.set_runtime_frontend_enabled(enabled));
        SessionOwnerRuntimeFrontendEnableResult::Handled
    }
}

impl CdpConnection {
    pub fn target_is_current_active_target(&self, target_id: &str) -> bool {
        self.browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id())
            .is_some_and(|active_target_id| active_target_id == target_id)
    }

    pub(crate) fn set_inspector_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> SessionOwnerInspectorEnableResult {
        let accepts_without_target =
            self.accepts_unmaterialized_page_command_for_session(session_id);
        let target_crashed = self
            .target_owner_state_for_session(session_id)
            .is_some_and(|owner_state| owner_state.target_crash_state.is_crashed());
        let primary_session_id = self.runtime_session_owner_primary_session_id(session_id);
        if self
            .with_target_session_owner_mut(session_id, |owner| owner.set_inspector_enabled(enabled))
            .is_none()
        {
            return if accepts_without_target {
                SessionOwnerInspectorEnableResult::Handled
            } else {
                SessionOwnerInspectorEnableResult::UnknownSession
            };
        }

        if enabled && target_crashed {
            let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
                state
                    .runtime_session_state
                    .record_inspector_target_crashed();
            });
            return SessionOwnerInspectorEnableResult::TargetCrashed {
                event_session_id: session_id.map(str::to_owned).or(primary_session_id),
            };
        }
        SessionOwnerInspectorEnableResult::Handled
    }

    pub(crate) fn set_runtime_frontend_enabled_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        enabled: bool,
    ) -> SessionOwnerRuntimeFrontendEnableResult {
        let accepts_without_target = self.accepts_unmaterialized_page_command(owner);
        let result = self
            .with_target_session_owner_mut_for_owner(owner, |owner| {
                owner.set_runtime_frontend_enabled(enabled)
            })
            .unwrap_or({
                if accepts_without_target {
                    SessionOwnerRuntimeFrontendEnableResult::Handled
                } else {
                    SessionOwnerRuntimeFrontendEnableResult::UnknownSession
                }
            });
        if matches!(result, SessionOwnerRuntimeFrontendEnableResult::Handled) && !enabled {
            let _ = self
                .set_renderer_runtime_agent_owns_page_console_api_events_for_owner(owner, false);
            let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
                state.clear_child_default_context_emission_state()
            });
        }
        result
    }

    pub(crate) fn set_renderer_runtime_agent_owns_page_console_api_events_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        owns: bool,
    ) -> bool {
        let session_id = owner.session_id();
        if session_id.is_some_and(|session_id| {
            self.shared_worker_target_for_session(Some(session_id))
                .is_some()
                || self
                    .service_worker_target_for_session(Some(session_id))
                    .is_some()
        }) {
            return false;
        }
        let owns = owns
            && self
                .runtime_session_owner_slot_for_owner(owner)
                .is_ok_and(|slot| slot.has_loaded_page());
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state
                .console_output_session_state
                .renderer_runtime_agent_owns_page_console_api_events = owns;
        })
        .is_some()
    }

    pub(crate) fn merge_v8_inspector_session_state_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        state: V8InspectorSessionState,
    ) -> bool {
        self.with_target_devtools_session_state_for_owner_mut(owner, |session| {
            session.inspector_session_state.v8_state = Some(state);
        })
        .is_some()
    }

    pub fn enable_runtime_listener_state_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        let owner = crate::conn::CommandOwnerScope::for_route(route);
        let enabled = matches!(
            self.set_runtime_frontend_enabled_for_owner(&owner, true),
            SessionOwnerRuntimeFrontendEnableResult::Handled
        );
        enabled
    }

    pub async fn enable_runtime_listener_for_target(
        &mut self,
        target_id: &str,
    ) -> Option<CdpRendererOwnerTurnOutcome> {
        let route = self.target_session_route_for_target_id(target_id)?;
        if matches!(route, CdpSessionRoute::ServiceWorkerTarget { .. }) {
            let session_id =
                self.ensure_service_worker_runtime_listener_session_for_route(&route)?;
            return Some(
                self.process_nested_target_message_adapter_async(
                    &json!({
                        "id": 0_u64,
                        "sessionId": session_id,
                        "method": "Runtime.enable",
                        "params": {}
                    })
                    .to_string(),
                )
                .await,
            );
        }
        if matches!(
            route,
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
        ) {
            let session_id =
                self.ensure_shared_worker_runtime_listener_session_for_route(&route)?;
            return Some(
                self.process_nested_target_message_adapter_async(
                    &json!({
                        "id": 0_u64,
                        "sessionId": session_id,
                        "method": "Runtime.enable",
                        "params": {}
                    })
                    .to_string(),
                )
                .await,
            );
        }
        let owner = crate::conn::CommandOwnerScope::for_route(route);
        Some(
            self.execute_page_runtime_listener_command_for_owner(owner, true)
                .await,
        )
    }

    pub async fn disable_runtime_listener_for_target(
        &mut self,
        target_id: &str,
    ) -> Option<CdpRendererOwnerTurnOutcome> {
        let route = self.target_session_route_for_target_id(target_id)?;
        if matches!(route, CdpSessionRoute::ServiceWorkerTarget { .. }) {
            let session_id = self.service_worker_runtime_listener_session_for_route(&route)?;
            return Some(
                self.process_nested_target_message_adapter_async(
                    &json!({
                        "id": 0_u64,
                        "sessionId": session_id,
                        "method": "Runtime.disable",
                        "params": {}
                    })
                    .to_string(),
                )
                .await,
            );
        }
        if matches!(
            route,
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
        ) {
            let session_id = self.shared_worker_runtime_listener_session_for_route(&route)?;
            return Some(
                self.process_nested_target_message_adapter_async(
                    &json!({
                        "id": 0_u64,
                        "sessionId": session_id,
                        "method": "Runtime.disable",
                        "params": {}
                    })
                    .to_string(),
                )
                .await,
            );
        }
        let owner = crate::conn::CommandOwnerScope::for_route(route);
        Some(
            self.execute_page_runtime_listener_command_for_owner(owner, false)
                .await,
        )
    }

    async fn execute_page_runtime_listener_command_for_owner(
        &mut self,
        owner: crate::conn::CommandOwnerScope,
        enabled: bool,
    ) -> CdpRendererOwnerTurnOutcome {
        let session_id = owner.session_id().map(str::to_owned);
        let plan = crate::domains::runtime::execute_runtime_listener_command_for_owner(
            self,
            owner.clone(),
            enabled,
        )
        .await;
        let mut command_context = crate::conn::CommandDispatchContext::default();
        crate::domains::activity::project_protocol_local_command_outputs(
            self,
            &owner,
            &mut command_context,
        )
        .await;
        match self.complete_with_output_plan(
            &mut command_context,
            plan,
            Some(0),
            session_id.as_deref(),
        ) {
            crate::conn::CdpCommandTaskStep::Complete(outcome) => outcome,
            crate::conn::CdpCommandTaskStep::Pending(_) => {
                unreachable!("a completed Runtime listener plan cannot become pending")
            }
        }
    }

    fn service_worker_runtime_listener_session_for_route(
        &self,
        route: &CdpSessionRoute,
    ) -> Option<String> {
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = route
        else {
            return None;
        };
        let session_id = service_worker_runtime_listener_session_id(target_id);
        self.browser_context_by_id(browser_context_id)?
            .service_worker_target(target_id)?
            .is_session(&session_id)
            .then_some(session_id)
    }

    fn ensure_service_worker_runtime_listener_session_for_route(
        &mut self,
        route: &CdpSessionRoute,
    ) -> Option<String> {
        let session_id = match self.service_worker_runtime_listener_session_for_route(route) {
            Some(session_id) => session_id,
            None => {
                let CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id,
                    target_id,
                } = route
                else {
                    return None;
                };
                let session_id = service_worker_runtime_listener_session_id(target_id);
                self.browser_context_by_id_mut(browser_context_id)?
                    .assign_session_to_service_worker_target(target_id, session_id.clone())
                    .then_some(session_id)?
            }
        };
        self.commit_worker_runtime_listener_session_route(&session_id, route)?;
        Some(session_id)
    }

    fn shared_worker_runtime_listener_session_for_route(
        &self,
        route: &CdpSessionRoute,
    ) -> Option<String> {
        match route {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let session_id = shared_worker_runtime_listener_session_id(target_id);
                self.browser_context_by_id(browser_context_id)?
                    .shared_worker_target(target_id)?
                    .is_session(&session_id)
                    .then_some(session_id)
            }
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let session_id = dedicated_worker_runtime_listener_session_id(target_id);
                self.browser_context_by_id(browser_context_id)?
                    .dedicated_worker_target(target_id)?
                    .is_session(&session_id)
                    .then_some(session_id)
            }
            _ => None,
        }
    }

    fn ensure_shared_worker_runtime_listener_session_for_route(
        &mut self,
        route: &CdpSessionRoute,
    ) -> Option<String> {
        let session_id = match self.shared_worker_runtime_listener_session_for_route(route) {
            Some(session_id) => session_id,
            None => match route {
                CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id,
                    target_id,
                } => {
                    let session_id = shared_worker_runtime_listener_session_id(target_id);
                    self.browser_context_by_id_mut(browser_context_id)?
                        .assign_session_to_shared_worker_target(target_id, session_id.clone())
                        .then_some(session_id)?
                }
                CdpSessionRoute::DedicatedWorkerTarget {
                    browser_context_id,
                    target_id,
                } => {
                    let session_id = dedicated_worker_runtime_listener_session_id(target_id);
                    self.browser_context_by_id_mut(browser_context_id)?
                        .assign_session_to_dedicated_worker_target(target_id, session_id.clone())
                        .then_some(session_id)?
                }
                _ => return None,
            },
        };
        self.commit_worker_runtime_listener_session_route(&session_id, route)?;
        Some(session_id)
    }

    fn commit_worker_runtime_listener_session_route(
        &mut self,
        session_id: &str,
        route: &CdpSessionRoute,
    ) -> Option<()> {
        if self.target_control.attached_session_route(session_id) == Some(route) {
            return Some(());
        }
        let target_id = match route {
            CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => target_id.clone(),
            _ => return None,
        };
        self.target_control.commit_attached_session(
            session_id.to_owned(),
            None,
            &target_id,
            route.clone(),
            false,
            false,
        );
        Some(())
    }
}

fn service_worker_runtime_listener_session_id(target_id: &str) -> String {
    format!("SID-bidi-runtime-listener-service-worker-{target_id}")
}

fn shared_worker_runtime_listener_session_id(target_id: &str) -> String {
    format!("SID-bidi-runtime-listener-shared-worker-{target_id}")
}

fn dedicated_worker_runtime_listener_session_id(target_id: &str) -> String {
    format!("SID-bidi-runtime-listener-dedicated-worker-{target_id}")
}

#[cfg(test)]
mod tests {
    use moli_core::RendererOwnerLocalHostId;
    use moli_shared_worker::SharedWorkerInstanceId;
    use serde_json::json;

    use crate::conn::{BrowserContext, CdpConnection, CdpSessionRoute, SharedWorkerTargetState};

    #[test]
    fn runtime_listener_session_ids_are_worker_type_scoped() {
        assert_eq!(
            super::service_worker_runtime_listener_session_id("TID-worker"),
            "SID-bidi-runtime-listener-service-worker-TID-worker"
        );
        assert_eq!(
            super::shared_worker_runtime_listener_session_id("TID-worker"),
            "SID-bidi-runtime-listener-shared-worker-TID-worker"
        );
        assert_ne!(
            super::service_worker_runtime_listener_session_id("TID-worker"),
            super::shared_worker_runtime_listener_session_id("TID-worker")
        );
    }

    #[tokio::test]
    async fn runtime_listener_enable_uses_shared_worker_target_session() {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-shared".to_owned());
        browser_context.insert_shared_worker_target(SharedWorkerTargetState::new(
            RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(91),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "shared-worker".to_owned(),
        ));
        conn.install_browser_context_fixture_for_test(browser_context);

        let outcome = conn
            .enable_runtime_listener_for_target("TID-shared-worker")
            .await
            .expect("shared worker target should open a Runtime listener");
        let (messages, scheduler_events) = outcome.into_parts();

        assert!(scheduler_events.is_empty());
        assert!(
            conn.shared_worker_target_for_session(Some(
                "SID-bidi-runtime-listener-shared-worker-TID-shared-worker"
            ))
            .is_some(),
            "Runtime listener should attach a target-local SharedWorker session"
        );
        assert_eq!(
            conn.session_route(Some(
                "SID-bidi-runtime-listener-shared-worker-TID-shared-worker"
            )),
            Some(CdpSessionRoute::SharedWorkerTarget {
                browser_context_id: "BID-shared".to_owned(),
                target_id: "TID-shared-worker".to_owned(),
            }),
            "Runtime listener should commit its exact route before dispatch"
        );
        assert!(
            messages.iter().any(|message| {
                message["id"] == json!(0)
                    && message["result"] == json!({})
                    && message["sessionId"]
                        == json!("SID-bidi-runtime-listener-shared-worker-TID-shared-worker")
            }),
            "Runtime listener should enable Runtime on the shared worker session: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != json!("Runtime.executionContextCreated")),
            "Runtime listener must not synthesize a shared worker execution context before renderer context creation: {messages:?}"
        );
    }
}

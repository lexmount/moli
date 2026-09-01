use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionStateMut};
use super::*;
use crate::conn::PageScreencastConfig;
use crate::conn::TargetRuntimeSlot;
use crate::conn::state::DevToolsSessionState;
use crate::conn::state::MainDocumentResourceSnapshot;
use crate::conn::state::PerformanceTimeDomain;

pub(crate) struct PageLifecycleReplayTarget {
    pub(crate) session_id: String,
}

pub(crate) enum PageLifecycleEventsEnableResult {
    Handled {
        replay_target: Option<PageLifecycleReplayTarget>,
    },
    UnknownSession,
}

impl TargetSessionStateMut<'_> {
    fn set_page_domain_enabled(mut self, enabled: bool, subscription_generation: u64) {
        if let Some(state) = self.page_session_state_mut() {
            if enabled {
                state.enable_page_domain(subscription_generation);
            } else {
                state.disable_page_domain();
            }
        }
    }

    fn set_console_enabled(mut self, enabled: bool) {
        if let Some(state) = self.devtools_session_state_mut() {
            state.console_output_session_state.console_enabled = enabled;
        }
    }

    fn enable_audits_with_storage(
        mut self,
        storage: &crate::domains::audits_output_state::TargetAuditsStorageState,
    ) -> Option<Option<crate::domains::audits_output_state::TargetAuditsOutputCursor>> {
        let state = self.page_session_state_mut()?;
        Some(state.audits.enable(storage))
    }

    fn disable_audits(mut self) {
        if let Some(state) = self.page_session_state_mut() {
            state.audits.disable();
        }
    }

    #[cfg(test)]
    fn set_log_enabled(mut self, enabled: bool) {
        if let Some(state) = self.page_session_state_mut() {
            state.log_enabled = enabled;
        }
    }

    fn enable_log_with_storage(
        mut self,
        storage: crate::domains::log_output_state::TargetLogStorageState,
    ) -> Option<bool> {
        let state = self.devtools_session_state_mut()?;
        if state.page_session_state.log_enabled {
            return Some(false);
        }
        state.page_session_state.log_enabled = true;
        state
            .console_output_session_state
            .reset_log_delivery_for_enable(storage);
        Some(true)
    }

    fn disable_log_and_violations(mut self) {
        if let Some(state) = self.devtools_session_state_mut() {
            state.page_session_state.log_enabled = false;
            state
                .console_output_session_state
                .clear_log_violation_thresholds();
        }
    }

    fn set_page_file_chooser_opened_event_enabled(mut self, enabled: bool) {
        if let Some(state) = self.page_session_state_mut() {
            state.page_file_chooser_opened_event_enabled = enabled;
        }
    }

    fn disable_page_domain(mut self) {
        if let Some(state) = self.page_session_state_mut() {
            state.disable_page_domain();
            state.page_lifecycle_events = false;
            state.page_bypass_csp_enabled = false;
            state.page_font_families.clear();
            state.page_file_chooser_opened_event_enabled = false;
            state.page_intercept_file_chooser_dialog_enabled = false;
            state.page_screencast.stop();
            state.javascript_dialog_state.clear();
        }
    }

    fn set_page_lifecycle_events_enabled(mut self, enabled: bool) {
        if let Some(state) = self.page_session_state_mut() {
            state.page_lifecycle_events = enabled;
        }
    }

    fn set_page_bypass_csp_enabled(mut self, enabled: bool) {
        if let Some(state) = self.page_session_state_mut() {
            state.page_bypass_csp_enabled = enabled;
        }
    }

    fn set_page_font_families(mut self, font_families: serde_json::Map<String, Value>) {
        if let Some(state) = self.page_session_state_mut() {
            state.page_font_families = font_families;
        }
    }

    fn set_page_intercept_file_chooser_dialog_enabled(mut self, enabled: bool) {
        if let Some(state) = self.page_session_state_mut() {
            state.page_intercept_file_chooser_dialog_enabled = enabled;
        }
    }

    fn start_page_screencast(mut self, config: PageScreencastConfig) -> Option<i32> {
        let state = self.page_session_state_mut()?;
        Some(state.page_screencast.start(config))
    }

    fn stop_page_screencast(mut self) -> bool {
        let Some(state) = self.page_session_state_mut() else {
            return false;
        };
        state.page_screencast.stop();
        true
    }

    fn begin_page_screencast_capture(mut self, generation: i32) -> Option<bool> {
        let state = self.page_session_state_mut()?;
        Some(state.page_screencast.begin_capture(generation))
    }

    fn complete_page_screencast_capture(
        mut self,
        generation: i32,
        frame_emitted: bool,
    ) -> Option<bool> {
        let state = self.page_session_state_mut()?;
        Some(
            state
                .page_screencast
                .complete_capture(generation, frame_emitted),
        )
    }

    fn acknowledge_page_screencast_frame(mut self, generation: i32) -> Option<bool> {
        let state = self.page_session_state_mut()?;
        Some(state.page_screencast.acknowledge_frame(generation))
    }

    fn enable_performance(mut self, time_domain: PerformanceTimeDomain) -> bool {
        self.page_session_state_mut()
            .map(|state| state.performance.enable(time_domain))
            .unwrap_or(true)
    }

    fn disable_performance(mut self) {
        if let Some(state) = self.page_session_state_mut() {
            state.performance.disable();
        }
    }

    fn set_performance_time_domain(mut self, time_domain: PerformanceTimeDomain) -> bool {
        self.page_session_state_mut()
            .map(|state| state.performance.set_time_domain(time_domain))
            .unwrap_or(true)
    }
}

impl TargetSessionOwnerMut<'_> {
    fn mutate_page_session_state_and_advance_console(
        mut self,
        f: impl FnOnce(TargetSessionStateMut<'_>),
    ) -> bool {
        self.mutate_session_state_ref(f);
        self.advance_console_domain_cursors_to_current();
        true
    }

    fn set_console_enabled(self, enabled: bool) -> bool {
        self.mutate_page_session_state_and_advance_console(|state| {
            state.set_console_enabled(enabled);
        })
    }

    fn clear_console_messages(mut self) -> bool {
        self.advance_console_domain_cursors_to_current();
        true
    }

    fn audits_output_snapshot(&mut self) -> Vec<moli_core::page::InspectorIssueSnapshot> {
        let Some(runtime_slot) = self.runtime_slot_mut() else {
            return Vec::new();
        };
        runtime_slot.ingest_owner_page_observable_output_updates();
        runtime_slot.inspector_issues().unwrap_or_default()
    }

    fn enable_audits(mut self) -> SessionOwnerAuditsEnableResult {
        let storage = self.sync_audits_storage();
        let cursor = self
            .mutate_session_state_ref(|state| state.enable_audits_with_storage(&storage))
            .flatten();
        let replay = cursor.and_then(|cursor| {
            let issues = storage.issues_for_cursor(cursor)?;
            self.mutate_session_state_ref(|mut state| {
                if let Some(session) = state.page_session_state_mut() {
                    session.audits.mark_emitted(cursor);
                }
            });
            Some(crate::domains::audits::TargetAuditsReplaySnapshot { issues })
        });
        SessionOwnerAuditsEnableResult::Handled { replay }
    }

    fn sync_audits_storage(
        &mut self,
    ) -> crate::domains::audits_output_state::TargetAuditsStorageState {
        let source_issues = self.audits_output_snapshot();
        self.mutate_target_owner_state(|owner_state| {
            owner_state.map(|state| {
                state
                    .audits_storage_state
                    .ingest_source_issues(&source_issues);
                state.audits_storage_state.clone()
            })
        })
        .unwrap_or_default()
    }

    fn disable_audits(mut self) -> bool {
        self.mutate_session_state_ref(|state| state.disable_audits());
        true
    }

    fn enable_log(mut self) -> SessionOwnerLogEnableResult {
        let output = self.log_output_snapshot();
        let storage = self
            .mutate_target_owner_state(|owner_state| {
                owner_state.map(|state| state.log_storage_state)
            })
            .unwrap_or_default();
        let should_replay = self
            .mutate_session_state_ref(|state| state.enable_log_with_storage(storage))
            .unwrap_or(false);
        let replay = should_replay.then_some(output).flatten().and_then(
            |(url, lifecycle_errors, network_entries)| {
                let lifecycle_end = lifecycle_errors.len();
                let network_end = network_entries.len();
                let lifecycle_start = storage.lifecycle_start().min(lifecycle_end);
                let network_start = storage.network_start().min(network_end);
                self.mutate_session_state_ref(|mut state| {
                    if let Some(session) = state.devtools_session_state_mut() {
                        session
                            .console_output_session_state
                            .mark_log_entries_emitted(
                                storage.generation(),
                                lifecycle_end,
                                network_end,
                            );
                    }
                });
                let lifecycle_errors = lifecycle_errors
                    .into_iter()
                    .skip(lifecycle_start)
                    .collect::<Vec<_>>();
                let network_entries = network_entries
                    .into_iter()
                    .skip(network_start)
                    .collect::<Vec<_>>();
                (!lifecycle_errors.is_empty() || !network_entries.is_empty()).then_some(
                    crate::domains::log::TargetLogReplaySnapshot {
                        url,
                        lifecycle_errors,
                        network_entries,
                    },
                )
            },
        );
        SessionOwnerLogEnableResult::Handled { replay }
    }

    fn disable_log(mut self) -> bool {
        self.mutate_session_state_ref(|state| state.disable_log_and_violations());
        true
    }

    fn clear_log(mut self) -> bool {
        let (lifecycle_end, network_end) = self
            .log_output_snapshot()
            .map(|(_, lifecycle_errors, network_entries)| {
                (lifecycle_errors.len(), network_entries.len())
            })
            .unwrap_or_default();
        self.mutate_target_owner_state(|owner_state| {
            if let Some(owner_state) = owner_state {
                owner_state
                    .log_storage_state
                    .clear_at(lifecycle_end, network_end);
            }
        });
        true
    }

    fn start_log_violations(
        mut self,
        thresholds: Vec<crate::conn::state::DevToolsLogViolationThreshold>,
    ) -> SessionOwnerLogControlResult {
        self.mutate_session_state_ref(|mut state| {
            let Some(session) = state.devtools_session_state_mut() else {
                return SessionOwnerLogControlResult::UnknownSession;
            };
            if !session.page_session_state.log_enabled {
                return SessionOwnerLogControlResult::LogNotEnabled;
            }
            session
                .console_output_session_state
                .set_log_violation_thresholds(thresholds);
            SessionOwnerLogControlResult::Handled
        })
    }

    fn stop_log_violations(mut self) -> bool {
        self.mutate_session_state_ref(|mut state| {
            if let Some(session) = state.devtools_session_state_mut() {
                session
                    .console_output_session_state
                    .clear_log_violation_thresholds();
            }
        });
        true
    }

    fn set_page_domain_enabled(self, enabled: bool, subscription_generation: u64) -> bool {
        self.mutate_session_state(|state| {
            state.set_page_domain_enabled(enabled, subscription_generation);
        });
        true
    }

    fn set_page_file_chooser_opened_event_enabled(self, enabled: bool) -> bool {
        self.mutate_session_state(|state| {
            state.set_page_file_chooser_opened_event_enabled(enabled);
        });
        true
    }

    fn disable_page_domain(self) -> bool {
        self.mutate_session_state(|state| {
            state.disable_page_domain();
        });
        true
    }

    fn set_page_lifecycle_events_enabled(
        mut self,
        enabled: bool,
    ) -> PageLifecycleEventsEnableResult {
        self.mutate_session_state_ref(|state| {
            state.set_page_lifecycle_events_enabled(enabled);
        });
        let replay_target = if !enabled {
            None
        } else {
            match &self {
                Self::PageTarget {
                    browser_context,
                    target_id,
                    session_id,
                    is_auxiliary_target_session,
                } => browser_context
                    .page_target(target_id)
                    .filter(|target| target.has_loaded_page())
                    .and_then(|target| {
                        let replay_session_id = if *is_auxiliary_target_session {
                            session_id.clone()
                        } else {
                            target.session_id().map(str::to_owned)
                        }?;
                        Some(PageLifecycleReplayTarget {
                            session_id: replay_session_id,
                        })
                    }),
                Self::NoLoadedBrowserContext => None,
            }
        };
        PageLifecycleEventsEnableResult::Handled { replay_target }
    }

    fn set_page_bypass_csp_enabled(self, enabled: bool) -> bool {
        self.mutate_session_state(|state| {
            state.set_page_bypass_csp_enabled(enabled);
        });
        true
    }

    fn set_page_font_families(self, font_families: serde_json::Map<String, Value>) -> bool {
        self.mutate_session_state(|state| {
            state.set_page_font_families(font_families);
        });
        true
    }

    fn set_page_intercept_file_chooser_dialog_enabled(self, enabled: bool) -> bool {
        self.mutate_session_state(|state| {
            state.set_page_intercept_file_chooser_dialog_enabled(enabled);
        });
        true
    }

    fn start_page_screencast(self, config: PageScreencastConfig) -> Option<i32> {
        self.mutate_session_state(|state| state.start_page_screencast(config))
    }

    fn stop_page_screencast(self) -> bool {
        self.mutate_session_state(|state| state.stop_page_screencast())
    }

    fn begin_page_screencast_capture(self, generation: i32) -> Option<bool> {
        self.mutate_session_state(|state| state.begin_page_screencast_capture(generation))
    }

    fn complete_page_screencast_capture(
        self,
        generation: i32,
        frame_emitted: bool,
    ) -> Option<bool> {
        self.mutate_session_state(|state| {
            state.complete_page_screencast_capture(generation, frame_emitted)
        })
    }

    fn acknowledge_page_screencast_frame(self, generation: i32) -> Option<bool> {
        self.mutate_session_state(|state| state.acknowledge_page_screencast_frame(generation))
    }

    fn enable_performance(self, time_domain: PerformanceTimeDomain) -> bool {
        self.mutate_session_state(|state| state.enable_performance(time_domain))
    }

    fn disable_performance(self) -> bool {
        self.mutate_session_state(|state| state.disable_performance());
        true
    }

    fn set_performance_time_domain(self, time_domain: PerformanceTimeDomain) -> bool {
        self.mutate_session_state(|state| state.set_performance_time_domain(time_domain))
    }

    fn console_counts(&self) -> Option<(usize, usize)> {
        self.runtime_slot_ref()
            .and_then(runtime_observable_console_payloads)
            .map(|(_, console_messages, lifecycle_errors)| {
                (console_messages.len(), lifecycle_errors.len())
            })
    }

    fn advance_console_domain_cursors_to_current(&mut self) {
        let Some((console_entries, lifecycle_errors)) = self.console_counts() else {
            return;
        };
        self.mutate_target_owner_state(|owner_state| {
            if let Some(owner_state) = owner_state {
                owner_state
                    .console_output_state
                    .advance_console_domain_to_current(console_entries, lifecycle_errors);
            }
        });
    }

    fn log_output_snapshot(
        &mut self,
    ) -> Option<(
        String,
        Vec<String>,
        Vec<crate::domains::log_output_state::TargetNetworkLogEntry>,
    )> {
        // Concrete renderer records are already admitted into target-owned
        // queues. Late `Log.enable` replay must read those queues rather than
        // rediscovering output from the immutable Page diagnostics snapshot.
        // Network storage is independent from Console/lifecycle storage: a
        // page that only produced a failed request has no observable source
        // tail, but Chromium must still replay its network Log entry.
        let url = self.target_url()?;
        let runtime_slot = self.runtime_slot_ref()?;
        let lifecycle_errors = runtime_slot
            .observable_output_latest_source_tail()
            .map(|source| {
                source
                    .observable_output_items()
                    .into_iter()
                    .filter_map(|item| match item {
                        moli_core::page::ScriptObservableOutputItem::LifecycleError(error) => {
                            Some(error)
                        }
                        moli_core::page::ScriptObservableOutputItem::ConsoleMessage(_)
                        | moli_core::page::ScriptObservableOutputItem::InspectorIssue(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some((
            url,
            lifecycle_errors,
            runtime_slot.network_log_entries()?.to_vec(),
        ))
    }
}

fn devtools_session_page_domain_enabled(state: &DevToolsSessionState) -> bool {
    state.page_session_state.page_domain_enabled
}

fn browser_context_has_page_domain_enabled_session(browser_context: &BrowserContext) -> bool {
    browser_context
        .active_page_target()
        .devtools_sessions
        .states()
        .any(devtools_session_page_domain_enabled)
        || browser_context
            .background_targets()
            .any(|target| target.has_page_domain_enabled_session())
}

impl CdpConnection {
    pub(crate) fn enable_audits_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> SessionOwnerAuditsEnableResult {
        self.with_target_session_owner_mut(session_id, |owner| owner.enable_audits())
            .unwrap_or(SessionOwnerAuditsEnableResult::UnknownSession)
    }

    pub(crate) fn disable_audits_for_session_owner(&mut self, session_id: Option<&str>) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.disable_audits())
            .unwrap_or(false)
    }

    pub(crate) fn page_domain_enabled_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.devtools_session_state())
            .map(devtools_session_page_domain_enabled)
    }

    pub(crate) fn page_domain_subscription_generation_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<u64> {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.devtools_session_state())
            .and_then(|state| {
                state
                    .page_session_state
                    .page_domain_subscription_generation()
            })
    }

    pub(crate) fn page_domain_subscription_is_current(
        &self,
        session_id: Option<&str>,
        generation: u64,
    ) -> bool {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.devtools_session_state())
            .is_some_and(|state| {
                state
                    .page_session_state
                    .page_domain_subscription_is_current(generation)
            })
    }

    pub(crate) fn record_main_document_resource_body_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        frame_id: String,
        loader_id: String,
        url: url::Url,
        response_headers: Vec<(String, String)>,
        from_cache: bool,
        body: crate::conn::CapturedBody,
    ) -> bool {
        self.with_target_owner_state_for_session_mut(session_id, |owner_state| {
            owner_state.page_resource_store.record_main_document_body(
                frame_id,
                loader_id,
                url,
                response_headers,
                from_cache,
                body,
            );
        })
        .is_some()
    }

    pub(crate) fn commit_main_document_resource_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        frame_id: String,
        loader_id: String,
        url: url::Url,
        response_headers: Vec<(String, String)>,
        from_cache: bool,
        body: Option<crate::conn::CapturedBody>,
    ) -> bool {
        if self
            .runtime_session_owner_slot(session_id)
            .ok()
            .and_then(TargetRuntimeSlot::committed_document_loader_id)
            != Some(loader_id.as_str())
        {
            return false;
        }
        self.with_target_owner_state_for_session_mut(session_id, |owner_state| {
            owner_state.page_resource_store.commit_main_document(
                frame_id,
                loader_id,
                url,
                response_headers,
                from_cache,
                body,
            );
        })
        .is_some()
    }

    pub(crate) fn current_main_document_resource_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<MainDocumentResourceSnapshot> {
        let loader_id = self
            .runtime_session_owner_slot(session_id)
            .ok()?
            .committed_document_loader_id()?
            .to_owned();
        self.target_owner_state_for_session(session_id)?
            .page_resource_store
            .main_document_for_loader(&loader_id)
    }

    pub(crate) fn target_owner_has_attached_child_frame_id_for_session(
        &self,
        session_id: Option<&str>,
        frame_id: &str,
    ) -> Option<bool> {
        self.target_owner_state_for_session(session_id)
            .map(|owner_state| owner_state.has_attached_child_frame_id(frame_id))
    }

    pub(crate) fn discard_uncommitted_main_document_resource_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        loader_id: &str,
    ) {
        let _ = self.with_target_owner_state_for_session_mut(session_id, |owner_state| {
            owner_state
                .page_resource_store
                .discard_uncommitted_loader(loader_id);
        });
    }

    pub(crate) fn set_page_domain_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        let owner_browser_context_id = self
            .target_owner_identity_for_session(session_id)
            .map(|(browser_context_id, _)| browser_context_id);
        self.next_page_domain_subscription_generation = self
            .next_page_domain_subscription_generation
            .wrapping_add(1);
        let subscription_generation = self.next_page_domain_subscription_generation;
        let handled = self
            .with_target_session_owner_mut(session_id, |owner| {
                owner.set_page_domain_enabled(enabled, subscription_generation)
            })
            .unwrap_or(false);
        if handled && let Some(browser_context_id) = owner_browser_context_id.as_deref() {
            self.sync_javascript_dialog_handler_enabled_for_browser_context(browser_context_id);
        }
        handled
    }

    fn sync_javascript_dialog_handler_enabled_for_browser_context(&self, browser_context_id: &str) {
        let Some(browser_context) = self.browser_context_by_id(browser_context_id) else {
            return;
        };
        browser_context
            .renderer_runtime()
            .set_javascript_dialog_handler_enabled(
                browser_context_has_page_domain_enabled_session(browser_context),
            );
    }

    pub(crate) fn set_console_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        let handled = self
            .with_target_session_owner_mut(session_id, |owner| owner.set_console_enabled(enabled))
            .unwrap_or(false);
        if handled {
            let renderer_console_agent_owns_page_console_api_events = enabled
                && self
                    .runtime_session_owner_slot(session_id)
                    .is_ok_and(|slot| slot.has_loaded_page());
            let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
                state
                    .console_output_session_state
                    .renderer_console_agent_owns_page_console_api_events =
                    renderer_console_agent_owns_page_console_api_events;
            });
        }
        handled
    }

    pub(crate) fn clear_console_messages_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.clear_console_messages())
            .unwrap_or(false)
    }

    pub(crate) fn enable_log_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> SessionOwnerLogEnableResult {
        self.with_target_session_owner_mut(session_id, |owner| owner.enable_log())
            .unwrap_or(SessionOwnerLogEnableResult::UnknownSession)
    }

    pub(crate) fn disable_log_for_session_owner(&mut self, session_id: Option<&str>) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.disable_log())
            .unwrap_or(false)
    }

    pub(crate) fn clear_log_for_session_owner(&mut self, session_id: Option<&str>) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.clear_log())
            .unwrap_or(false)
    }

    pub(crate) fn start_log_violations_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        thresholds: Vec<crate::conn::state::DevToolsLogViolationThreshold>,
    ) -> SessionOwnerLogControlResult {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.start_log_violations(thresholds)
        })
        .unwrap_or(SessionOwnerLogControlResult::UnknownSession)
    }

    pub(crate) fn stop_log_violations_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.stop_log_violations())
            .unwrap_or(false)
    }

    pub(crate) fn set_page_file_chooser_opened_event_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.set_page_file_chooser_opened_event_enabled(enabled)
        })
        .unwrap_or(false)
    }

    pub(crate) fn disable_page_domain_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        let owner_browser_context_id = self
            .target_owner_identity_for_session(session_id)
            .map(|(browser_context_id, _)| browser_context_id);
        let handled = self
            .with_target_session_owner_mut(session_id, |owner| owner.disable_page_domain())
            .unwrap_or(false);
        if handled && let Some(browser_context_id) = owner_browser_context_id.as_deref() {
            self.sync_javascript_dialog_handler_enabled_for_browser_context(browser_context_id);
        }
        handled
    }

    pub fn enable_file_dialog_opened_listener_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .set_page_file_chooser_opened_event_enabled_for_session_owner(None, true)
    }

    pub fn disable_file_dialog_opened_listener_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .set_page_file_chooser_opened_event_enabled_for_session_owner(None, false)
    }

    pub(crate) fn set_page_lifecycle_events_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> PageLifecycleEventsEnableResult {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.set_page_lifecycle_events_enabled(enabled)
        })
        .unwrap_or(PageLifecycleEventsEnableResult::UnknownSession)
    }

    pub(crate) fn set_page_bypass_csp_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.set_page_bypass_csp_enabled(enabled)
        })
        .unwrap_or(false)
    }

    pub(crate) fn set_page_font_families_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        font_families: serde_json::Map<String, Value>,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.set_page_font_families(font_families)
        })
        .unwrap_or(false)
    }

    pub(crate) fn set_page_intercept_file_chooser_dialog_enabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.set_page_intercept_file_chooser_dialog_enabled(enabled)
        })
        .unwrap_or(false)
    }

    pub(crate) fn start_page_screencast_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        config: PageScreencastConfig,
    ) -> Option<i32> {
        self.with_target_session_owner_mut(session_id, |owner| owner.start_page_screencast(config))
            .flatten()
    }

    pub(crate) fn page_screencast_visible_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
        let browser_context = self.browser_context_by_id(&browser_context_id)?;
        let target_id = target_id.or_else(|| browser_context.active_target_id_owned())?;
        Some(browser_context.is_active_target(&target_id))
    }

    pub(crate) fn stop_page_screencast_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.stop_page_screencast())
            .unwrap_or(false)
    }

    pub(crate) fn begin_page_screencast_capture_for_route(
        &mut self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
        generation: i32,
    ) -> Option<bool> {
        self.target_session_owner_mut_for_route(session_id, owner_route)?
            .begin_page_screencast_capture(generation)
    }

    pub(crate) fn complete_page_screencast_capture_for_route(
        &mut self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
        generation: i32,
        frame_emitted: bool,
    ) -> Option<bool> {
        self.target_session_owner_mut_for_route(session_id, owner_route)?
            .complete_page_screencast_capture(generation, frame_emitted)
    }

    pub(crate) fn acknowledge_page_screencast_frame_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        generation: i32,
    ) -> Option<bool> {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.acknowledge_page_screencast_frame(generation)
        })
        .flatten()
    }

    pub(crate) fn performance_enabled_for_session_owner(&self, session_id: Option<&str>) -> bool {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.devtools_session_state())
            .is_some_and(|state| state.page_session_state.performance.enabled())
    }

    pub(crate) fn enable_performance_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        time_domain: PerformanceTimeDomain,
    ) -> Option<bool> {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.enable_performance(time_domain)
        })
    }

    pub(crate) fn disable_performance_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| owner.disable_performance())
            .unwrap_or(false)
    }

    pub(crate) fn set_performance_time_domain_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        time_domain: PerformanceTimeDomain,
    ) -> Option<bool> {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.set_performance_time_domain(time_domain)
        })
    }
}

fn runtime_observable_console_payloads(
    runtime_slot: &TargetRuntimeSlot,
) -> Option<(String, Vec<String>, Vec<String>)> {
    let source = runtime_slot.observable_output_latest_source_tail()?;
    let mut console_messages = Vec::new();
    let mut lifecycle_errors = Vec::new();
    for item in source.observable_output_items() {
        match item {
            moli_core::page::ScriptObservableOutputItem::ConsoleMessage(message) => {
                console_messages.push(message);
            }
            moli_core::page::ScriptObservableOutputItem::LifecycleError(error) => {
                lifecycle_errors.push(error);
            }
            moli_core::page::ScriptObservableOutputItem::InspectorIssue(_) => {}
        }
    }
    Some((source.url().to_owned(), console_messages, lifecycle_errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::PageTargetHost;

    fn active_session_state_mut(browser_context: &mut BrowserContext) -> TargetSessionStateMut<'_> {
        let state = browser_context.active_page_state_mut();
        TargetSessionStateMut::Loaded {
            devtools_session_state: &mut state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary],
            network_policy: &mut state.network_policy,
            tls_verify_host_override: &mut state.tls_verify_host_override,
        }
    }

    fn parked_session_state_mut(state: &mut PageTargetHost) -> TargetSessionStateMut<'_> {
        TargetSessionStateMut::Loaded {
            devtools_session_state: &mut state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary],
            network_policy: &mut state.network_policy,
            tls_verify_host_override: &mut state.tls_verify_host_override,
        }
    }

    #[test]
    fn page_session_state_mut_applies_same_flags_to_active_and_parked_state() {
        let mut font_families = serde_json::Map::new();
        font_families.insert("standard".to_owned(), serde_json::json!("Inter"));

        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        active_session_state_mut(&mut active).set_console_enabled(true);
        active_session_state_mut(&mut active).set_log_enabled(true);
        active_session_state_mut(&mut active).set_page_file_chooser_opened_event_enabled(true);
        active_session_state_mut(&mut active).set_page_bypass_csp_enabled(true);
        active_session_state_mut(&mut active).set_page_font_families(font_families.clone());
        active_session_state_mut(&mut active).set_page_intercept_file_chooser_dialog_enabled(true);
        assert!(
            active_session_state_mut(&mut active)
                .enable_performance(PerformanceTimeDomain::TimeTicks)
        );

        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_bypass_csp_enabled
        );
        assert_eq!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_font_families,
            font_families
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled()
        );

        let mut parked = PageTargetHost::empty("TID-page-owner-test".to_owned());
        parked_session_state_mut(&mut parked).set_console_enabled(true);
        parked_session_state_mut(&mut parked).set_log_enabled(true);
        parked_session_state_mut(&mut parked).set_page_file_chooser_opened_event_enabled(true);
        parked_session_state_mut(&mut parked).set_page_bypass_csp_enabled(true);
        parked_session_state_mut(&mut parked).set_page_font_families(font_families.clone());
        parked_session_state_mut(&mut parked).set_page_intercept_file_chooser_dialog_enabled(true);
        assert!(
            parked_session_state_mut(&mut parked)
                .enable_performance(PerformanceTimeDomain::TimeTicks)
        );

        assert!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled
        );
        assert!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled
        );
        assert!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled
        );
        assert!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_bypass_csp_enabled
        );
        assert_eq!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_font_families,
            font_families
        );
        assert!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled
        );
        assert!(
            parked.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled()
        );

        TargetSessionStateMut::NoLoaded.set_console_enabled(true);
        TargetSessionStateMut::NoLoaded.set_log_enabled(true);
        TargetSessionStateMut::NoLoaded.set_page_file_chooser_opened_event_enabled(true);
        TargetSessionStateMut::NoLoaded.set_page_bypass_csp_enabled(true);
        TargetSessionStateMut::NoLoaded.set_page_font_families(font_families);
        TargetSessionStateMut::NoLoaded.set_page_intercept_file_chooser_dialog_enabled(true);
        assert!(
            TargetSessionStateMut::NoLoaded.enable_performance(PerformanceTimeDomain::TimeTicks)
        );
    }

    #[test]
    fn page_domain_dialog_handler_tracks_the_session_owner_browser_context() {
        let mut conn = CdpConnection::default();

        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        active.set_active_target_id("TID-active".to_owned());
        let active_runtime = active.renderer_runtime();

        let mut inactive = BrowserContext::new("BID-inactive".to_owned());
        inactive.set_active_target_id("TID-inactive".to_owned());
        assert!(
            inactive
                .assign_auxiliary_session_to_target("TID-inactive", "SID-inactive-a".to_owned(),)
        );
        assert!(
            inactive
                .assign_auxiliary_session_to_target("TID-inactive", "SID-inactive-b".to_owned(),)
        );
        let inactive_runtime = inactive.renderer_runtime();

        conn.browser_context = Some(active);
        conn.inactive_browser_contexts.push(inactive);

        assert!(conn.set_page_domain_enabled_for_session_owner(Some("SID-inactive-a"), true));
        assert!(inactive_runtime.javascript_dialog_handler_enabled());
        assert!(
            !active_runtime.javascript_dialog_handler_enabled(),
            "enabling an inactive target session must not mutate the active browser context"
        );

        assert!(conn.set_page_domain_enabled_for_session_owner(Some("SID-inactive-b"), true));
        assert!(conn.disable_page_domain_for_session_owner(Some("SID-inactive-a")));
        assert!(
            inactive_runtime.javascript_dialog_handler_enabled(),
            "one frontend must not disable dialog handling while a peer remains subscribed"
        );

        assert!(conn.disable_page_domain_for_session_owner(Some("SID-inactive-b")));
        assert!(!inactive_runtime.javascript_dialog_handler_enabled());
    }

    #[test]
    fn file_dialog_opened_target_listener_can_be_disabled_after_enable() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-file-dialog".to_owned()));
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_active_target_id("TID-file-dialog");

        assert!(conn.enable_file_dialog_opened_listener_for_target("TID-file-dialog"));
        assert!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .active_page_state()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled
        );

        assert!(conn.disable_file_dialog_opened_listener_for_target("TID-file-dialog"));
        assert!(
            !conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_state()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled
        );
    }
}

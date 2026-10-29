use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
};

use moli_core::page::NavigationResponse;
use moli_shared_worker::SharedWorkerInstanceId;

use super::{SharedWorkerTargetState, TargetPageResidenceIdentity};

/// Protocol state for one renderer-owned DedicatedWorker lifetime.
///
/// The V8 inspector/session bookkeeping is identical to a SharedWorker target,
/// so the inner state intentionally reuses that implementation. Page ownership
/// and main-script Network delivery remain DedicatedWorker-specific here.
#[derive(Debug)]
pub(crate) struct DedicatedWorkerTargetState {
    pub(crate) renderer_instance_id: u64,
    pub(crate) owner_page: TargetPageResidenceIdentity,
    pub(crate) owner_page_network_sessions: Vec<Option<String>>,
    pub(crate) inner: SharedWorkerTargetState,
    retiring: bool,
    main_script: Option<DedicatedWorkerMainScriptSnapshot>,
    delivered_main_script_sessions: BTreeSet<String>,
    replayable_main_script_sessions: BTreeSet<String>,
    defer_failed_load_destroy_until_debugger_resume: bool,
    renderer_destroyed_while_waiting_for_debugger: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum DedicatedWorkerMainScriptOutcome {
    Loaded(Box<NavigationResponse>),
    Failed {
        error_message: String,
        response: Option<Box<NavigationResponse>>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct DedicatedWorkerMainScriptSnapshot {
    pub(crate) outcome: DedicatedWorkerMainScriptOutcome,
}

impl DedicatedWorkerTargetState {
    pub(crate) fn new(
        owner_page: TargetPageResidenceIdentity,
        renderer_owner_local_host_id: moli_core::RendererOwnerLocalHostId,
        renderer_instance_id: u64,
        target_id: String,
        name: String,
        owner_page_network_sessions: Vec<Option<String>>,
    ) -> Self {
        Self {
            renderer_instance_id,
            owner_page,
            owner_page_network_sessions,
            inner: SharedWorkerTargetState::new(
                renderer_owner_local_host_id,
                SharedWorkerInstanceId::from_u64(renderer_instance_id),
                target_id,
                None,
                String::new(),
                name,
            ),
            retiring: false,
            main_script: None,
            delivered_main_script_sessions: BTreeSet::new(),
            replayable_main_script_sessions: BTreeSet::new(),
            defer_failed_load_destroy_until_debugger_resume: false,
            renderer_destroyed_while_waiting_for_debugger: false,
        }
    }

    pub(crate) fn is_retiring(&self) -> bool {
        self.retiring
    }

    pub(crate) fn begin_retirement(&mut self) -> bool {
        !std::mem::replace(&mut self.retiring, true)
    }

    pub(crate) fn record_main_script(
        &mut self,
        script_url: String,
        outcome: DedicatedWorkerMainScriptOutcome,
        pause_failed_target_until_debugger_resume: bool,
    ) {
        self.defer_failed_load_destroy_until_debugger_resume =
            pause_failed_target_until_debugger_resume
                && matches!(&outcome, DedicatedWorkerMainScriptOutcome::Failed { .. });
        self.inner.url = script_url.clone();
        self.main_script = Some(DedicatedWorkerMainScriptSnapshot { outcome });
        self.delivered_main_script_sessions.clear();
        self.replayable_main_script_sessions.clear();
    }

    pub(crate) fn main_script(&self) -> Option<&DedicatedWorkerMainScriptSnapshot> {
        self.main_script.as_ref()
    }

    pub(crate) fn main_script_was_delivered_to(&self, session_id: &str) -> bool {
        self.delivered_main_script_sessions.contains(session_id)
    }

    pub(crate) fn allow_main_script_network_replay_to(&mut self, session_id: &str) {
        self.replayable_main_script_sessions
            .insert(session_id.to_owned());
    }

    pub(crate) fn main_script_network_replay_allowed_for(&self, session_id: &str) -> bool {
        self.replayable_main_script_sessions.contains(session_id)
    }

    pub(crate) fn discard_main_script_network_replay_for(&mut self, session_id: &str) {
        self.replayable_main_script_sessions.remove(session_id);
    }

    pub(crate) fn mark_main_script_delivered_to(&mut self, session_id: &str) {
        self.delivered_main_script_sessions
            .insert(session_id.to_owned());
        self.discard_main_script_network_replay_for(session_id);
    }

    pub(crate) fn defer_renderer_destroyed_for_debugger_resume(&mut self) -> bool {
        if !self.defer_failed_load_destroy_until_debugger_resume {
            return false;
        }
        self.renderer_destroyed_while_waiting_for_debugger = true;
        true
    }

    pub(crate) fn failed_load_waits_for_debugger(&self) -> bool {
        self.defer_failed_load_destroy_until_debugger_resume
    }

    pub(crate) fn release_deferred_renderer_destroyed_for_debugger_resume(&mut self) -> bool {
        if !self.renderer_destroyed_while_waiting_for_debugger {
            return false;
        }
        self.defer_failed_load_destroy_until_debugger_resume = false;
        self.renderer_destroyed_while_waiting_for_debugger = false;
        true
    }
}

impl Deref for DedicatedWorkerTargetState {
    type Target = SharedWorkerTargetState;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for DedicatedWorkerTargetState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

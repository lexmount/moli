use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use moli_core::page::{
    RendererDedicatedWorkerMainScript, RendererDedicatedWorkerMainScriptOutcome,
    RendererWorkerIdentity,
};
use moli_shared_worker::SharedWorkerInstanceId;

use super::{BrowserContext, SharedWorkerTargetState, TargetPageResidenceIdentity};

/// The creator is an exact Document or Worker execution, never an inferred
/// active Page. Nested workers can also be created by Shared/Service workers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DedicatedWorkerOwner {
    Document(TargetPageResidenceIdentity),
    Worker(RendererWorkerIdentity),
}

impl DedicatedWorkerOwner {
    pub(crate) fn target_id<'a>(&'a self, context: &'a BrowserContext) -> Option<&'a str> {
        match self {
            Self::Document(page) => context
                .target_page_residence_is_current(page)
                .then(|| page.target_id())
                .flatten(),
            Self::Worker(RendererWorkerIdentity::Dedicated(instance)) => {
                let target = context.dedicated_worker_targets.get(instance)?;
                target.owner.target_id(context)?;
                Some(&target.target_id)
            }
            Self::Worker(RendererWorkerIdentity::Shared(instance)) => {
                Some(&context.shared_worker_targets.get(instance)?.target_id)
            }
            Self::Worker(RendererWorkerIdentity::Service { version, run }) => {
                let target = context.service_worker_targets.get(version)?;
                (target.active_renderer_run() == Some(run)).then_some(target.target_id.as_str())
            }
        }
    }

    pub(crate) fn belongs_to_document(
        &self,
        context: &BrowserContext,
        document: &TargetPageResidenceIdentity,
    ) -> bool {
        match self {
            Self::Document(owner) => owner == document,
            Self::Worker(RendererWorkerIdentity::Dedicated(instance)) => context
                .dedicated_worker_targets
                .get(instance)
                .is_some_and(|target| target.owner.belongs_to_document(context, document)),
            Self::Worker(_) => false,
        }
    }
}

/// Protocol state for one renderer-owned DedicatedWorker lifetime.
///
/// The V8 inspector/session bookkeeping is identical to a SharedWorker target,
/// so the inner state intentionally reuses that implementation. Creator ownership
/// and main-script Network delivery remain DedicatedWorker-specific here.
#[derive(Debug)]
pub(crate) struct DedicatedWorkerTargetState {
    pub(crate) renderer_instance_id: u64,
    pub(crate) owner: DedicatedWorkerOwner,
    pub(crate) owner_network_sessions: Vec<Option<String>>,
    pub(crate) inner: SharedWorkerTargetState,
    main_script: Option<Arc<RendererDedicatedWorkerMainScript>>,
    delivered_main_script_sessions: BTreeSet<String>,
    replayable_main_script_sessions: BTreeSet<String>,
    defer_failed_load_destroy_until_debugger_resume: bool,
    renderer_destroyed_while_waiting_for_debugger: bool,
}

impl DedicatedWorkerTargetState {
    pub(crate) fn new(
        owner: DedicatedWorkerOwner,
        renderer_instance_id: u64,
        target_id: String,
        name: String,
        owner_network_sessions: Vec<Option<String>>,
    ) -> Self {
        Self {
            renderer_instance_id,
            owner,
            owner_network_sessions,
            inner: SharedWorkerTargetState::new(
                SharedWorkerInstanceId::from_u64(renderer_instance_id),
                target_id,
                None,
                String::new(),
                name,
            ),
            main_script: None,
            delivered_main_script_sessions: BTreeSet::new(),
            replayable_main_script_sessions: BTreeSet::new(),
            defer_failed_load_destroy_until_debugger_resume: false,
            renderer_destroyed_while_waiting_for_debugger: false,
        }
    }

    pub(crate) fn record_main_script(
        &mut self,
        script: Arc<RendererDedicatedWorkerMainScript>,
        pause_failed_target_until_debugger_resume: bool,
    ) {
        self.defer_failed_load_destroy_until_debugger_resume =
            pause_failed_target_until_debugger_resume
                && matches!(
                    &script.outcome,
                    RendererDedicatedWorkerMainScriptOutcome::Failed { .. }
                );
        self.inner.url = script.script_url.clone();
        self.main_script = Some(script);
        self.delivered_main_script_sessions.clear();
        self.replayable_main_script_sessions.clear();
    }

    pub(crate) fn main_script(&self) -> Option<&RendererDedicatedWorkerMainScript> {
        self.main_script.as_deref()
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

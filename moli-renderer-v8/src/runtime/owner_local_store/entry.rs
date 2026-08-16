use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum StandaloneNavigationFollowState {
    #[default]
    Idle,
    Following {
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    },
    FailedWithPendingNavigation {
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct PublishedReplacementDocument {
    pub(in crate::runtime) navigation_handoff:
        crate::page_task_queue::RendererTopLevelNavigationHandoff,
    pub(in crate::runtime) vm_creation_id: u64,
    pub(in crate::runtime) view_generation: u64,
}

impl StandaloneNavigationFollowState {
    pub(super) fn claim(
        &mut self,
        current: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
        requested: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
    ) -> bool {
        if matches!(
            *self,
            Self::FailedWithPendingNavigation { handoff } if Some(handoff) != current
        ) {
            *self = Self::Idle;
        }
        let Some(current) = current else {
            return false;
        };
        if requested.is_some_and(|requested| requested != current) {
            return false;
        }
        if !matches!(*self, Self::Idle) {
            return false;
        }
        *self = Self::Following { handoff: current };
        true
    }

    pub(super) fn settle(
        &mut self,
        current: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
        succeeded: bool,
    ) {
        *self = match *self {
            Self::Following { .. } if !succeeded => current
                .map(|handoff| Self::FailedWithPendingNavigation { handoff })
                .unwrap_or(Self::Idle),
            Self::Following { .. } => Self::Idle,
            Self::FailedWithPendingNavigation { handoff } if Some(handoff) != current => Self::Idle,
            state => state,
        };
    }
}

pub(in crate::runtime) struct LivePageEntry {
    pub(in crate::runtime) slot: RendererPageSlotHandle,
    pub(super) top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    pub(super) standalone_navigation_follow: StandaloneNavigationFollowState,
    // Keep executable continuation state before `vm`: Rust drops fields in
    // declaration order, so an exceptional entry drop still releases any
    // ScriptVm-bound task before releasing the PageVm itself.
    pub(super) pending_document_lifecycle_turn: Option<PendingDocumentLifecycleTurn>,
    pub(super) post_response_document_lifecycle: Option<RendererDocumentLifecycleIdentity>,
    pub(in crate::runtime) vm: Option<PageVm>,
    pub(super) pending_phase_one_navigation: Option<PageVmPendingPhaseOneNavigation>,
    pub(super) last_published_replacement_document: Option<PublishedReplacementDocument>,
}

/// A checked-out Page entry whose final active `PageVm` has been consumed by
/// teardown and which can therefore only return through the retiring
/// residence boundary.
///
/// The wrapped entry is deliberately private: owner code cannot convert this
/// state back into [`LivePageEntry`] or accidentally pass it to a live-entry
/// restore path.
pub(in crate::runtime) struct RetiringPageEntry {
    pub(super) entry: LivePageEntry,
}

impl RetiringPageEntry {
    pub(super) fn new(entry: LivePageEntry) -> Self {
        assert!(
            entry.active_page_vm().is_none(),
            "a retiring Page entry must not retain an active PageVm"
        );
        Self { entry }
    }

    pub(in crate::runtime) fn settle_standalone_navigation_follow(&mut self, succeeded: bool) {
        self.entry.settle_standalone_navigation_follow(succeeded);
    }
}

impl std::fmt::Debug for LivePageEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LivePageEntry")
            .field("slot", &self.slot)
            .field(
                "top_level_navigation_dispatch",
                &self.top_level_navigation_dispatch,
            )
            .field(
                "standalone_navigation_follow",
                &self.standalone_navigation_follow,
            )
            .field("vm", &self.vm)
            .field(
                "pending_document_lifecycle_turn",
                &self
                    .pending_document_lifecycle_turn
                    .as_ref()
                    .map(|pending| pending.document),
            )
            .field(
                "post_response_document_lifecycle",
                &self.post_response_document_lifecycle,
            )
            .field(
                "has_pending_phase_one_navigation",
                &self.pending_phase_one_navigation.is_some(),
            )
            .field(
                "last_published_replacement_document",
                &self.last_published_replacement_document,
            )
            .finish()
    }
}

impl LivePageEntry {
    pub(super) fn new(slot: RendererPageSlotHandle, mut vm: PageVm) -> Result<Self> {
        vm.bind_script_execution_control(slot.script_execution_control());
        Ok(Self {
            slot,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            standalone_navigation_follow: StandaloneNavigationFollowState::Idle,
            pending_document_lifecycle_turn: None,
            post_response_document_lifecycle: None,
            vm: Some(vm),
            pending_phase_one_navigation: None,
            last_published_replacement_document: None,
        })
    }

    pub(super) fn new_with_pending_phase_one_navigation(
        slot: RendererPageSlotHandle,
        mut pending: PageVmPendingPhaseOneNavigation,
    ) -> Result<Self> {
        pending
            .page_vm_mut()
            .bind_script_execution_control(slot.script_execution_control());
        let mut entry = Self {
            slot,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            standalone_navigation_follow: StandaloneNavigationFollowState::Idle,
            pending_document_lifecycle_turn: None,
            post_response_document_lifecycle: None,
            vm: None,
            pending_phase_one_navigation: None,
            last_published_replacement_document: None,
        };
        entry.prepare_pending_phase_one_navigation_install(&mut pending)?;
        entry.pending_phase_one_navigation = Some(pending);
        Ok(entry)
    }

    pub(in crate::runtime) fn page_vm(&self) -> &PageVm {
        self.active_page_vm()
            .expect("resident renderer page entry must retain an active PageVm")
    }

    pub(in crate::runtime) fn set_top_level_navigation_dispatch(
        &mut self,
        dispatch: RendererTopLevelNavigationDispatch,
    ) {
        self.top_level_navigation_dispatch = dispatch;
    }

    pub(in crate::runtime) fn top_level_navigation_dispatch(
        &self,
    ) -> RendererTopLevelNavigationDispatch {
        self.top_level_navigation_dispatch
    }

    /// Claim the single standalone owner chain for the current pending
    /// location navigation. A failed chain remains suppressed while that same
    /// descriptor is pending, so a duplicate producer handoff cannot restart
    /// the chain with a fresh limit.
    pub(in crate::runtime) fn begin_standalone_navigation_follow(&mut self) -> bool {
        self.begin_standalone_navigation_follow_for_handoff(None)
    }

    /// Claim a producer handoff only while the same request still occupies
    /// the ScriptVm's unique pending navigation slot. A delayed wake for an
    /// overwritten request therefore cannot start the replacement request.
    pub(in crate::runtime) fn begin_standalone_navigation_follow_from_handoff(
        &mut self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) -> bool {
        self.begin_standalone_navigation_follow_for_handoff(Some(handoff))
    }

    pub(super) fn begin_standalone_navigation_follow_for_handoff(
        &mut self,
        requested: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
    ) -> bool {
        if !matches!(
            self.top_level_navigation_dispatch,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter
        ) {
            return false;
        }
        let current = self
            .active_page_vm()
            .and_then(|page_vm| page_vm.vm().pending_location_navigation_handoff());
        self.standalone_navigation_follow.claim(current, requested)
    }

    pub(in crate::runtime) fn settle_standalone_navigation_follow(&mut self, succeeded: bool) {
        let current = self
            .active_page_vm()
            .and_then(|page_vm| page_vm.vm().pending_location_navigation_handoff());
        self.standalone_navigation_follow.settle(current, succeeded);
    }

    /// A replacement PageVm becomes the active owner-local runtime before its
    /// view is committed to the stable cross-thread Page slot.
    pub(in crate::runtime) fn has_uncommitted_page_vm(&self) -> bool {
        self.uncommitted_page_vm_creation_id().is_some()
    }

    pub(in crate::runtime) fn uncommitted_page_vm_creation_id(&self) -> Option<u64> {
        let stable_entry = self.slot.entry();
        if !stable_entry.is_active() {
            return None;
        }
        self.active_page_vm().and_then(|page_vm| {
            (stable_entry.vm_creation_id() != page_vm.creation_id).then_some(page_vm.creation_id)
        })
    }

    pub(super) fn publish_replacement_document_commit(
        &mut self,
    ) -> Result<PublishedReplacementDocument> {
        let stable_before = self.slot.entry();
        ensure!(
            stable_before.is_active(),
            "replacement Document cannot commit into a retired Page slot"
        );
        let page_vm = self
            .active_page_vm()
            .ok_or_else(|| anyhow!("replacement Document commit lost its active PageVm"))?;
        let vm_creation_id = page_vm.creation_id;
        let navigation_handoff =
            page_vm
                .replacement_document_commit_handoff()
                .ok_or_else(|| {
                    anyhow!("replacement PageVm is missing its navigation commit identity")
                })?;
        ensure!(
            stable_before.vm_creation_id() != vm_creation_id,
            "replacement Document commit attempted to republish stable PageVm {vm_creation_id}"
        );
        ensure!(
            self.last_published_replacement_document
                .is_none_or(|published| {
                    published.vm_creation_id != vm_creation_id
                        && published.navigation_handoff != navigation_handoff
                }),
            "replacement Document commit attempted to reuse a published navigation or PageVm identity"
        );

        self.page_vm_mut()
            .settle_replacement_document_commit(navigation_handoff)?;
        RendererOwnerLocalStore::commit_active_vm_page_state_on_entry(self)?;
        let stable_after = self.slot.entry();
        ensure!(
            stable_after.vm_creation_id() == vm_creation_id,
            "replacement Document publication did not install its PageVm identity"
        );
        ensure!(
            stable_after.view_generation > stable_before.view_generation,
            "replacement Document publication did not advance the stable view generation"
        );
        let published = PublishedReplacementDocument {
            navigation_handoff,
            vm_creation_id,
            view_generation: stable_after.view_generation,
        };
        self.last_published_replacement_document = Some(published);

        // Pending phase one owns the committed replacement. The old PageVm
        // was already terminated before the response commit and can no longer
        // be a rollback candidate.
        if self.pending_phase_one_navigation.is_some() {
            self.vm = None;
        }
        Ok(published)
    }

    pub(in crate::runtime) fn active_page_vm(&self) -> Option<&PageVm> {
        self.pending_phase_one_navigation
            .as_ref()
            .map(PageVmPendingPhaseOneNavigation::page_vm)
            .or(self.vm.as_ref())
    }

    pub(in crate::runtime) fn page_vm_mut(&mut self) -> &mut PageVm {
        let control = self.slot.script_execution_control();
        let page_vm = if let Some(pending) = self.pending_phase_one_navigation.as_mut() {
            pending.page_vm_mut()
        } else {
            self.vm
                .as_mut()
                .expect("resident renderer page entry must retain an active PageVm")
        };
        page_vm.bind_script_execution_control(control);
        page_vm
    }

    pub(in crate::runtime) fn pending_phase_one_navigation_has_ready_streaming_input(
        &mut self,
    ) -> bool {
        self.pending_phase_one_navigation
            .as_mut()
            .is_some_and(PageVmPendingPhaseOneNavigation::has_ready_streaming_input)
    }

    pub(super) fn page_vm_and_document_lifecycle_turn_mut(
        &mut self,
    ) -> (&mut PageVm, &mut Option<PendingDocumentLifecycleTurn>) {
        self.retire_stale_document_lifecycle_turn();
        let Self {
            vm,
            pending_document_lifecycle_turn,
            pending_phase_one_navigation,
            ..
        } = self;
        let page_vm = if let Some(pending) = pending_phase_one_navigation.as_mut() {
            pending.page_vm_mut()
        } else {
            vm.as_mut()
                .expect("resident renderer page entry must retain an active PageVm")
        };
        (page_vm, pending_document_lifecycle_turn)
    }

    pub(super) fn retire_document_lifecycle_turn(&mut self) {
        self.pending_document_lifecycle_turn = None;
        self.post_response_document_lifecycle = None;
    }

    pub(super) fn retire_stale_document_lifecycle_turn(&mut self) {
        let Some(pending_document) = self
            .pending_document_lifecycle_turn
            .as_ref()
            .map(|pending| pending.document)
        else {
            // A bounded lifecycle action may retire its resident before a
            // later operation in the same action fails. A post-response
            // continuation cannot survive without that exact resident.
            self.post_response_document_lifecycle = None;
            return;
        };
        let current_document = self.active_page_vm().map(|page_vm| {
            RendererDocumentLifecycleIdentity::from(page_vm.document_lifecycle.current_snapshot())
        });
        if current_document == Some(pending_document) {
            return;
        }
        tracing::debug!(
            ?pending_document,
            ?current_document,
            "retired stale lifecycle continuation at the stable page-residence boundary"
        );
        self.retire_document_lifecycle_turn();
    }

    pub(in crate::runtime) fn pending_document_lifecycle_identity(
        &mut self,
    ) -> Option<RendererDocumentLifecycleIdentity> {
        self.retire_stale_document_lifecycle_turn();
        self.pending_document_lifecycle_turn
            .as_ref()
            .map(|pending| pending.document)
    }

    pub(in crate::runtime) fn defer_document_lifecycle_until_response(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> Result<()> {
        anyhow::ensure!(
            self.pending_document_lifecycle_identity() == Some(document),
            "post-response lifecycle continuation does not match the resident Document"
        );
        self.post_response_document_lifecycle = Some(document);
        Ok(())
    }

    pub(super) fn document_lifecycle_is_deferred_until_response(&mut self) -> bool {
        let pending_document = self.pending_document_lifecycle_identity();
        if self.post_response_document_lifecycle == pending_document {
            return pending_document.is_some();
        }
        if self.post_response_document_lifecycle.is_some() {
            self.post_response_document_lifecycle = None;
        }
        false
    }

    pub(super) fn release_document_lifecycle_after_response(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> bool {
        if self.post_response_document_lifecycle != Some(document) {
            return false;
        }
        self.post_response_document_lifecycle = None;
        self.pending_document_lifecycle_identity() == Some(document)
    }

    pub(in crate::runtime) fn has_ready_main_parser_script_continuation(&mut self) -> bool {
        self.retire_stale_document_lifecycle_turn();
        let has_sealed_queue = self
            .pending_document_lifecycle_turn
            .as_ref()
            .is_some_and(|pending| pending.has_sealed_main_parser_script_queue);
        has_sealed_queue
            && self
                .page_vm_mut()
                .sealed_main_parser_script_continuation_is_ready()
    }

    pub(in crate::runtime) fn document_lifecycle_owner_turn_is_runnable(&mut self) -> bool {
        self.retire_stale_document_lifecycle_turn();
        self.pending_document_lifecycle_turn
            .as_ref()
            .is_some_and(|pending| pending.owner_turn_is_runnable)
    }

    pub(super) fn observe_document_lifecycle(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
    ) -> DocumentLifecycleObserverOutcome {
        self.retire_stale_document_lifecycle_turn();
        let page_vm = self.page_vm();
        let current_document = page_vm.document_lifecycle.identity();
        if current_document != document {
            return DocumentLifecycleObserverOutcome::DocumentReplaced {
                document: current_document,
            };
        }

        match page_vm.document_lifecycle_wait_outcome(
            renderer_document_lifecycle_milestone_for_stage(target_stage),
        ) {
            RendererDocumentLifecycleWaitOutcome::Reached(_) => {
                DocumentLifecycleObserverOutcome::Reached
            }
            RendererDocumentLifecycleWaitOutcome::Interrupted(termination) => {
                DocumentLifecycleObserverOutcome::Interrupted(termination)
            }
            RendererDocumentLifecycleWaitOutcome::Pending => {
                classify_pending_document_lifecycle_residence(
                    page_vm.vm().has_pending_location_navigation(),
                    self.pending_phase_one_navigation.is_some(),
                    self.pending_document_lifecycle_turn
                        .as_ref()
                        .is_some_and(|pending| pending.document == document),
                    page_vm.has_blocked_document_replacement_lifecycle_admission(document),
                )
            }
        }
    }

    pub(super) fn prepare_pending_phase_one_navigation_install(
        &self,
        pending: &mut PageVmPendingPhaseOneNavigation,
    ) -> Result<RendererPageToken> {
        pending
            .page_vm_mut()
            .bind_script_execution_control(self.slot.script_execution_control());
        let validation = if self.pending_phase_one_navigation.is_some() {
            Err(anyhow!(
                "renderer page already owns a pending phase-one navigation"
            ))
        } else if self.vm.as_ref().is_some_and(PageVm::has_live_script_vm) {
            Err(anyhow!(
                "phase-one-blocked replacement must be installed after the old document context is detached"
            ))
        } else {
            Ok(())
        };
        if let Err(error) = validation {
            Self::reject_pending_phase_one_navigation_state(
                pending,
                format!("Cannot install navigation: {error}"),
            );
            return Err(error);
        }
        let Some(wake_token) = pending.owner_wake_token() else {
            let error = anyhow!("pending phase-one navigation requires an owner wake token");
            Self::reject_pending_phase_one_navigation_state(
                pending,
                format!("Cannot install navigation: {error}"),
            );
            return Err(error);
        };
        Ok(wake_token)
    }

    /// Install a newly created replacement Document while retaining the Page's
    /// stable typed resource source. The replacement PageVm must share the
    /// source carried by its Page runtime environment; no receiver transfer is
    /// part of Document installation.
    pub(in crate::runtime) fn install_new_pending_phase_one_navigation(
        &mut self,
        mut pending: PageVmPendingPhaseOneNavigation,
    ) -> Result<RendererPageToken> {
        let wake_token = self.prepare_pending_phase_one_navigation_install(&mut pending)?;
        pending.attach_committed_response();
        self.retire_document_lifecycle_turn();
        self.pending_phase_one_navigation = Some(pending);
        Ok(wake_token)
    }

    /// Re-park the same phase-one creation runtime after one bounded
    /// phase-one turn. Its typed resource source already lives in the stable
    /// Page runtime environment and must not be replaced.
    pub(super) fn restore_pending_phase_one_navigation(
        &mut self,
        mut pending: PageVmPendingPhaseOneNavigation,
    ) -> Result<RendererPageToken> {
        let wake_token = self.prepare_pending_phase_one_navigation_install(&mut pending)?;
        self.pending_phase_one_navigation = Some(pending);
        Ok(wake_token)
    }

    pub(super) fn install_resumed_phase_one_page_vm(&mut self, mut page_vm: PageVm) {
        debug_assert!(
            self.pending_phase_one_navigation.is_none(),
            "a resumed phase-one PageVm cannot coexist with its consumed residence"
        );
        self.retire_document_lifecycle_turn();
        page_vm.bind_script_execution_control(self.slot.script_execution_control());
        self.vm = Some(page_vm);
    }

    pub(super) fn reject_pending_phase_one_navigation_state(
        pending: &mut PageVmPendingPhaseOneNavigation,
        message: String,
    ) {
        let browser_context_runtime = pending
            .page_vm()
            .runtime_hooks
            .browser_context_runtime
            .clone();
        pending
            .metadata
            .reject(None, &browser_context_runtime, message);
        pending.page_vm_mut().close_for_context_teardown();
    }

    pub(in crate::runtime) fn take_pending_phase_one_navigation(
        &mut self,
    ) -> Result<PageVmPendingPhaseOneNavigation> {
        self.pending_phase_one_navigation
            .take()
            .ok_or_else(|| anyhow!("renderer page has no pending phase-one navigation to resume"))
    }

    pub(super) fn reject_pending_phase_one_navigation_in_place(&mut self, message: &str) {
        let Some(mut pending) = self.pending_phase_one_navigation.take() else {
            return;
        };
        Self::reject_pending_phase_one_navigation_state(&mut pending, message.to_owned());
    }

    /// Reject the committed phase-one navigation and irreversibly transition
    /// this checked-out entry out of the live state.
    pub(in crate::runtime) fn reject_pending_phase_one_navigation(
        mut self,
        message: &str,
    ) -> RetiringPageEntry {
        self.reject_pending_phase_one_navigation_in_place(message);
        RetiringPageEntry::new(self)
    }

    pub(super) fn close_for_context_teardown(&mut self) {
        self.retire_document_lifecycle_turn();
        self.reject_pending_phase_one_navigation_in_place(
            "Location navigation was cancelled because its page was retired.",
        );
        if let Some(vm) = self.vm.as_mut() {
            vm.close_for_context_teardown();
        }
    }

    pub(super) fn next_javascript_timer_deadline(&self) -> Option<std::time::Instant> {
        self.page_vm().vm().next_timeout_deadline()
    }
}

pub(super) fn classify_pending_document_lifecycle_residence(
    has_pending_location_navigation: bool,
    has_pending_phase_one_navigation: bool,
    has_exact_document_lifecycle_turn: bool,
    has_blocked_document_replacement_lifecycle_admission: bool,
) -> DocumentLifecycleObserverOutcome {
    if has_pending_location_navigation {
        DocumentLifecycleObserverOutcome::NavigationPending
    } else if has_pending_phase_one_navigation
        || has_exact_document_lifecycle_turn
        || has_blocked_document_replacement_lifecycle_admission
    {
        DocumentLifecycleObserverOutcome::Pending
    } else {
        DocumentLifecycleObserverOutcome::MissingResident
    }
}

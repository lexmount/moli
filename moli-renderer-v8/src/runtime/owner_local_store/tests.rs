use super::*;

#[cfg(test)]
mod entry_local_task_guard_tests {
    use super::*;

    #[test]
    fn owner_local_store_binding_rejects_rebinding_without_replacing_the_active_store() {
        let mut active_store = RendererOwnerLocalStore::default();
        let mut rejected_store = RendererOwnerLocalStore::default();
        let binding = bind_render_runtime_owner_local_store(&mut active_store);

        let rebind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _rejected_binding = bind_render_runtime_owner_local_store(&mut rejected_store);
        }));

        assert!(rebind.is_err(), "a nested owner-local binding must fail");
        assert!(
            has_current_render_runtime_owner_local_store(),
            "rejecting a nested binding must preserve the active store"
        );
        drop(binding);
        assert!(
            !has_current_render_runtime_owner_local_store(),
            "dropping the active binding must clear the thread-local store"
        );
    }

    #[test]
    fn owner_local_store_binding_drop_does_not_double_panic_during_unwind() {
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut store = RendererOwnerLocalStore::default();
            let _binding = bind_render_runtime_owner_local_store(&mut store);
            CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
                assert!(
                    current_store.borrow_mut().take().is_some(),
                    "the test must clear an active binding before unwinding"
                );
            });
            panic!("primary owner-loop failure");
        }));

        assert!(unwind.is_err(), "the primary panic must remain catchable");
        assert!(
            !has_current_render_runtime_owner_local_store(),
            "unwinding must leave no thread-local owner store binding"
        );
    }

    #[test]
    fn guard_returns_entry_when_task_future_is_dropped_before_first_poll() {
        let (reply_tx, mut reply_rx) = oneshot::channel();
        let guard = bound::EntryLocalTaskGuard::<_, ()>::new(42_u8, reply_tx);
        let never_polled_task = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };

        drop(never_polled_task);

        let (entry, result) = reply_rx
            .try_recv()
            .expect("dropping the unpolled task should return its guarded entry");
        assert_eq!(entry, 42);
        assert!(
            result
                .expect_err("an unpolled task must not report successful completion")
                .to_string()
                .contains("before restoring its page entry")
        );
    }
}

#[cfg(test)]
mod navigation_dispatch_tests {
    use super::*;

    #[test]
    fn runnable_page_creation_lifecycle_clears_displaced_ordinary_grant() {
        let mut gate = LifecycleGate::new(PageVmInitStage::DomContentLoaded);

        gate.settle_lifecycle_turn(true);
        assert!(gate.reconsider_ordinary_on_next_turn);

        gate.settle_lifecycle_turn(false);
        assert!(!gate.reconsider_ordinary_on_next_turn);
    }

    fn phase_one_entry_shell_without_active_page_vm(
        standalone_navigation_follow: StandaloneNavigationFollowState,
    ) -> LivePageEntry {
        let page_id = PageId::new_for_testing(1);
        let (page_context_cancel_tx, _page_context_cancel_rx) =
            renderer_page_context_cancel_channel();
        let slot = RendererPageSlotHandle::new(
            std::sync::Weak::new(),
            RendererPageEntry::removed(page_id),
            page_context_cancel_tx,
            Default::default(),
        );
        LivePageEntry {
            slot,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            standalone_navigation_follow,
            pending_document_lifecycle_turn: None,
            post_response_document_lifecycle: None,
            vm: None,
            pending_phase_one_navigation: None,
            last_published_replacement_document: None,
        }
    }

    fn retiring_entry_without_active_page_vm(
        standalone_navigation_follow: StandaloneNavigationFollowState,
    ) -> RetiringPageEntry {
        RetiringPageEntry::new(phase_one_entry_shell_without_active_page_vm(
            standalone_navigation_follow,
        ))
    }

    #[test]
    fn retiring_navigation_settlement_does_not_require_an_active_vm() {
        let handoff = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(1);
        for succeeded in [false, true] {
            for state in [
                StandaloneNavigationFollowState::Idle,
                StandaloneNavigationFollowState::Following { handoff },
                StandaloneNavigationFollowState::FailedWithPendingNavigation { handoff },
            ] {
                let mut entry = retiring_entry_without_active_page_vm(state);

                entry.settle_standalone_navigation_follow(succeeded);

                assert_eq!(
                    entry.entry.standalone_navigation_follow,
                    StandaloneNavigationFollowState::Idle,
                    "an empty phase-one shell must settle {state:?} to Idle after succeeded={succeeded}"
                );
            }
        }
    }

    #[test]
    fn failed_phase_one_advance_returns_a_retiring_entry() {
        let advance = phase_one::classify_pending_phase_one_entry_advance(
            phase_one_entry_shell_without_active_page_vm(StandaloneNavigationFollowState::Idle),
            Err(anyhow!("resume failed")),
        );

        match advance {
            PendingPhaseOneEntryAdvance::Retiring { entry, error } => {
                assert!(entry.entry.active_page_vm().is_none());
                assert_eq!(error.to_string(), "resume failed");
            }
            PendingPhaseOneEntryAdvance::Live { .. } => {
                panic!("an empty phase-one shell must not escape as a LivePageEntry")
            }
        }
    }

    #[test]
    fn successful_phase_one_advance_without_a_vm_is_retired_as_an_invariant_error() {
        let advance = phase_one::classify_pending_phase_one_entry_advance(
            phase_one_entry_shell_without_active_page_vm(StandaloneNavigationFollowState::Idle),
            Ok(
                LivePagePendingNavigationPhaseOneAdvance::TriggeredNavigation {
                    stage: PageVmInitStage::DomContentLoaded,
                },
            ),
        );

        match advance {
            PendingPhaseOneEntryAdvance::Retiring { entry, error } => {
                assert!(entry.entry.active_page_vm().is_none());
                assert!(
                    error
                        .to_string()
                        .contains("completed without restoring an active PageVm")
                );
            }
            PendingPhaseOneEntryAdvance::Live { .. } => {
                panic!("an empty phase-one shell must not escape as a LivePageEntry")
            }
        }
    }

    #[test]
    fn navigation_handoff_claim_rejects_stale_and_duplicate_requests() {
        let first = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(1);
        let second = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(2);
        let mut state = StandaloneNavigationFollowState::Idle;

        assert!(!state.claim(Some(second), Some(first)));
        assert_eq!(state, StandaloneNavigationFollowState::Idle);
        assert!(state.claim(Some(second), Some(second)));
        assert_eq!(
            state,
            StandaloneNavigationFollowState::Following { handoff: second }
        );
        assert!(!state.claim(Some(second), Some(second)));
    }

    #[test]
    fn failed_navigation_suppresses_only_the_same_request_identity() {
        let first = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(1);
        let second = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(2);
        let mut state = StandaloneNavigationFollowState::Following { handoff: first };

        state.settle(Some(first), false);
        assert_eq!(
            state,
            StandaloneNavigationFollowState::FailedWithPendingNavigation { handoff: first }
        );
        assert!(!state.claim(Some(first), Some(first)));
        assert!(state.claim(Some(second), Some(second)));
        assert_eq!(
            state,
            StandaloneNavigationFollowState::Following { handoff: second }
        );
    }

    #[test]
    fn pending_document_lifecycle_classifies_each_stable_residence() {
        let cases = [
            (
                (true, false, false, false),
                DocumentLifecycleObserverOutcome::NavigationPending,
            ),
            (
                (false, true, false, false),
                DocumentLifecycleObserverOutcome::Pending,
            ),
            (
                (false, false, true, false),
                DocumentLifecycleObserverOutcome::Pending,
            ),
            (
                (false, false, false, true),
                DocumentLifecycleObserverOutcome::Pending,
            ),
            (
                (false, false, false, false),
                DocumentLifecycleObserverOutcome::MissingResident,
            ),
        ];

        for ((location, phase_one, lifecycle_turn, replacement), expected) in cases {
            assert_eq!(
                entry::classify_pending_document_lifecycle_residence(
                    location,
                    phase_one,
                    lifecycle_turn,
                    replacement,
                ),
                expected
            );
        }
    }

    #[test]
    fn page_creation_prioritizes_navigation_enqueued_by_reached_milestone() {
        assert_eq!(
            reconcile_page_creation_lifecycle_observation(
                DocumentLifecycleObserverOutcome::Reached,
                true,
            ),
            DocumentLifecycleObserverOutcome::NavigationPending
        );
    }

    #[test]
    fn page_creation_does_not_hide_an_interrupted_lifecycle() {
        let termination = RendererLifecycleTerminationStamp {
            sequence: 7,
            timestamp_micros: 11,
            reason: RendererDocumentTerminationReason::Detached,
        };

        assert_eq!(
            reconcile_page_creation_lifecycle_observation(
                DocumentLifecycleObserverOutcome::Interrupted(termination),
                true,
            ),
            DocumentLifecycleObserverOutcome::Interrupted(termination)
        );
    }

    #[test]
    fn published_page_creation_discards_reply_policy_when_observer_detaches() {
        let completion = LivePagePendingNavigationCompletion::PublishedPageCreation {
            navigation_reply_policy: NavigationReplyPolicy::ReturnWithPendingNavigation,
        };

        let (completion, detached) = completion.detach_command_observer();

        assert!(detached);
        assert!(matches!(
            completion,
            LivePagePendingNavigationCompletion::Background
        ));
    }

    #[test]
    fn detached_navigation_failure_routes_to_pending_page_creation() {
        assert_eq!(
            LivePagePendingNavigationCompletion::Background.failure_recipient(),
            LivePageNavigationFailureRecipient::PageCreationObserver
        );
    }

    #[test]
    fn already_published_page_creation_reports_later_navigation_failure_as_background() {
        let completion = LivePagePendingNavigationCompletion::PublishedPageCreation {
            navigation_reply_policy: NavigationReplyPolicy::FollowBeforeReply,
        };

        assert_eq!(
            completion.failure_recipient(),
            LivePageNavigationFailureRecipient::Background
        );
    }

    #[test]
    fn command_owned_navigation_failure_returns_to_its_initiator() {
        let completion = LivePagePendingNavigationCompletion::ReplyWithSnapshot {
            reply: Box::new(RendererPageReply::Unit),
            capture_policy: super::RendererPageStateCapturePolicy::FullReport,
        };

        assert_eq!(
            completion.failure_recipient(),
            LivePageNavigationFailureRecipient::Initiator
        );
    }
}

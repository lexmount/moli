use super::*;
use moli_protocol::CdpConnection;

#[tokio::test]
async fn ace_page_task_waiter_rejoins_once_and_releases_input_blocking() {
    #[derive(Debug)]
    struct UnreachableInput;
    impl moli_core::ace_page_task::AcePageTask for UnreachableInput {
        fn is_input(&self) -> bool {
            true
        }
        fn run(
            self: Box<Self>,
            _: &mut dyn moli_core::ace_page_task::PageExtensionHost,
        ) -> anyhow::Result<moli_core::ace_page_task::AcePageTaskOutput> {
            panic!("a missing target cannot execute renderer work")
        }
    }

    tokio::task::LocalSet::new()
        .run_until(async {
            for abort_recovery in [false, true] {
                let (mut scheduler, _protocol_completions) =
                    CdpScheduler::new(CdpConnection::new());
                let router = CdpFrontendRouter::new();
                let mut inputs = empty_scheduler_inputs();
                let mut deferred = VecDeque::new();
                let mut adapters = ProtocolAdapterScheduler::default();
                let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
                let (response_tx, _response_rx) = mpsc::unbounded_channel();
                let (reply, mut received) = oneshot::channel();
                let mut in_flight = InFlightCommands::new();
                let mut next_token = 1;
                ace_page_tasks::start(
                    &mut scheduler,
                    "missing-target".into(),
                    Box::new(UnreachableInput),
                    reply,
                    &completion_tx,
                    &mut in_flight,
                    &mut next_token,
                );
                assert!(page_javascript_owner_is_blocked(&scheduler, &in_flight));
                assert!(in_flight[&1].command().is_none(), "no CDP metadata");
                for turn in 0..2 {
                    let InFlightWork::AcePageTask(task) = &in_flight[&1] else {
                        panic!("independent page task entry");
                    };
                    if turn == 0 || abort_recovery {
                        task.wait_task.abort();
                    }
                    let completion = completion_rx.recv().await.unwrap();
                    assert_eq!(completion.identity.turn, turn);
                    let continued = handle_pending_command_completion(
                        &router,
                        &mut scheduler,
                        &mut inputs,
                        &mut deferred,
                        &response_tx,
                        &mut adapters,
                        &completion_tx,
                        &mut in_flight,
                        completion,
                    )
                    .await;
                    assert_eq!(continued, turn == 0 || !abort_recovery);
                    if turn == 0 {
                        assert!(
                            received.try_recv().is_err(),
                            "recovery still owns the reply"
                        );
                        assert!(page_javascript_owner_is_blocked(&scheduler, &in_flight));
                    }
                }
                assert!(in_flight.is_empty());
                assert!(!page_javascript_owner_is_blocked(&scheduler, &in_flight));
                let error = received.await.unwrap().unwrap_err();
                assert_eq!(error.code, if abort_recovery { 500 } else { 404 });
            }
        })
        .await;
}

fn empty_scheduler_inputs() -> SchedulerInputReceivers {
    let (_background_event_tx, background_event_rx) = mpsc::unbounded_channel();
    let (_navigation_tx, background_navigation_completion_rx) = mpsc::unbounded_channel();
    let (_renderer_tx, renderer_publication_rx) = moli_core::renderer_output_transport_channel();
    let (_activity_tx, document_activity_completion_rx) = mpsc::unbounded_channel();
    SchedulerInputReceivers::new(CdpSchedulerEventReceivers {
        state_events_ready: Default::default(),
        protocol_work_completion_rx: mpsc::unbounded_channel().1,
        document_activity_completion_rx,
        background_event_rx,
        background_navigation_completion_rx,
        renderer_publication_rx,
    })
}

use super::super::{
    context::FrameParserClassicScriptContext,
    owner::{
        FrameParserExternalLoadOwner, FrameParserRunnerTaskOwner, FrameParserScriptOwner,
        FrameParserSourceLoadClientOwner, FrameParserSourceLoadCompletionOwner,
        FrameParserSourceResultOwner,
    },
};
use super::{
    FrameParserClassicScriptItem, external_pending_frame_parser_classic_script_item,
    inline_frame_parser_classic_script_item,
};
use crate::{
    document_runtime::DomHandle,
    document_script_scheduler::{
        FrameDocumentClassicReadyWork, FrameDocumentClassicScriptSchedulerWork,
        FrameDocumentClassicSourceFailureWork,
    },
    dom::NodeId,
    frame_owner_model::{
        DocumentId, FrameDocumentClassicScriptBeginExecutionAction,
        FrameDocumentClassicScriptCompletionAction, FrameDocumentClassicScriptCompletionTarget,
        FrameDocumentClassicScriptScheduling, FrameDocumentClassicScriptSourceLoadClient,
        FrameDocumentClassicScriptSourceLoadCompletionAction,
        FrameDocumentClassicScriptSourceLoadOwner, FrameDocumentClassicScriptSourceLoadRequest,
        FrameDocumentOwner, FrameDocumentScriptElementEvent, FrameDocumentScriptElementEventKind,
        FrameDocumentTaskOwner, FrameRealmId, FrameRequestId, FrameSchedulerLaneId, LocalWindowId,
    },
    parser_script::{
        action::ParserPendingClassicScriptNotification,
        payload::{
            ParserClassicScriptMetadata, ParserClassicScriptSourceIdentity,
            ParserClassicScriptSourceResult, ParserPreparedClassicScript,
        },
        pending::ParserPendingClassicScriptEntry,
        runner::ParserClassicScriptRunner,
        slot::ParserClassicScriptRunnerSlot,
    },
    planning::{PreparedScript, ScriptFetchMetadata, ScriptSource},
    types::{
        ChildClassicScriptLoadCompletion, ChildClassicScriptNetworkAttribution, ScriptKind,
        ScriptMode, ScriptSourceKind,
    },
};
use url::Url;

#[derive(Debug, Clone)]
struct TestChildParserClassicScriptHarness {
    runner: ParserClassicScriptRunner<FrameParserClassicScriptContext>,
}

impl TestChildParserClassicScriptHarness {
    fn new(scripts: Vec<FrameParserClassicScriptItem>) -> Self {
        Self {
            runner: ParserClassicScriptRunner::new_parser_blocking(scripts),
        }
    }

    fn is_complete(&self) -> bool {
        self.runner.is_complete()
    }

    fn next_task(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
        owner_current: bool,
    ) -> Option<FrameDocumentClassicScriptSchedulerWork> {
        let mut owner = FrameParserRunnerTaskOwner {
            child_handle,
            task_owner: test_task_owner(owner),
            realm_id: Some(test_realm_id()),
            scheduling: FrameDocumentClassicScriptScheduling::ParserBlocking,
            owner_current,
        };
        self.runner
            .take_current_parser_blocking_next_action_with_owner(&mut owner)
    }

    fn begin_ready_execution(
        &mut self,
        child_handle: DomHandle,
        script_handle: DomHandle,
        owner: FrameDocumentOwner,
        owner_current: bool,
    ) -> Option<FrameDocumentClassicScriptBeginExecutionAction> {
        let mut owner = FrameParserScriptOwner {
            child_handle,
            task_owner: test_task_owner(owner),
            realm_id: Some(test_realm_id()),
            scheduling: FrameDocumentClassicScriptScheduling::ParserBlocking,
            pending_script_key: None,
            load_delay_token: None,
            owner_current,
        };
        self.runner
            .take_current_parser_blocking_begin_execution_action_with_owner(
                script_handle,
                &mut owner,
            )
    }

    fn dispose_ready_script(
        &mut self,
        child_handle: DomHandle,
        script_handle: DomHandle,
        owner: FrameDocumentOwner,
        owner_current: bool,
    ) -> Option<FrameDocumentClassicScriptCompletionAction> {
        let mut owner = FrameParserScriptOwner {
            child_handle,
            task_owner: test_task_owner(owner),
            realm_id: Some(test_realm_id()),
            scheduling: FrameDocumentClassicScriptScheduling::ParserBlocking,
            pending_script_key: None,
            load_delay_token: None,
            owner_current,
        };
        self.runner
            .take_current_parser_blocking_disposed_ready_action_with_owner(
                script_handle,
                &mut owner,
            )
    }

    fn finish_executing(
        &mut self,
        child_handle: DomHandle,
        script_handle: DomHandle,
        owner: FrameDocumentOwner,
        owner_current: bool,
    ) -> Option<FrameDocumentClassicScriptCompletionAction> {
        let mut owner = FrameParserScriptOwner {
            child_handle,
            task_owner: test_task_owner(owner),
            realm_id: Some(test_realm_id()),
            scheduling: FrameDocumentClassicScriptScheduling::ParserBlocking,
            pending_script_key: None,
            load_delay_token: None,
            owner_current,
        };
        self.runner
            .take_current_parser_blocking_finished_execution_action_with_owner(
                script_handle,
                &mut owner,
            )
    }

    fn source_load_client(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
    ) -> Option<FrameDocumentClassicScriptSourceLoadClient> {
        let mut owner = FrameParserSourceLoadClientOwner {
            child_handle,
            owner,
        };
        self.runner
            .current_parser_blocking_source_load_client_action_with_owner(&mut owner)
    }

    fn begin_external_load(
        &mut self,
        client: &FrameDocumentClassicScriptSourceLoadClient,
        load_id: u64,
        current_owner: FrameDocumentOwner,
        task_owner: FrameDocumentTaskOwner,
        owner_request_id: FrameRequestId,
    ) -> Option<FrameDocumentClassicScriptSourceLoadRequest> {
        let target = client.target();
        let mut owner = FrameParserExternalLoadOwner {
            child_handle: target.child_handle(),
            current_owner,
            client_owner: target.owner(),
            client_metadata: client.metadata(),
            client_script_url: client.script_url().clone(),
            task_owner,
            owner_request_id,
        };
        self.runner
            .take_current_parser_blocking_external_load_action_with_owner(load_id, &mut owner)
    }

    fn fail_external_pending_before_load(
        &mut self,
        client: &FrameDocumentClassicScriptSourceLoadClient,
        error: impl Into<String>,
    ) -> bool {
        self.runner
            .fail_current_parser_blocking_external_pending_before_load(
                client.metadata(),
                client.script_url(),
                error.into(),
            )
    }

    fn external_load_owner(
        &self,
        completion: &ChildClassicScriptLoadCompletion,
    ) -> Option<FrameDocumentClassicScriptSourceLoadCompletionAction> {
        let mut owner = FrameParserSourceLoadCompletionOwner { completion };
        self.runner
            .current_parser_blocking_source_load_completion_action_with_owner(&mut owner)
    }

    fn notify_external_source_result(
        &mut self,
        source_result: ParserClassicScriptSourceResult,
        owner_current: bool,
    ) -> Option<ParserPendingClassicScriptNotification> {
        let mut owner = FrameParserSourceResultOwner { owner_current };
        self.runner
            .take_current_parser_blocking_source_result_action_with_owner(source_result, &mut owner)
    }
}

fn child_parser_classic_script_item_from_runner_slot_for_test(
    runner_slot: ParserClassicScriptRunnerSlot,
    source_load_owner: Option<FrameDocumentClassicScriptSourceLoadOwner>,
) -> FrameParserClassicScriptItem {
    let mut context = FrameParserClassicScriptContext::new(
        test_task_owner(FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11))),
        test_owner_document_handle(),
        crate::document_script_scheduler::ParserPendingScriptKey::from_parts_for_test(
            1,
            NodeId::new(2),
        ),
    );
    context.source_load_owner = source_load_owner;
    crate::parser_script::item::ParserClassicScriptRunnerItem::from_slot_for_test(
        runner_slot,
        context,
    )
}

fn test_owner_document_handle() -> DomHandle {
    DomHandle::new(3)
}

fn test_task_owner(owner: FrameDocumentOwner) -> FrameDocumentTaskOwner {
    FrameDocumentTaskOwner::new(
        FrameSchedulerLaneId(91),
        owner.local_window_id,
        owner.document_id,
    )
}

fn test_realm_id() -> FrameRealmId {
    FrameRealmId(101)
}

fn ready_work(
    work: FrameDocumentClassicScriptSchedulerWork,
    owner: FrameDocumentOwner,
) -> FrameDocumentClassicReadyWork {
    match work {
        FrameDocumentClassicScriptSchedulerWork::Ready(ready) => {
            assert_eq!(ready.target().task_owner(), test_task_owner(owner));
            assert_eq!(ready.target().realm_id(), Some(test_realm_id()));
            ready
        }
        FrameDocumentClassicScriptSchedulerWork::SourceFailed(_) => {
            panic!("expected a ready child classic script task")
        }
    }
}

fn execution_source(execution: FrameDocumentClassicScriptBeginExecutionAction) -> String {
    let (_target, _execution, executable) = execution.into_parts();
    let PreparedScript { source, .. } = executable.into_prepared_script();
    match source {
        ScriptSource::Inline(source) | ScriptSource::Loaded(source) => source,
        ScriptSource::LoadedBinary { source, .. } => source,
        ScriptSource::External => unreachable!("test execution script should have loaded source"),
    }
}

fn source_failure_payload(
    work: FrameDocumentClassicScriptSchedulerWork,
    owner: FrameDocumentOwner,
) -> FrameDocumentClassicSourceFailureWork {
    match work {
        FrameDocumentClassicScriptSchedulerWork::SourceFailed(failure) => {
            assert_eq!(failure.target().task_owner(), test_task_owner(owner));
            assert_eq!(failure.target().realm_id(), Some(test_realm_id()));
            failure
        }
        FrameDocumentClassicScriptSchedulerWork::Ready(_) => {
            panic!("expected a source-failure child classic script task")
        }
    }
}

fn external_script(script_handle: DomHandle, script_url: Url) -> PreparedScript {
    PreparedScript {
        position: 0,
        node_id: NodeId::new(script_handle.index()),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Normal,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::External,
        url: script_url.clone(),
        base_url: script_url.clone(),
        initiator_url: script_url,
        host_script_handle: None,
    }
}

fn metadata(script_handle: DomHandle, start_line: u64) -> ParserClassicScriptMetadata {
    ParserClassicScriptMetadata::new(script_handle, start_line)
}

fn external_prepared_script(
    script_handle: DomHandle,
    start_line: u64,
    script_url: Url,
) -> ParserPreparedClassicScript {
    ParserPreparedClassicScript::new(
        metadata(script_handle, start_line),
        external_script(script_handle, script_url),
    )
}

fn external_loaded_prepared_script(
    script_handle: DomHandle,
    start_line: u64,
    script_url: Url,
    source: impl Into<String>,
) -> ParserPreparedClassicScript {
    let script = external_script(script_handle, script_url).with_loaded_source(source.into());
    ParserPreparedClassicScript::new(metadata(script_handle, start_line), script)
}

fn inline_prepared_script(
    script_handle: DomHandle,
    start_line: u64,
    script_url: Url,
    source: impl Into<String>,
) -> ParserPreparedClassicScript {
    let mut script = external_script(script_handle, script_url);
    script.source_kind = ScriptSourceKind::Inline;
    script.source = ScriptSource::Inline(source.into());
    ParserPreparedClassicScript::new(metadata(script_handle, start_line), script)
}

#[test]
fn child_pending_classic_source_failure_finishes_with_error_event() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-failure.test/failing.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_failed(
                    metadata(script_handle, 9),
                    script_url.clone(),
                    "network failure",
                ),
            ),
            None,
        ),
    ]);

    let failed = source_failure_payload(
        queue
            .next_task(child_handle, owner, true)
            .expect("failed external classic script should finish as a source failure task"),
        owner,
    );
    assert_eq!(failed.target().child_handle(), child_handle);
    assert_eq!(failed.script_handle(), script_handle);
    assert_eq!(failed.script_url(), &script_url);
    assert_eq!(failed.error(), "network failure");
    assert_eq!(
        failed.script_element_event(),
        Some(FrameDocumentScriptElementEvent {
            child_handle,
            owner,
            script_handle,
            kind: FrameDocumentScriptElementEventKind::Error,
        })
    );
    assert!(queue.is_complete());
    assert!(
        queue.next_task(child_handle, owner, true).is_none(),
        "source failure should advance the pending script queue"
    );
}

#[test]
fn child_pending_source_failure_ignores_stale_owner() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-failure.test/stale.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_failed(
                    metadata(script_handle, 9),
                    script_url.clone(),
                    "network failure",
                ),
            ),
            None,
        ),
    ]);

    assert!(
        queue.next_task(child_handle, owner, false).is_none(),
        "stale owner must not finish or advance the failed pending script"
    );
    assert!(
        !queue.is_complete(),
        "stale owner must leave the failed pending script in place"
    );

    let failed = source_failure_payload(
        queue
            .next_task(child_handle, owner, true)
            .expect("current owner should still be able to report the source failure"),
        owner,
    );
    assert_eq!(failed.script_handle(), script_handle);
    assert_eq!(failed.script_url(), &script_url);
}

#[test]
fn child_pending_external_classic_finish_queues_load_event_before_complete() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-load-event.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_ready(external_loaded_prepared_script(
                    script_handle,
                    13,
                    script_url,
                    "window.__ran = true",
                )),
            ),
            None,
        ),
    ]);

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready external classic script should execute first"),
        owner,
    );
    assert_eq!(ready.target().child_handle(), child_handle);
    assert_eq!(ready.script_handle(), script_handle);
    assert!(
        queue
            .begin_ready_execution(child_handle, script_handle, owner, true)
            .is_some()
    );
    let finished = queue.finish_executing(child_handle, script_handle, owner, true);
    assert_eq!(
        finished,
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            Some(FrameDocumentScriptElementEvent {
                child_handle,
                owner,
                script_handle,
                kind: FrameDocumentScriptElementEventKind::Load,
            },),
        ))
    );
    assert!(queue.is_complete());
}

#[test]
fn child_pending_ready_script_ignores_stale_execution_owner() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-stale-exec.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_ready(external_loaded_prepared_script(
                    script_handle,
                    17,
                    script_url,
                    "window.__ran = true",
                )),
            ),
            None,
        ),
    ]);

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready external classic script should be available"),
        owner,
    );
    assert_eq!(ready.script_handle(), script_handle);

    assert!(
        queue
            .begin_ready_execution(child_handle, script_handle, owner, false)
            .is_none(),
        "stale owner must not move the pending script into Executing"
    );
    assert_eq!(
        queue.finish_executing(child_handle, script_handle, owner, true,),
        None,
        "stale begin must not leave an executing script behind"
    );

    let ready_again = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("stale begin should keep the script ready for the current owner"),
        owner,
    );
    assert_eq!(ready_again.script_handle(), script_handle);
    assert!(
        queue
            .begin_ready_execution(child_handle, script_handle, owner, true)
            .is_some()
    );
    assert_eq!(
        queue.finish_executing(child_handle, script_handle, owner, true,),
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            Some(FrameDocumentScriptElementEvent {
                child_handle,
                owner,
                script_handle,
                kind: FrameDocumentScriptElementEventKind::Load,
            },),
        ))
    );
}

#[test]
fn child_pending_ready_task_ignores_stale_owner() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-stale-ready-task.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_ready(external_loaded_prepared_script(
                    script_handle,
                    18,
                    script_url,
                    "window.__ran = true",
                )),
            ),
            None,
        ),
    ]);

    assert_eq!(
        queue.next_task(child_handle, owner, false),
        None,
        "stale owner must not project a ready parser script task"
    );
    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("current owner should still see the ready parser script"),
        owner,
    );
    assert_eq!(ready.script_handle(), script_handle);
}

#[test]
fn child_parser_classic_pending_script_cannot_rebind_to_replacement_owner() {
    let child_handle = DomHandle::new(1);
    let captured_owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let replacement_owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(12));
    let script_handle = DomHandle::new(2);
    let mut queue =
        TestChildParserClassicScriptHarness::new(vec![inline_frame_parser_classic_script_item(
            inline_prepared_script(
                script_handle,
                18,
                Url::parse("https://child-classic-stable-owner.test/script.js")
                    .expect("script url"),
                "window.__ran = true",
            ),
            test_task_owner(captured_owner),
            test_owner_document_handle(),
        )]);

    assert_eq!(
        queue.next_task(child_handle, replacement_owner, true),
        None,
        "a current replacement owner must not claim an older PendingScript"
    );
    let ready = ready_work(
        queue
            .next_task(child_handle, captured_owner, true)
            .expect("the captured owner should remain the only valid route"),
        captured_owner,
    );
    assert_eq!(ready.target().task_owner(), test_task_owner(captured_owner));
    assert_eq!(ready.script_handle(), script_handle);
}

#[test]
fn child_pending_finish_ignores_stale_execution_owner() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-stale-finish.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_ready(external_loaded_prepared_script(
                    script_handle,
                    19,
                    script_url,
                    "window.__ran = true",
                )),
            ),
            None,
        ),
    ]);

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready external classic script should be available"),
        owner,
    );
    assert_eq!(ready.script_handle(), script_handle);
    assert!(
        queue
            .begin_ready_execution(child_handle, script_handle, owner, true)
            .is_some()
    );
    assert_eq!(
        queue.finish_executing(child_handle, script_handle, owner, false,),
        None,
        "stale finish must not advance the executing pending script"
    );
    assert_eq!(
        queue.finish_executing(child_handle, script_handle, owner, true,),
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            Some(FrameDocumentScriptElementEvent {
                child_handle,
                owner,
                script_handle,
                kind: FrameDocumentScriptElementEventKind::Load,
            },),
        ))
    );
}

#[test]
fn child_pending_external_load_request_carries_owner_and_load_identity() {
    let child_handle = DomHandle::new(1);
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-load.test/script.js").expect("script url");
    let owner_document_id = DocumentId(11);
    let owner_request_id = FrameRequestId(12);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), owner_document_id);
    let task_owner = test_task_owner(owner);
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        external_pending_frame_parser_classic_script_item(
            external_prepared_script(script_handle, 17, script_url.clone()),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
    ]);

    let client = queue
        .source_load_client(child_handle, owner)
        .expect("front external pending script should expose a source-load client");
    assert_eq!(client.target().child_handle(), child_handle);
    assert_eq!(client.target().owner(), owner);
    assert_eq!(client.metadata().script_handle(), script_handle);
    assert_eq!(client.metadata().start_line(), 17);
    assert_eq!(client.script_url(), &script_url);
    let stale_owner = FrameDocumentOwner::new(owner.local_window_id, DocumentId(99));
    assert!(
        queue
            .begin_external_load(&client, 16, stale_owner, task_owner, owner_request_id)
            .is_none(),
        "stale owner should not start an external load"
    );
    let request = queue
        .begin_external_load(&client, 17, owner, task_owner, owner_request_id)
        .expect("front external pending script should produce a source load request");

    assert_eq!(request.target().child_handle(), child_handle);
    assert_eq!(
        request
            .source_load_request()
            .source_identity()
            .metadata()
            .script_handle(),
        script_handle
    );
    assert_eq!(
        request
            .source_load_request()
            .source_identity()
            .metadata()
            .start_line(),
        17
    );
    assert_eq!(
        request.source_load_request().source_identity().load_id(),
        Some(17)
    );
    assert_eq!(request.target().owner_document_id(), owner_document_id);
    assert_eq!(request.target().task_owner(), task_owner);
    assert_eq!(request.target().owner_request_id(), owner_request_id);
    assert_eq!(
        request.source_load_request().input().script().url,
        script_url
    );
    assert!(queue.source_load_client(child_handle, owner).is_none());
    assert!(
        queue
            .begin_external_load(&client, 18, owner, task_owner, owner_request_id)
            .is_none(),
        "a loading pending script must not produce a second source load request"
    );
}

#[test]
fn child_pending_external_load_admission_failure_becomes_exact_source_failure() {
    let child_handle = DomHandle::new(31);
    let script_handle = DomHandle::new(37);
    let owner = FrameDocumentOwner::new(LocalWindowId(41), DocumentId(43));
    let script_url = Url::parse("https://child-classic-source-load.test/route-closed.js")
        .expect("script URL should parse");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        external_pending_frame_parser_classic_script_item(
            external_prepared_script(script_handle, 47, script_url.clone()),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
    ]);
    let client = queue
        .source_load_client(child_handle, owner)
        .expect("front pending script should expose its exact source-load client");

    assert!(queue.fail_external_pending_before_load(
        &client,
        "Page task route closed before source fetch start"
    ));
    assert!(
        !queue.fail_external_pending_before_load(&client, "duplicate failure"),
        "the same source-start reservation must settle only once"
    );
    assert!(queue.source_load_client(child_handle, owner).is_none());

    let failed = source_failure_payload(
        queue
            .next_task(child_handle, owner, true)
            .expect("pre-start admission failure must remain a runnable parser failure"),
        owner,
    );
    assert_eq!(failed.script_handle(), script_handle);
    assert_eq!(failed.script_url(), &script_url);
    assert_eq!(
        failed.error(),
        "Page task route closed before source fetch start"
    );
}

#[test]
fn child_pending_external_load_completion_carries_owner_and_record() {
    let child_handle = DomHandle::new(1);
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-load.test/complete.js").expect("script url");
    let task_owner = test_task_owner(FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11)));
    let queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_loading(
                    external_prepared_script(script_handle, 19, script_url.clone()),
                    ParserClassicScriptSourceIdentity::for_external_load(
                        metadata(script_handle, 19),
                        21,
                    ),
                ),
            ),
            Some(FrameDocumentClassicScriptSourceLoadOwner {
                task_owner,
                request_id: FrameRequestId(12),
            }),
        ),
    ]);
    let mut completion = ChildClassicScriptLoadCompletion {
        owner: task_owner,
        load_id: 21,
        handle: child_handle,
        script_handle,
        result: Ok("globalThis.__complete = true".to_owned()),
        network_result: None,
        network_attribution: ChildClassicScriptNetworkAttribution {
            frame_id: Some("child-frame".to_owned()),
            document_url: Url::parse("https://child-classic-source-load.test/document").unwrap(),
            request_url: script_url.clone(),
        },
    };

    completion.owner = FrameDocumentTaskOwner::new(
        FrameSchedulerLaneId(task_owner.scheduler_lane_id.0 + 1),
        task_owner.local_window_id,
        task_owner.document_id,
    );
    assert!(
        queue.external_load_owner(&completion).is_none(),
        "matching load/script IDs must not rebind a pending source across scheduler lanes"
    );
    completion.owner = task_owner;

    let owner = queue
        .external_load_owner(&completion)
        .expect("matching external load completion should expose the source-load owner");
    let (target, record) = owner.into_parts();
    assert_eq!(target.owner_document_id(), DocumentId(11));
    assert_eq!(target.task_owner(), completion.owner);
    assert_eq!(target.owner_request_id(), FrameRequestId(12));
    assert_eq!(
        record.source_identity().metadata().script_handle(),
        script_handle
    );
    assert_eq!(record.source_identity().metadata().start_line(), 19);
    assert_eq!(record.source_identity().load_id(), Some(21));
    assert_eq!(completion.network_attribution.request_url, script_url);
}

#[test]
fn child_pending_external_source_result_readies_front_pending_script() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-ready.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_loading(
                    external_prepared_script(script_handle, 23, script_url.clone()),
                    ParserClassicScriptSourceIdentity::for_external_load(
                        metadata(script_handle, 23),
                        17,
                    ),
                ),
            ),
            Some(FrameDocumentClassicScriptSourceLoadOwner {
                task_owner: test_task_owner(owner),
                request_id: FrameRequestId(12),
            }),
        ),
    ]);

    assert_eq!(
        queue.notify_external_source_result(
            ParserClassicScriptSourceResult::new(
                17,
                metadata(script_handle, 23),
                Ok("globalThis.__externalReady = true".to_owned()),
            ),
            true,
        ),
        Some(ParserPendingClassicScriptNotification::SourceReady)
    );

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready external source result should produce an executable parser script task"),
        owner,
    );
    assert_eq!(ready.target().child_handle(), child_handle);
    assert_eq!(ready.script_handle(), script_handle);
    assert_eq!(ready.script_url(), &script_url);
    let execution = queue
        .begin_ready_execution(child_handle, script_handle, owner, true)
        .expect("ready source should produce a child execution entry");
    assert_eq!(
        execution_source(execution),
        "globalThis.__externalReady = true"
    );
}

#[test]
fn child_pending_external_source_result_ignores_stale_owner() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-stale.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_loading(
                    external_prepared_script(script_handle, 23, script_url.clone()),
                    ParserClassicScriptSourceIdentity::for_external_load(
                        metadata(script_handle, 23),
                        17,
                    ),
                ),
            ),
            Some(FrameDocumentClassicScriptSourceLoadOwner {
                task_owner: test_task_owner(owner),
                request_id: FrameRequestId(12),
            }),
        ),
    ]);

    assert_eq!(
        queue.notify_external_source_result(
            ParserClassicScriptSourceResult::new(
                17,
                metadata(script_handle, 23),
                Ok("globalThis.__stale = true".to_owned()),
            ),
            false,
        ),
        None,
        "stale owner completion must not apply source result"
    );
    assert!(
        queue.next_task(child_handle, owner, true).is_none(),
        "stale owner completion must not ready the pending script"
    );
    assert_eq!(
        queue.notify_external_source_result(
            ParserClassicScriptSourceResult::new(
                17,
                metadata(script_handle, 23),
                Ok("globalThis.__fresh = true".to_owned()),
            ),
            true,
        ),
        Some(ParserPendingClassicScriptNotification::SourceReady)
    );

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("fresh source result should still ready the pending script"),
        owner,
    );
    assert_eq!(ready.script_handle(), script_handle);
    let execution = queue
        .begin_ready_execution(child_handle, script_handle, owner, true)
        .expect("fresh source should produce a child execution entry");
    assert_eq!(execution_source(execution), "globalThis.__fresh = true");
}

#[test]
fn child_pending_external_source_result_notifies_source_failure() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let script_url =
        Url::parse("https://child-classic-source-failed.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_loading(
                    external_prepared_script(script_handle, 29, script_url.clone()),
                    ParserClassicScriptSourceIdentity::for_external_load(
                        metadata(script_handle, 29),
                        18,
                    ),
                ),
            ),
            Some(FrameDocumentClassicScriptSourceLoadOwner {
                task_owner: test_task_owner(owner),
                request_id: FrameRequestId(12),
            }),
        ),
    ]);

    assert_eq!(
        queue.notify_external_source_result(
            ParserClassicScriptSourceResult::new(
                18,
                metadata(script_handle, 29),
                Err("network failure".to_owned()),
            ),
            true,
        ),
        Some(ParserPendingClassicScriptNotification::SourceFailed)
    );

    let failed = source_failure_payload(
        queue
            .next_task(child_handle, owner, true)
            .expect("failed external source result should produce a source failure task"),
        owner,
    );
    assert_eq!(failed.script_handle(), script_handle);
    assert_eq!(failed.script_url(), &script_url);
    assert_eq!(failed.error(), "network failure");
    assert_eq!(
        failed.script_element_event(),
        Some(FrameDocumentScriptElementEvent {
            child_handle,
            owner,
            script_handle,
            kind: FrameDocumentScriptElementEventKind::Error,
        })
    );
}
#[test]
fn child_parser_blocking_external_script_blocks_following_inline_script() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let external_script_handle = DomHandle::new(2);
    let inline_script_handle = DomHandle::new(4);
    let external_url =
        Url::parse("https://child-parser-blocking.test/blocking.js").expect("external url");
    let inline_url =
        Url::parse("https://child-parser-blocking.test/inline.html").expect("inline url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        external_pending_frame_parser_classic_script_item(
            external_prepared_script(external_script_handle, 31, external_url.clone()),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
        inline_frame_parser_classic_script_item(
            inline_prepared_script(
                inline_script_handle,
                37,
                inline_url,
                "globalThis.__afterBlocking = true",
            ),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
    ]);

    assert!(
        queue.next_task(child_handle, owner, true).is_none(),
        "later inline scripts must stay blocked behind the front external pending script"
    );
    let client = queue
        .source_load_client(child_handle, owner)
        .expect("front external pending script should expose a source-load client");
    assert_eq!(client.metadata().script_handle(), external_script_handle);
    let request = queue
        .begin_external_load(
            &client,
            17,
            owner,
            test_task_owner(owner),
            FrameRequestId(12),
        )
        .expect("front external pending script should start loading");
    assert_eq!(
        request
            .source_load_request()
            .source_identity()
            .metadata()
            .script_handle(),
        external_script_handle
    );
    assert!(
        queue.next_task(child_handle, owner, true).is_none(),
        "loading parser-blocking script must still block later inline scripts"
    );
    assert_eq!(
        queue.notify_external_source_result(
            ParserClassicScriptSourceResult::new(
                17,
                metadata(external_script_handle, 31),
                Ok("globalThis.__blockingRan = true".to_owned()),
            ),
            true,
        ),
        Some(ParserPendingClassicScriptNotification::SourceReady)
    );

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready external source should execute before later inline script"),
        owner,
    );
    assert_eq!(ready.script_handle(), external_script_handle);
    assert!(
        queue
            .begin_ready_execution(child_handle, external_script_handle, owner, true)
            .is_some()
    );
    assert_eq!(
        queue.finish_executing(child_handle, external_script_handle, owner, true,),
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            Some(FrameDocumentScriptElementEvent {
                child_handle,
                owner,
                script_handle: external_script_handle,
                kind: FrameDocumentScriptElementEventKind::Load,
            },),
        ))
    );

    let after = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("later inline script should run after parser-blocking external completes"),
        owner,
    );
    assert_eq!(after.script_handle(), inline_script_handle);
}

#[test]
fn child_pending_finish_with_wrong_script_handle_keeps_executing_script() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let wrong_script_handle = DomHandle::new(99);
    let script_url =
        Url::parse("https://child-classic-wrong-finish.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        child_parser_classic_script_item_from_runner_slot_for_test(
            ParserClassicScriptRunnerSlot::from_pending_entry(
                ParserPendingClassicScriptEntry::external_ready(external_loaded_prepared_script(
                    script_handle,
                    39,
                    script_url,
                    "window.__ran = true",
                )),
            ),
            None,
        ),
    ]);

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready external classic script should execute first"),
        owner,
    );
    assert_eq!(ready.script_handle(), script_handle);
    assert!(
        queue
            .begin_ready_execution(child_handle, script_handle, owner, true)
            .is_some()
    );
    assert_eq!(
        queue.finish_executing(child_handle, wrong_script_handle, owner, true,),
        None,
        "wrong script handle must not finish or corrupt the executing pending script"
    );
    assert_eq!(
        queue.finish_executing(child_handle, script_handle, owner, true,),
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            Some(FrameDocumentScriptElementEvent {
                child_handle,
                owner,
                script_handle,
                kind: FrameDocumentScriptElementEventKind::Load,
            },),
        ))
    );
}

#[test]
fn child_pending_ready_script_can_be_disposed_without_load_event() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let original_owner_document_handle = DomHandle::new(3);
    let script_url =
        Url::parse("https://child-classic-dispose.test/script.js").expect("script url");
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        inline_frame_parser_classic_script_item(
            inline_prepared_script(
                script_handle,
                41,
                script_url,
                "globalThis.__shouldNotRun = true",
            ),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
        inline_frame_parser_classic_script_item(
            inline_prepared_script(
                DomHandle::new(4),
                43,
                Url::parse("https://child-classic-dispose.test/after.js").expect("after url"),
                "globalThis.__afterRuns = true",
            ),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
    ]);

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready inline classic script should execute first"),
        owner,
    );
    assert_eq!(
        ready.target().original_owner_document_handle(),
        original_owner_document_handle
    );
    assert_eq!(
        queue.dispose_ready_script(child_handle, script_handle, owner, true),
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            None,
        ))
    );

    let after = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("disposing stale ready script should advance to the next parser script"),
        owner,
    );
    assert_eq!(after.script_handle(), DomHandle::new(4));
}

#[test]
fn child_pending_ready_script_dispose_ignores_stale_owner() {
    let child_handle = DomHandle::new(1);
    let owner = FrameDocumentOwner::new(LocalWindowId(10), DocumentId(11));
    let script_handle = DomHandle::new(2);
    let mut queue = TestChildParserClassicScriptHarness::new(vec![
        inline_frame_parser_classic_script_item(
            inline_prepared_script(
                script_handle,
                47,
                Url::parse("https://child-classic-dispose-stale.test/script.js")
                    .expect("script url"),
                "globalThis.__shouldNotRun = true",
            ),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
        inline_frame_parser_classic_script_item(
            inline_prepared_script(
                DomHandle::new(4),
                49,
                Url::parse("https://child-classic-dispose-stale.test/after.js").expect("after url"),
                "globalThis.__afterRuns = true",
            ),
            test_task_owner(owner),
            test_owner_document_handle(),
        ),
    ]);

    let ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("ready inline classic script should execute first"),
        owner,
    );
    assert_eq!(ready.script_handle(), script_handle);
    assert_eq!(
        queue.dispose_ready_script(child_handle, script_handle, owner, false),
        None,
        "stale owner must not dispose or advance the ready pending script"
    );

    let still_ready = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("stale dispose should keep the same ready script current"),
        owner,
    );
    assert_eq!(still_ready.script_handle(), script_handle);
    assert_eq!(
        queue.dispose_ready_script(child_handle, script_handle, owner, true),
        Some(FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                test_task_owner(owner),
                test_realm_id()
            ),
            None,
        ))
    );
    let after = ready_work(
        queue
            .next_task(child_handle, owner, true)
            .expect("current dispose should advance to following parser script"),
        owner,
    );
    assert_eq!(after.script_handle(), DomHandle::new(4));
}

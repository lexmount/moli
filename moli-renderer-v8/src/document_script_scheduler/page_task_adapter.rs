use super::{
    DocumentScriptExecutionLane, DocumentScriptScheduler, DocumentScriptSourceFailureLane,
    PageOwnedDocumentScriptWork,
    completion_port::ParseTimeAsyncCompletionPort,
    parse_time_task::ParseTimeDocumentScriptEvent,
    post_parse_task::{PostParseAsyncScriptTask, PostParseDocumentScriptTask},
    runner::DocumentScriptRunnerPostParsePlan,
};
use crate::dom::NodeId;
use crate::page_task_queue::{
    PostParseLifecycleWork, PostParsePageOwnedWork, RendererOwnerWakeSender,
};
use crate::planning::PreparedScriptSourceLoadOutcome;
use crate::stylesheet_blocking::{
    DocumentOwnedBlockingStylesheetDiscoveryInput, StylesheetBlockingReadView,
    collect_document_owned_blocking_stylesheets,
};

pub(crate) struct ParserPreparedPostParseHandoff {
    page_owned_work: Vec<PostParsePageOwnedWork>,
}

impl ParserPreparedPostParseHandoff {
    pub(crate) fn into_page_owned_work(self) -> Vec<PostParsePageOwnedWork> {
        self.page_owned_work
    }
}

impl<Target, ParserModuleEvaluation, ParserModuleGraphFailure>
    DocumentScriptScheduler<Target, ParserModuleEvaluation, ParserModuleGraphFailure>
{
    pub(crate) fn bind_parse_time_completion_event_injection(
        &mut self,
        tx: tokio::sync::mpsc::UnboundedSender<ParseTimeDocumentScriptEvent>,
        owner_wake: Option<RendererOwnerWakeSender>,
    ) {
        self.runner
            .bind_parse_time_async_completion_port(parse_time_completion_event_port(
                tx, owner_wake,
            ));
    }

    pub(crate) async fn finalize_live_parser_post_parse_handoff(
        self,
        document: &impl StylesheetBlockingReadView,
    ) -> ParserPreparedPostParseHandoff {
        let stylesheet_seed_inputs =
            collect_post_parse_stylesheet_seed_inputs_from_read_view(document);
        self.finalize_parser_prepared_post_parse_handoff(stylesheet_seed_inputs)
            .await
    }

    pub(crate) async fn finalize_parser_prepared_post_parse_handoff(
        self,
        stylesheet_seed_inputs: Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) -> ParserPreparedPostParseHandoff {
        let plan = self.finalize_owned_script_work().await;
        post_parse_plan_into_owner_handoff(plan, stylesheet_seed_inputs)
    }

    pub(super) async fn finalize_owned_script_work(self) -> DocumentScriptRunnerPostParsePlan {
        let Self {
            parser_runner: _,
            runner,
        } = self;
        runner.finalize_owned_script_work()
    }

    pub(crate) fn absorb_stranded_parse_time_document_script_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = ParseTimeDocumentScriptEvent>,
    {
        for event in events {
            self.absorb_stranded_parse_time_document_script_event(event);
        }
    }

    fn absorb_stranded_parse_time_document_script_event(
        &mut self,
        event: ParseTimeDocumentScriptEvent,
    ) {
        match event {
            ParseTimeDocumentScriptEvent::ReadyTask(task) => {
                self.runner
                    .absorb_stranded_parse_time_document_script_task(*task);
            }
            ParseTimeDocumentScriptEvent::AsyncCompletion(completion) => {
                let (node_id, outcome) = completion.into_parts();
                let _ = self
                    .runner
                    .accept_injected_parse_time_async_completion(node_id, outcome);
            }
        }
    }
}

fn parse_time_completion_event_port(
    tx: tokio::sync::mpsc::UnboundedSender<ParseTimeDocumentScriptEvent>,
    owner_wake: Option<RendererOwnerWakeSender>,
) -> ParseTimeAsyncCompletionPort {
    ParseTimeAsyncCompletionPort::new(
        move |node_id: NodeId, outcome: PreparedScriptSourceLoadOutcome| {
            if tx
                .send(ParseTimeDocumentScriptEvent::async_completion(
                    node_id, outcome,
                ))
                .is_err()
            {
                return false;
            }
            // Publish the concrete terminal before notifying the owner. The
            // open-stream continuation can therefore materialize the exact
            // payload even when this wake races the raw body source.
            if let Some(owner_wake) = owner_wake.as_ref() {
                owner_wake.signal_parse_time_document_script_work();
            }
            true
        },
    )
}

fn post_parse_document_script_task_into_page_owned_work(
    task: PostParseDocumentScriptTask,
) -> PostParsePageOwnedWork {
    match task {
        PostParseDocumentScriptTask::AsyncScript(task) => {
            post_parse_async_script_task_into_page_owned_work(*task)
        }
    }
}

fn post_parse_async_script_task_into_page_owned_work(
    task: PostParseAsyncScriptTask,
) -> PostParsePageOwnedWork {
    match task {
        PostParseAsyncScriptTask::Ready {
            script,
            load_delay_binding,
        } => PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::parser_async_script(
                DocumentScriptExecutionLane::AsyncPhase,
                script,
                load_delay_binding,
            ),
        ),
        PostParseAsyncScriptTask::WaitingForSource {
            script,
            source_load,
            load_delay_binding,
        } => PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::parser_async_script_waiting_for_source(
                DocumentScriptExecutionLane::AsyncPhase,
                script,
                source_load,
                load_delay_binding,
            ),
        ),
        PostParseAsyncScriptTask::Failure {
            script,
            error,
            source_network_result,
            load_delay_binding,
        } => PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::parser_async_source_failure(
                DocumentScriptSourceFailureLane::AsyncPhase,
                script,
                error,
                source_network_result,
                load_delay_binding,
            ),
        ),
    }
}

pub(super) fn post_parse_plan_into_owner_handoff(
    plan: DocumentScriptRunnerPostParsePlan,
    stylesheet_seed_inputs: Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
) -> ParserPreparedPostParseHandoff {
    let async_tasks = plan.into_async_tasks();
    let mut work = Vec::with_capacity(runtime_ready_post_parse_page_task_capacity(&async_tasks));
    if !stylesheet_seed_inputs.is_empty() {
        work.push(PostParsePageOwnedWork::lifecycle_work(
            PostParseLifecycleWork::SeedDocumentOwnedBlockingStylesheets(stylesheet_seed_inputs),
        ));
    }
    work.extend(
        async_tasks
            .into_iter()
            .map(post_parse_document_script_task_into_page_owned_work),
    );
    ParserPreparedPostParseHandoff {
        page_owned_work: work,
    }
}

fn collect_post_parse_stylesheet_seed_inputs_from_read_view(
    read_view: &impl StylesheetBlockingReadView,
) -> Vec<DocumentOwnedBlockingStylesheetDiscoveryInput> {
    collect_document_owned_blocking_stylesheets(read_view)
        .iter()
        .map(DocumentOwnedBlockingStylesheetDiscoveryInput::from)
        .collect()
}

fn runtime_ready_post_parse_page_task_capacity(
    async_tasks: &[PostParseDocumentScriptTask],
) -> usize {
    async_tasks.len() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{HtmlParser, ParserScriptHandoff};
    use url::Url;

    #[tokio::test]
    async fn parse_time_completion_is_resident_before_its_owner_wake() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(91)),
        );
        let port = parse_time_completion_event_port(event_tx, Some(owner_wake));

        assert!(port.send(
            NodeId::new(17),
            PreparedScriptSourceLoadOutcome {
                source_result: Ok("window.ready = true;".to_owned()),
                source_bytes: None,
                network_result: None,
                muted_errors: false,
            },
        ));

        let wake = wake_rx
            .recv()
            .await
            .expect("stored parse-time completion should wake its owner");
        assert_eq!(
            wake.source_for_test(),
            crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork
        );
        let event = event_rx
            .try_recv()
            .expect("parse-time completion payload must be resident before its wake");
        assert!(matches!(
            event,
            ParseTimeDocumentScriptEvent::AsyncCompletion(_)
        ));
    }

    #[tokio::test]
    async fn live_parser_finalization_does_not_rediscover_parser_scripts_from_dom() {
        let document = HtmlParser::SCRIPTING_ENABLED.parse(
            Url::parse("https://post-parse.test/page.html").expect("test url"),
            "<!doctype html><script type='module'>export const value = 1;</script>".to_owned(),
        );

        let handoff = DocumentScriptScheduler::new()
            .finalize_live_parser_post_parse_handoff(&document)
            .await;
        let page_owned_work = handoff.into_page_owned_work();
        assert!(
            page_owned_work.is_empty(),
            "normal live-parser finalization must consume only work accepted during parser handoff"
        );
    }

    #[tokio::test]
    async fn parser_defer_finalization_preserves_handoff_stylesheet_snapshot() {
        let document_url =
            Url::parse("https://post-parse.test/page.html").expect("test document URL");
        let (mut document, handoffs, _) = crate::parse_html_test_fixture_with_parser_outputs(
            document_url,
            concat!(
                "<!doctype html><link rel='stylesheet' href='/before.css'>",
                "<script type='module'>export const value = 1;</script>",
            )
            .to_owned(),
        );
        let ParserScriptHandoff::NonAsyncPostParse {
            script,
            blocking_signatures_before,
            ..
        } = handoffs.into_iter().next().expect("module script handoff")
        else {
            panic!("expected non-async module script handoff");
        };
        assert_eq!(blocking_signatures_before.len(), 1);

        let link = document
            .elements_by_tag_name(document.document_node_id(), "link", false)
            .into_iter()
            .next()
            .expect("stylesheet link");
        assert!(document.set_attribute(link, "href", "/after.css"));

        let owner = 7_u64;
        let mut store: crate::document_script_scheduler::DocumentScriptSchedulerStore<u64> =
            Default::default();
        assert!(matches!(
            store.claim_parser_deferred_script(
                owner,
                script,
                None,
                None,
                blocking_signatures_before.clone(),
                crate::frame_owner_model::DocumentLoadDelayTokenId(1),
            ),
            Some(crate::document_script_scheduler::ParserDeferredScriptStartAction::ModuleGraph(_))
        ));
        assert_eq!(store.seal_parser_deferred_scripts(owner), Ok(1));

        assert_eq!(
            store.next_after_parsing_blocking_signatures(owner),
            Some(&blocking_signatures_before),
            "post-parse finalization must use the parser handoff snapshot, not the mutated DOM"
        );
    }
}

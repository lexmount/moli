use anyhow::{Result, anyhow};
use moli_encoding::HtmlDocumentStreamingDecoder;
use std::time::Instant;

use super::streaming::{
    RawDocumentBodySource, enqueue_streaming_raw_chunk, sync_document_character_set_from_decoder,
};
use super::streaming_input::{StreamingDocumentInputEvent, StreamingDocumentInputSource};
use super::{
    ConcurrentParseTimeRuntime, PageVm, ParseTimePageVmCreationOutcome,
    ParseTimePageVmStreamingProgress, PendingPhaseOneResidence, PendingPhaseOneResumeOutcome,
    ServiceWorkerScriptPreloadContext,
};

/// Exact, parkable continuation for an open main-Document response.
///
/// The parser runtime, decoder, preload state and raw-input residence move as
/// one value into the stable Page slot. Page task wakes and body-input wakes
/// can then resume the same continuation without keeping an owner-local future
/// borrowed across network input.
pub(in crate::runtime) struct PendingStreamingPhaseOneContinuation {
    runtime: Box<ConcurrentParseTimeRuntime>,
    input: StreamingDocumentInputSource,
    decoder: HtmlDocumentStreamingDecoder,
    service_worker_preload_context: Option<ServiceWorkerScriptPreloadContext>,
    started: Instant,
    deferred_main_resource_failure: Option<anyhow::Error>,
}

impl PendingStreamingPhaseOneContinuation {
    pub(super) fn publish_pending_page_creation_phase(&self) {
        self.runtime.publish_pending_page_creation_phase();
    }

    pub(super) fn bridge(
        runtime: ConcurrentParseTimeRuntime,
        raw_body: RawDocumentBodySource,
        decoder: HtmlDocumentStreamingDecoder,
        service_worker_preload_context: Option<ServiceWorkerScriptPreloadContext>,
        started: Instant,
    ) -> Result<Self> {
        let parser_continuation = runtime
            .page_vm()
            .vm()
            .document_runtime
            .main_parser_continuation_producer()
            .ok_or_else(|| {
                anyhow!("open streaming parser continuation requires an active Networking producer")
            })?;
        let task_runner = runtime.page_vm().resource_task_runner();
        Ok(Self {
            runtime: Box::new(runtime),
            input: StreamingDocumentInputSource::bridge(raw_body, parser_continuation, task_runner),
            decoder,
            service_worker_preload_context,
            started,
            deferred_main_resource_failure: None,
        })
    }

    pub(in crate::runtime) fn page_vm(&self) -> &PageVm {
        self.runtime.page_vm()
    }

    pub(in crate::runtime) fn page_vm_mut(&mut self) -> &mut PageVm {
        self.runtime.page_vm_mut()
    }

    pub(in crate::runtime) fn owner_wake_token(&self) -> Option<crate::runtime::RendererPageToken> {
        self.runtime.owner_wake_token()
    }

    pub(in crate::runtime) fn has_ready_input(&mut self) -> bool {
        self.input.has_ready_input()
    }

    pub(in crate::runtime) async fn resume(self) -> Result<PendingPhaseOneResumeOutcome> {
        self.runtime.publish_processing_main_document_phase();
        let Self {
            runtime,
            mut input,
            mut decoder,
            service_worker_preload_context,
            started,
            mut deferred_main_resource_failure,
        } = self;
        let mut runtime = *runtime;
        let mut input_finished = false;
        let mut observed_input = deferred_main_resource_failure.is_some();
        if deferred_main_resource_failure.is_none() {
            while let Some(event) = input.try_next()? {
                observed_input = true;
                match event {
                    StreamingDocumentInputEvent::Chunks(chunks) => {
                        for chunk in chunks {
                            enqueue_streaming_raw_chunk(
                                &mut runtime,
                                &mut decoder,
                                chunk,
                                service_worker_preload_context.as_ref(),
                            );
                        }
                    }
                    StreamingDocumentInputEvent::Finished(result) => {
                        if let Err(message) = result {
                            deferred_main_resource_failure = Some(anyhow!(message));
                            break;
                        }
                        if let Some(tail) = decoder.finish() {
                            runtime.enqueue_streaming_html_chunk(tail);
                        }
                        sync_document_character_set_from_decoder(&mut runtime, &decoder);
                        runtime.close_streaming_html_input();
                        input_finished = true;
                        break;
                    }
                }
            }
        }

        // A body terminal is sequenced after every chunk produced before it.
        // If those bytes are resident but their Networking continuation has
        // not yet been selected by the stable Page scheduler, keep the failed
        // terminal with this exact parser residence. Applying it now would
        // destroy parser state before the already-delivered partial DOM can be
        // materialized. This is an ordering edge, not a retry: the existing
        // parser task owns the only wake and admission needed to resume us.
        let selected_main_parser_continuation = runtime
            .page_vm()
            .vm()
            .document_runtime
            .has_main_parser_continuation_admission();
        if deferred_main_resource_failure.is_some()
            && !selected_main_parser_continuation
            && !runtime.state.parser_session.input_is_empty()
        {
            let continuation = Self {
                runtime: Box::new(runtime),
                input,
                decoder,
                service_worker_preload_context,
                started,
                deferred_main_resource_failure,
            };
            return Ok(PendingPhaseOneResumeOutcome::progress(
                ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::open_streaming(Box::new(continuation)),
                ),
            ));
        }

        // Body input and parse-time script terminals wake the same parked
        // continuation, but they do not grant the same owner transition.
        // Input (including EOF) belongs to the parser and must reach its fresh
        // owner/cutoff decision before an unrelated async terminal that merely
        // happened to be resident at the same time. Only a wake with no input
        // fact may materialize the exact parse-time Document task here.
        if !observed_input {
            let _ = runtime.admit_ready_open_stream_document_work();
        }

        let progress = runtime
            .continue_streaming_creation_on_execution_context(started)
            .await?;
        if let Some(error) = deferred_main_resource_failure {
            return Ok(match progress {
                ParseTimePageVmStreamingProgress::NeedMoreInput(runtime)
                | ParseTimePageVmStreamingProgress::PendingPageTask(runtime) => {
                    PendingPhaseOneResumeOutcome::main_resource_load_failed(*runtime, error)
                }
                // Script in the bytes delivered before the failed terminal may
                // replace the Document or start a navigation. The replacement
                // owns the Page; do not apply the old loader's terminal to it.
                ParseTimePageVmStreamingProgress::TriggeredNavigation { page_vm, stage } => {
                    PendingPhaseOneResumeOutcome::progress(
                        ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage },
                    )
                }
                ParseTimePageVmStreamingProgress::ContinuePhaseTwo {
                    page_vm,
                    page_tasks,
                    stage,
                    started,
                } => PendingPhaseOneResumeOutcome::progress(
                    ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                        page_vm,
                        page_tasks,
                        stage,
                        started,
                    },
                ),
            });
        }
        let outcome = match progress {
            ParseTimePageVmStreamingProgress::NeedMoreInput(runtime) if !input_finished => {
                let continuation = Self {
                    runtime,
                    input,
                    decoder,
                    service_worker_preload_context,
                    started,
                    deferred_main_resource_failure: None,
                };
                ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::open_streaming(Box::new(continuation)),
                )
            }
            ParseTimePageVmStreamingProgress::PendingPageTask(runtime) if !input_finished => {
                let continuation = Self {
                    runtime,
                    input,
                    decoder,
                    service_worker_preload_context,
                    started,
                    deferred_main_resource_failure: None,
                };
                ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::open_streaming(Box::new(continuation)),
                )
            }
            ParseTimePageVmStreamingProgress::NeedMoreInput(runtime)
                if runtime.has_pending_parser_blocking_source_load() =>
            {
                ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::parser_blocking_source_load(runtime, started),
                )
            }
            ParseTimePageVmStreamingProgress::PendingPageTask(runtime) => {
                ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::closed_input_page_work(runtime, started),
                )
            }
            ParseTimePageVmStreamingProgress::NeedMoreInput(_) => {
                return Err(anyhow!(
                    "closed streaming html input should not stall waiting for more input"
                ));
            }
            ParseTimePageVmStreamingProgress::TriggeredNavigation { page_vm, stage } => {
                ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage }
            }
            ParseTimePageVmStreamingProgress::ContinuePhaseTwo {
                page_vm,
                page_tasks,
                stage,
                started,
            } => ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                page_vm,
                page_tasks,
                stage,
                started,
            },
        };
        Ok(PendingPhaseOneResumeOutcome::progress(outcome))
    }
}

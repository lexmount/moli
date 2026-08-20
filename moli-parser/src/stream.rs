use html5ever::tendril::StrTendril;
use moli_dom::{
    NodeId,
    native::{DomHost, NativeDom, NativeNodeId},
};
use moli_page_types::{ScriptKind, ScriptMode, ScriptRun, ScriptSkipReason, ScriptSourceKind};
use moli_script::ScriptPreparationDisposition;
use moli_stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, StylesheetBlockingReadView,
    collect_document_owned_blocking_stylesheets_before_in_view,
};
use std::collections::HashMap;

use crate::script_planning::{
    ParserPlanningReadView, PrepareScriptOutcome, PreparedImportMap, PreparedScript,
    ScriptClassification, ScriptFilterSkipReason, build_prepared_import_map, build_prepared_script,
    classify_parser_script,
};

use super::{
    html::{
        ParserBlockingStylesheetPause, ParserFinishDiscoverySignals, ParserInputQueue,
        ParserInputSession, ParserPumpOutcome, ParserPumpStep, ParserScriptElementStateTransition,
        ParserScriptHandoff, ParserScriptNoExecutionOutcome, ParserScriptPreparationFailure,
        ParserYield,
    },
    live_target::{ParserRuntimeDomSinks, ParserStreamHtmlTreeSinkTarget},
    session::{
        HtmlParserSession, HtmlParserSessionResult, new_fragment_html_tree_sink_session,
        new_html_tree_sink_session,
    },
};

pub(super) struct HtmlTreeSinkStream {
    parser: HtmlParserSession,
    script_input: ParserInputQueue,
    parser_script_positions: HashMap<NativeNodeId, usize>,
    next_parser_script_position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawParserStep {
    Script(NativeNodeId),
    CustomElementConstruction,
    BlockingStylesheet(NativeNodeId),
    InputDrained,
}

#[derive(Debug, Clone)]
enum ParserScriptPreparation {
    BlockingClassic(PreparedScript),
    AsyncPostParse(PreparedScript),
    NonAsyncPostParse(PreparedScript),
    ImportMap(PreparedImportMap),
    NoExecution(ParserScriptNoExecutionOutcome),
    PreparationFailure(ParserScriptPreparationFailure),
}

#[derive(Debug, Clone, Copy)]
enum ParserScriptPreparationLane {
    BlockingClassic,
    AsyncPostParse,
    NonAsyncPostParse,
}

impl ParserScriptPreparation {
    fn async_prefetch_script(&self) -> Option<PreparedScript> {
        match self {
            Self::AsyncPostParse(script) => Some(script.clone()),
            Self::BlockingClassic(_)
            | Self::NonAsyncPostParse(_)
            | Self::ImportMap(_)
            | Self::NoExecution(_)
            | Self::PreparationFailure(_) => None,
        }
    }

    fn is_blocking_classic(&self) -> bool {
        matches!(self, Self::BlockingClassic(_))
    }

    fn into_handoff(
        self,
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        blocking_signatures_before: std::collections::HashSet<DocumentBlockingStylesheetSignature>,
    ) -> ParserScriptHandoff {
        match self {
            Self::BlockingClassic(script) => ParserScriptHandoff::BlockingClassic {
                node_id,
                start_line,
                start_column,
                blocking_signatures_before,
                script,
            },
            Self::AsyncPostParse(script) => ParserScriptHandoff::AsyncPostParse {
                node_id,
                start_line,
                start_column,
                script,
            },
            Self::NonAsyncPostParse(script) => ParserScriptHandoff::NonAsyncPostParse {
                node_id,
                start_line,
                start_column,
                blocking_signatures_before,
                script,
            },
            Self::ImportMap(import_map) => ParserScriptHandoff::ImportMap {
                node_id,
                start_line,
                start_column,
                import_map,
            },
            Self::NoExecution(outcome) => ParserScriptHandoff::NoExecution {
                node_id,
                start_line,
                start_column,
                outcome,
            },
            Self::PreparationFailure(failure) => ParserScriptHandoff::PreparationFailure {
                node_id,
                start_line,
                start_column,
                failure,
            },
        }
    }
}

fn unprepared_script_mode(
    classification: &crate::script_planning::ScriptClassification,
) -> ScriptMode {
    classification
        .mode()
        .unwrap_or_else(|| match classification.kind() {
            ScriptKind::ImportMap => ScriptMode::ImportMapInOrder,
            ScriptKind::DataBlock => ScriptMode::Normal,
            ScriptKind::Classic | ScriptKind::Module => {
                unreachable!("executable script kinds always have a mode")
            }
        })
}

fn ignored_parser_script(
    position: usize,
    mode: ScriptMode,
    transition: ParserScriptElementStateTransition,
) -> ParserScriptPreparation {
    ParserScriptPreparation::NoExecution(
        ParserScriptNoExecutionOutcome::ignored(position, mode)
            .with_element_state_transition(transition),
    )
}

fn failed_parser_script(
    position: usize,
    mode: ScriptMode,
    message: String,
    transition: ParserScriptElementStateTransition,
) -> ParserScriptPreparation {
    ParserScriptPreparation::PreparationFailure(
        ParserScriptPreparationFailure::new(position, mode, message)
            .with_element_state_transition(transition),
    )
}

fn parser_script_report_url(
    document: &impl ParserPlanningReadView,
    classification: &ScriptClassification,
) -> Option<url::Url> {
    let document_url = document.final_url_clone()?;
    let Some(src) = classification.script.script_src.as_deref() else {
        return Some(document_url);
    };
    Some(
        document
            .document_base_url_clone()
            .unwrap_or_else(|| document_url.clone())
            .join(src)
            .unwrap_or(document_url),
    )
}

fn skipped_parser_script(
    document: &impl ParserPlanningReadView,
    node_id: NativeNodeId,
    position: usize,
    mode: ScriptMode,
    classification: &ScriptClassification,
    reason: ScriptSkipReason,
    transition: ParserScriptElementStateTransition,
) -> ParserScriptPreparation {
    let Some(url) = parser_script_report_url(document, classification) else {
        return ignored_parser_script(position, mode, transition);
    };
    ParserScriptPreparation::NoExecution(
        ParserScriptNoExecutionOutcome::skipped(
            position,
            mode,
            ScriptRun::skipped(
                NodeId::new(node_id.index()),
                classification.kind(),
                mode,
                classification.source_kind,
                url,
                reason,
            ),
        )
        .with_element_state_transition(transition),
    )
}

fn consume_parser_inserted_transition(
    classification: &ScriptClassification,
) -> ParserScriptElementStateTransition {
    if classification.script.parser_inserted {
        ParserScriptElementStateTransition::ConsumeParserInserted {
            force_async: !classification.script.async_attribute_present,
        }
    } else {
        ParserScriptElementStateTransition::None
    }
}

fn parser_filter_skip_reason(
    classification: &ScriptClassification,
    reason: ScriptFilterSkipReason,
) -> ScriptSkipReason {
    match reason {
        ScriptFilterSkipReason::AlreadyStarted => ScriptSkipReason::AlreadyStarted,
        ScriptFilterSkipReason::NoModule => ScriptSkipReason::NoModule,
        ScriptFilterSkipReason::DataBlock => ScriptSkipReason::UnsupportedType(
            classification
                .script
                .script_type
                .clone()
                .unwrap_or_default(),
        ),
        ScriptFilterSkipReason::LegacyEventForMismatch => ScriptSkipReason::UnsupportedType(
            "legacy for/event script did not match window.onload".to_owned(),
        ),
    }
}

fn prepare_parser_script(
    document: &impl ParserPlanningReadView,
    node_id: NativeNodeId,
    parser_position: Option<usize>,
) -> ParserScriptPreparation {
    let connected = document.is_connected(node_id);
    let position = parser_position.or_else(|| document.document_order_position(node_id));
    let fallback_position = position.unwrap_or_else(|| document.script_handles().len());
    let Some(c) = classify_parser_script(document, node_id) else {
        return failed_parser_script(
            fallback_position,
            ScriptMode::Normal,
            "script node disappeared during parser preparation".to_owned(),
            ParserScriptElementStateTransition::None,
        );
    };
    let classification_mode = unprepared_script_mode(&c);
    if !connected {
        return skipped_parser_script(
            document,
            node_id,
            fallback_position,
            classification_mode,
            &c,
            ScriptSkipReason::NotInMainDocument,
            consume_parser_inserted_transition(&c),
        );
    }
    if let Some(reason) = c.skip_reason() {
        let transition = match reason {
            ScriptFilterSkipReason::AlreadyStarted => ParserScriptElementStateTransition::None,
            ScriptFilterSkipReason::DataBlock => consume_parser_inserted_transition(&c),
            ScriptFilterSkipReason::NoModule | ScriptFilterSkipReason::LegacyEventForMismatch => {
                ParserScriptElementStateTransition::MarkAlreadyStarted
            }
        };
        return skipped_parser_script(
            document,
            node_id,
            fallback_position,
            classification_mode,
            &c,
            parser_filter_skip_reason(&c, reason),
            transition,
        );
    }

    if c.disposition == ScriptPreparationDisposition::ImportMap {
        let Some(position) = position else {
            return failed_parser_script(
                fallback_position,
                classification_mode,
                "script node is not in document order during parser preparation".to_owned(),
                ParserScriptElementStateTransition::MarkAlreadyStarted,
            );
        };
        let Some(final_url) = document.final_url_clone() else {
            return failed_parser_script(
                position,
                classification_mode,
                "document URL missing during parser script preparation".to_owned(),
                ParserScriptElementStateTransition::MarkAlreadyStarted,
            );
        };
        let document_base_url = document
            .document_base_url_clone()
            .unwrap_or_else(|| final_url.clone());
        return build_prepared_import_map(&c, final_url, document_base_url, node_id, position)
            .map(ParserScriptPreparation::ImportMap)
            .unwrap_or_else(|| {
                failed_parser_script(
                    position,
                    classification_mode,
                    "failed to prepare parser import map".to_owned(),
                    ParserScriptElementStateTransition::MarkAlreadyStarted,
                )
            });
    }
    let Some((kind, mode)) = c.executable() else {
        return ignored_parser_script(
            fallback_position,
            classification_mode,
            consume_parser_inserted_transition(&c),
        );
    };
    let lane = match (kind, mode, c.source_kind) {
        (ScriptKind::Classic, ScriptMode::Normal, _) => {
            ParserScriptPreparationLane::BlockingClassic
        }
        (ScriptKind::Classic, ScriptMode::Async, ScriptSourceKind::External)
        | (ScriptKind::Module, ScriptMode::Async, _) => ParserScriptPreparationLane::AsyncPostParse,
        (
            ScriptKind::Classic,
            ScriptMode::Defer | ScriptMode::ModuleDefer,
            ScriptSourceKind::External,
        )
        | (ScriptKind::Module, _, _) => ParserScriptPreparationLane::NonAsyncPostParse,
        _ => {
            return ignored_parser_script(
                fallback_position,
                mode,
                ParserScriptElementStateTransition::MarkAlreadyStarted,
            );
        }
    };

    let Some(position) = position else {
        return failed_parser_script(
            fallback_position,
            mode,
            "script node is not in document order during parser preparation".to_owned(),
            ParserScriptElementStateTransition::MarkAlreadyStarted,
        );
    };
    let Some(final_url) = document.final_url_clone() else {
        return failed_parser_script(
            position,
            mode,
            "document URL missing during parser script preparation".to_owned(),
            ParserScriptElementStateTransition::MarkAlreadyStarted,
        );
    };
    let document_base_url = document
        .document_base_url_clone()
        .unwrap_or_else(|| final_url.clone());
    match build_prepared_script(&c, final_url, document_base_url, node_id, position) {
        PrepareScriptOutcome::Prepared(script) => match lane {
            ParserScriptPreparationLane::BlockingClassic => {
                ParserScriptPreparation::BlockingClassic(*script)
            }
            ParserScriptPreparationLane::AsyncPostParse => {
                ParserScriptPreparation::AsyncPostParse(*script)
            }
            ParserScriptPreparationLane::NonAsyncPostParse => {
                ParserScriptPreparation::NonAsyncPostParse(*script)
            }
        },
        PrepareScriptOutcome::UrlResolutionFailed(error)
        | PrepareScriptOutcome::EmptyExternalSource(error) => failed_parser_script(
            position,
            mode,
            error,
            ParserScriptElementStateTransition::MarkAlreadyStarted,
        ),
        PrepareScriptOutcome::EmptyInlineSource => skipped_parser_script(
            document,
            node_id,
            position,
            mode,
            &c,
            ScriptSkipReason::EmptyInlineScript,
            consume_parser_inserted_transition(&c),
        ),
        PrepareScriptOutcome::NonExecutableKind(_) => {
            ignored_parser_script(position, mode, consume_parser_inserted_transition(&c))
        }
    }
}

pub(crate) fn prepare_parser_script_handoff_for_static_document(
    document: &(impl ParserPlanningReadView + StylesheetBlockingReadView),
    node_id: NativeNodeId,
    start_line: u64,
    start_column: u64,
) -> ParserScriptHandoff {
    let blocking_signatures_before = collect_document_owned_blocking_stylesheets_before_in_view(
        document,
        NodeId::new(node_id.index()),
    )
    .into_iter()
    .map(|blocker| blocker.signature().clone())
    .collect();
    prepare_parser_script(document, node_id, None).into_handoff(
        node_id,
        start_line,
        start_column,
        blocking_signatures_before,
    )
}

impl HtmlTreeSinkStream {
    pub(super) fn from_target(target: ParserStreamHtmlTreeSinkTarget) -> Self {
        let session = new_html_tree_sink_session(target);
        Self {
            parser: session.parser,
            script_input: session.script_input,
            parser_script_positions: HashMap::new(),
            next_parser_script_position: 0,
        }
    }

    pub(super) fn from_fragment_target(
        target: ParserStreamHtmlTreeSinkTarget,
        context_handle: NativeNodeId,
        context_namespace: &str,
        context_local_name: &str,
    ) -> Self {
        let session = new_fragment_html_tree_sink_session(
            target,
            context_handle,
            context_namespace,
            context_local_name,
        );
        Self {
            parser: session.parser,
            script_input: session.script_input,
            parser_script_positions: HashMap::new(),
            next_parser_script_position: 0,
        }
    }

    fn parser_script_position(&mut self, node_id: NativeNodeId) -> usize {
        *self
            .parser_script_positions
            .entry(node_id)
            .or_insert_with(|| {
                let position = self.next_parser_script_position;
                self.next_parser_script_position =
                    self.next_parser_script_position.saturating_add(1);
                position
            })
    }

    pub fn script_input_session(&self) -> ParserInputSession {
        self.script_input.session()
    }

    pub fn take_next_script_input(&self) -> Option<String> {
        self.script_input.take_next_script_input()
    }

    pub fn next_script_input_len(&self) -> Option<usize> {
        self.script_input.next_script_input_len()
    }

    pub fn snapshot_script_input(&self) -> String {
        self.script_input.snapshot_script_input()
    }

    pub fn has_script_input(&self) -> bool {
        self.script_input.has_script_input()
    }

    pub fn take_next_insertion_preload_input(&self) -> Option<String> {
        self.script_input.take_next_insertion_preload_input()
    }

    pub fn take_processed_insertion_meta_csp_count(&self) -> usize {
        self.script_input.take_processed_insertion_meta_csp_count()
    }

    pub fn feed(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }

        self.parser.process(StrTendril::from(chunk));
    }

    pub fn pump_parser_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        self.pump_parser_step_with_source(chunk, false)
    }

    pub fn pump_parser_inserted_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        if chunk.is_empty() {
            return ParserPumpOutcome {
                result: ParserPumpStep::InputDrained,
                discovered_async_prefetch_scripts: Vec::new(),
                discovered_modulepreload_link_candidates: Vec::new(),
                discovered_blocking_stylesheet_inputs: Vec::new(),
            };
        }
        self.pump_parser_step_with_source(chunk, true)
    }

    pub fn append_to_current_inserted_input(&mut self, chunk: &str) -> bool {
        self.parser
            .append_to_current_inserted_input(StrTendril::from(chunk))
    }

    fn pump_parser_step_with_source(
        &mut self,
        chunk: &str,
        inserted_source: bool,
    ) -> ParserPumpOutcome {
        if !chunk.is_empty() {
            if inserted_source {
                self.parser.begin_inserted_input(StrTendril::from(chunk));
            } else {
                self.parser.push_back(StrTendril::from(chunk));
            }
        }

        let tokenizer_result = self.parser.feed();
        let paused_for_custom_element =
            matches!(tokenizer_result, HtmlParserSessionResult::Script(_))
                && self.has_pending_custom_element_construction_handoff();
        let paused_for_stylesheet = matches!(tokenizer_result, HtmlParserSessionResult::Script(_))
            && !paused_for_custom_element
            && self.peek_pending_blocking_stylesheet_pause().is_some();
        let result = match tokenizer_result {
            HtmlParserSessionResult::Script(handle) if paused_for_custom_element => {
                debug_assert_eq!(
                    self.peek_pending_custom_element_construction_handoff_placeholder(),
                    Some(handle.node_id())
                );
                RawParserStep::CustomElementConstruction
            }
            HtmlParserSessionResult::Script(handle) if paused_for_stylesheet => {
                debug_assert_eq!(
                    self.peek_pending_blocking_stylesheet_pause(),
                    Some(handle.node_id())
                );
                RawParserStep::BlockingStylesheet(handle.node_id())
            }
            HtmlParserSessionResult::Script(handle) => RawParserStep::Script(handle.node_id()),
            HtmlParserSessionResult::InputDrained => RawParserStep::InputDrained,
        };
        let discovered_async_prefetch_candidate_node_ids =
            self.drain_discovered_async_prefetch_candidates();
        let discovered_modulepreload_link_candidate_node_ids =
            self.drain_discovered_modulepreload_link_candidates();
        let discovered_blocking_stylesheet_inputs =
            self.drain_discovered_blocking_stylesheet_inputs();
        let captured_blocking_stylesheet_signatures =
            self.captured_blocking_stylesheet_signatures();
        let discovered_async_prefetch_script_positions =
            discovered_async_prefetch_candidate_node_ids
                .iter()
                .copied()
                .map(|node_id| (node_id, self.parser_script_position(node_id)))
                .collect::<HashMap<_, _>>();
        let handoff_parser_position = match result {
            RawParserStep::Script(node_id) => Some(self.parser_script_position(node_id)),
            RawParserStep::CustomElementConstruction
            | RawParserStep::BlockingStylesheet(_)
            | RawParserStep::InputDrained => None,
        };

        let (
            result,
            discovered_async_prefetch_scripts,
            discovered_modulepreload_link_candidates,
            discovered_blocking_stylesheet_inputs,
        ) = {
            let target = self.parser.sink().borrow_target();
            let handoff_preparation = match result {
                RawParserStep::Script(node_id) => {
                    let (start_line, start_column) =
                        target.script_start_position(node_id).unwrap_or((0, 0));
                    Some((
                        node_id,
                        start_line,
                        start_column,
                        captured_blocking_stylesheet_signatures.clone(),
                        prepare_parser_script(&*target, node_id, handoff_parser_position),
                    ))
                }
                RawParserStep::CustomElementConstruction
                | RawParserStep::BlockingStylesheet(_)
                | RawParserStep::InputDrained => None,
            };
            let discovered_async_prefetch_scripts = discovered_async_prefetch_candidate_node_ids
                .iter()
                .filter_map(|node_id| {
                    if let Some((handoff_node_id, _, _, _, preparation)) =
                        handoff_preparation.as_ref()
                        && handoff_node_id == node_id
                    {
                        preparation.async_prefetch_script()
                    } else {
                        prepare_parser_script(
                            &*target,
                            *node_id,
                            discovered_async_prefetch_script_positions
                                .get(node_id)
                                .copied(),
                        )
                        .async_prefetch_script()
                    }
                })
                .collect::<Vec<_>>();
            let blocking_classic_handoff_node_id =
                handoff_preparation
                    .as_ref()
                    .and_then(|(node_id, _, _, _, preparation)| {
                        preparation.is_blocking_classic().then_some(*node_id)
                    });
            drop(target);

            let result = if let (
                RawParserStep::Script(_),
                Some((node_id, start_line, start_column, blocking_signatures_before, preparation)),
            ) = (result, handoff_preparation)
            {
                ParserPumpStep::Yield(ParserYield::Script(Box::new(preparation.into_handoff(
                    node_id,
                    start_line,
                    start_column,
                    blocking_signatures_before,
                ))))
            } else if let RawParserStep::BlockingStylesheet(node_id) = result {
                let pending_node_id = self.pop_pending_blocking_stylesheet_pause();
                assert_eq!(
                    pending_node_id,
                    Some(node_id),
                    "tokenizer stylesheet yield must consume its matching pending parser pause"
                );
                ParserPumpStep::Yield(ParserYield::BlockingStylesheet(
                    ParserBlockingStylesheetPause { node_id },
                ))
            } else if let Some(handoff) = self.pop_pending_custom_element_construction_handoff() {
                ParserPumpStep::Yield(ParserYield::CustomElementConstruction(Box::new(handoff)))
            } else {
                ParserPumpStep::InputDrained
            };
            debug_assert_eq!(
                blocking_classic_handoff_node_id.is_some(),
                matches!(
                    result,
                    ParserPumpStep::Yield(ParserYield::Script(ref handoff))
                        if matches!(handoff.as_ref(), ParserScriptHandoff::BlockingClassic { .. })
                )
            );
            (
                result,
                discovered_async_prefetch_scripts,
                discovered_modulepreload_link_candidate_node_ids,
                discovered_blocking_stylesheet_inputs,
            )
        };

        ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts,
            discovered_modulepreload_link_candidates,
            discovered_blocking_stylesheet_inputs,
        }
    }

    pub fn finish_live_runtime_dom_sink_parser(self) -> ParserFinishDiscoverySignals {
        let HtmlTreeSinkStream {
            parser,
            script_input: _,
            parser_script_positions: _,
            next_parser_script_position: _,
        } = self;
        parser.finish_live_runtime_dom_sink_parser()
    }

    pub fn with_stylesheet_blocking_read_view<R>(
        &self,
        f: impl FnOnce(&dyn StylesheetBlockingReadView) -> R,
    ) -> R {
        let target = self.parser.sink().borrow_target();
        f(&*target)
    }

    pub fn snapshot_parser_stream_document(&self) -> NativeDom {
        self.parser.sink().snapshot_parser_stream_document()
    }

    pub fn snapshot_parser_stream_dom_host(&self) -> DomHost {
        self.parser.sink().snapshot_parser_stream_dom_host()
    }

    pub fn take_parser_stream_null_custom_element_registry_elements(
        &mut self,
    ) -> Vec<NativeNodeId> {
        self.parser
            .sink()
            .take_parser_stream_null_custom_element_registry_elements()
    }

    pub fn take_parser_stream_dom_host(&mut self) -> DomHost {
        self.parser.sink().take_parser_stream_dom_host()
    }

    pub fn restore_parser_stream_dom_host(&mut self, dom_host: DomHost) {
        self.parser.sink().restore_parser_stream_dom_host(dom_host);
    }

    pub(super) fn enter_runtime_dom_sinks_parse_step(&mut self, sinks: ParserRuntimeDomSinks) {
        self.parser.sink().enter_runtime_dom_sinks_parse_step(sinks);
    }

    pub fn clear_runtime_dom_sinks_after_parse_step(&mut self) {
        self.parser
            .sink()
            .clear_runtime_dom_sinks_after_parse_step()
    }

    pub fn replace_parser_stream_document(&mut self, document: NativeDom) {
        self.parser.sink().replace_parser_stream_document(document);
    }

    pub fn drain_ready_parser_scripts(&mut self) -> Vec<NativeNodeId> {
        self.parser.sink().drain_ready_parser_scripts()
    }

    pub fn drain_discovered_async_prefetch_candidates(&mut self) -> Vec<NativeNodeId> {
        self.parser
            .sink()
            .drain_discovered_async_prefetch_candidates()
    }

    pub fn drain_discovered_modulepreload_link_candidates(&mut self) -> Vec<NativeNodeId> {
        self.parser
            .sink()
            .drain_discovered_modulepreload_link_candidates()
    }

    pub fn drain_discovered_parser_meta_csp_candidates(&mut self) -> Vec<NativeNodeId> {
        self.parser
            .sink()
            .drain_discovered_parser_meta_csp_candidates()
    }

    pub fn note_defined_autonomous_custom_element(&mut self, local_name: &str) {
        self.parser
            .sink()
            .note_defined_autonomous_custom_element(local_name);
    }

    pub fn drain_pending_custom_element_construction_handoffs(
        &mut self,
    ) -> Vec<crate::html::ParserCustomElementConstructionHandoff> {
        self.parser
            .sink()
            .drain_pending_custom_element_construction_handoffs()
    }

    fn has_pending_custom_element_construction_handoff(&self) -> bool {
        self.parser
            .sink()
            .has_pending_custom_element_construction_handoff()
    }

    fn peek_pending_custom_element_construction_handoff_placeholder(&self) -> Option<NativeNodeId> {
        self.parser
            .sink()
            .pending_custom_element_construction_handoff_placeholder()
    }

    fn pop_pending_custom_element_construction_handoff(
        &mut self,
    ) -> Option<crate::html::ParserCustomElementConstructionHandoff> {
        self.parser
            .sink()
            .pop_pending_custom_element_construction_handoff()
    }

    fn peek_pending_blocking_stylesheet_pause(&self) -> Option<NativeNodeId> {
        self.parser.sink().pending_blocking_stylesheet_pause()
    }

    fn pop_pending_blocking_stylesheet_pause(&mut self) -> Option<NativeNodeId> {
        self.parser.sink().pop_pending_blocking_stylesheet_pause()
    }

    fn drain_discovered_blocking_stylesheet_inputs(
        &mut self,
    ) -> Vec<moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput> {
        self.parser
            .sink()
            .drain_discovered_blocking_stylesheet_inputs()
    }

    fn captured_blocking_stylesheet_signatures(
        &self,
    ) -> std::collections::HashSet<DocumentBlockingStylesheetSignature> {
        self.parser.sink().captured_blocking_stylesheet_signatures()
    }

    pub fn mark_script_already_started(&mut self, node_id: NativeNodeId) {
        self.parser.sink().mark_script_already_started(node_id);
    }

    pub fn has_buffered_input(&self) -> bool {
        self.parser.has_buffered_input()
    }

    pub fn buffered_input_len(&self) -> usize {
        self.parser.buffered_input_len()
    }

    pub fn snapshot_buffered_input(&self) -> String {
        self.parser.snapshot_buffered_input()
    }

    pub fn finish(self) -> NativeDom {
        self.finish_dom_host().into_dom()
    }

    pub fn finish_dom_host(self) -> DomHost {
        self.finish_dom_host_with_discovery_signals().0
    }

    pub(super) fn finish_dom_host_with_discovery_signals(
        self,
    ) -> (DomHost, ParserFinishDiscoverySignals) {
        let HtmlTreeSinkStream {
            parser,
            script_input: _,
            parser_script_positions: _,
            next_parser_script_position: _,
        } = self;
        let mut target = parser.finish();
        let signals = ParserFinishDiscoverySignals {
            parser_created_null_registry_elements: target
                .take_parser_stream_null_custom_element_registry_elements(),
            discovered_modulepreload_link_candidates: target
                .drain_discovered_modulepreload_link_candidates(),
            discovered_parser_meta_csp_candidates: target
                .drain_discovered_parser_meta_csp_candidates(),
            discovered_blocking_stylesheet_inputs: target
                .drain_discovered_blocking_stylesheet_inputs(),
        };
        (target.finish_dom_host(), signals)
    }
}

#[cfg(test)]
mod tests {
    use super::{ParserScriptPreparation, prepare_parser_script};
    use crate::{
        DocumentStream, HtmlParser, ParserPlanningReadView, ParserPumpStep, ParserScriptHandoff,
        ParserYield, PreparedScript, ScriptSource,
    };
    use moli_dom::native::{DomHost, DomMutationEffects, NativeDom, NativeNodeId, Node};
    use moli_page_types::{ScriptKind, ScriptMode, ScriptSkipReason, ScriptSourceKind};
    use moli_stylesheet_blocking::DocumentBlockingStylesheetSignature;
    use url::Url;

    struct TestMutationEffectCollector<'a> {
        host: *mut DomHost,
        effects: &'a mut DomMutationEffects,
    }

    impl crate::ParserDomReadConsumer for TestMutationEffectCollector<'_> {
        fn node_exists(&mut self, node_id: NativeNodeId) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.node(node_id).is_some()
        }

        fn is_connected(&mut self, node_id: NativeNodeId) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.is_connected(node_id)
        }

        fn is_text_node(&mut self, node_id: NativeNodeId) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }
                .node(node_id)
                .and_then(moli_dom::native::Node::as_text)
                .is_some()
        }

        fn owner_document(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.owner_document_handle(node_id)
        }

        fn parent_node(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }
                .node(node_id)
                .and_then(moli_dom::native::Node::parent_node)
        }

        fn previous_sibling(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }
                .node(node_id)
                .and_then(moli_dom::native::Node::prev_sibling)
        }

        fn last_child(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }
                .node(node_id)
                .and_then(moli_dom::native::Node::last_child)
        }

        fn child_handles(&mut self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.child_handles(node_id).collect()
        }

        fn document_body_handle_for_document(
            &mut self,
            document_handle: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.document_body_handle_for_document(document_handle)
        }

        fn template_contents_handle(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.parser_template_contents_handle(node_id)
        }

        fn is_html_element_named(&mut self, node_id: NativeNodeId, local_name: &str) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }
                .dom()
                .is_html_element_named(node_id, local_name)
        }

        fn is_external_async_classic_candidate(&mut self, node_id: NativeNodeId) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let Some(element) = (unsafe { &*self.host })
                .node(node_id)
                .and_then(moli_dom::native::Node::as_element)
            else {
                return false;
            };
            if !element.is_script_element() {
                return false;
            }
            if element.script_source_attribute().is_none() || element.attribute("async").is_none() {
                return false;
            }
            if element.is_html_script() && element.attribute("nomodule").is_some() {
                return false;
            }
            let Some(script_type) = element.attribute("type") else {
                return true;
            };
            if script_type.is_empty() {
                return true;
            }
            moli_script::classify_script_kind(Some(script_type)) == ScriptKind::Classic
        }

        fn parser_script_read(&mut self, node_id: NativeNodeId) -> Option<crate::ParserScriptRead> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            <DomHost as ParserPlanningReadView>::parser_script_read(unsafe { &*self.host }, node_id)
        }

        fn stylesheet_element(
            &mut self,
            node_id: NativeNodeId,
        ) -> Option<moli_stylesheet_blocking::StylesheetElementRead> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            (unsafe { &*self.host })
                .node(node_id)
                .and_then(moli_stylesheet_blocking::StylesheetElementRead::from_node)
        }

        fn text_content(&mut self, node_id: NativeNodeId) -> Option<String> {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &*self.host }.text_content(node_id)
        }
    }

    impl crate::ParserDomMutationConsumer for TestMutationEffectCollector<'_> {
        fn apply_parser_dom_mutation(&mut self, mutation: crate::ParserDomMutation) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let effects = mutation.apply_to_dom_host(unsafe { &mut *self.host });
            self.effects.merge(effects);
        }

        fn create_parser_element_without_attributes(
            &mut self,
            local_name: String,
            namespace: String,
            prefix: Option<String>,
        ) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }
                .create_parser_element_without_attributes(local_name, namespace, prefix)
        }

        fn create_parser_element_for_document_without_attributes(
            &mut self,
            document_handle: NativeNodeId,
            local_name: String,
            namespace: String,
            prefix: Option<String>,
        ) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.create_parser_element_without_attributes_for_document(
                document_handle,
                local_name,
                namespace,
                prefix,
            )
        }

        fn add_attrs_if_missing_for_parser(
            &mut self,
            node_id: NativeNodeId,
            attrs: Vec<moli_dom::native::Attribute>,
        ) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.add_attrs_if_missing_for_parser(node_id, attrs);
        }

        fn create_text_node(&mut self, text: String) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.create_text_node(&text)
        }

        fn create_comment(&mut self, text: String) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.create_comment(&text)
        }

        fn create_processing_instruction(&mut self, target: String, data: String) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.create_processing_instruction(&target, &data)
        }

        fn create_cdata_section(&mut self, data: String) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.create_cdata_section(&data)
        }

        fn create_document_type(
            &mut self,
            name: String,
            public_id: String,
            system_id: String,
        ) -> NativeNodeId {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.create_document_type(&name, &public_id, &system_id)
        }

        fn prepend_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let host = unsafe { &mut *self.host };
            if let Some(text_node) = host
                .node_mut(node_id)
                .and_then(|node| node.data_mut().as_text_mut())
            {
                let mut merged = text;
                merged.push_str(text_node.data());
                text_node.set_data(merged);
            }
        }

        fn append_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let host = unsafe { &mut *self.host };
            if let Some(text_node) = host
                .node_mut(node_id)
                .and_then(|node| node.data_mut().as_text_mut())
            {
                let mut merged = text_node.data().to_owned();
                merged.push_str(&text);
                text_node.set_data(merged);
            }
        }

        fn push_parse_error(&mut self, error: String) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.push_parse_error(error);
        }

        fn set_html_quirks_mode_for_parser(
            &mut self,
            quirks_mode: html5ever::tree_builder::QuirksMode,
        ) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.set_html_quirks_mode_for_parser(quirks_mode);
        }

        fn mark_script_already_started_for_parser(&mut self, node_id: NativeNodeId) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let _ = unsafe { &mut *self.host }.set_script_already_started(node_id, true);
        }

        fn finish_parsing_script_children(&mut self, node_id: NativeNodeId) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let _ = unsafe { &mut *self.host }.finish_parsing_script_children(node_id);
        }

        fn finish_parsing_link_children(&mut self, node_id: NativeNodeId) {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            let _ = unsafe { &mut *self.host }.finish_parsing_link_children(node_id);
        }

        fn attach_declarative_shadow_for_parser(
            &mut self,
            host_id: NativeNodeId,
            template_id: NativeNodeId,
            attrs: Vec<moli_dom::native::Attribute>,
        ) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.attach_declarative_shadow_for_parser(
                host_id,
                template_id,
                &attrs,
            )
        }

        fn associate_parser_form_owner(
            &mut self,
            target: NativeNodeId,
            form: NativeNodeId,
        ) -> bool {
            // SAFETY: the test keeps the DomHost alive for this parser pump step.
            unsafe { &mut *self.host }.associate_parser_form_owner(target, form)
        }
    }

    impl crate::ParserMutationEffectConsumer for TestMutationEffectCollector<'_> {
        fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
            self.effects.merge(effects);
        }
    }

    fn first_script_handle(html: &str) -> (NativeDom, NativeNodeId) {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.test/").expect("test url should parse"),
            html.to_owned(),
        );
        let handle = document
            .script_handles()
            .into_iter()
            .next()
            .expect("fixture should contain a script");
        (document, handle)
    }

    fn prepare_parser_blocking_classic_script(
        document: &impl ParserPlanningReadView,
        node_id: NativeNodeId,
    ) -> Option<PreparedScript> {
        match prepare_parser_script(document, node_id, None) {
            ParserScriptPreparation::BlockingClassic(script) => Some(script),
            ParserScriptPreparation::AsyncPostParse(_)
            | ParserScriptPreparation::NonAsyncPostParse(_)
            | ParserScriptPreparation::ImportMap(_)
            | ParserScriptPreparation::NoExecution(_)
            | ParserScriptPreparation::PreparationFailure(_) => None,
        }
    }

    fn prepare_parser_async_post_parse_script(
        document: &impl ParserPlanningReadView,
        node_id: NativeNodeId,
    ) -> Option<PreparedScript> {
        match prepare_parser_script(document, node_id, None) {
            ParserScriptPreparation::AsyncPostParse(script) => Some(script),
            ParserScriptPreparation::BlockingClassic(_)
            | ParserScriptPreparation::NonAsyncPostParse(_)
            | ParserScriptPreparation::ImportMap(_)
            | ParserScriptPreparation::NoExecution(_)
            | ParserScriptPreparation::PreparationFailure(_) => None,
        }
    }

    fn prepare_parser_non_async_post_parse_script(
        document: &impl ParserPlanningReadView,
        node_id: NativeNodeId,
    ) -> Option<PreparedScript> {
        match prepare_parser_script(document, node_id, None) {
            ParserScriptPreparation::NonAsyncPostParse(script) => Some(script),
            ParserScriptPreparation::BlockingClassic(_)
            | ParserScriptPreparation::AsyncPostParse(_)
            | ParserScriptPreparation::ImportMap(_)
            | ParserScriptPreparation::NoExecution(_)
            | ParserScriptPreparation::PreparationFailure(_) => None,
        }
    }

    fn prepare_parser_post_parse_script(
        document: &impl ParserPlanningReadView,
        node_id: NativeNodeId,
    ) -> Option<PreparedScript> {
        match prepare_parser_script(document, node_id, None) {
            ParserScriptPreparation::AsyncPostParse(script)
            | ParserScriptPreparation::NonAsyncPostParse(script) => Some(script),
            ParserScriptPreparation::BlockingClassic(_)
            | ParserScriptPreparation::ImportMap(_)
            | ParserScriptPreparation::NoExecution(_)
            | ParserScriptPreparation::PreparationFailure(_) => None,
        }
    }

    fn prepare_parser_import_map(
        document: &impl ParserPlanningReadView,
        node_id: NativeNodeId,
    ) -> Option<crate::PreparedImportMap> {
        match prepare_parser_script(document, node_id, None) {
            ParserScriptPreparation::ImportMap(import_map) => Some(import_map),
            ParserScriptPreparation::BlockingClassic(_)
            | ParserScriptPreparation::AsyncPostParse(_)
            | ParserScriptPreparation::NonAsyncPostParse(_)
            | ParserScriptPreparation::NoExecution(_)
            | ParserScriptPreparation::PreparationFailure(_) => None,
        }
    }

    #[test]
    fn parser_stream_feed_continues_past_definitive_encoding_indicator() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        stream.feed("<!doctype html><meta charset='utf-8'><body><p>after meta</p>");

        let document = stream.snapshot_parser_stream_document();
        assert_eq!(
            document
                .elements_by_tag_name(document.document_node_id(), "p", false)
                .len(),
            1,
            "the advisory encoding indicator must not leave decoded input buffered"
        );
    }

    #[test]
    fn parser_stream_pump_continues_past_definitive_encoding_indicator() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome =
            stream.pump_parser_step("<!doctype html><meta charset='utf-8'><body><p>after meta</p>");

        assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
        let document = stream.snapshot_parser_stream_document();
        assert_eq!(
            document
                .elements_by_tag_name(document.document_node_id(), "p", false)
                .len(),
            1,
            "the parser pump must consume the tail after an encoding indicator"
        );
    }

    #[test]
    fn parser_stream_finish_continues_past_definitive_encoding_indicator() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let outcome = stream.pump_parser_step(concat!(
            "<!doctype html><script>window.ready = true;</script>",
            "<meta charset='utf-8'><p>after meta</p>"
        ));
        assert!(matches!(
            outcome.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));

        let document = stream.finish();
        assert_eq!(
            document
                .elements_by_tag_name(document.document_node_id(), "p", false)
                .len(),
            1,
            "finishing must consume the buffered tail after an encoding indicator"
        );
    }

    #[test]
    fn prepare_parser_blocking_classic_script_accepts_inline_classic() {
        let (document, handle) = first_script_handle("<script>window.ran = true;</script>");
        assert_eq!(
            document
                .node(handle)
                .and_then(Node::as_element)
                .map(|element| element.script_text_internal_slot()),
            Some("window.ran = true;")
        );
        let prepared = prepare_parser_blocking_classic_script(&document, handle)
            .expect("inline classic parser script should be prepared");
        assert_eq!(prepared.kind, ScriptKind::Classic);
        assert_eq!(prepared.mode, ScriptMode::Normal);
        assert_eq!(prepared.source_kind, ScriptSourceKind::Inline);
    }

    #[test]
    fn prepare_parser_svg_script_uses_shared_script_identity_and_href_source() {
        let (document, handle) =
            first_script_handle("<svg><script async href=\"/svg-script.js\"></script></svg>");
        let element = document
            .node(handle)
            .and_then(Node::as_element)
            .expect("SVG script element");
        assert!(element.is_script_element());
        assert_eq!(element.wrapper_prototype_name(), "SVGScriptElement");
        assert_eq!(document.script_src(handle), Some("/svg-script.js"));
        assert!(element.script_parser_inserted_for_prepare());

        let prepared = prepare_parser_async_post_parse_script(&document, handle)
            .expect("external async SVG script should be prepared");
        assert_eq!(prepared.source_kind, ScriptSourceKind::External);
        assert_eq!(prepared.url.as_str(), "https://example.test/svg-script.js");
    }

    #[test]
    fn parser_finished_svg_script_seeds_trusted_source_state() {
        let (document, handle) =
            first_script_handle("<svg><script type=unknown>window.svgReady = true;</script></svg>");
        let element = document
            .node(handle)
            .and_then(Node::as_element)
            .expect("SVG script element");

        assert_eq!(
            element.script_text_internal_slot(),
            "window.svgReady = true;"
        );
        assert!(!element.script_children_changed_by_api());
    }

    #[test]
    fn prepare_parser_blocking_classic_script_rejects_external_defer() {
        let (document, handle) = first_script_handle("<script defer src=\"/defer.js\"></script>");
        assert!(prepare_parser_blocking_classic_script(&document, handle).is_none());
    }

    #[test]
    fn prepare_parser_blocking_classic_script_rejects_external_async() {
        let (document, handle) = first_script_handle("<script async src=\"/async.js\"></script>");
        assert!(prepare_parser_blocking_classic_script(&document, handle).is_none());
    }

    #[test]
    fn prepare_parser_blocking_classic_script_rejects_module_and_importmap() {
        let (module_document, module_handle) =
            first_script_handle("<script type=\"module\">export {};</script>");
        assert!(prepare_parser_blocking_classic_script(&module_document, module_handle).is_none());

        let (importmap_document, importmap_handle) = first_script_handle(
            "<script type=\"importmap\">{\"imports\":{\"x\":\"/x.js\"}}</script>",
        );
        assert!(
            prepare_parser_blocking_classic_script(&importmap_document, importmap_handle).is_none()
        );
    }

    #[test]
    fn prepare_parser_blocking_classic_script_rejects_already_started_script() {
        let (mut document, handle) = first_script_handle("<script>window.ran = true;</script>");
        let Some(node) = document.node_mut(handle) else {
            panic!("script node should exist");
        };
        let Some(element) = node.data_mut().as_element_mut() else {
            panic!("script handle should resolve to an element");
        };
        let _ = element.set_script_already_started(true);

        assert!(prepare_parser_blocking_classic_script(&document, handle).is_none());
    }

    #[test]
    fn prepare_parser_post_parse_script_accepts_external_defer() {
        let (document, handle) = first_script_handle("<script defer src=\"/defer.js\"></script>");
        let prepared = prepare_parser_post_parse_script(&document, handle)
            .expect("external defer parser script should be prepared");
        assert_eq!(prepared.kind, ScriptKind::Classic);
        assert_eq!(prepared.mode, ScriptMode::Defer);
        assert_eq!(prepared.source_kind, ScriptSourceKind::External);
    }

    #[test]
    fn prepare_parser_post_parse_script_accepts_external_async() {
        let (document, handle) = first_script_handle("<script async src=\"/async.js\"></script>");
        let prepared = prepare_parser_post_parse_script(&document, handle)
            .expect("external async parser script should be prepared");
        assert_eq!(prepared.kind, ScriptKind::Classic);
        assert_eq!(prepared.mode, ScriptMode::Async);
        assert_eq!(prepared.source_kind, ScriptSourceKind::External);
    }

    #[test]
    fn parser_preparation_separates_module_execution_from_import_map_registration() {
        let (module_document, module_handle) =
            first_script_handle("<script type=\"module\">window.moduleReady = true;</script>");
        let prepared = prepare_parser_post_parse_script(&module_document, module_handle)
            .expect("inline module parser script should be prepared");
        assert_eq!(prepared.kind, ScriptKind::Module);
        assert_eq!(prepared.mode, ScriptMode::ModuleDefer);
        assert_eq!(prepared.source_kind, ScriptSourceKind::Inline);

        let (importmap_document, importmap_handle) = first_script_handle(
            "<script type=\"importmap\">{\"imports\":{\"x\":\"/x.js\"}}</script>",
        );
        let prepared = prepare_parser_import_map(&importmap_document, importmap_handle)
            .expect("inline importmap should produce a registration payload");
        assert!(matches!(
            prepared.source,
            crate::PreparedImportMapSource::Inline(ref source)
                if source == "{\"imports\":{\"x\":\"/x.js\"}}"
        ));
    }

    #[test]
    fn parser_handoff_preserves_defer_modes_for_owner_acceptance() {
        let mut classic_stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let classic = classic_stream
            .pump_parser_step("<script defer src='/defer.js'></script>")
            .result;
        let ParserPumpStep::Yield(ParserYield::Script(classic)) = classic else {
            panic!("expected classic defer parser handoff");
        };
        let ParserScriptHandoff::NonAsyncPostParse { script, .. } = *classic else {
            panic!("classic defer must use the parser-owned post-parse handoff");
        };
        assert_eq!(script.kind, ScriptKind::Classic);
        assert_eq!(script.mode, ScriptMode::Defer);

        let mut module_stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let module = module_stream
            .pump_parser_step("<script type='module'>export const value = 1;</script>")
            .result;
        let ParserPumpStep::Yield(ParserYield::Script(module)) = module else {
            panic!("expected module-defer parser handoff");
        };
        let ParserScriptHandoff::NonAsyncPostParse { script, .. } = *module else {
            panic!("module defer must use the parser-owned post-parse handoff");
        };
        assert_eq!(script.kind, ScriptKind::Module);
        assert_eq!(script.mode, ScriptMode::ModuleDefer);
    }

    #[test]
    fn parser_stream_assigns_stable_positions_to_shadow_root_scripts() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let first = stream
            .pump_parser_step(
                "<div><template shadowrootmode='open'><script>window.first = true</script><script>window.second = true</script></template></div>",
            )
            .result;
        let ParserPumpStep::Yield(ParserYield::Script(first)) = first else {
            panic!("expected first shadow-root script handoff");
        };
        let ParserScriptHandoff::BlockingClassic { script: first, .. } = *first else {
            panic!("expected parser-blocking classic shadow-root script");
        };
        assert_eq!(first.position, 0);
        assert!(
            matches!(first.source, ScriptSource::Inline(ref source) if source.contains("window.first"))
        );

        let second = stream.pump_parser_step("").result;
        let ParserPumpStep::Yield(ParserYield::Script(second)) = second else {
            panic!("expected second shadow-root script handoff");
        };
        let ParserScriptHandoff::BlockingClassic { script: second, .. } = *second else {
            panic!("expected second parser-blocking classic shadow-root script");
        };
        assert_eq!(second.position, 1);
        assert!(
            matches!(second.source, ScriptSource::Inline(ref source) if source.contains("window.second"))
        );
    }

    #[test]
    fn parser_stream_keeps_ordinary_template_scripts_inert() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let handoff = stream
            .pump_parser_step(
                "<template><script>window.templateScriptRan = true</script></template>",
            )
            .result;
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = handoff else {
            panic!("expected template script handoff");
        };
        let ParserScriptHandoff::NoExecution { outcome, .. } = *handoff else {
            panic!("ordinary template script must remain inert");
        };
        assert_eq!(
            outcome.element_state_transition(),
            crate::ParserScriptElementStateTransition::ConsumeParserInserted { force_async: true }
        );
        let (position, mode, run) = outcome.into_parts();
        assert_eq!(position, 0);
        assert_eq!(mode, ScriptMode::Normal);
        assert!(matches!(
            run,
            Some(run)
                if matches!(
                    run.outcome(),
                    moli_page_types::ScriptRunOutcome::Skipped(
                        ScriptSkipReason::NotInMainDocument
                    )
                )
        ));
    }

    #[test]
    fn parser_handoff_keeps_import_maps_out_of_executable_script_lanes() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let result = stream
            .pump_parser_step(
                "<script type='importmap' async defer>{\"imports\":{\"x\":\"/x.js\"}}</script>",
            )
            .result;
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected import-map parser handoff");
        };
        let ParserScriptHandoff::ImportMap { import_map, .. } = *handoff else {
            panic!("import map must not use an executable parser handoff");
        };
        assert!(matches!(
            import_map.source,
            crate::PreparedImportMapSource::Inline(source)
                if source.contains("\"x\":\"/x.js\"")
        ));

        let mut external_stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let result = external_stream
            .pump_parser_step("<script type='importmap' src='/map.json'></script>")
            .result;
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected external import-map handoff");
        };
        assert!(matches!(
            *handoff,
            ParserScriptHandoff::ImportMap {
                import_map: crate::PreparedImportMap {
                    source: crate::PreparedImportMapSource::ExternalUnsupported,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn prepare_parser_post_parse_script_accepts_async_module_via_async_lane() {
        let (document, handle) = first_script_handle(
            "<script type=\"module\" async>window.moduleReady = true;</script>",
        );
        let prepared = prepare_parser_async_post_parse_script(&document, handle)
            .expect("async module parser script should use async post-parse lane");
        assert_eq!(prepared.kind, ScriptKind::Module);
        assert_eq!(prepared.mode, ScriptMode::Async);
        assert_eq!(prepared.source_kind, ScriptSourceKind::Inline);
    }

    #[test]
    fn prepare_parser_non_async_post_parse_script_rejects_async_modules() {
        let (document, handle) = first_script_handle(
            "<script type=\"module\" async>window.moduleReady = true;</script>",
        );
        assert!(
            prepare_parser_non_async_post_parse_script(&document, handle).is_none(),
            "async module script should not use non-async post-parse path"
        );
    }

    #[test]
    fn prepare_parser_post_parse_script_rejects_inline_and_normal_classic() {
        let (inline_document, inline_handle) =
            first_script_handle("<script>window.ran = true;</script>");
        assert!(prepare_parser_post_parse_script(&inline_document, inline_handle).is_none());

        let (normal_external_document, normal_external_handle) =
            first_script_handle("<script src=\"/normal.js\"></script>");
        assert!(
            prepare_parser_post_parse_script(&normal_external_document, normal_external_handle)
                .is_none()
        );
    }

    #[test]
    fn empty_src_parser_script_handoff_is_unprepared_not_fetchable() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome =
            stream.pump_parser_step("<!doctype html><html><head><script src=\"\"></script>");

        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected parser script handoff");
        };
        let ParserScriptHandoff::PreparationFailure { failure, .. } = *handoff else {
            panic!("empty src must not become a blocking classic fetch");
        };
        assert_eq!(
            failure.element_state_transition(),
            crate::ParserScriptElementStateTransition::MarkAlreadyStarted
        );
        let (position, mode, message) = failure.into_parts();
        assert_eq!(position, 0);
        assert_eq!(mode, ScriptMode::Normal);
        assert_eq!(message, "empty script src is not fetchable");
    }

    #[test]
    fn invalid_src_parser_script_handoff_preserves_preparation_failure() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome = stream
            .pump_parser_step("<!doctype html><html><head><script src=\"http://[::1\"></script>");

        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected parser script handoff");
        };
        let ParserScriptHandoff::PreparationFailure { failure, .. } = *handoff else {
            panic!("invalid src must stay a failed parser preparation");
        };
        assert_eq!(
            failure.element_state_transition(),
            crate::ParserScriptElementStateTransition::MarkAlreadyStarted
        );
        let (position, mode, failure) = failure.into_parts();
        assert_eq!(position, 0);
        assert_eq!(mode, ScriptMode::Normal);
        assert!(
            failure.contains("failed to resolve script src"),
            "invalid src failure should survive the parser boundary"
        );
    }

    #[test]
    fn parser_script_handoff_uses_html5ever_line_with_unknown_column_across_input_chunks() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let first = stream.pump_parser_step("<!doctype html>\r\n  <scr");
        assert!(matches!(first.result, ParserPumpStep::InputDrained));
        let second = stream.pump_parser_step("ipt>window.ran = true;</script>");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = second.result else {
            panic!("expected parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected blocking classic handoff");
        };

        assert_eq!(start_line, 2);
        assert_eq!(start_column, 0);
    }

    #[test]
    fn nonempty_parser_inserted_input_permanently_degrades_source_locations() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outer = stream.pump_parser_step(
            "<!doctype html>\n<script>outer()</script>\n  <script>original()</script>",
        );
        assert!(matches!(
            outer.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));
        let inserted = stream.pump_parser_inserted_step("\n    <script>inserted()</script>\n");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = inserted.result else {
            panic!("expected inserted parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected inserted blocking classic handoff");
        };
        assert_eq!((start_line, start_column), (0, 0));

        assert!(matches!(
            stream.pump_parser_step("").result,
            ParserPumpStep::InputDrained
        ));

        let original = stream.pump_parser_step("");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = original.result else {
            panic!("expected original parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected original blocking classic handoff");
        };
        assert_eq!((start_line, start_column), (0, 0));
    }

    #[test]
    fn blocked_nested_writes_keep_each_input_at_its_insertion_depth() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outer = stream.pump_parser_step("<script>outer()</script>e");
        assert!(matches!(
            outer.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));
        let nested = stream.pump_parser_inserted_step("<script src='nested.js'></script>r");
        assert!(matches!(
            nested.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));

        assert!(stream.append_to_current_inserted_input("k"));
        stream.append_to_end("d".to_owned());
        stream
            .script_input_session()
            .enqueue_script_input_html("wo".to_owned());

        assert_eq!(stream.snapshot_pending_input(), "worked");
    }

    #[test]
    fn parser_token_crossing_inserted_and_original_input_stays_unknown() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outer = stream.pump_parser_step(
            "<script>outer()</script>ipt>cross()</script>\n<script>original()</script>",
        );
        assert!(matches!(
            outer.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));
        assert!(matches!(
            stream.pump_parser_inserted_step("<scr").result,
            ParserPumpStep::InputDrained
        ));

        let cross_source = stream.pump_parser_step("");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = cross_source.result else {
            panic!("expected cross-source parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected cross-source blocking classic handoff");
        };
        assert_eq!((start_line, start_column), (0, 0));

        let original = stream.pump_parser_step("");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = original.result else {
            panic!("expected following original parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected following original blocking classic handoff");
        };
        assert_eq!((start_line, start_column), (0, 0));
    }

    #[test]
    fn parser_inserted_character_reference_chunks_degrade_without_disrupting_parsing() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        let markup =
            "<a href='?a=b&c=d&a0b=c&copy=1&noti=n&not=in&notin=&notin;&not;&;& &'>Link</a>";

        for character in markup.chars() {
            assert!(matches!(
                stream
                    .pump_parser_inserted_step(&character.to_string())
                    .result,
                ParserPumpStep::InputDrained
            ));
        }

        let outcome = stream.pump_parser_step("original text\n<script>original()</script>");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected original parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected original blocking classic handoff");
        };

        assert_eq!((start_line, start_column), (0, 0));
    }

    #[test]
    fn parser_original_source_uses_html5ever_line_without_tracking_utf16_columns() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome = stream.pump_parser_step("\n😀<script>original()</script>");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected blocking classic handoff");
        };

        assert_eq!((start_line, start_column), (2, 0));
    }

    #[test]
    fn empty_parser_inserted_input_preserves_following_line_location() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outer =
            stream.pump_parser_step("<script>outer()</script>\n<script>original()</script>");
        assert!(matches!(
            outer.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));
        assert!(matches!(
            stream.pump_parser_inserted_step("").result,
            ParserPumpStep::InputDrained
        ));

        let original = stream.pump_parser_step("");
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = original.result else {
            panic!("expected original parser script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            start_line,
            start_column,
            ..
        } = *handoff
        else {
            panic!("expected original blocking classic handoff");
        };
        assert_eq!((start_line, start_column), (2, 0));
    }

    #[test]
    fn parser_stream_surfaces_inline_svg_script_handoff() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome = stream.pump_parser_step(
            "<!doctype html><body><svg><script>window.svgReady = true;</script></svg><p>late</p>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected SVG script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            node_id, script, ..
        } = *handoff
        else {
            panic!("expected inline SVG script to use the blocking classic lane");
        };
        let document = stream.snapshot_parser_stream_document();
        let element = document
            .node(node_id)
            .and_then(Node::as_element)
            .expect("SVG script element");

        assert_eq!(element.wrapper_prototype_name(), "SVGScriptElement");
        assert_eq!(script.source_kind, ScriptSourceKind::Inline);
        assert_eq!(
            document.script_text(node_id).as_deref(),
            Some("window.svgReady = true;")
        );
        assert!(
            document
                .elements_by_tag_name(document.document_node_id(), "p", false)
                .is_empty(),
            "parser must not consume content after the SVG script handoff"
        );
    }

    #[test]
    fn parser_stream_surfaces_self_closing_external_svg_script_handoff() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome = stream.pump_parser_step(
            "<!doctype html><body><svg><script href='/self-closing.js'/></svg><p>late</p>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected self-closing SVG script handoff");
        };
        let ParserScriptHandoff::BlockingClassic { script, .. } = *handoff else {
            panic!("expected external SVG script to use the blocking classic lane");
        };

        assert_eq!(script.source_kind, ScriptSourceKind::External);
        assert_eq!(script.url.as_str(), "https://example.test/self-closing.js");
        let document = stream.snapshot_parser_stream_document();
        assert!(
            document
                .elements_by_tag_name(document.document_node_id(), "p", false)
                .is_empty(),
            "parser must pause immediately after a self-closing SVG script"
        );
    }

    #[test]
    fn parser_stream_does_not_execute_svg_script_popped_by_an_ancestor_end_tag() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome = stream.pump_parser_step(
            "<!doctype html><body><svg><script>window.mustStayInert = true;</svg><p>after</p>",
        );

        assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
        let document = stream.snapshot_parser_stream_document();
        assert_eq!(
            document
                .elements_by_tag_name(document.document_node_id(), "p", false)
                .len(),
            1,
            "foreign stack reconciliation should still restore the HTML parent"
        );
    }

    #[test]
    fn data_block_handoff_consumes_parser_inserted_prepare_state() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let result = stream
            .pump_parser_step("<script type='application/json'>{\"ok\":true}</script>")
            .result;

        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected data-block parser handoff");
        };
        let ParserScriptHandoff::NoExecution { outcome, .. } = *handoff else {
            panic!("data block must remain non-executable");
        };
        assert_eq!(
            outcome.element_state_transition(),
            crate::ParserScriptElementStateTransition::ConsumeParserInserted { force_async: true }
        );
        assert!(matches!(
            outcome.run(),
            Some(run)
                if matches!(
                    run.outcome(),
                    moli_page_types::ScriptRunOutcome::Skipped(
                        ScriptSkipReason::UnsupportedType(_)
                    )
                )
        ));
    }

    #[test]
    fn parser_stream_records_custom_element_construction_handoff_candidates() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        stream.note_defined_autonomous_custom_element("x-ready");

        let outcome = stream.pump_parser_step(
            "<!doctype html><body><x-ready id='first' data-probe='yes'></x-ready><x-late></x-late>",
        );
        let ParserPumpStep::Yield(ParserYield::CustomElementConstruction(handoff)) = outcome.result
        else {
            panic!("expected parser pump to surface custom element construction handoff");
        };
        let handoff = &*handoff;
        assert_eq!(handoff.local_name, "x-ready");
        assert_eq!(handoff.namespace, "http://www.w3.org/1999/xhtml");
        assert_eq!(handoff.prefix, None);
        assert_eq!(handoff.parent_at_creation, None);
        assert_eq!(handoff.owner_document.index(), 0);
        assert!(handoff.placeholder.index() > handoff.owner_document.index());
        assert!(
            handoff
                .attributes
                .iter()
                .any(|attribute| attribute.name() == "id" && attribute.value() == "first")
        );
        assert!(
            handoff
                .attributes
                .iter()
                .any(|attribute| attribute.name() == "data-probe" && attribute.value() == "yes")
        );
        assert!(
            stream
                .drain_pending_custom_element_construction_handoffs()
                .is_empty(),
            "handoff queue should drain exactly once"
        );
    }

    #[test]
    fn parser_stream_surfaces_multiple_custom_element_handoffs_one_at_a_time() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        stream.note_defined_autonomous_custom_element("x-ready");

        let first = stream.pump_parser_step(
            "<!doctype html><body><x-ready id='a'></x-ready><x-ready id='b'></x-ready>",
        );
        let ParserPumpStep::Yield(ParserYield::CustomElementConstruction(first_handoff)) =
            first.result
        else {
            panic!("expected first custom element construction handoff");
        };
        assert_eq!(first_handoff.local_name, "x-ready");
        assert!(
            first_handoff
                .attributes
                .iter()
                .any(|attribute| attribute.name() == "id" && attribute.value() == "a")
        );

        let second = stream.pump_parser_step("");
        let ParserPumpStep::Yield(ParserYield::CustomElementConstruction(second_handoff)) =
            second.result
        else {
            panic!("expected second custom element construction handoff");
        };
        assert_eq!(second_handoff.local_name, "x-ready");
        assert!(
            second_handoff
                .attributes
                .iter()
                .any(|attribute| attribute.name() == "id" && attribute.value() == "b")
        );

        assert!(matches!(
            stream.pump_parser_step("").result,
            ParserPumpStep::InputDrained
        ));
    }

    #[test]
    fn parser_stream_custom_element_handoff_pauses_before_following_sibling_tokens() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        stream.note_defined_autonomous_custom_element("x-ready");

        let first = stream.pump_parser_step(
            "<!doctype html><body><x-ready id='a'></x-ready><x-late id='later'></x-late>",
        );
        let ParserPumpStep::Yield(ParserYield::CustomElementConstruction(first_handoff)) =
            first.result
        else {
            panic!("expected custom element construction handoff");
        };
        assert_eq!(first_handoff.local_name, "x-ready");

        let snapshot = stream.snapshot_parser_stream_document();
        assert!(
            snapshot.node(first_handoff.placeholder).is_some(),
            "placeholder should exist at the handoff boundary"
        );
        assert!(
            snapshot
                .nodes()
                .iter()
                .all(|node| node.local_name() != Some("x-late")),
            "parser must not consume following sibling tokens before custom element construction handoff"
        );
    }

    #[test]
    fn parser_stream_consumes_blocking_stylesheet_pause_before_resuming_tokenizer() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let first = stream.pump_parser_step(concat!(
            "<!doctype html><body>",
            "<link rel=stylesheet href='/slow.css'>",
            "\n<script>window.afterStylesheet = true;</script>",
            "<footer id=after>after</footer>",
            "</body>"
        ));
        let ParserPumpStep::Yield(ParserYield::BlockingStylesheet(pause)) = first.result else {
            panic!("expected body stylesheet parser pause");
        };
        let paused_document = stream.snapshot_parser_stream_document();
        assert!(
            paused_document.node(pause.node_id).is_some(),
            "stylesheet owner should exist at the parser pause boundary"
        );
        assert!(
            paused_document
                .elements_by_tag_name(paused_document.document_node_id(), "script", false)
                .is_empty(),
            "parser must not consume the following script start tag before the stylesheet settles"
        );
        assert!(
            paused_document
                .elements_by_tag_name(paused_document.document_node_id(), "footer", false)
                .is_empty(),
            "parser must retain the tail after the stylesheet pause"
        );

        let second = stream.pump_parser_step("");
        assert!(
            matches!(second.result, ParserPumpStep::Yield(ParserYield::Script(_))),
            "resuming after the stylesheet pause should reach the following script"
        );

        assert!(matches!(
            stream.pump_parser_step("").result,
            ParserPumpStep::InputDrained
        ));
        let completed_document = stream.snapshot_parser_stream_document();
        assert_eq!(
            completed_document
                .elements_by_tag_name(completed_document.document_node_id(), "footer", false)
                .len(),
            1,
            "parser should consume the retained tail after the script handoff"
        );
    }

    #[test]
    fn parser_stream_feed_consumes_custom_element_handoff_without_runtime_owner() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );
        stream.note_defined_autonomous_custom_element("x-ready");

        stream.feed("<!doctype html><body><x-ready id='a'></x-ready><x-late id='later'></x-late>");

        let snapshot = stream.snapshot_parser_stream_document();
        assert!(
            stream
                .drain_pending_custom_element_construction_handoffs()
                .is_empty(),
            "non-pump parsing has no runtime owner to consume parser-side handoffs"
        );
        assert!(
            snapshot
                .nodes()
                .iter()
                .any(|node| node.local_name() == Some("x-ready")),
            "defined custom element token should still be parsed"
        );
        assert!(
            snapshot
                .nodes()
                .iter()
                .any(|node| node.local_name() == Some("x-late")),
            "non-pump feed should continue parsing later sibling tokens"
        );
    }

    #[test]
    fn parser_stream_does_not_record_custom_element_handoff_without_runtime_definition() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page.html").expect("test url"),
        );

        let outcome = stream.pump_parser_step(
            "<!doctype html><body><x-ready id='first'></x-ready><x-late></x-late>",
        );
        assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
        assert!(
            stream
                .drain_pending_custom_element_construction_handoffs()
                .is_empty()
        );
    }

    #[test]
    fn prepare_parser_post_parse_script_rejects_data_block_and_already_started() {
        let (data_block_document, data_block_handle) =
            first_script_handle("<script type=\"application/json\">{\"ok\":true}</script>");
        assert!(
            prepare_parser_post_parse_script(&data_block_document, data_block_handle).is_none()
        );

        let (mut document, handle) =
            first_script_handle("<script defer src=\"/defer.js\"></script>");
        let Some(node) = document.node_mut(handle) else {
            panic!("script node should exist");
        };
        let Some(element) = node.data_mut().as_element_mut() else {
            panic!("script handle should resolve to an element");
        };
        let _ = element.set_script_already_started(true);
        assert!(prepare_parser_post_parse_script(&document, handle).is_none());
    }

    #[test]
    fn prepare_parser_helpers_can_read_from_live_dom_host() {
        let (document, blocking_handle) =
            first_script_handle("<script>window.ran = true;</script>");
        let host = DomHost::from_dom(document);
        let prepared = prepare_parser_blocking_classic_script(&host, blocking_handle)
            .expect("live dom host should support parser-blocking planning reads");
        assert_eq!(prepared.mode, ScriptMode::Normal);
        assert_eq!(prepared.kind, ScriptKind::Classic);

        let (document, defer_handle) =
            first_script_handle("<script defer src=\"/defer.js\"></script>");
        let host = DomHost::from_dom(document);
        let prepared = prepare_parser_non_async_post_parse_script(&host, defer_handle)
            .expect("live dom host should support parser post-parse planning reads");
        assert_eq!(prepared.mode, ScriptMode::Defer);
        assert_eq!(prepared.kind, ScriptKind::Classic);
    }

    #[test]
    fn parser_stream_caches_document_url_before_runtime_dom_takeover() {
        let url = Url::parse("https://example.test/page.html").expect("test url");
        let mut stream = DocumentStream::new_parser_stream_for_testing(url.clone());
        let dom_host = stream.take_parser_stream_dom_host();

        let cached_url = stream.with_stylesheet_blocking_read_view(|view| view.final_url_clone());
        assert_eq!(
            cached_url.as_ref(),
            Some(&url),
            "parser planning should retain document URL after bootstrap DOM is taken by runtime"
        );

        stream.restore_parser_stream_dom_host(dom_host);
    }

    #[test]
    fn parser_stream_reports_style_import_blocker_before_classic_handoff() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page").expect("test url"),
        );

        let outcome = stream.pump_parser_step(
            "<!doctype html><html><head><style>@import url('/style.css');</style><script src='/app.js'></script></head></html>",
        );

        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected parser-blocking classic handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            blocking_signatures_before,
            ..
        } = *handoff
        else {
            panic!("expected parser-blocking classic handoff");
        };

        let expected_url = Url::parse("https://example.test/style.css").unwrap();
        assert_eq!(outcome.discovered_blocking_stylesheet_inputs.len(), 1);
        assert!(
            blocking_signatures_before
                .iter()
                .any(|signature| matches!(signature, DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls } if urls == &vec![expected_url.clone()])),
            "handoff should carry parser-created style import blocker signature"
        );
    }

    #[test]
    fn parser_stream_reports_split_connected_meta_csp_once_and_ignores_template_contents() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page").expect("test url"),
        );

        let first = stream.pump_parser_step(r#"<meta http-equiv="Content-Security-"#);
        assert!(matches!(first.result, ParserPumpStep::InputDrained));
        assert!(
            stream
                .drain_discovered_parser_meta_csp_candidates()
                .is_empty(),
            "an incomplete tag must not publish a parser policy checkpoint"
        );

        let second = stream.pump_parser_step(
            r#"Policy" content="script-src 'self'">
               <template><meta http-equiv="content-security-policy" content="script-src 'none'"></template>
               <script src="/app.js"></script>"#,
        );
        assert!(matches!(
            second.result,
            ParserPumpStep::Yield(ParserYield::Script(_))
        ));
        let candidates = stream.drain_discovered_parser_meta_csp_candidates();
        assert_eq!(candidates.len(), 1);
        assert!(
            stream
                .drain_discovered_parser_meta_csp_candidates()
                .is_empty(),
            "a parser checkpoint must be consumed exactly once"
        );
    }

    #[test]
    fn parser_stream_reports_style_import_blocker_on_runtime_dom_sinks() {
        let mut stream = DocumentStream::new_parser_stream_for_testing(
            Url::parse("https://example.test/page").expect("test url"),
        );
        let mut dom_host = stream.take_parser_stream_dom_host();
        let ptr = &mut dom_host as *mut DomHost;
        let mut effects = DomMutationEffects::default();

        let outcome = {
            let mut collector = TestMutationEffectCollector {
                host: ptr,
                effects: &mut effects,
            };
            stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
                "<!doctype html><html><head><style>@import url('/style.css');</style><script src='/app.js'></script></head></html>",
                &mut collector,
            )
        };
        stream.restore_parser_stream_dom_host(dom_host);

        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = outcome.result else {
            panic!("expected parser-blocking classic handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            blocking_signatures_before,
            ..
        } = *handoff
        else {
            panic!("expected parser-blocking classic handoff");
        };

        let expected_url = Url::parse("https://example.test/style.css").unwrap();
        assert!(
            effects.did_change(),
            "runtime DOM sink pump should report runtime-visible mutation effects through an explicit sink"
        );
        assert_eq!(outcome.discovered_blocking_stylesheet_inputs.len(), 1);
        assert!(
            blocking_signatures_before
                .iter()
                .any(|signature| matches!(signature, DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls } if urls == &vec![expected_url.clone()])),
            "handoff should carry parser-created style import blocker signature through runtime DOM sinks step"
        );
    }
}

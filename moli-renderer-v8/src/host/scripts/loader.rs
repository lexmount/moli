use crate::{
    dom::native::{DomHost, NativeNodeId, Node},
    {
        host::HostDocumentState,
        planning::{PreparedScript, ScriptFetchMetadata, ScriptSource},
        types::{
            ScriptElementClassificationInput, ScriptKind, ScriptMode,
            ScriptPreparationClassificationInput, ScriptSkipReason, ScriptSourceKind,
            classify_script_preparation,
        },
    },
};
use moli_script::{ScriptPreparationDisposition, script_element_nonce_is_nonceable};
use url::Url;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeScriptPreparationContext {
    pub(super) document_url: Url,
    pub(super) base_url: Url,
    pub(super) fetch_metadata: ScriptFetchMetadata,
}

impl RuntimeScriptPreparationContext {
    pub(crate) fn capture(
        dom_host: &DomHost,
        document: &HostDocumentState,
        node: NativeNodeId,
    ) -> RuntimeScriptPreparationContext {
        let script_element = dom_host.node(node).and_then(Node::as_element);
        let nonce = script_element_nonce_for_csp(dom_host, node).map(str::to_owned);
        let parser_inserted =
            script_element.is_some_and(|element| element.script_parser_inserted_for_prepare());
        let owner_document = dom_host.owner_document_handle(node);
        let document_url = owner_document
            .and_then(|document| dom_host.document_url_for_handle(document).cloned())
            .unwrap_or_else(|| document.url().clone());
        let base_url = owner_document
            .and_then(|document| dom_host.document_base_url_for_handle(document))
            .unwrap_or_else(|| document_url.clone());
        RuntimeScriptPreparationContext {
            document_url,
            base_url,
            fetch_metadata: ScriptFetchMetadata::from_script_attributes(
                dom_host.get_attribute(node, "crossorigin").as_deref(),
                dom_host.get_attribute(node, "referrerpolicy").as_deref(),
                dom_host.get_attribute(node, "charset").as_deref(),
                dom_host.get_attribute(node, "integrity").as_deref(),
                nonce.as_deref(),
                dom_host.get_attribute(node, "fetchpriority").as_deref(),
            )
            .with_parser_inserted(parser_inserted),
        }
    }
}

pub(crate) fn script_element_nonce_for_csp(dom_host: &DomHost, node: NativeNodeId) -> Option<&str> {
    let element = dom_host.node(node).and_then(Node::as_element)?;
    let nonce = element
        .cryptographic_nonce()
        .or_else(|| element.attribute("nonce"));
    script_element_nonce_is_nonceable(
        nonce,
        false,
        element
            .attributes()
            .iter()
            .map(|attribute| (attribute.local_name(), attribute.value())),
    )
    .then_some(nonce)
    .flatten()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeScriptStartDecision {
    Skip {
        commit_start: bool,
        reason: Option<ScriptSkipReason>,
    },
    ExecuteInlineClassic {
        source: String,
    },
    RegisterImportMap {
        source: String,
    },
    RejectExternalImportMap,
    Queue {
        source: String,
        kind: ScriptKind,
        mode: ScriptMode,
        source_kind: ScriptSourceKind,
    },
    QueueFailed {
        source: String,
        kind: ScriptKind,
        mode: ScriptMode,
        source_kind: ScriptSourceKind,
        message: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptElementLoaderOptions {
    pub(crate) allow_parser_blocking_modes: bool,
    pub(crate) suppress_force_async: bool,
    pub(crate) document_write_connected: bool,
    /// Scripting is disabled for the script element's relevant settings object.
    /// Valid scripts still commit their already-started state, but they must not
    /// fetch, register an import map, or execute.
    pub(crate) scripting_disabled: bool,
    /// Trusted Types script-text preparation has already updated the script's
    /// internal slot. Read that slot before the empty-source and type checks.
    pub(crate) use_prepared_script_text: bool,
}

impl ScriptElementLoaderOptions {
    pub(crate) const fn with_scripting_enabled(scripting_enabled: bool) -> Self {
        Self {
            allow_parser_blocking_modes: false,
            suppress_force_async: false,
            document_write_connected: false,
            scripting_disabled: !scripting_enabled,
            use_prepared_script_text: false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedScriptElementStart {
    pub(super) preparation: RuntimeScriptPreparationContext,
    pub(super) decision: RuntimeScriptStartDecision,
}

impl PreparedScriptElementStart {
    pub(crate) fn into_parts(
        self,
    ) -> (RuntimeScriptPreparationContext, RuntimeScriptStartDecision) {
        (self.preparation, self.decision)
    }
}

pub(crate) struct ScriptElementLoader;

fn script_source_attribute(dom_host: &DomHost, node: NativeNodeId) -> Option<String> {
    dom_host
        .node(node)
        .and_then(Node::as_element)
        .and_then(|element| element.script_source_attribute())
        .map(str::to_owned)
}

impl ScriptElementLoader {
    pub(crate) fn prepare(
        dom_host: &mut DomHost,
        document: &HostDocumentState,
        node: NativeNodeId,
        options: ScriptElementLoaderOptions,
    ) -> PreparedScriptElementStart {
        let preparation = RuntimeScriptPreparationContext::capture(dom_host, document, node);
        let decision = reject_unresolvable_external_script_src(
            &preparation.base_url,
            decide_runtime_script_start(dom_host, node, options),
        );
        PreparedScriptElementStart {
            preparation,
            decision,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_runtime_prepared_script(
    preparation: &RuntimeScriptPreparationContext,
    node: NativeNodeId,
    position: usize,
    host_script_handle: Option<String>,
    source: &str,
    source_kind: ScriptSourceKind,
    kind: ScriptKind,
    mode: ScriptMode,
) -> std::result::Result<PreparedScript, String> {
    let (url, source) = match source_kind {
        ScriptSourceKind::External => {
            let url = preparation
                .base_url
                .join(source)
                .or_else(|_| Url::parse(source))
                .map_err(|error| format!("failed to resolve dynamic script `{source}`: {error}"))?;
            (url, ScriptSource::External)
        }
        ScriptSourceKind::Inline => (
            preparation.base_url.clone(),
            ScriptSource::Inline(source.to_owned()),
        ),
    };

    Ok(PreparedScript {
        position,
        node_id: crate::dom::NodeId::new(node.index()),
        kind,
        mode,
        source_kind,
        fetch_metadata: preparation.fetch_metadata.clone(),
        source,
        initiator_url: preparation.document_url.clone(),
        base_url: url.clone(),
        url,
        host_script_handle,
    })
}

fn reject_unresolvable_external_script_src(
    base_url: &Url,
    decision: RuntimeScriptStartDecision,
) -> RuntimeScriptStartDecision {
    let RuntimeScriptStartDecision::Queue {
        source,
        kind,
        mode,
        source_kind: ScriptSourceKind::External,
    } = decision
    else {
        return decision;
    };

    match base_url.join(&source).or_else(|_| Url::parse(&source)) {
        Ok(_) => RuntimeScriptStartDecision::Queue {
            source,
            kind,
            mode,
            source_kind: ScriptSourceKind::External,
        },
        Err(error) => RuntimeScriptStartDecision::QueueFailed {
            message: format!("failed to resolve script src `{source}`: {error}"),
            source,
            kind,
            mode,
            source_kind: ScriptSourceKind::External,
        },
    }
}

pub(crate) fn classify_document_write_connected_mode(
    kind: ScriptKind,
    mode: ScriptMode,
) -> ScriptMode {
    match (kind, mode) {
        // `document.write`-connected plain classic external scripts keep the
        // parser-blocking immediate path and are handled before this coercion.
        //
        // For the remaining parser-connected non-classic/defer-like cases we do
        // not yet have a fully parser-stream-owned queue. Route them into the
        // closest owner-supported lane instead of falling back to the dynamic
        // scheduler's `unreachable!()` arm.
        (ScriptKind::Classic, ScriptMode::Defer) => ScriptMode::InOrder,
        (ScriptKind::Module, ScriptMode::ModuleDefer) => ScriptMode::ModuleInOrder,
        (ScriptKind::ImportMap, ScriptMode::Normal) => ScriptMode::ImportMapInOrder,
        (_, mode) => mode,
    }
}

pub(crate) fn decide_runtime_script_start(
    dom_host: &mut DomHost,
    node: NativeNodeId,
    options: ScriptElementLoaderOptions,
) -> RuntimeScriptStartDecision {
    if !dom_host.is_script_element(node) {
        return RuntimeScriptStartDecision::Skip {
            commit_start: false,
            reason: Some(ScriptSkipReason::NotInMainDocument),
        };
    }

    let Some((
        already_started,
        was_parser_inserted,
        async_attribute_present,
        defer_attribute_present,
        nomodule_attribute_present,
        force_async_at_prepare_entry,
        is_html_script,
    )) = dom_host.node(node).and_then(|native| {
        native.as_element().map(|element| {
            let async_attribute_present = element.has_attribute("async");
            let is_html_script = element.is_html_script();
            (
                element.script_already_started(),
                element.script_parser_inserted_for_prepare(),
                async_attribute_present,
                is_html_script && element.has_attribute("defer"),
                is_html_script && element.has_attribute("nomodule"),
                element.script_async() && !async_attribute_present,
                is_html_script,
            )
        })
    })
    else {
        return RuntimeScriptStartDecision::Skip {
            commit_start: false,
            reason: Some(ScriptSkipReason::NotInMainDocument),
        };
    };

    if already_started {
        return RuntimeScriptStartDecision::Skip {
            commit_start: false,
            reason: Some(ScriptSkipReason::AlreadyStarted),
        };
    }

    consume_parser_inserted_script_prepare_state_with_async_attribute(
        dom_host,
        node,
        was_parser_inserted,
        async_attribute_present,
    );

    let script_src = script_source_attribute(dom_host, node);
    let source_kind = ScriptSourceKind::from_script_src(&script_src);
    let script_type = dom_host.get_attribute(node, "type");
    let language = is_html_script
        .then(|| dom_host.get_attribute(node, "language"))
        .flatten();
    let event = is_html_script
        .then(|| dom_host.get_attribute(node, "event"))
        .flatten();
    let for_attribute = is_html_script
        .then(|| dom_host.get_attribute(node, "for"))
        .flatten();
    let classification = classify_script_preparation(ScriptPreparationClassificationInput {
        element: ScriptElementClassificationInput {
            script_type: script_type.as_deref(),
            language: language.as_deref(),
            event: event.as_deref(),
            for_attribute: for_attribute.as_deref(),
        },
        parser_inserted: was_parser_inserted,
        allow_parser_blocking_modes: options.allow_parser_blocking_modes,
        force_async: !options.suppress_force_async && force_async_at_prepare_entry,
        async_attribute_present,
        defer_attribute_present,
        source_kind,
    });
    let disposition = classification.disposition;
    let kind = disposition.kind();

    if disposition == ScriptPreparationDisposition::DataBlock {
        return RuntimeScriptStartDecision::Skip {
            commit_start: false,
            reason: Some(ScriptSkipReason::UnsupportedType(
                dom_host.get_attribute(node, "type").unwrap_or_default(),
            )),
        };
    }
    if options.scripting_disabled {
        return RuntimeScriptStartDecision::Skip {
            commit_start: true,
            reason: Some(ScriptSkipReason::ScriptExecutionDisabled),
        };
    }
    let source = match script_src {
        Some(source) => source,
        None => {
            let source = if options.use_prepared_script_text {
                dom_host
                    .node(node)
                    .and_then(Node::as_element)
                    .map(|element| element.script_text_internal_slot().to_owned())
                    .unwrap_or_default()
            } else {
                dom_host.dom().direct_text_content(node).unwrap_or_default()
            };
            if source.is_empty() {
                return RuntimeScriptStartDecision::Skip {
                    commit_start: false,
                    reason: Some(ScriptSkipReason::EmptyInlineScript),
                };
            }
            source
        }
    };

    if !dom_host.is_connected(node) {
        return RuntimeScriptStartDecision::Skip {
            commit_start: false,
            reason: Some(ScriptSkipReason::NotInMainDocument),
        };
    }

    if classification.legacy_event_for_mismatch {
        return RuntimeScriptStartDecision::Skip {
            commit_start: false,
            reason: Some(ScriptSkipReason::UnsupportedType(
                "legacy for/event script did not match window.onload".to_owned(),
            )),
        };
    }

    if matches!(kind, ScriptKind::Classic) && nomodule_attribute_present {
        return RuntimeScriptStartDecision::Skip {
            commit_start: true,
            reason: Some(ScriptSkipReason::NoModule),
        };
    }

    if disposition == ScriptPreparationDisposition::ImportMap {
        return if source_kind == ScriptSourceKind::External {
            RuntimeScriptStartDecision::RejectExternalImportMap
        } else {
            RuntimeScriptStartDecision::RegisterImportMap { source }
        };
    }

    if was_parser_inserted {
        let _ = dom_host.set_script_parser_inserted_for_prepare(node, true);
        let _ = dom_host.set_script_force_async(node, false);
    }

    let (kind, mode) = disposition
        .executable()
        .expect("non-executable script dispositions return before queueing");
    let mode = if options.document_write_connected {
        classify_document_write_connected_mode(kind, mode)
    } else {
        mode
    };

    if source_kind == ScriptSourceKind::External && source.trim().is_empty() {
        return RuntimeScriptStartDecision::QueueFailed {
            source,
            kind,
            mode,
            source_kind,
            message: "empty script src is not fetchable".to_owned(),
        };
    }

    if matches!(kind, ScriptKind::Classic) && source_kind == ScriptSourceKind::Inline {
        return RuntimeScriptStartDecision::ExecuteInlineClassic { source };
    }

    RuntimeScriptStartDecision::Queue {
        source,
        kind,
        mode,
        source_kind,
    }
}

pub(crate) fn apply_parser_script_element_state_transition(
    dom_host: &mut DomHost,
    node: NativeNodeId,
    transition: crate::parser::ParserScriptElementStateTransition,
) {
    match transition {
        crate::parser::ParserScriptElementStateTransition::None => {}
        crate::parser::ParserScriptElementStateTransition::ConsumeParserInserted {
            force_async,
        } => {
            let _ = dom_host.set_script_parser_inserted_for_prepare(node, false);
            let _ = dom_host.set_script_force_async(node, force_async);
        }
        crate::parser::ParserScriptElementStateTransition::MarkAlreadyStarted => {
            let _ = dom_host.set_script_already_started(node, true);
        }
    }
}

pub(crate) fn apply_parser_script_element_state_without_execution(
    dom_host: &mut DomHost,
    handoff: &crate::parser::ParserScriptHandoff,
) {
    let transition = match handoff {
        crate::parser::ParserScriptHandoff::NoExecution { outcome, .. } => {
            outcome.element_state_transition()
        }
        crate::parser::ParserScriptHandoff::PreparationFailure { failure, .. } => {
            failure.element_state_transition()
        }
        crate::parser::ParserScriptHandoff::BlockingClassic { .. }
        | crate::parser::ParserScriptHandoff::AsyncPostParse { .. }
        | crate::parser::ParserScriptHandoff::NonAsyncPostParse { .. }
        | crate::parser::ParserScriptHandoff::ImportMap { .. } => {
            crate::parser::ParserScriptElementStateTransition::MarkAlreadyStarted
        }
    };
    apply_parser_script_element_state_transition(dom_host, handoff.node_id(), transition);
}

fn consume_parser_inserted_script_prepare_state_with_async_attribute(
    dom_host: &mut DomHost,
    node: NativeNodeId,
    was_parser_inserted: bool,
    async_attribute_present: bool,
) {
    if !was_parser_inserted {
        return;
    }
    let _ = dom_host.set_script_parser_inserted_for_prepare(node, false);
    if !async_attribute_present {
        let _ = dom_host.set_script_force_async(node, true);
    }
}

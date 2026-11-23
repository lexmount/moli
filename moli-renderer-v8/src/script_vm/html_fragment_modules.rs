use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    module_script_continuation::MainParserDocumentOwner,
    native_bridge::{self, HtmlFragmentModuleTerminal, OwnerDispatchScope, WindowDocumentOwner},
    page_task_queue::HtmlFragmentModuleTaskTarget,
    planning::PreparedScript,
    types::ScriptMode,
};
use anyhow::Result;

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn start_html_fragment_module(
        &mut self,
        route: HtmlFragmentModuleTaskTarget,
    ) -> Result<bool> {
        let Some(start) = self.with_default_context_scope(|scope, host_ptr| {
            Ok(native_bridge::take_html_fragment_module_start(
                scope, host_ptr, route,
            ))
        })?
        else {
            return Ok(false);
        };
        let mut script = start.script;
        let node = script.node_id;
        let WindowDocumentOwner::Frame(owner) = route.document.owner() else {
            return Ok(self.publish_html_fragment_module_terminal(
                route,
                HtmlFragmentModuleTerminal::Unavailable(node),
            ));
        };
        if !start.still_in_preparation_document {
            return Ok(self.publish_html_fragment_module_terminal(
                route,
                HtmlFragmentModuleTerminal::Unavailable(node),
            ));
        }
        // The stream is the only execution-order owner. Reuse the parser
        // graph machinery without entering the live Document's deferred lane.
        script.mode = ScriptMode::Async;
        script.document_referrer_policy = {
            let host = self._context_host.borrow();
            host.dom_host()
                .owner_document_handle(node)
                .and_then(|document| {
                    crate::context_bootstrap::document_referrer_policy_for_native_document(
                        &host, document,
                    )
                })
        };
        let started = match route.document.dispatch_scope() {
            OwnerDispatchScope::Top => {
                script.host_script_handle = Some(
                    self.document_runtime
                        .bind_parser_owned_script_handle_for_node(node),
                );
                self.start_main_html_fragment_module(route, owner, &script)
            }
            OwnerDispatchScope::Child(child) => {
                let mut host = self._context_host.borrow_mut();
                let accepted = host.queue_child_html_fragment_module(child, script);
                if accepted {
                    host.register_html_fragment_module_load(owner, node, route);
                }
                Ok(accepted)
            }
            OwnerDispatchScope::LightweightPopup(_) => Ok(false),
        };
        match started {
            Ok(true) => Ok(true),
            Ok(false) => Ok(self.publish_html_fragment_module_terminal(
                route,
                HtmlFragmentModuleTerminal::Unavailable(node),
            )),
            Err(error) => {
                self.publish_html_fragment_module_terminal(
                    route,
                    HtmlFragmentModuleTerminal::Unavailable(node),
                );
                Err(error)
            }
        }
    }

    fn start_main_html_fragment_module(
        &mut self,
        route: HtmlFragmentModuleTaskTarget,
        owner: FrameDocumentTaskOwner,
        script: &PreparedScript,
    ) -> Result<bool> {
        // The parser graph adapter decodes data URLs without a network fetch.
        // Apply the same initial module policy before it can compile that source.
        if script.source_kind == crate::types::ScriptSourceKind::External
            && script.url.scheme() == "data"
        {
            use crate::module_runtime::{ModuleFetchMetadata, ModuleKind};
            let context = self.document_runtime.module_fetch_csp_context(
                &script.url,
                ModuleKind::JavaScript,
                &ModuleFetchMetadata::from_parser_owned_script_fetch_metadata(
                    &script.fetch_metadata,
                ),
            );
            let allowed = context.check_initial_request(&script.url).is_ok();
            self.queue_module_fetch_csp_violations(&context);
            if !allowed {
                return Ok(false);
            }
        }
        let pending = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .register_module_script(MainParserDocumentOwner::new(owner), script);
        self._context_host
            .borrow_mut()
            .register_html_fragment_module_load(owner, script.node_id, route);
        let started = self.accept_registered_parser_pending_module_script_graph_for_document_owner(
            pending, script, None,
        );
        if !matches!(started, Ok(true)) {
            self.document_runtime
                .parser_module_document_scripts_mut()
                .discard_module_script(pending);
            self._context_host
                .borrow_mut()
                .take_html_fragment_module_load(owner, script.node_id);
        }
        started
    }

    pub(in crate::script_vm) fn publish_html_fragment_module_terminal(
        &mut self,
        route: HtmlFragmentModuleTaskTarget,
        terminal: HtmlFragmentModuleTerminal,
    ) -> bool {
        match self.with_default_context_scope(|scope, host_ptr| {
            Ok(native_bridge::accept_html_fragment_module_terminal(
                scope, host_ptr, route, terminal,
            ))
        }) {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::debug!(?route, %error, "streamed module owner is no longer runnable");
                false
            }
        }
    }

    pub(crate) fn take_html_fragment_module_terminal(
        &mut self,
        route: HtmlFragmentModuleTaskTarget,
    ) -> Result<Option<HtmlFragmentModuleTerminal>> {
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(native_bridge::take_html_fragment_module_terminal(
                scope, host_ptr, route,
            ))
        })
    }

    pub(crate) fn finish_html_fragment_module_turn(
        &mut self,
        route: HtmlFragmentModuleTaskTarget,
        failure: Option<DomHandle>,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            native_bridge::finish_html_fragment_module_turn(scope, host_ptr, route, failure);
            Ok(())
        })
    }

    pub(crate) async fn run_child_html_fragment_module_terminal(
        &mut self,
        work: crate::document_script_scheduler::FrameDocumentModuleScriptReadyWork,
    ) -> bool {
        self.run_child_module_script_ready_work(work)
            .await
            .activity()
            == crate::document_script_scheduler::FrameModuleScriptTaskActivity::ScriptOrEvent
    }
}

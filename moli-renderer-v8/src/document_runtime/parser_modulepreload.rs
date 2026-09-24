use super::*;
use crate::module_runtime::{
    ModuleKind, ModuleLoadError, ModuleLoadStage, ModulePreloadJobRun,
    NativeModuleSingleFetchRequest,
};
use crate::modulepreload::{
    ParserDiscoveredModulepreloadResult, invalid_modulepreload_as_value,
    invalid_modulepreload_as_warning, modulepreload_fetch_candidate, modulepreload_href,
    resolve_parser_network_resource_url,
};
use crate::renderer_resource_scheduler::RendererResourceScheduler;

/// Result of synchronously registering one main-Document modulepreload and, when
/// this call wins the module-map fetch reservation, starting its network fetch.
///
/// CSP events are returned to the caller because only the V8 entry that owns the
/// current scope may enqueue DOM events. The module-map reservation and resource
/// scheduling themselves are completed before this value is returned.
#[must_use = "CSP violations and the observable job result must be consumed"]
pub(crate) struct MainDocumentModulepreloadFetchOutcome {
    job_run: Option<ModulePreloadJobRun>,
    csp_violations: Vec<DocumentContentSecurityPolicyViolation>,
    runtime_warning: Option<String>,
}

impl MainDocumentModulepreloadFetchOutcome {
    pub(crate) fn into_parts(
        self,
    ) -> (
        Option<ModulePreloadJobRun>,
        Vec<DocumentContentSecurityPolicyViolation>,
        Option<String>,
    ) {
        (self.job_run, self.csp_violations, self.runtime_warning)
    }
}

impl DocumentRuntime {
    /// Accepts only the exact link elements reported by the parser step that
    /// just inserted them.
    ///
    /// This is deliberately not a DOM scan. Runtime-inserted links have their
    /// own connected-link lifecycle, while parser-discovered links arrive here
    /// in parser token order. An old or disconnected handle therefore cannot
    /// be reclassified as work for the current Document.
    pub(crate) fn accept_parser_discovered_modulepreload_links(
        &mut self,
        link_handles: impl IntoIterator<Item = DomHandle>,
    ) -> ParserDiscoveredModulepreloadResult {
        if !self.document_scripting_enabled() {
            return ParserDiscoveredModulepreloadResult::default();
        }
        let Some(document_url) = self.dom_host.final_url().cloned() else {
            return ParserDiscoveredModulepreloadResult::default();
        };
        let document_base_url = self
            .dom_host
            .document_base_url()
            .unwrap_or_else(|| document_url.clone());
        let document_handle = self.dom_host.document_handle();
        let mut accepted = ParserDiscoveredModulepreloadResult::default();

        for link_handle in link_handles {
            if !self.dom_host.is_connected(link_handle)
                || self.dom_host.owner_document_handle(link_handle) != Some(document_handle)
            {
                continue;
            }
            let Some(element) = self
                .dom_host
                .node(link_handle)
                .and_then(crate::dom::native::Node::as_element)
            else {
                continue;
            };
            let Some(raw_href) = modulepreload_href(element) else {
                continue;
            };
            let Some(request_url) =
                resolve_parser_network_resource_url(&document_base_url, raw_href)
            else {
                continue;
            };
            if let Some(invalid_as) = invalid_modulepreload_as_value(element) {
                if self
                    .modulepreload_invalid_as_link_errors
                    .insert(link_handle)
                {
                    accepted.push_runtime_warning(invalid_modulepreload_as_warning(&invalid_as));
                    self.post_modulepreload_link_error_event(link_handle);
                    accepted.push_link_error_task();
                }
                continue;
            }
            let Some(candidate) = modulepreload_fetch_candidate(
                element,
                request_url.clone(),
                &document_url,
                self.resolve_module_integrity(&request_url),
            ) else {
                continue;
            };
            if self.parser_discovered_modulepreloads.insert(candidate.key) {
                accepted.push_request(candidate.request);
            }
        }

        accepted
    }

    /// Performs the synchronous part of Blink's `ModulePreloadIfNeeded` model:
    /// reserve or join the module-map entry now, and start the resource fetch
    /// now when this caller owns the reservation. Only the eventual network
    /// terminal and link error delivery are Page tasks.
    pub(crate) fn start_main_document_modulepreload_fetch(
        &mut self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        resource_scheduler: &RendererResourceScheduler,
        request: NativeModuleSingleFetchRequest,
    ) -> std::result::Result<MainDocumentModulepreloadFetchOutcome, ModuleLoadError> {
        let start = self.fetch_single_native_module_for_modulepreload(request)?;
        let Some(request) = start.started_request() else {
            return Ok(MainDocumentModulepreloadFetchOutcome {
                job_run: None,
                csp_violations: Vec::new(),
                runtime_warning: None,
            });
        };
        self.schedule_reserved_main_document_modulepreload_fetch(
            document_owner,
            resource_scheduler,
            request,
        )
    }

    /// Schedules a fetch whose module-map entry was already reserved by a
    /// caller that also attached a concrete link client.
    pub(crate) fn schedule_reserved_main_document_modulepreload_fetch(
        &mut self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        resource_scheduler: &RendererResourceScheduler,
        request: NativeModuleSingleFetchRequest,
    ) -> std::result::Result<MainDocumentModulepreloadFetchOutcome, ModuleLoadError> {
        let key = request.module_key().clone();
        let fetch_request = request.fetch_request();
        let mut csp_violations = Vec::new();
        if matches!(key.kind(), ModuleKind::JavaScript | ModuleKind::WebAssembly) {
            if let Some(violation) = self
                .script_element_request_csp_report_only_violation_with_nonce(
                    key.url(),
                    fetch_request.nonce(),
                )
            {
                csp_violations.push(violation);
            }
            if let Some(violation) = self
                .script_element_request_csp_violation_with_nonce(key.url(), fetch_request.nonce())
            {
                let error = ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    format!(
                        "Refused to load module `{}` because it violates the document Content Security Policy directive `{}`",
                        key.url(),
                        violation.effective_directive
                    ),
                );
                csp_violations.push(violation);
                self.mark_native_module_failed(key, error.clone());
                return Ok(MainDocumentModulepreloadFetchOutcome {
                    job_run: Some(ModulePreloadJobRun::CompletedSynchronously),
                    csp_violations,
                    runtime_warning: Some(format!(
                        "native modulepreload failed: {}",
                        error.message()
                    )),
                });
            }
        }

        let load_id = self.suspend_native_modulepreload_fetch(request);
        let loader = self
            .current_document_resource_loader()
            .expect("main modulepreload requires the committed Document resource authority");
        resource_scheduler.schedule_main_modulepreload_fetch(
            loader,
            crate::page_resource_completion::MainModulepreloadFetchTarget::new(
                document_owner,
                load_id,
            ),
            fetch_request,
        );
        Ok(MainDocumentModulepreloadFetchOutcome {
            job_run: Some(ModulePreloadJobRun::Scheduled),
            csp_violations,
            runtime_warning: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::HtmlParser;
    use url::Url;

    #[test]
    fn parser_modulepreload_candidate_cannot_cross_document_open_replacement() {
        let document = HtmlParser::SCRIPTING_ENABLED.parse(
            Url::parse("https://example.test/initial.html").expect("test URL"),
            concat!(
                "<!doctype html><html><head>",
                "<link id='old-preload' rel='modulepreload' href='/old.mjs'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let old_link = runtime
            .dom_host()
            .element_handle_by_id("old-preload")
            .expect("old modulepreload link");

        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links([old_link])
            .into_parts();
        assert_eq!(requests.len(), 1);
        assert!(warnings.is_empty());
        assert_eq!(link_error_tasks, 0);

        runtime.open_document();
        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links([old_link])
            .into_parts();

        assert!(
            requests.is_empty(),
            "an exact parser candidate from the retired DOM must not start work in the replacement"
        );
        assert!(warnings.is_empty());
        assert_eq!(link_error_tasks, 0);
        assert!(
            runtime.take_next_native_module_owner_event().is_none(),
            "a stale parser candidate must not publish a link terminal into the replacement"
        );
    }

    #[test]
    fn parser_modulepreload_is_inert_when_script_execution_is_disabled() {
        let document = HtmlParser::SCRIPTING_ENABLED.parse(
            Url::parse("https://example.test/page.html").expect("test URL"),
            concat!(
                "<!doctype html><html><head>",
                "<link id='preload' rel='modulepreload' href='/app.mjs'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        runtime.set_script_execution_disabled(true);
        let preload = runtime
            .dom_host()
            .element_handle_by_id("preload")
            .expect("modulepreload link");

        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links([preload])
            .into_parts();

        assert!(requests.is_empty());
        assert!(warnings.is_empty());
        assert_eq!(link_error_tasks, 0);
    }
}

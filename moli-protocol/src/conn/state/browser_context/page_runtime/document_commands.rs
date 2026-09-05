use super::BrowserContext;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererCaptureScreencastFrameReply,
    RendererCaptureScreencastFrameRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererCommandTurnOutput, RendererLayoutMetrics,
    RendererResourceTextSearchOutcome, RendererSetDocumentContentResult, SubresourceNetworkRecord,
};

impl BrowserContext {
    #[cfg(test)]
    pub(crate) async fn target_runtime_heap_usage_for_test(
        &mut self,
        target_id: &str,
    ) -> Result<moli_core::page::RendererRuntimeHeapUsage, String> {
        let completion = self
            .loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_runtime_heap_usage()
            .map_err(|error| error.to_string())?
            .wait()
            .await
            .map_err(|error| error.to_string())?;
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_runtime_heap_usage(completion)
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(crate) fn target_idle_override_for_test(
        &self,
        target_id: &str,
    ) -> Result<Option<moli_core::page::EmulatedIdleOverride>, String> {
        Ok(self
            .loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .idle_override())
    }

    #[cfg(test)]
    pub(crate) fn target_cached_observable_output_for_test(
        &self,
        target_id: &str,
    ) -> Option<&[moli_core::page::ScriptObservableOutputItem]> {
        Some(
            self.loaded_page_for_target(target_id)?
                .script_execution()
                .observable_output_items(),
        )
    }

    #[cfg(test)]
    pub(crate) async fn evaluate_target_expression_for_test(
        &mut self,
        target_id: &str,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .evaluate_runtime_expression_without_navigation_follow_with_await_async(
                expression,
                await_promise,
            )
            .await
            .map_err(|error| format!("runtime evaluation failed: {error}"))
    }

    #[cfg(test)]
    pub(crate) async fn serialize_target_html_for_test(
        &mut self,
        target_id: &str,
    ) -> Result<String, String> {
        let completion = self
            .start_serialize_html_for_target(target_id)?
            .wait()
            .await
            .map_err(|error| error.to_string())?;
        self.finish_serialize_html_for_target(target_id, completion)
    }

    pub(crate) fn start_set_document_content_for_target(
        &self,
        target_id: &str,
        frame_id: String,
        html: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_document_content(frame_id, html)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_set_document_content_command_turn_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(RendererSetDocumentContentResult, RendererCommandTurnOutput), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_set_document_content_command_turn(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_top_level_same_document_navigation_for_target(
        &self,
        target_id: &str,
        url: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_top_level_same_document_navigation(url)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_top_level_same_document_navigation_command_turn_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_top_level_same_document_navigation_command_turn(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_capture_screencast_frame_for_target(
        &self,
        target_id: &str,
        request: RendererCaptureScreencastFrameRequest,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_capture_screencast_frame(request)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_capture_screencast_frame_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererCaptureScreencastFrameReply, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_capture_screencast_frame(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_serialize_html_for_target(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_serialize_html()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_serialize_html_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<String, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_serialize_html(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_layout_metrics_for_target(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_layout_metrics()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_layout_metrics_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererLayoutMetrics, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_layout_metrics(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_capture_screenshot_with_request_for_target(
        &self,
        target_id: &str,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_capture_screenshot_with_request(request)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_capture_screenshot_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererCaptureScreenshotReply, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_capture_screenshot(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_top_level_history_traversal_by_delta_for_target(
        &self,
        target_id: &str,
        delta: i64,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_top_level_history_traversal_by_delta(delta)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_top_level_history_traversal_by_delta_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<bool, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_top_level_history_traversal_by_delta(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_reset_navigation_history_for_target(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_reset_navigation_history()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_reset_navigation_history_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<bool, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_reset_navigation_history(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_child_frame_navigation_to_url_for_target(
        &self,
        target_id: &str,
        frame_id: &str,
        url: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_child_frame_navigation_to_url(frame_id, url)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_child_frame_navigation_to_url_command_turn_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_child_frame_navigation_to_url_command_turn(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_child_frame_resource_search_by_lines_for_target(
        &self,
        target_id: &str,
        frame_id: String,
        url: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_child_frame_resource_search_by_lines(
                frame_id,
                url,
                query,
                case_sensitive,
                is_regex,
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_text_search_by_lines_for_target(
        &self,
        target_id: &str,
        text: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_text_search_by_lines(text, query, case_sensitive, is_regex)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_resource_search_by_lines_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererResourceTextSearchOutcome, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_resource_search_by_lines(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_javascript_dialog_handler_enabled_for_target(
        &self,
        target_id: &str,
        enabled: bool,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_bypass_content_security_policy_for_target(
        &self,
        target_id: &str,
        bypass: bool,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_bypass_content_security_policy(bypass)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_set_bypass_content_security_policy_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_set_bypass_content_security_policy(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn target_response_headers(&self, target_id: &str) -> Option<&[(String, String)]> {
        self.loaded_page_for_target(target_id)
            .map(|page| page.headers())
    }

    pub(crate) fn target_subresource_network_records(
        &self,
        target_id: &str,
    ) -> Option<&[SubresourceNetworkRecord]> {
        self.loaded_page_for_target(target_id)
            .map(|page| page.subresource_network_records())
    }

    pub(crate) async fn stop_target_document_lifecycle_async(
        &mut self,
        target_id: &str,
    ) -> Result<(), String> {
        let Some(page) = self.loaded_page_for_target_mut(target_id) else {
            return Ok(());
        };
        page.stop_document_lifecycle_async()
            .await
            .map_err(|error| error.to_string())
    }

    pub(crate) fn crash_target_renderer_from_io(&mut self, target_id: &str) {
        if let Some(page) = self.loaded_page_for_target_mut(target_id) {
            page.crash_devtools_target_from_io();
        }
    }
}

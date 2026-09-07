use super::BrowserContext;
use moli_core::browser::{DocumentHandle, WebContentsHandle};
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererCaptureScreencastFrameReply,
    RendererCaptureScreencastFrameRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererCommandTurnOutput, RendererResourceTextSearchOutcome,
    RendererSetDocumentContentResult, SubresourceNetworkRecord,
};

struct PendingDocumentCommand {
    document: DocumentHandle,
    pending: PendingPageCommand,
}

struct CompletedDocumentCommand {
    document: DocumentHandle,
    completed: Result<CompletedPageCommand, String>,
}

impl PendingDocumentCommand {
    async fn wait(self) -> CompletedDocumentCommand {
        CompletedDocumentCommand {
            document: self.document,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedDocumentCommand {
    fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::renderer_output_predecessor)
    }

    fn into_parts(self) -> (DocumentHandle, Result<CompletedPageCommand, String>) {
        (self.document, self.completed)
    }
}

macro_rules! define_document_command {
    ($pending:ident, $completed:ident) => {
        pub(crate) struct $pending(PendingDocumentCommand);

        pub(crate) struct $completed(CompletedDocumentCommand);

        impl $pending {
            pub(super) fn new(document: DocumentHandle, pending: PendingPageCommand) -> Self {
                Self(PendingDocumentCommand { document, pending })
            }

            pub(crate) async fn wait(self) -> $completed {
                $completed(self.0.wait().await)
            }
        }

        impl $completed {
            pub(crate) fn document(&self) -> DocumentHandle {
                self.0.document
            }

            pub(super) fn into_parts(
                self,
            ) -> (DocumentHandle, Result<CompletedPageCommand, String>) {
                self.0.into_parts()
            }
        }
    };
    ($pending:ident, $completed:ident, renderer_output_predecessor) => {
        define_document_command!($pending, $completed);

        impl $completed {
            pub(crate) fn renderer_output_predecessor(
                &self,
            ) -> Option<moli_core::RendererOutputFence> {
                self.0.renderer_output_predecessor()
            }
        }
    };
}

define_document_command!(
    PendingSetDocumentContent,
    CompletedSetDocumentContent,
    renderer_output_predecessor
);
define_document_command!(
    PendingCaptureDocumentSnapshot,
    CompletedCaptureDocumentSnapshot,
    renderer_output_predecessor
);
define_document_command!(
    PendingCaptureDocumentImage,
    CompletedCaptureDocumentImage,
    renderer_output_predecessor
);
define_document_command!(
    PendingCaptureDocumentScreencastFrame,
    CompletedCaptureDocumentScreencastFrame
);
define_document_command!(
    PendingDocumentResourceTextSearch,
    CompletedDocumentResourceTextSearch,
    renderer_output_predecessor
);
define_document_command!(
    PendingDocumentCspBypassUpdate,
    CompletedDocumentCspBypassUpdate,
    renderer_output_predecessor
);
define_document_command!(
    PendingTopLevelSameDocumentNavigation,
    CompletedTopLevelSameDocumentNavigation,
    renderer_output_predecessor
);
define_document_command!(
    PendingTopLevelHistoryTraversal,
    CompletedTopLevelHistoryTraversal,
    renderer_output_predecessor
);
define_document_command!(
    PendingNavigationHistoryReset,
    CompletedNavigationHistoryReset,
    renderer_output_predecessor
);
define_document_command!(
    PendingChildFrameNavigation,
    CompletedChildFrameNavigation,
    renderer_output_predecessor
);
define_document_command!(
    PendingDocumentStorageKeySnapshot,
    CompletedDocumentStorageKeySnapshot
);
define_document_command!(
    PendingChildFrameTreeSnapshot,
    CompletedChildFrameTreeSnapshot
);
define_document_command!(
    PendingDocumentCookieOwnerSnapshot,
    CompletedDocumentCookieOwnerSnapshot
);
define_document_command!(PendingDocumentBlobRead, CompletedDocumentBlobRead);
define_document_command!(
    PendingNetworkResourceLoadPreparation,
    CompletedNetworkResourceLoadPreparation
);
define_document_command!(
    PendingAppManifestLoadPreparation,
    CompletedAppManifestLoadPreparation,
    renderer_output_predecessor
);
define_document_command!(
    PendingAppManifestPublication,
    CompletedAppManifestPublication,
    renderer_output_predecessor
);

impl CompletedTopLevelHistoryTraversal {
    pub(crate) fn renderer_accepted(&self) -> Option<bool> {
        self.0
            .completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::bool_reply_value)
    }
}

pub(crate) struct DocumentSnapshot {
    pub(crate) url: String,
    pub(crate) html: String,
}

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
        let document = self
            .document_handle_for_target(target_id)
            .ok_or("NoDocumentLoaded")?;
        let completion = self.start_capture_document_snapshot(document)?.wait().await;
        Ok(self.finish_capture_document_snapshot(completion)?.html)
    }

    pub(crate) fn document_handle_for_target(&self, target_id: &str) -> Option<DocumentHandle> {
        self.document_handle_for_web_contents(self.web_contents_handle_for_target(target_id)?)
            .ok()
            .flatten()
    }

    pub(crate) fn document_handle_for_web_contents(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<DocumentHandle>, String> {
        let document = self
            .physical
            .web_contents(handle)?
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.id);
        #[cfg(test)]
        let document = document.or_else(|| {
            let target = self.page_targets.get_for_web_contents(handle.id())?;
            self.target_document_id(target.target_id())
        });
        Ok(document.map(|document| DocumentHandle::new(handle, document)))
    }

    pub(crate) fn start_set_document_content(
        &self,
        document: DocumentHandle,
        frame_id: String,
        html: String,
    ) -> Result<PendingSetDocumentContent, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_set_document_content(frame_id, html)
            .map_err(|error| error.to_string())?;
        Ok(PendingSetDocumentContent::new(document, pending))
    }

    pub(crate) fn finish_set_document_content(
        &mut self,
        completed: CompletedSetDocumentContent,
    ) -> Result<(RendererSetDocumentContentResult, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_set_document_content_command_turn(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_top_level_same_document_navigation(
        &self,
        document: DocumentHandle,
        url: String,
    ) -> Result<PendingTopLevelSameDocumentNavigation, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_top_level_same_document_navigation(url)
            .map_err(|error| error.to_string())?;
        Ok(PendingTopLevelSameDocumentNavigation::new(
            document, pending,
        ))
    }

    pub(crate) fn finish_top_level_same_document_navigation(
        &mut self,
        completed: CompletedTopLevelSameDocumentNavigation,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_top_level_same_document_navigation_command_turn(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_capture_document_screencast_frame(
        &self,
        document: DocumentHandle,
        request: RendererCaptureScreencastFrameRequest,
    ) -> Result<PendingCaptureDocumentScreencastFrame, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_capture_screencast_frame(request)
            .map_err(|error| error.to_string())?;
        Ok(PendingCaptureDocumentScreencastFrame::new(
            document, pending,
        ))
    }

    pub(crate) fn finish_capture_document_screencast_frame(
        &mut self,
        completed: CompletedCaptureDocumentScreencastFrame,
    ) -> Result<RendererCaptureScreencastFrameReply, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_capture_screencast_frame(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_capture_document_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingCaptureDocumentSnapshot, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_serialize_html()
            .map_err(|error| error.to_string())?;
        Ok(PendingCaptureDocumentSnapshot::new(document, pending))
    }

    pub(crate) fn finish_capture_document_snapshot(
        &mut self,
        completed: CompletedCaptureDocumentSnapshot,
    ) -> Result<DocumentSnapshot, String> {
        let (document, completion) = completed.into_parts();
        let document = self.physical.document_mut(document)?;
        let html = document
            .page
            .finish_serialize_html(completion?)
            .map_err(|error| error.to_string())?;
        Ok(DocumentSnapshot {
            url: document.page.final_url().as_str().to_owned(),
            html,
        })
    }

    pub(crate) fn start_capture_document_image(
        &self,
        document: DocumentHandle,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<PendingCaptureDocumentImage, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_capture_screenshot_with_request(request)
            .map_err(|error| error.to_string())?;
        Ok(PendingCaptureDocumentImage::new(document, pending))
    }

    pub(crate) fn finish_capture_document_image(
        &mut self,
        completed: CompletedCaptureDocumentImage,
    ) -> Result<RendererCaptureScreenshotReply, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_capture_screenshot(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_top_level_history_traversal(
        &self,
        document: DocumentHandle,
        delta: i64,
    ) -> Result<PendingTopLevelHistoryTraversal, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_top_level_history_traversal_by_delta(delta)
            .map_err(|error| error.to_string())?;
        Ok(PendingTopLevelHistoryTraversal::new(document, pending))
    }

    pub(crate) fn finish_top_level_history_traversal(
        &mut self,
        completed: CompletedTopLevelHistoryTraversal,
    ) -> Result<bool, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_top_level_history_traversal_by_delta(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_navigation_history_reset(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingNavigationHistoryReset, String> {
        self.physical.document(document)?;
        let pending = self
            .physical
            .web_contents
            .get(&document.web_contents().id())
            .ok_or("NoDocumentLoaded")?
            .start_reset_navigation_history()?;
        Ok(PendingNavigationHistoryReset::new(document, pending))
    }

    pub(crate) fn finish_navigation_history_reset(
        &mut self,
        completed: CompletedNavigationHistoryReset,
    ) -> Result<bool, String> {
        let (document, completion) = completed.into_parts();
        self.physical.document(document)?;
        self.physical
            .web_contents
            .get_mut(&document.web_contents().id())
            .ok_or("NoDocumentLoaded")?
            .finish_reset_navigation_history(completion?)
    }

    pub(crate) fn start_child_frame_navigation(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: String,
    ) -> Result<PendingChildFrameNavigation, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_child_frame_navigation_to_url(&frame_id, &url)
            .map_err(|error| error.to_string())?;
        Ok(PendingChildFrameNavigation::new(document, pending))
    }

    pub(crate) fn finish_child_frame_navigation(
        &mut self,
        completed: CompletedChildFrameNavigation,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_child_frame_navigation_to_url_command_turn(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_child_frame_document_resource_text_search(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingDocumentResourceTextSearch, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_child_frame_resource_search_by_lines(
                frame_id,
                url,
                query,
                case_sensitive,
                is_regex,
            )
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentResourceTextSearch::new(document, pending))
    }

    pub(crate) fn start_document_text_search(
        &self,
        document: DocumentHandle,
        text: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingDocumentResourceTextSearch, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_text_search_by_lines(text, query, case_sensitive, is_regex)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentResourceTextSearch::new(document, pending))
    }

    pub(crate) fn finish_document_resource_text_search(
        &mut self,
        completed: CompletedDocumentResourceTextSearch,
    ) -> Result<RendererResourceTextSearchOutcome, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_resource_search_by_lines(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_document_javascript_dialog_handler_enabled(
        &self,
        document: DocumentHandle,
        enabled: bool,
    ) -> Result<PendingPageCommand, String> {
        self.physical
            .document(document)?
            .page
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_document_csp_bypass_update(
        &self,
        document: DocumentHandle,
        bypass: bool,
    ) -> Result<PendingDocumentCspBypassUpdate, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_set_bypass_content_security_policy(bypass)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentCspBypassUpdate::new(document, pending))
    }

    pub(crate) fn finish_document_csp_bypass_update(
        &mut self,
        completed: CompletedDocumentCspBypassUpdate,
    ) -> Result<(), String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_set_bypass_content_security_policy(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn document_subresource_network_records(
        &self,
        document: DocumentHandle,
    ) -> Result<Vec<SubresourceNetworkRecord>, String> {
        Ok(self
            .physical
            .document(document)?
            .page
            .subresource_network_records()
            .to_vec())
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

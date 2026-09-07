use super::BrowserContext;
use crate::browser::{DocumentHandle, WebContentsHandle};
use crate::page::{
    CompletedPageCommand, PendingPageCommand, RendererCaptureScreencastFrameReply,
    RendererCaptureScreencastFrameRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererCommandTurnOutput, RendererPageDiagnosticsSnapshot,
    RendererResourceTextSearchOutcome, RendererSetDocumentContentResult, SubresourceNetworkRecord,
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
    fn renderer_output_predecessor(&self) -> Option<crate::RendererOutputFence> {
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
        pub struct $pending(PendingDocumentCommand);

        pub struct $completed(CompletedDocumentCommand);

        impl $pending {
            pub fn new(document: DocumentHandle, pending: PendingPageCommand) -> Self {
                Self(PendingDocumentCommand { document, pending })
            }

            pub async fn wait(self) -> $completed {
                $completed(self.0.wait().await)
            }
        }

        impl $completed {
            pub fn document(&self) -> DocumentHandle {
                self.0.document
            }

            pub fn into_parts(self) -> (DocumentHandle, Result<CompletedPageCommand, String>) {
                self.0.into_parts()
            }
        }
    };
    ($pending:ident, $completed:ident, renderer_output_predecessor) => {
        define_document_command!($pending, $completed);

        impl $completed {
            pub fn renderer_output_predecessor(&self) -> Option<crate::RendererOutputFence> {
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
define_document_command!(PendingDocumentInputCommand, CompletedDocumentInputCommand);
define_document_command!(
    PendingDocumentAutofillTrigger,
    CompletedDocumentAutofillTrigger
);
define_document_command!(
    PendingDocumentLifecycleStop,
    CompletedDocumentLifecycleStop,
    renderer_output_predecessor
);
define_document_command!(
    PendingDocumentDiagnosticsSnapshot,
    CompletedDocumentDiagnosticsSnapshot,
    renderer_output_predecessor
);

#[cfg(any(test, feature = "test-support"))]
impl CompletedDocumentDiagnosticsSnapshot {
    pub fn page_state_for_test(
        &self,
    ) -> Option<&std::sync::Arc<moli_renderer_v8::RendererPageState>> {
        self.0
            .completed
            .as_ref()
            .ok()
            .map(CompletedPageCommand::page_state)
    }

    pub fn into_page_completion_for_test(self) -> Result<CompletedPageCommand, String> {
        self.0.completed
    }
}
define_document_command!(
    PendingChildFrameLifecycleWork,
    CompletedChildFrameLifecycleWork
);
define_document_command!(
    PendingDocumentResourceRuntimeUpdate,
    CompletedDocumentResourceRuntimeUpdate
);

impl CompletedTopLevelHistoryTraversal {
    pub fn renderer_accepted(&self) -> Option<bool> {
        self.0
            .completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::bool_reply_value)
    }
}

pub struct DocumentSnapshot {
    pub url: String,
    pub html: String,
}

impl BrowserContext {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn apply_document_cookie_facade_overrides_for_test(
        &mut self,
        document: DocumentHandle,
        overrides: Option<moli_cookie_jar::BrowserCookieFacadeOverrides>,
    ) -> Result<(), String> {
        let page = &mut self.document_mut(document)?.page;
        match overrides {
            Some(overrides) if overrides != Default::default() => page
                .apply_document_cookie_facade_overrides_async(&overrides)
                .await
                .map_err(|error| error.to_string()),
            Some(_) | None => page
                .clear_document_cookie_facade_overrides_async()
                .await
                .map_err(|error| error.to_string()),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn document_runtime_heap_usage_for_test(
        &mut self,
        document: DocumentHandle,
    ) -> Result<crate::page::RendererRuntimeHeapUsage, String> {
        let completion = self
            .document(document)?
            .page
            .start_runtime_heap_usage()
            .map_err(|error| error.to_string())?
            .wait()
            .await
            .map_err(|error| error.to_string())?;
        self.document_mut(document)?
            .page
            .finish_runtime_heap_usage(completion)
            .map_err(|error| error.to_string())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_idle_override_for_test(
        &self,
        document: DocumentHandle,
    ) -> Result<Option<crate::page::EmulatedIdleOverride>, String> {
        Ok(self.document(document)?.page.idle_override())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn evaluate_document_expression_for_test(
        &mut self,
        document: DocumentHandle,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value, String> {
        self.document_mut(document)?
            .page
            .evaluate_runtime_expression_without_navigation_follow_with_await_async(
                expression,
                await_promise,
            )
            .await
            .map_err(|error| format!("runtime evaluation failed: {error}"))
    }

    pub fn document_handle_for_web_contents(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<DocumentHandle>, String> {
        let document = self
            .web_contents(handle)?
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.id);
        Ok(document.map(|document| DocumentHandle::new(handle, document)))
    }

    pub fn start_set_document_content(
        &self,
        document: DocumentHandle,
        frame_id: String,
        html: String,
    ) -> Result<PendingSetDocumentContent, String> {
        let pending = self
            .document(document)?
            .page
            .start_set_document_content(frame_id, html)
            .map_err(|error| error.to_string())?;
        Ok(PendingSetDocumentContent::new(document, pending))
    }

    pub fn finish_set_document_content(
        &mut self,
        completed: CompletedSetDocumentContent,
    ) -> Result<(RendererSetDocumentContentResult, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_set_document_content_command_turn(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_top_level_same_document_navigation(
        &self,
        document: DocumentHandle,
        url: String,
    ) -> Result<PendingTopLevelSameDocumentNavigation, String> {
        let pending = self
            .document(document)?
            .page
            .start_top_level_same_document_navigation(url)
            .map_err(|error| error.to_string())?;
        Ok(PendingTopLevelSameDocumentNavigation::new(
            document, pending,
        ))
    }

    pub fn finish_top_level_same_document_navigation(
        &mut self,
        completed: CompletedTopLevelSameDocumentNavigation,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_top_level_same_document_navigation_command_turn(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_capture_document_screencast_frame(
        &self,
        document: DocumentHandle,
        request: RendererCaptureScreencastFrameRequest,
    ) -> Result<PendingCaptureDocumentScreencastFrame, String> {
        let pending = self
            .document(document)?
            .page
            .start_capture_screencast_frame(request)
            .map_err(|error| error.to_string())?;
        Ok(PendingCaptureDocumentScreencastFrame::new(
            document, pending,
        ))
    }

    pub fn finish_capture_document_screencast_frame(
        &mut self,
        completed: CompletedCaptureDocumentScreencastFrame,
    ) -> Result<RendererCaptureScreencastFrameReply, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_capture_screencast_frame(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_capture_document_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingCaptureDocumentSnapshot, String> {
        let pending = self
            .document(document)?
            .page
            .start_serialize_html()
            .map_err(|error| error.to_string())?;
        Ok(PendingCaptureDocumentSnapshot::new(document, pending))
    }

    pub fn finish_capture_document_snapshot(
        &mut self,
        completed: CompletedCaptureDocumentSnapshot,
    ) -> Result<DocumentSnapshot, String> {
        let (document, completion) = completed.into_parts();
        let document = self.document_mut(document)?;
        let html = document
            .page
            .finish_serialize_html(completion?)
            .map_err(|error| error.to_string())?;
        Ok(DocumentSnapshot {
            url: document.page.final_url().as_str().to_owned(),
            html,
        })
    }

    pub fn start_capture_document_image(
        &self,
        document: DocumentHandle,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<PendingCaptureDocumentImage, String> {
        let pending = self
            .document(document)?
            .page
            .start_capture_screenshot_with_request(request)
            .map_err(|error| error.to_string())?;
        Ok(PendingCaptureDocumentImage::new(document, pending))
    }

    pub fn finish_capture_document_image(
        &mut self,
        completed: CompletedCaptureDocumentImage,
    ) -> Result<RendererCaptureScreenshotReply, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_capture_screenshot(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_top_level_history_traversal(
        &self,
        document: DocumentHandle,
        delta: i64,
    ) -> Result<PendingTopLevelHistoryTraversal, String> {
        let pending = self
            .document(document)?
            .page
            .start_top_level_history_traversal_by_delta(delta)
            .map_err(|error| error.to_string())?;
        Ok(PendingTopLevelHistoryTraversal::new(document, pending))
    }

    pub fn finish_top_level_history_traversal(
        &mut self,
        completed: CompletedTopLevelHistoryTraversal,
    ) -> Result<bool, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_top_level_history_traversal_by_delta(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_navigation_history_reset(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingNavigationHistoryReset, String> {
        self.document(document)?;
        let pending = self
            .web_contents
            .get(&document.web_contents().id())
            .ok_or("NoDocumentLoaded")?
            .start_reset_navigation_history()?;
        Ok(PendingNavigationHistoryReset::new(document, pending))
    }

    pub fn finish_navigation_history_reset(
        &mut self,
        completed: CompletedNavigationHistoryReset,
    ) -> Result<bool, String> {
        let (document, completion) = completed.into_parts();
        self.document(document)?;
        self.web_contents
            .get_mut(&document.web_contents().id())
            .ok_or("NoDocumentLoaded")?
            .finish_reset_navigation_history(completion?)
    }

    pub fn start_child_frame_navigation(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: String,
    ) -> Result<PendingChildFrameNavigation, String> {
        let pending = self
            .document(document)?
            .page
            .start_child_frame_navigation_to_url(&frame_id, &url)
            .map_err(|error| error.to_string())?;
        Ok(PendingChildFrameNavigation::new(document, pending))
    }

    pub fn finish_child_frame_navigation(
        &mut self,
        completed: CompletedChildFrameNavigation,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_child_frame_navigation_to_url_command_turn(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_child_frame_document_resource_text_search(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingDocumentResourceTextSearch, String> {
        let pending = self
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

    pub fn start_document_text_search(
        &self,
        document: DocumentHandle,
        text: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingDocumentResourceTextSearch, String> {
        let pending = self
            .document(document)?
            .page
            .start_text_search_by_lines(text, query, case_sensitive, is_regex)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentResourceTextSearch::new(document, pending))
    }

    pub fn finish_document_resource_text_search(
        &mut self,
        completed: CompletedDocumentResourceTextSearch,
    ) -> Result<RendererResourceTextSearchOutcome, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_resource_search_by_lines(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn set_document_javascript_dialog_handler_enabled(
        &self,
        document: DocumentHandle,
        enabled: bool,
    ) -> Result<(), String> {
        let pending = self
            .document(document)?
            .page
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map_err(|error| error.to_string())?;
        drop(pending);
        Ok(())
    }

    pub fn start_document_csp_bypass_update(
        &self,
        document: DocumentHandle,
        bypass: bool,
    ) -> Result<PendingDocumentCspBypassUpdate, String> {
        let pending = self
            .document(document)?
            .page
            .start_set_bypass_content_security_policy(bypass)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentCspBypassUpdate::new(document, pending))
    }

    pub fn finish_document_csp_bypass_update(
        &mut self,
        completed: CompletedDocumentCspBypassUpdate,
    ) -> Result<(), String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_set_bypass_content_security_policy(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn document_subresource_network_records(
        &self,
        document: DocumentHandle,
    ) -> Result<Vec<SubresourceNetworkRecord>, String> {
        Ok(self
            .document(document)?
            .page
            .subresource_network_records()
            .to_vec())
    }

    pub fn ensure_document_current(&self, document: DocumentHandle) -> Result<(), String> {
        self.document(document).map(|_| ())
    }

    pub fn document_observable_output_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<Vec<crate::page::ScriptObservableOutputItem>, String> {
        Ok(self
            .document(document)?
            .page
            .script_execution()
            .observable_output_items()
            .to_vec())
    }

    pub fn document_response_headers(
        &self,
        document: DocumentHandle,
    ) -> Result<&[(String, String)], String> {
        Ok(self.document(document)?.page.headers())
    }

    pub fn start_document_lifecycle_stop(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentLifecycleStop, String> {
        let pending = self
            .document(document)?
            .page
            .start_stop_document_lifecycle()
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentLifecycleStop::new(document, pending))
    }

    pub fn finish_document_lifecycle_stop(
        &mut self,
        completed: CompletedDocumentLifecycleStop,
    ) -> Result<RendererCommandTurnOutput, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_stop_document_lifecycle(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_document_diagnostics_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentDiagnosticsSnapshot, String> {
        let pending = self
            .document(document)?
            .page
            .start_page_diagnostics_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentDiagnosticsSnapshot::new(document, pending))
    }

    pub fn finish_document_diagnostics_snapshot(
        &mut self,
        completed: CompletedDocumentDiagnosticsSnapshot,
    ) -> Result<RendererPageDiagnosticsSnapshot, String> {
        let (document, completion) = completed.into_parts();
        let snapshot = self
            .document_mut(document)?
            .page
            .finish_page_diagnostics_snapshot(completion?)
            .map_err(|error| error.to_string())?;
        Ok(snapshot)
    }

    pub fn start_document_child_frame_lifecycle_work(
        &mut self,
        document: DocumentHandle,
        timeout: std::time::Duration,
    ) -> Result<PendingChildFrameLifecycleWork, String> {
        self.document(document)?;
        let storage = self.storage_partition.handles.clone();
        let pending = self
            .web_contents_mut(document.web_contents())?
            .start_child_frame_lifecycle_work(&storage, timeout)?;
        Ok(PendingChildFrameLifecycleWork::new(document, pending))
    }

    pub fn finish_document_child_frame_lifecycle_work(
        &mut self,
        completed: CompletedChildFrameLifecycleWork,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let (document, completion) = completed.into_parts();
        self.document(document)?;
        let completed = self
            .web_contents_mut(document.web_contents())?
            .complete_child_frame_lifecycle_work(completion?)?;
        Ok(completed)
    }

    pub fn crash_web_contents_renderer_from_io(
        &self,
        web_contents: WebContentsHandle,
    ) -> Result<(), String> {
        if let Some(document) = self
            .web_contents(web_contents)?
            .main_frame
            .current_document
            .as_ref()
        {
            document.page.crash_devtools_target_from_io();
        }
        Ok(())
    }
}

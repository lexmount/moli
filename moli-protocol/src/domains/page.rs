use crate::devtools_runtime::{
    DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand,
    DevToolsCaptureScreenshotResult, DevToolsCommand, DevToolsCommandContext,
    DevToolsCommandResult, DevToolsError, DevToolsErrorKind, DevToolsGetFrameTreeCommand,
    DevToolsGetFrameTreeResult, DevToolsGetFrameTreesCommand, DevToolsGetFrameTreesResult,
    DevToolsGetJavaScriptDialogCommand, DevToolsGetLayoutMetricsCommand,
    DevToolsHandleJavaScriptDialogCommand, DevToolsJavaScriptDialogResult,
    DevToolsLayoutMetricsResult, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
    DevToolsProtocol, DevToolsScreenshotClip, DevToolsSetJavaScriptDialogPromptTextCommand,
    DevToolsTargetInfo, DevToolsTargetKind, UserPromptClosedEvent,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chromiumoxide_cdp::cdp::browser_protocol::page::{
    EnableParams as EnablePageParams, HandleJavaScriptDialogParams, PrintToPdfParams,
    PrintToPdfTransferMode, SetBypassCspParams, SetDocumentContentParams,
    SetInterceptFileChooserDialogParams, SetLifecycleEventsEnabledParams as LifecycleParams,
};
use moli_core::page::{
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentOpenedSnapshot,
    ChildFrameNavigationSnapshot, ChildFrameTreeEventSnapshot, ChildFrameTreeSnapshot,
    CompletedPageCommand, Page, PendingPageCommand, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererDocumentLifecycleEvent,
    RendererDocumentLifecycleIdentity, RendererDocumentLifecycleMilestone,
    RendererDocumentLifecycleWaitOutcome, RendererDocumentLifecycleWaiter,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererLayoutMetrics,
    RendererPendingTopLevelHistoryTraversal, RendererPendingWindowOpenEvent,
    RendererScreenshotClip, RendererScreenshotFormat, RendererScreenshotPurpose,
    RendererScreenshotRegion, RendererSetDocumentContentResult,
};
use moli_core::{RendererDocumentTitleChanged, RendererRuntimeCommandCausalIdentity};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

use super::input;
use crate::conn::{
    BackgroundProtocolEvent, CapturedBody, CdpSessionRoute, CommandDispatchContext,
    CommandOwnerScope, NETWORK_ERROR_PAGE_URL, PageLifecycleEventsEnableResult,
    PageScreencastConfig, PageScreencastFormat,
};
use crate::conn::{CdpConnection, Cmd, EmulatedViewportSurface};
pub(crate) use crate::conn::{DEFAULT_LOADER_ID as LOADER_ID, monotonic_timestamp_seconds};
use crate::domains::actions::PageAction;
use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink, ProtocolOutputSlot,
};
use crate::domains::command_output::CommandOutputPlan;

mod app_manifest;
mod child_frame_activity;
mod javascript_dialog;
mod lifecycle;
mod main_document_commit;
mod navigation;
mod navigation_commit;
mod pdf;
mod popup;
mod preload;
mod prepared_navigation;
mod resource_search;
mod resource_tree;
mod termination;
#[cfg(test)]
mod tests;

/// Builds a letter-sized raster PDF using the same defaults as
/// `Page.printToPDF`.
pub fn build_default_raster_pdf(
    jpeg: &[u8],
    image_width: u32,
    image_height: u32,
) -> anyhow::Result<Vec<u8>> {
    pdf::build_raster_pdf(
        jpeg,
        image_width,
        image_height,
        &pdf::RasterPdfOptions::default(),
    )
    .map_err(|error| anyhow::anyhow!(error.message().to_owned()))
}

use child_frame_activity::PagePreparedChildFrameDocumentActivity;
pub(crate) use child_frame_activity::{
    PagePreparedChildFrameActivity, PagePreparedChildFrameTreeEvent,
};
pub(crate) use lifecycle::{
    emit_bound_renderer_document_lifecycle_background_events,
    emit_navigation_frame_commit_background_events,
    emit_navigation_frame_stop_after_download_background_events,
    emit_navigation_frame_stopped_loading_background_events,
    emit_navigation_lifecycle_init_background_events,
    emit_navigation_network_idle_background_events,
};
pub(in crate::domains) use main_document_commit::{
    MainDocumentCommitPreparedOutput, append_renderer_main_document_commit_to_output_sink,
    project_main_document_commit_async,
};
pub use navigation::BackgroundNavigationCompletion;
#[cfg(test)]
pub(crate) use navigation::emit_prepared_child_frame_tree_background_events;
pub(crate) use navigation::navigation_cookie_access_report;
pub(crate) use navigation::{
    MaterializedNavigationCompletion, complete_materialized_navigation_into_buffer_async,
    emit_prepared_child_frame_activity, push_superseded_navigation_result,
};
use prepared_navigation::{
    PagePreparedSameDocumentNavigation, PagePreparedTopLevelLocationNavigation,
};
pub(crate) use termination::{
    PageTargetTerminationKind, PageTargetTerminationOwnerAction,
    complete_page_target_termination_owner_action_async,
    fail_pending_fetch_state_background_events_async, take_pending_fetch_state,
};

const DEFAULT_PRINT_MARGIN_INCHES: f64 = 1.0 / 2.54;
const DEFAULT_PRINT_PAGE_WIDTH_INCHES: f64 = 8.5;
const DEFAULT_PRINT_PAGE_HEIGHT_INCHES: f64 = 11.0;
const CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE: &str =
    "Page.captureScreenshot is not supported: renderer screenshots are not implemented.";
const START_SCREENCAST_LAYOUT_DISABLED_MESSAGE: &str =
    "Page.startScreencast is not supported: renderer layout is disabled.";
const PRINT_TO_PDF_LAYOUT_DISABLED_MESSAGE: &str =
    "Page.printToPDF is not supported: renderer layout is disabled.";
const PRINT_TO_PDF_UNSUPPORTED_MESSAGE: &str =
    "Page.printToPDF is not supported: PDF generation is not implemented.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameTreeCommandOutputKind {
    FrameTree,
    ResourceTree,
}

pub(crate) struct PendingPageCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: Box<PendingPageCommandKind>,
}

enum PendingPageCommandKind {
    BringToFront {
        route: CdpSessionRoute,
        restore_browser_context_id: Option<String>,
    },
    AppendDefaultDocumentStartScript {
        identifier: String,
        pending: PendingPageCommand,
    },
    RemoveDocumentStartScript {
        pending: PendingPageCommand,
    },
    AddScriptToEvaluateOnNewDocument(preload::PendingAddScriptToEvaluateOnNewDocumentCommand),
    GetFrameTree {
        output_kind: FrameTreeCommandOutputKind,
        target_id: String,
        target_loader_id: String,
        target_url: String,
        target_unreachable_url: Option<String>,
        target_security_origin: String,
        target_secure_context_type: String,
        target_mime_type: String,
        pending: PendingPageCommand,
    },
    SearchInResource(resource_search::PendingSearchInResourceCommand),
    GetAppManifest(app_manifest::PendingGetAppManifestCommand),
    ResetNavigationHistory {
        pending: PendingPageCommand,
    },
    SetDocumentContent {
        pending: PendingPageCommand,
    },
    SetBypassContentSecurityPolicy {
        pending: PendingPageCommand,
    },
    SameDocumentNavigate(Box<navigation::PendingSameDocumentNavigateCommand>),
    CaptureSnapshot {
        pending: PendingPageCommand,
    },
    GetLayoutMetrics {
        pending: PendingPageCommand,
    },
    CaptureScreenshot {
        pending: PendingPageCommand,
    },
    PrintToPdf {
        pending: PendingPageCommand,
        options: pdf::RasterPdfOptions,
        transfer_mode: DevToolsPrintToPdfTransferMode,
    },
    Navigate(Box<navigation::PendingNavigateLoadCommand>),
    TraverseSameDocumentHistory(Box<navigation::PendingSameDocumentHistoryTraversalCommand>),
    ChildFrameNavigate(Box<navigation::PendingChildFrameNavigateCommand>),
    ContinueNavigationWithoutRequestPause(
        Box<navigation::PendingContinueNavigationWithoutRequestPauseCommand>,
    ),
    StopLoading,
    Crash,
    Close,
    CreateIsolatedWorld(preload::PendingCreateIsolatedWorldCommand),
}

pub(crate) struct CompletedPageCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: Box<CompletedPageCommandKind>,
}

enum CompletedPageCommandKind {
    BringToFront {
        route: CdpSessionRoute,
        restore_browser_context_id: Option<String>,
    },
    AppendDefaultDocumentStartScript {
        identifier: String,
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    RemoveDocumentStartScript {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    AddScriptToEvaluateOnNewDocument(preload::CompletedAddScriptToEvaluateOnNewDocumentCommand),
    GetFrameTree {
        output_kind: FrameTreeCommandOutputKind,
        target_id: String,
        target_loader_id: String,
        target_url: String,
        target_unreachable_url: Option<String>,
        target_security_origin: String,
        target_secure_context_type: String,
        target_mime_type: String,
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    SearchInResource(Box<resource_search::CompletedSearchInResourceCommand>),
    GetAppManifest(Box<app_manifest::CompletedGetAppManifestCommand>),
    ResetNavigationHistory {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    SetDocumentContent {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    SetBypassContentSecurityPolicy {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    SameDocumentNavigate(Box<navigation::CompletedSameDocumentNavigateCommand>),
    CaptureSnapshot {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    GetLayoutMetrics {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    CaptureScreenshot {
        completed: Box<Result<CompletedPageCommand, String>>,
    },
    PrintToPdf {
        completed: Box<Result<CompletedPageCommand, String>>,
        options: pdf::RasterPdfOptions,
        transfer_mode: DevToolsPrintToPdfTransferMode,
    },
    Navigate(Box<navigation::CompletedNavigateLoadCommand>),
    TraverseSameDocumentHistory(Box<navigation::CompletedSameDocumentHistoryTraversalCommand>),
    ChildFrameNavigate(Box<navigation::CompletedChildFrameNavigateCommand>),
    ContinueNavigationWithoutRequestPause(
        Box<navigation::CompletedContinueNavigationWithoutRequestPauseCommand>,
    ),
    StopLoading,
    Crash,
    Close,
    CreateIsolatedWorld(Box<preload::CompletedCreateIsolatedWorldCommand>),
}

impl CompletedPageCommandKind {
    fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        fn direct(
            completed: &Result<CompletedPageCommand, String>,
        ) -> Option<moli_core::RendererOutputFence> {
            completed
                .as_ref()
                .ok()
                .and_then(CompletedPageCommand::renderer_output_predecessor)
        }

        match self {
            Self::AppendDefaultDocumentStartScript { completed, .. }
            | Self::RemoveDocumentStartScript { completed }
            | Self::GetFrameTree { completed, .. }
            | Self::ResetNavigationHistory { completed }
            | Self::SetDocumentContent { completed }
            | Self::SetBypassContentSecurityPolicy { completed }
            | Self::CaptureSnapshot { completed }
            | Self::GetLayoutMetrics { completed }
            | Self::CaptureScreenshot { completed }
            | Self::PrintToPdf { completed, .. } => direct(completed),
            Self::SearchInResource(completed) => completed.renderer_output_predecessor(),
            Self::GetAppManifest(completed) => completed.renderer_output_predecessor(),
            Self::SameDocumentNavigate(completed) => completed.renderer_output_predecessor(),
            Self::TraverseSameDocumentHistory(completed) => completed.renderer_output_predecessor(),
            Self::ChildFrameNavigate(completed) => completed.renderer_output_predecessor(),
            Self::BringToFront { .. }
            | Self::AddScriptToEvaluateOnNewDocument(_)
            | Self::Navigate(_)
            | Self::ContinueNavigationWithoutRequestPause(_)
            | Self::StopLoading
            | Self::Crash
            | Self::Close
            // createIsolatedWorld may restart on a replacement renderer attachment. Its
            // completion handler records the fence only after rejecting a stale completion,
            // so an abandoned stream cannot become the predecessor of the final response.
            | Self::CreateIsolatedWorld(_) => None,
        }
    }
}

pub(crate) enum PageCommandTaskStep {
    Pending(PendingPageCommandDispatch),
    Complete(CommandOutputPlan),
}

impl PendingPageCommandDispatch {
    pub async fn wait(self) -> CompletedPageCommandDispatch {
        let kind = match *self.kind {
            PendingPageCommandKind::BringToFront {
                route,
                restore_browser_context_id,
            } => CompletedPageCommandKind::BringToFront {
                route,
                restore_browser_context_id,
            },
            PendingPageCommandKind::AppendDefaultDocumentStartScript {
                identifier,
                pending,
            } => CompletedPageCommandKind::AppendDefaultDocumentStartScript {
                identifier,
                completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
            },
            PendingPageCommandKind::RemoveDocumentStartScript { pending } => {
                CompletedPageCommandKind::RemoveDocumentStartScript {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::AddScriptToEvaluateOnNewDocument(pending) => {
                CompletedPageCommandKind::AddScriptToEvaluateOnNewDocument(pending.wait().await)
            }
            PendingPageCommandKind::GetFrameTree {
                output_kind,
                target_id,
                target_loader_id,
                target_url,
                target_unreachable_url,
                target_security_origin,
                target_secure_context_type,
                target_mime_type,
                pending,
            } => CompletedPageCommandKind::GetFrameTree {
                output_kind,
                target_id,
                target_loader_id,
                target_url,
                target_unreachable_url,
                target_security_origin,
                target_secure_context_type,
                target_mime_type,
                completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
            },
            PendingPageCommandKind::SearchInResource(pending) => {
                CompletedPageCommandKind::SearchInResource(Box::new(pending.wait().await))
            }
            PendingPageCommandKind::GetAppManifest(pending) => {
                CompletedPageCommandKind::GetAppManifest(Box::new(pending.wait().await))
            }
            PendingPageCommandKind::ResetNavigationHistory { pending } => {
                CompletedPageCommandKind::ResetNavigationHistory {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::SetDocumentContent { pending } => {
                CompletedPageCommandKind::SetDocumentContent {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::SetBypassContentSecurityPolicy { pending } => {
                CompletedPageCommandKind::SetBypassContentSecurityPolicy {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::SameDocumentNavigate(pending) => {
                CompletedPageCommandKind::SameDocumentNavigate(Box::new(pending.wait().await))
            }
            PendingPageCommandKind::CaptureSnapshot { pending } => {
                CompletedPageCommandKind::CaptureSnapshot {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::GetLayoutMetrics { pending } => {
                CompletedPageCommandKind::GetLayoutMetrics {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::CaptureScreenshot { pending } => {
                CompletedPageCommandKind::CaptureScreenshot {
                    completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                }
            }
            PendingPageCommandKind::PrintToPdf {
                pending,
                options,
                transfer_mode,
            } => CompletedPageCommandKind::PrintToPdf {
                completed: Box::new(pending.wait().await.map_err(|error| error.to_string())),
                options,
                transfer_mode,
            },
            PendingPageCommandKind::Navigate(pending) => {
                CompletedPageCommandKind::Navigate(Box::new(pending.wait().await))
            }
            PendingPageCommandKind::TraverseSameDocumentHistory(pending) => {
                CompletedPageCommandKind::TraverseSameDocumentHistory(Box::new(
                    pending.wait().await,
                ))
            }
            PendingPageCommandKind::ChildFrameNavigate(pending) => {
                CompletedPageCommandKind::ChildFrameNavigate(Box::new(pending.wait().await))
            }
            PendingPageCommandKind::ContinueNavigationWithoutRequestPause(pending) => {
                CompletedPageCommandKind::ContinueNavigationWithoutRequestPause(Box::new(
                    pending.wait().await,
                ))
            }
            PendingPageCommandKind::StopLoading => CompletedPageCommandKind::StopLoading,
            PendingPageCommandKind::Crash => CompletedPageCommandKind::Crash,
            PendingPageCommandKind::Close => CompletedPageCommandKind::Close,
            PendingPageCommandKind::CreateIsolatedWorld(pending) => {
                CompletedPageCommandKind::CreateIsolatedWorld(Box::new(pending.wait().await))
            }
        };
        CompletedPageCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: Box::new(kind),
        }
    }
}

impl CompletedPageCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<PageAction>() {
        Some(PageAction::Enable) => enable_page_command(conn, cmd),
        Some(PageAction::Disable) => disable_page_command(conn, cmd),
        Some(PageAction::SetLifecycleEventsEnabled) => {
            set_lifecycle_events_enabled_command(conn, cmd)
        }
        Some(PageAction::SetFontFamilies) => set_font_families_command(conn, cmd),
        Some(PageAction::SetInterceptFileChooserDialog) => {
            set_intercept_file_chooser_dialog_command(conn, cmd)
        }
        Some(PageAction::HandleJavaScriptDialog) => handle_javascript_dialog_command(conn, cmd),
        _ => CommandOutputPlan::error(-32601, "Unknown Page command-output method"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageOutputProjectionStep {
    Download,
    FileChooser,
    JavascriptDialog,
    WindowOpen,
    Popup,
    DocumentTitleChanged,
    DocumentLifecycle,
    ChildFrameActivity,
    SameDocumentNavigation,
    TopLevelLocationNavigation,
    TopLevelHistoryTraversal,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct PagePreparedOutputs {
    javascript_dialogs: Vec<javascript_dialog::PreparedJavaScriptDialog>,
    window_open_events: Vec<popup::PagePreparedWindowOpenEvent>,
    popup_activations: Vec<popup::PagePreparedPopupActivation>,
    document_title_changes: Vec<RendererDocumentTitleChanged>,
    document_lifecycle_events: Vec<RendererDocumentLifecycleEvent>,
    child_frame_activities: Vec<PagePreparedChildFrameActivity>,
    same_document_navigations: Vec<PagePreparedSameDocumentNavigation>,
    top_level_location_navigation: Option<PagePreparedTopLevelLocationNavigation>,
    top_level_history_traversal: Option<RendererPendingTopLevelHistoryTraversal>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct PagePreparedOutputSlot {
    outputs: PagePreparedOutputs,
}

impl PagePreparedOutputs {
    pub(crate) fn from_renderer_javascript_dialog(
        conn: &CdpConnection,
        session_id: Option<&str>,
        dialog: moli_core::page::RendererPendingJavaScriptDialog,
    ) -> Self {
        let Some(source_attachment) =
            conn.target_page_protocol_attachment_identity_for_session(session_id)
        else {
            let _ = dialog.finish(false, String::new());
            return Self::default();
        };
        let Some((root_frame_id, _, _, _)) =
            conn.target_session_owner_frame_tree_identity(session_id)
        else {
            let _ = dialog.finish(false, String::new());
            return Self::default();
        };
        let Ok(runtime_slot) = conn.runtime_session_owner_slot(session_id) else {
            let _ = dialog.finish(false, String::new());
            return Self::default();
        };
        Self {
            javascript_dialogs: vec![crate::conn::TargetPreparedJavaScriptDialog::capture(
                source_attachment,
                runtime_slot.javascript_dialog_scope_observer(),
                &root_frame_id,
                dialog,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_popup_activation(
        conn: &CdpConnection,
        session_id: Option<&str>,
        activation: moli_core::page::RendererPendingPopupActivation,
    ) -> Self {
        let Some(page_owner) = conn.target_page_residence_identity_for_session(session_id) else {
            return Self::default();
        };
        Self {
            popup_activations: vec![popup::PagePreparedPopupActivation::new(
                page_owner, activation,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_window_open_event(
        conn: &CdpConnection,
        session_id: Option<&str>,
        event: RendererPendingWindowOpenEvent,
    ) -> Self {
        Self {
            window_open_events: vec![popup::PagePreparedWindowOpenEvent::new(
                conn.subscribed_page_event_session_ids_for_session_owner(session_id),
                event,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_same_document_navigation(
        conn: &CdpConnection,
        session_id: Option<&str>,
        navigation: RendererDocumentSourcedSameDocumentNavigation,
    ) -> Self {
        let Some(owner) = conn.target_page_residence_identity_for_session(session_id) else {
            return Self::default();
        };
        Self {
            same_document_navigations: vec![PagePreparedSameDocumentNavigation::new(
                owner, navigation,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_top_level_location_navigation(
        conn: &CdpConnection,
        session_id: Option<&str>,
        navigation: RendererDocumentSourcedTopLevelLocationNavigation,
    ) -> Self {
        let Some(owner) = conn.target_page_residence_identity_for_session(session_id) else {
            return Self::default();
        };
        Self {
            top_level_location_navigation: Some(PagePreparedTopLevelLocationNavigation::new(
                owner, navigation,
            )),
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_top_level_history_traversal(
        traversal: RendererPendingTopLevelHistoryTraversal,
    ) -> Self {
        Self {
            top_level_history_traversal: Some(traversal),
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_tree_event(
        conn: &CdpConnection,
        session_id: Option<&str>,
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameTreeEventSnapshot,
    ) -> Self {
        let Some(binding) = conn.target_root_document_protocol_attachment_identity_for_session(
            session_id,
            source_document,
        ) else {
            return Self::default();
        };
        let Some((root_frame_id, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity(session_id)
        else {
            return Self::default();
        };
        let event = match event {
            ChildFrameTreeEventSnapshot::Attached(attachment) => {
                PagePreparedChildFrameTreeEvent::Attached {
                    frame_id: attachment.frame_id,
                    parent_frame_id: attachment.parent_frame_id.unwrap_or(root_frame_id),
                }
            }
            ChildFrameTreeEventSnapshot::Detached(detachment) => {
                PagePreparedChildFrameTreeEvent::Detached {
                    frame_id: detachment.frame_id,
                }
            }
        };
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            vec![event],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_document_opened(
        conn: &CdpConnection,
        session_id: Option<&str>,
        source_document: RendererDocumentLifecycleIdentity,
        mut event: ChildFrameDocumentOpenedSnapshot,
    ) -> Self {
        let Some(binding) = conn.target_root_document_protocol_attachment_identity_for_session(
            session_id,
            source_document,
        ) else {
            return Self::default();
        };
        let Some((root_frame_id, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity(session_id)
        else {
            return Self::default();
        };
        if event.parent_frame_id.is_none() {
            event.parent_frame_id = Some(root_frame_id);
        }
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            Vec::new(),
            vec![event],
            Vec::new(),
            Vec::new(),
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_document_network(
        conn: &CdpConnection,
        session_id: Option<&str>,
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameDocumentNetworkActivitySnapshot,
    ) -> Self {
        let Some(binding) = conn.target_root_document_protocol_attachment_identity_for_session(
            session_id,
            source_document,
        ) else {
            return Self::default();
        };
        let Some((_, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity(session_id)
        else {
            return Self::default();
        };
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            Vec::new(),
            Vec::new(),
            vec![event],
            Vec::new(),
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_load(
        conn: &CdpConnection,
        session_id: Option<&str>,
        source_document: RendererDocumentLifecycleIdentity,
        mut event: ChildFrameNavigationSnapshot,
    ) -> Self {
        let Some(binding) = conn.target_root_document_protocol_attachment_identity_for_session(
            session_id,
            source_document,
        ) else {
            return Self::default();
        };
        let Some((root_frame_id, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity(session_id)
        else {
            return Self::default();
        };
        if event.parent_frame_id.is_none() {
            event.parent_frame_id = Some(root_frame_id);
        }
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![event],
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    fn has_child_frame_activity(&self) -> bool {
        !self.child_frame_activities.is_empty()
    }

    fn push_child_frame_activity(&mut self, activity: PagePreparedChildFrameActivity) {
        self.child_frame_activities.push(activity);
    }

    pub(crate) fn from_renderer_document_lifecycle_event(
        event: moli_core::page::RendererDocumentLifecycleEvent,
    ) -> Self {
        Self {
            document_lifecycle_events: vec![event],
            ..Default::default()
        }
    }

    pub(crate) fn from_renderer_document_title_change(
        change: RendererDocumentTitleChanged,
    ) -> Self {
        Self {
            document_title_changes: vec![change],
            ..Default::default()
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.javascript_dialogs.extend(other.javascript_dialogs);
        self.window_open_events.extend(other.window_open_events);
        self.popup_activations.extend(other.popup_activations);
        self.document_title_changes
            .extend(other.document_title_changes);
        self.document_lifecycle_events
            .extend(other.document_lifecycle_events);
        for activity in other.child_frame_activities {
            self.push_child_frame_activity(activity);
        }
        self.same_document_navigations
            .extend(other.same_document_navigations);
        if self.top_level_location_navigation.is_none() {
            self.top_level_location_navigation = other.top_level_location_navigation;
        }
        if self.top_level_history_traversal.is_none() {
            self.top_level_history_traversal = other.top_level_history_traversal;
        }
    }

    pub(in crate::domains) fn append_to_javascript_dialog_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.javascript_dialogs.is_empty() {
            sink.push_produced_slot(SLOT_JAVASCRIPT_DIALOG);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_popup_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.popup_activations.is_empty() {
            sink.push_produced_slot(SLOT_POPUP);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_window_open_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.window_open_events.is_empty() {
            sink.push_produced_slot(SLOT_WINDOW_OPEN);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_document_lifecycle_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.document_lifecycle_events.is_empty() {
            sink.push_produced_slot(SLOT_DOCUMENT_LIFECYCLE);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_document_title_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.document_title_changes.is_empty() {
            sink.push_produced_slot(SLOT_DOCUMENT_TITLE_CHANGED);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_child_frame_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.has_child_frame_activity() {
            sink.push_produced_slot(SLOT_CHILD_FRAME_ACTIVITY);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_same_document_navigation_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.same_document_navigations.is_empty() {
            sink.push_produced_slot(SLOT_SAME_DOCUMENT_NAVIGATION);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_top_level_location_navigation_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.top_level_location_navigation.is_some() {
            sink.push_produced_slot(SLOT_TOP_LEVEL_LOCATION_NAVIGATION);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_top_level_history_traversal_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.top_level_history_traversal.is_some() {
            sink.push_produced_slot(SLOT_TOP_LEVEL_HISTORY_TRAVERSAL);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    #[cfg(test)]
    pub(crate) fn from_javascript_dialogs_for_test(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        source_session_id: Option<&str>,
        dialog_scope: crate::conn::TargetJavaScriptDialogScopeObserver,
        root_frame_id: &str,
        dialogs: Vec<moli_core::page::RendererPendingJavaScriptDialog>,
    ) -> Self {
        Self {
            javascript_dialogs: dialogs
                .into_iter()
                .map(|dialog| {
                    javascript_dialog::capture_for_test(
                        page_owner.clone(),
                        source_session_id,
                        dialog_scope.clone(),
                        root_frame_id,
                        dialog,
                    )
                })
                .collect(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_popup_activations_for_test(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        activations: Vec<moli_core::page::RendererPendingPopupActivation>,
    ) -> Self {
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: activations
                .into_iter()
                .map(|activation| {
                    popup::PagePreparedPopupActivation::from_renderer_for_test(
                        page_owner.clone(),
                        activation,
                    )
                })
                .collect(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    fn child_frame_document_activity_for_test() -> PagePreparedChildFrameDocumentActivity {
        PagePreparedChildFrameDocumentActivity::from_parts(
            12.5,
            vec![PagePreparedChildFrameTreeEvent::Attached {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: "TID-1".to_owned(),
            }],
            Vec::new(),
            Vec::new(),
            vec![ChildFrameNavigationSnapshot {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: None,
                loader_id: Some("LOADER-CHILD-FRAME-1".to_owned()),
                name: Some("child-frame".to_owned()),
                url: "https://example.test/child".to_owned(),
                document_open_replacement: false,
                security_origin_inherited: false,
                security_origin_opaque: false,
                document_network: None,
            }],
            "https://example.test".to_owned(),
            "Secure".to_owned(),
        )
    }

    #[cfg(test)]
    pub(crate) fn from_child_frame_activity_for_test(
        binding: crate::conn::TargetRootDocumentProtocolAttachmentIdentity,
    ) -> Self {
        let activity = PagePreparedChildFrameActivity::from_document(
            binding,
            Self::child_frame_document_activity_for_test(),
        );
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: vec![activity],
            same_document_navigations: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_same_document_navigations_for_test(
        owner: crate::conn::TargetPageResidenceIdentity,
        navigations: Vec<RendererDocumentSourcedSameDocumentNavigation>,
    ) -> Self {
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: navigations
                .into_iter()
                .map(|navigation| {
                    PagePreparedSameDocumentNavigation::new(owner.clone(), navigation)
                })
                .collect(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_top_level_location_navigation_for_test(
        owner: crate::conn::TargetPageResidenceIdentity,
        navigation: Option<RendererDocumentSourcedTopLevelLocationNavigation>,
    ) -> Self {
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: Vec::new(),
            top_level_location_navigation: navigation
                .map(|navigation| PagePreparedTopLevelLocationNavigation::new(owner, navigation)),
            top_level_history_traversal: None,
        }
    }
}

impl PagePreparedOutputSlot {
    pub(crate) fn from_outputs(outputs: PagePreparedOutputs) -> Self {
        Self { outputs }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    fn take_javascript_dialogs(
        &mut self,
    ) -> Option<Vec<javascript_dialog::PreparedJavaScriptDialog>> {
        (!self.outputs.javascript_dialogs.is_empty())
            .then(|| std::mem::take(&mut self.outputs.javascript_dialogs))
    }

    pub(crate) fn take_popup_activations(
        &mut self,
    ) -> Option<Vec<popup::PagePreparedPopupActivation>> {
        (!self.outputs.popup_activations.is_empty())
            .then(|| std::mem::take(&mut self.outputs.popup_activations))
    }

    pub(crate) fn take_window_open_events(
        &mut self,
    ) -> Option<Vec<popup::PagePreparedWindowOpenEvent>> {
        (!self.outputs.window_open_events.is_empty())
            .then(|| std::mem::take(&mut self.outputs.window_open_events))
    }

    pub(crate) fn take_document_lifecycle_events(
        &mut self,
    ) -> Option<Vec<RendererDocumentLifecycleEvent>> {
        (!self.outputs.document_lifecycle_events.is_empty())
            .then(|| std::mem::take(&mut self.outputs.document_lifecycle_events))
    }

    fn take_document_title_changes(&mut self) -> Option<Vec<RendererDocumentTitleChanged>> {
        (!self.outputs.document_title_changes.is_empty())
            .then(|| std::mem::take(&mut self.outputs.document_title_changes))
    }

    pub(crate) fn take_child_frame_activity(
        &mut self,
    ) -> Option<Vec<PagePreparedChildFrameActivity>> {
        (!self.outputs.child_frame_activities.is_empty())
            .then(|| std::mem::take(&mut self.outputs.child_frame_activities))
    }

    fn take_same_document_navigations(
        &mut self,
    ) -> Option<Vec<PagePreparedSameDocumentNavigation>> {
        (!self.outputs.same_document_navigations.is_empty())
            .then(|| std::mem::take(&mut self.outputs.same_document_navigations))
    }

    fn take_top_level_location_navigation(
        &mut self,
    ) -> Option<PagePreparedTopLevelLocationNavigation> {
        self.outputs.top_level_location_navigation.take()
    }

    pub(in crate::domains) fn top_level_location_navigation_runtime_command_cause(
        &self,
    ) -> Option<&RendererRuntimeCommandCausalIdentity> {
        self.outputs
            .top_level_location_navigation
            .as_ref()
            .and_then(PagePreparedTopLevelLocationNavigation::runtime_command_cause)
    }

    pub(in crate::domains) fn take_top_level_location_navigation_for_runtime_command(
        &mut self,
        cause: &RendererRuntimeCommandCausalIdentity,
    ) -> Option<Self> {
        let navigation = (self.top_level_location_navigation_runtime_command_cause()
            == Some(cause))
        .then(|| self.outputs.top_level_location_navigation.take())
        .flatten()?;
        Some(Self {
            outputs: PagePreparedOutputs {
                top_level_location_navigation: Some(navigation),
                ..Default::default()
            },
        })
    }

    pub(crate) fn take_top_level_history_traversal(
        &mut self,
    ) -> Option<RendererPendingTopLevelHistoryTraversal> {
        self.outputs.top_level_history_traversal.take()
    }
}

pub(in crate::domains) const SLOT_DOWNLOAD: ProtocolOutputSlot = ProtocolOutputSlot::Download;
pub(in crate::domains) const SLOT_FILE_CHOOSER: ProtocolOutputSlot =
    ProtocolOutputSlot::FileChooser;

// The renderer has already opened the dialog before publishing this concrete
// record. Chromium flushes the corresponding Inspector notification before
// the Runtime response; that response order now lives on the closed enum.
pub(in crate::domains) const SLOT_JAVASCRIPT_DIALOG: ProtocolOutputSlot =
    ProtocolOutputSlot::JavascriptDialog;
pub(in crate::domains) const SLOT_WINDOW_OPEN: ProtocolOutputSlot = ProtocolOutputSlot::WindowOpen;

// Creating the auxiliary browsing context is part of `window.open()`, not
// follow-up work owned by the command response.
pub(in crate::domains) const SLOT_POPUP: ProtocolOutputSlot = ProtocolOutputSlot::Popup;
pub(in crate::domains) const SLOT_DOCUMENT_LIFECYCLE: ProtocolOutputSlot =
    ProtocolOutputSlot::DocumentLifecycle;
pub(in crate::domains) const SLOT_DOCUMENT_TITLE_CHANGED: ProtocolOutputSlot =
    ProtocolOutputSlot::DocumentTitleChanged;
pub(in crate::domains) const SLOT_CHILD_FRAME_ACTIVITY: ProtocolOutputSlot =
    ProtocolOutputSlot::ChildFrameActivity;

// The history mutation has already completed when this fact is captured.
// Chromium publishes Page.navigatedWithinDocument before both synchronous and
// awaited Runtime command responses caused by that mutation, so this event
// must not be held behind the command response barrier.
pub(in crate::domains) const SLOT_SAME_DOCUMENT_NAVIGATION: ProtocolOutputSlot =
    ProtocolOutputSlot::SameDocumentNavigation;
pub(in crate::domains) const SLOT_TOP_LEVEL_LOCATION_NAVIGATION: ProtocolOutputSlot =
    ProtocolOutputSlot::TopLevelLocationNavigation;
pub(in crate::domains) const SLOT_TOP_LEVEL_HISTORY_TRAVERSAL: ProtocolOutputSlot =
    ProtocolOutputSlot::TopLevelHistoryTraversal;

impl PageOutputProjectionStep {
    async fn project_async(
        self,
        conn: &mut CdpConnection,
        context: &mut ProtocolOutputProjectionContext<'_>,
        prepared_outputs: Option<&mut ProtocolOutputPayloads>,
    ) {
        match self {
            PageOutputProjectionStep::Download => {
                let mut events = Vec::new();
                input::emit_download_activity_background_events_async(
                    conn,
                    &mut events,
                    context.session_id,
                    prepared_outputs,
                    context.command,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::DocumentLifecycle => {
                if let Some(renderer_events) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_document_lifecycle_events)
                {
                    let (binding, accepted) = conn
                        .ingest_renderer_document_lifecycle_events_for_session_owner(
                            context.session_id,
                            renderer_events,
                        );
                    if let Some(binding) = binding {
                        let mut events = Vec::new();
                        emit_bound_renderer_document_lifecycle_background_events(
                            conn,
                            &mut events,
                            context.session_id,
                            &binding,
                            &accepted,
                        );
                        context.command.protocol_events_mut().extend(events);
                    }
                }
            }
            PageOutputProjectionStep::DocumentTitleChanged => {
                if let Some(changes) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_document_title_changes)
                {
                    let mut events = Vec::new();
                    for change in changes {
                        if conn
                            .apply_renderer_document_title_for_session_owner(
                                context.session_id,
                                &change,
                            )
                            .unwrap_or(false)
                        {
                            crate::domains::target::emit_target_info_changed_for_session_owner_background_event(
                                conn,
                                &mut events,
                                context.session_id,
                            );
                        }
                    }
                    context.command.protocol_events_mut().extend(events);
                }
            }
            PageOutputProjectionStep::FileChooser => {
                let mut events = Vec::new();
                input::emit_file_chooser_activity_background_events_async(
                    conn,
                    &mut events,
                    context.session_id,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::JavascriptDialog => {
                let mut events = Vec::new();
                emit_javascript_dialog_activity_background_events_async(
                    conn,
                    &mut events,
                    context.session_id,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::WindowOpen => {
                if let Some(events) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_window_open_events)
                {
                    let mut protocol_events = Vec::new();
                    popup::emit_window_open_events(&mut protocol_events, events);
                    context
                        .command
                        .protocol_events_mut()
                        .extend(protocol_events);
                }
            }
            PageOutputProjectionStep::Popup => {
                let mut events = Vec::new();
                emit_popup_activity_background_events_async(
                    conn,
                    &mut events,
                    context.session_id,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::ChildFrameActivity => {
                if let Some(activities) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_child_frame_activity)
                {
                    let mut events = Vec::new();
                    for activity in activities {
                        emit_prepared_child_frame_activity(conn, &mut events, activity, None).await;
                    }
                    context.command.protocol_events_mut().extend(events);
                }
            }
            PageOutputProjectionStep::SameDocumentNavigation => {
                let mut events = Vec::new();
                emit_same_document_navigation_activity_background_events_async(
                    conn,
                    &mut events,
                    context.session_id,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::TopLevelLocationNavigation => {
                publish_prepared_top_level_location_navigation_owner_action(
                    conn,
                    context.session_id,
                    prepared_outputs,
                );
            }
            PageOutputProjectionStep::TopLevelHistoryTraversal => {
                let mut events = Vec::new();
                emit_top_level_history_traversal_activity_background_events_async(
                    conn,
                    &mut events,
                    context.session_id,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
        }
    }
}

pub(in crate::domains) async fn project_page_output_async(
    output: ProtocolOutputSlot,
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    let step = match output {
        ProtocolOutputSlot::Download => PageOutputProjectionStep::Download,
        ProtocolOutputSlot::FileChooser => PageOutputProjectionStep::FileChooser,
        ProtocolOutputSlot::JavascriptDialog => PageOutputProjectionStep::JavascriptDialog,
        ProtocolOutputSlot::WindowOpen => PageOutputProjectionStep::WindowOpen,
        ProtocolOutputSlot::Popup => PageOutputProjectionStep::Popup,
        ProtocolOutputSlot::DocumentTitleChanged => PageOutputProjectionStep::DocumentTitleChanged,
        ProtocolOutputSlot::DocumentLifecycle => PageOutputProjectionStep::DocumentLifecycle,
        ProtocolOutputSlot::ChildFrameActivity => PageOutputProjectionStep::ChildFrameActivity,
        ProtocolOutputSlot::SameDocumentNavigation => {
            PageOutputProjectionStep::SameDocumentNavigation
        }
        ProtocolOutputSlot::TopLevelLocationNavigation => {
            PageOutputProjectionStep::TopLevelLocationNavigation
        }
        ProtocolOutputSlot::TopLevelHistoryTraversal => {
            PageOutputProjectionStep::TopLevelHistoryTraversal
        }
        _ => panic!("non-Page output routed through the Page projector: {output:?}"),
    };
    step.project_async(conn, context, prepared_outputs).await;
}

fn start_bring_to_front_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> PageCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::success());
    };
    let Some(route) = conn.session_route(Some(session_id)) else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    PageCommandTaskStep::Pending(PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        kind: Box::new(PendingPageCommandKind::BringToFront {
            route,
            restore_browser_context_id: conn.browser_context.as_ref().map(|bc| bc.id.clone()),
        }),
    })
}

async fn bring_session_route_to_front_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
) -> Result<(), String> {
    let (browser_context_id, target_id) = match route {
        CdpSessionRoute::Browser => return Ok(()),
        CdpSessionRoute::ActiveTarget {
            browser_context_id, ..
        } => {
            if !conn
                .activate_browser_context_by_id_async(&browser_context_id)
                .await
            {
                return Err("BrowserContextNotLoaded".into());
            }
            return Ok(());
        }
        CdpSessionRoute::AuxiliaryTarget {
            browser_context_id,
            target_id,
        }
        | CdpSessionRoute::BackgroundTarget {
            browser_context_id,
            target_id,
        } => (browser_context_id, target_id),
        CdpSessionRoute::TabTarget { .. }
        | CdpSessionRoute::SharedWorkerTarget { .. }
        | CdpSessionRoute::DedicatedWorkerTarget { .. }
        | CdpSessionRoute::ServiceWorkerTarget { .. } => {
            return Err("UnsupportedTargetType".into());
        }
    };

    if !conn
        .activate_browser_context_by_id_async(&browser_context_id)
        .await
    {
        return Err("BrowserContextNotLoaded".into());
    }

    let target_is_active = conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_target_identity())
        .is_some_and(|(active_target_id, _)| active_target_id == target_id);
    if target_is_active {
        return Ok(());
    }

    match conn
        .promote_background_target_to_active_for_connection_async(&target_id)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err("UnknownTargetId".into()),
        Err(message) => Err(message),
    }
}

async fn restore_page_bring_to_front_context_async(
    conn: &mut CdpConnection,
    browser_context_id: Option<&str>,
) {
    if let Some(browser_context_id) = browser_context_id
        && conn.has_browser_context_id(browser_context_id)
        && conn
            .browser_context
            .as_ref()
            .is_none_or(|bc| bc.id != browser_context_id)
    {
        let _ = conn
            .activate_browser_context_by_id_async(browser_context_id)
            .await;
    }
}

fn enable_page_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: EnablePageParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => EnablePageParams::default(),
        Err(error) => return CommandOutputPlan::error(-32602, error),
    };
    if !conn.set_page_domain_enabled_for_session_owner(cmd.session_id, true) {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    if let Err(error) = start_set_javascript_dialog_handler_enabled(conn, cmd.session_id, true) {
        return CommandOutputPlan::error(
            -32000,
            format!("failed to update JavaScript dialog handling: {error}"),
        );
    }
    if let Some(enabled) = params.enable_file_chooser_opened_event
        && !conn
            .set_page_file_chooser_opened_event_enabled_for_session_owner(cmd.session_id, enabled)
    {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    CommandOutputPlan::success()
}

fn disable_page_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if !conn.disable_page_domain_for_session_owner(cmd.session_id) {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    let dialog_handler_enabled = conn
        .navigation_load_inputs_for_session_owner(cmd.session_id)
        .renderer_runtime
        .runtime()
        .javascript_dialog_handler_enabled();
    if let Err(error) =
        start_set_javascript_dialog_handler_enabled(conn, cmd.session_id, dialog_handler_enabled)
    {
        return CommandOutputPlan::error(
            -32000,
            format!("failed to update JavaScript dialog handling: {error}"),
        );
    }
    CommandOutputPlan::success()
}

fn start_set_javascript_dialog_handler_enabled(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    enabled: bool,
) -> Result<(), String> {
    if let Ok(slot) = conn.runtime_session_owner_slot_mut(session_id)
        && let Some(page) = slot.loaded_page_mut()
    {
        return page
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    if let Some(page) = conn
        .browser_context
        .as_mut()
        .and_then(|browser_context| browser_context.active_target.runtime_slot.loaded_page_mut())
    {
        return page
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    Ok(())
}

fn set_lifecycle_events_enabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: LifecycleParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    let was_enabled = conn
        .target_page_session_state_for_session(cmd.session_id)
        .is_some_and(|state| state.page_lifecycle_events);
    let mut plan = CommandOutputPlan::default();
    match conn.set_page_lifecycle_events_enabled_for_session_owner(cmd.session_id, params.enabled) {
        PageLifecycleEventsEnableResult::Handled {
            replay_target: Some(target),
        } => {
            // Chromium may already have committed the requested target URL
            // while its renderer is waiting for Runtime.runIfWaitingForDebugger.
            // Moli currently retains a materialized initial about:blank until
            // that release. Do not expose its completed lifecycle as if it
            // belonged to the requested URL: clients commonly wait for the
            // first replayed `load` before reading the new frame tree.
            let current_document_has_replacement_url = conn
                .runtime_session_owner_initial_empty_document_has_replacement_url(cmd.session_id);
            if params.enabled
                && !was_enabled
                && !current_document_has_replacement_url
                && let Some((binding, snapshot)) =
                    conn.renderer_document_lifecycle_visible_state_for_session_owner(cmd.session_id)
            {
                let milestones = [
                    (
                        "DOMContentLoaded",
                        RendererDocumentLifecycleMilestone::DomContentLoaded,
                    ),
                    ("load", RendererDocumentLifecycleMilestone::Load),
                ];
                for (name, milestone) in milestones {
                    let RendererDocumentLifecycleWaitOutcome::Reached(stamp) =
                        RendererDocumentLifecycleWaiter::from_snapshot(snapshot, milestone)
                            .outcome()
                    else {
                        continue;
                    };
                    plan.push_page_lifecycle_event(
                        Some(&target.session_id),
                        name,
                        &binding.frame_id,
                        &binding.loader_id,
                        stamp.timestamp_micros as f64 / 1_000_000.0,
                    );
                }
            }
            plan.push_success();
            plan
        }
        PageLifecycleEventsEnableResult::Handled {
            replay_target: None,
        } => CommandOutputPlan::success(),
        PageLifecycleEventsEnableResult::UnknownSession => {
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded")
        }
    }
}

fn try_start_set_bypass_csp_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: SetBypassCspParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if !conn.set_page_bypass_csp_enabled_for_session_owner(cmd.session_id, params.enabled) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let Some(effective_bypass) =
        conn.effective_page_bypass_csp_enabled_for_session_owner(cmd.session_id)
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(page) = conn
        .loaded_page_mut_for_protocol_access(cmd.session_id)
        .ok()
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::success());
    };
    match page.start_set_bypass_content_security_policy(effective_bypass) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id: cmd.id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::SetBypassContentSecurityPolicy { pending }),
        }),
        Err(error) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn set_font_families_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let font_families = match cmd.get_params::<Value>() {
        Ok(Some(Value::Object(params))) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if !conn.set_page_font_families_for_session_owner(cmd.session_id, font_families) {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    CommandOutputPlan::success()
}

fn set_intercept_file_chooser_dialog_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: SetInterceptFileChooserDialogParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if !conn.set_page_intercept_file_chooser_dialog_enabled_for_session_owner(
        cmd.session_id,
        params.enabled,
    ) {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    CommandOutputPlan::success()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StartScreencastParams {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    quality: Option<i64>,
    #[serde(default)]
    max_width: Option<i64>,
    #[serde(default)]
    max_height: Option<i64>,
    #[serde(default)]
    every_nth_frame: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreencastFrameAckParams {
    session_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageScreencastSubscriptionStatus {
    Inactive,
    Ready,
    CaptureInProgress,
    AwaitingAck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageScreencastRegistration {
    owner_scope: CommandOwnerScope,
    generation: i32,
    every_nth_frame: u32,
}

impl PageScreencastRegistration {
    fn new(owner_scope: CommandOwnerScope, generation: i32, every_nth_frame: u32) -> Self {
        Self {
            owner_scope,
            generation,
            every_nth_frame,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }

    pub fn generation(&self) -> i32 {
        self.generation
    }

    pub fn every_nth_frame(&self) -> u32 {
        self.every_nth_frame
    }
}

pub enum PageScreencastCaptureStart {
    Pending(PendingPageScreencastCapture),
    Retry,
    Stale,
}

pub struct PendingPageScreencastCapture {
    session_id: Option<String>,
    generation: i32,
    owner_scope: CommandOwnerScope,
    viewport: EmulatedViewportSurface,
    pending: PendingPageCommand,
}

pub struct CompletedPageScreencastCapture {
    session_id: Option<String>,
    generation: i32,
    owner_scope: CommandOwnerScope,
    viewport: EmulatedViewportSurface,
    completed: Result<Box<CompletedPageCommand>, String>,
}

impl CompletedPageScreencastCapture {
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn generation(&self) -> i32 {
        self.generation
    }
}

impl PendingPageScreencastCapture {
    pub async fn wait(self) -> CompletedPageScreencastCapture {
        CompletedPageScreencastCapture {
            session_id: self.session_id,
            generation: self.generation,
            owner_scope: self.owner_scope,
            viewport: self.viewport,
            completed: self
                .pending
                .wait()
                .await
                .map(Box::new)
                .map_err(|error| error.to_string()),
        }
    }
}

pub enum PageScreencastCaptureCompletion {
    Frame(BackgroundProtocolEvent),
    Retry,
    Stale,
}

fn start_screencast_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: StartScreencastParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => StartScreencastParams::default(),
        Err(message) => return CommandOutputPlan::error(-32602, message),
    };
    let Some(config) = normalize_start_screencast_params(params) else {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    };
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
        return CommandOutputPlan::error(-32000, START_SCREENCAST_LAYOUT_DISABLED_MESSAGE);
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let every_nth_frame = config.every_nth_frame();
    let Some(generation) = conn.start_page_screencast_for_session_owner(cmd.session_id, config)
    else {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    };
    conn.push_scheduler_event(crate::conn::CdpSchedulerEvent::PageScreencastStarted {
        registration: PageScreencastRegistration::new(owner_scope, generation, every_nth_frame),
    });
    let mut plan = CommandOutputPlan::default();
    plan.push_background_event(BackgroundProtocolEvent::page_screencast_visibility_changed(
        cmd.session_id,
        true,
    ));
    plan.push_success();
    plan
}

fn normalize_start_screencast_params(
    params: StartScreencastParams,
) -> Option<PageScreencastConfig> {
    let format = match params.format.as_deref() {
        None | Some("png") => PageScreencastFormat::Png,
        Some("jpeg") => PageScreencastFormat::Jpeg,
        Some(_) => return None,
    };
    let quality = u8::try_from(params.quality.unwrap_or(80)).ok()?;
    if quality > 100 {
        return None;
    }
    let max_width = normalize_screencast_dimension(params.max_width)?;
    let max_height = normalize_screencast_dimension(params.max_height)?;
    let every_nth_frame = u32::try_from(params.every_nth_frame.unwrap_or(1)).ok()?;
    if every_nth_frame == 0 {
        return None;
    }
    Some(PageScreencastConfig::new(
        format,
        quality,
        max_width,
        max_height,
        every_nth_frame,
    ))
}

fn normalize_screencast_dimension(value: Option<i64>) -> Option<Option<u32>> {
    match value {
        None | Some(0) => Some(None),
        Some(value) => u32::try_from(value).ok().map(Some),
    }
}

fn stop_screencast_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if !conn.stop_page_screencast_for_session_owner(cmd.session_id) {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    CommandOutputPlan::success()
}

fn screencast_frame_ack_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: ScreencastFrameAckParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if params.session_id <= 0 {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    }
    if conn
        .acknowledge_page_screencast_frame_for_session_owner(cmd.session_id, params.session_id)
        .is_none()
    {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    CommandOutputPlan::success()
}

impl CdpConnection {
    pub fn page_screencast_subscription_status(
        &mut self,
        registration: &PageScreencastRegistration,
    ) -> PageScreencastSubscriptionStatus {
        let mut route_scope = registration.owner_scope.enter(self);
        page_screencast_subscription_status_for_current_route(
            route_scope.conn_mut(),
            registration.session_id(),
            registration.generation,
        )
    }

    pub fn start_page_screencast_frame_capture(
        &mut self,
        registration: &PageScreencastRegistration,
    ) -> PageScreencastCaptureStart {
        let session_id = registration.session_id().map(str::to_owned);
        let generation = registration.generation;
        let owner_scope = registration.owner_scope.clone();
        let mut route_scope = owner_scope.enter(self);
        let conn = route_scope.conn_mut();
        let session_id_ref = session_id.as_deref();
        if page_screencast_subscription_status_for_current_route(conn, session_id_ref, generation)
            != PageScreencastSubscriptionStatus::Ready
        {
            return PageScreencastCaptureStart::Stale;
        }
        let Some(config) = conn
            .target_page_session_state_for_session(session_id_ref)
            .and_then(|state| state.page_screencast.config())
            .cloned()
        else {
            return PageScreencastCaptureStart::Stale;
        };
        let viewport = current_viewport_surface(conn, session_id_ref);
        let request = RendererCaptureScreenshotRequest {
            purpose: RendererScreenshotPurpose::Screencast,
            format: match config.format() {
                PageScreencastFormat::Png => RendererScreenshotFormat::Png,
                PageScreencastFormat::Jpeg => RendererScreenshotFormat::Jpeg,
            },
            quality: config.quality(),
            region: moli_core::page::RendererScreenshotRegion::Viewport,
            optimize_for_speed: true,
            max_width: config.max_width(),
            max_height: config.max_height(),
        };
        let pending = match conn.loaded_page_mut_for_protocol_access(session_id_ref) {
            Ok(page) => match page.start_capture_screenshot_with_request(request) {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::debug!(?error, "failed to start screencast frame capture");
                    return PageScreencastCaptureStart::Retry;
                }
            },
            Err(_) => return PageScreencastCaptureStart::Retry,
        };
        if conn.begin_page_screencast_capture_for_session_owner(session_id_ref, generation)
            != Some(true)
        {
            tracing::debug!(
                generation,
                ?session_id,
                "screencast became stale while capture started"
            );
            return PageScreencastCaptureStart::Stale;
        }
        PageScreencastCaptureStart::Pending(PendingPageScreencastCapture {
            session_id,
            generation,
            owner_scope,
            viewport,
            pending,
        })
    }

    pub fn complete_page_screencast_frame_capture(
        &mut self,
        completed: CompletedPageScreencastCapture,
    ) -> PageScreencastCaptureCompletion {
        let CompletedPageScreencastCapture {
            session_id,
            generation,
            owner_scope,
            viewport,
            completed,
        } = completed;
        let mut route_scope = owner_scope.enter(self);
        let conn = route_scope.conn_mut();
        let session_id_ref = session_id.as_deref();
        if page_screencast_subscription_status_for_current_route(conn, session_id_ref, generation)
            != PageScreencastSubscriptionStatus::CaptureInProgress
        {
            return PageScreencastCaptureCompletion::Stale;
        }

        let image = match completed {
            Ok(completion) => {
                let page = match conn.loaded_page_mut_for_protocol_access(session_id_ref) {
                    Ok(page) => page,
                    Err(_) => {
                        let _ = conn.complete_page_screencast_capture_for_session_owner(
                            session_id_ref,
                            generation,
                            false,
                        );
                        return PageScreencastCaptureCompletion::Retry;
                    }
                };
                match page.finish_capture_screenshot(*completion) {
                    Ok(RendererCaptureScreenshotReply::Captured(image)) => image,
                    Ok(
                        RendererCaptureScreenshotReply::LayoutDisabled
                        | RendererCaptureScreenshotReply::NoDocument,
                    )
                    | Err(_) => {
                        let _ = conn.complete_page_screencast_capture_for_session_owner(
                            session_id_ref,
                            generation,
                            false,
                        );
                        return PageScreencastCaptureCompletion::Retry;
                    }
                }
            }
            Err(_) => {
                let _ = conn.complete_page_screencast_capture_for_session_owner(
                    session_id_ref,
                    generation,
                    false,
                );
                return PageScreencastCaptureCompletion::Retry;
            }
        };

        if conn.complete_page_screencast_capture_for_session_owner(session_id_ref, generation, true)
            != Some(true)
        {
            return PageScreencastCaptureCompletion::Stale;
        }
        let metadata = crate::conn::PageScreencastFrameMetadata {
            offset_top: 0.0,
            page_scale_factor: 1.0,
            device_width: f64::from(viewport.inner_width),
            device_height: f64::from(viewport.inner_height),
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        };
        PageScreencastCaptureCompletion::Frame(BackgroundProtocolEvent::page_screencast_frame(
            session_id_ref,
            BASE64_STANDARD.encode(&image.bytes),
            metadata,
            generation,
        ))
    }
}

fn page_screencast_subscription_status_for_current_route(
    conn: &CdpConnection,
    session_id: Option<&str>,
    generation: i32,
) -> PageScreencastSubscriptionStatus {
    let Some(state) = conn.target_page_session_state_for_session(session_id) else {
        return PageScreencastSubscriptionStatus::Inactive;
    };
    let screencast = &state.page_screencast;
    if !screencast.is_active() || screencast.generation() != generation {
        PageScreencastSubscriptionStatus::Inactive
    } else if screencast.capture_in_progress() {
        PageScreencastSubscriptionStatus::CaptureInProgress
    } else if screencast.awaiting_ack() {
        PageScreencastSubscriptionStatus::AwaitingAck
    } else {
        PageScreencastSubscriptionStatus::Ready
    }
}

fn handle_javascript_dialog_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let command = match build_cdp_handle_javascript_dialog_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return plan,
    };
    match start_devtools_page_command(
        conn,
        cmd.id,
        DevToolsCommand::HandleJavaScriptDialog(command),
    ) {
        PageCommandTaskStep::Complete(plan) => plan,
        PageCommandTaskStep::Pending(_) => {
            CommandOutputPlan::error(-32000, "Unexpected pending handleJavaScriptDialog command")
        }
    }
}

fn build_cdp_handle_javascript_dialog_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsHandleJavaScriptDialogCommand, CommandOutputPlan> {
    let params: HandleJavaScriptDialogParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsHandleJavaScriptDialogCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        accept: params.accept,
        prompt_text: params.prompt_text.unwrap_or_default(),
    })
}

fn complete_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
) -> CommandOutputPlan {
    match finish_devtools_handle_javascript_dialog_command(conn, command) {
        Ok(closed_event) => {
            let mut plan = CommandOutputPlan::default();
            plan.push_background_event(closed_event);
            plan.push_success();
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

fn finish_devtools_get_javascript_dialog_command(
    conn: &CdpConnection,
    command: DevToolsGetJavaScriptDialogCommand,
) -> Result<DevToolsJavaScriptDialogResult, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let current_page_owner = conn.target_page_residence_identity_for_session(session_id);
    let Some(dialog) = conn
        .target_page_session_state_for_session(session_id)
        .and_then(|page_state| page_state.javascript_dialog_state.peek_next())
        .filter(|dialog| current_page_owner.as_ref() == Some(dialog.page_owner()))
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchAlert,
            "No dialog is showing",
        ));
    };
    Ok(DevToolsJavaScriptDialogResult {
        dialog_type: dialog.dialog_type().to_owned(),
        message: dialog.message().to_owned(),
        default_prompt: dialog.default_prompt().to_owned(),
    })
}

fn finish_devtools_set_javascript_dialog_prompt_text_command(
    conn: &mut CdpConnection,
    command: DevToolsSetJavaScriptDialogPromptTextCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let current_page_owner = conn.target_page_residence_identity_for_session(session_id);
    let Some(result) =
        conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            let dialog_state = &mut state.page_session_state.javascript_dialog_state;
            let Some(dialog) = dialog_state
                .peek_next()
                .filter(|dialog| current_page_owner.as_ref() == Some(dialog.page_owner()))
            else {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchAlert,
                    "No dialog is showing",
                ));
            };
            if dialog.dialog_type() != "prompt" {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::InvalidArgument,
                    "Dialog is not a prompt",
                ));
            }
            if !dialog_state.set_next_prompt_text(command.prompt_text) {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchAlert,
                    "No dialog is showing",
                ));
            }
            Ok(DevToolsCommandResult::Empty)
        })
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchAlert,
            "No dialog is showing",
        ));
    };
    result
}

fn finish_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
) -> Result<BackgroundProtocolEvent, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let command_prompt_text = command.prompt_text;
    let current_page_owner = conn.target_page_residence_identity_for_session(session_id);
    let Some(dialog) = conn
        .with_target_devtools_session_state_for_session_mut(session_id, |state| {
            let dialog_state = &mut state.page_session_state.javascript_dialog_state;
            if dialog_state
                .peek_next()
                .is_some_and(|dialog| current_page_owner.as_ref() != Some(dialog.page_owner()))
            {
                dialog_state.clear();
                return None;
            }
            dialog_state.pop_next_with_prompt_text()
        })
        .flatten()
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchAlert,
            "No dialog is showing",
        ));
    };
    let (dialog, stored_prompt_text) = dialog;
    let user_input = if command_prompt_text.is_empty() {
        stored_prompt_text.unwrap_or_default()
    } else {
        command_prompt_text
    };
    let _ = dialog.finish(command.accept, user_input.clone());
    let closed_event = UserPromptClosedEvent {
        target_id: command.context.target_id,
        frame_id: dialog.source_frame_id().into(),
        prompt_type: dialog.dialog_type().to_owned(),
        accepted: command.accept,
        user_text: user_input,
    };
    Ok(BackgroundProtocolEvent::page_javascript_dialog_closed(
        session_id,
        closed_event,
    ))
}

pub(in crate::domains) async fn emit_javascript_dialog_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    _session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(dialogs) = prepared_outputs
        .and_then(ProtocolOutputPayloads::page_mut)
        .and_then(PagePreparedOutputSlot::take_javascript_dialogs)
    {
        javascript_dialog::emit_prepared(conn, out, dialogs);
    }
}

pub(in crate::domains) async fn emit_popup_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    _session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(popups) = prepared_outputs
        .and_then(ProtocolOutputPayloads::page_mut)
        .and_then(PagePreparedOutputSlot::take_popup_activations)
    {
        popup::emit_prepared(conn, out, popups).await;
    }
}

/// Moves one prepared navigation into protocol scheduler residence.
///
/// Preparing the output already claimed the renderer value. This projection
/// must only publish its concrete owner action; executing navigation here
/// would let network/download side effects bypass scheduler predecessors.
pub(in crate::domains) fn publish_prepared_top_level_location_navigation_owner_action(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(navigation) = prepared_outputs
        .and_then(ProtocolOutputPayloads::page_mut)
        .and_then(PagePreparedOutputSlot::take_top_level_location_navigation)
    {
        let (owner, navigation) = navigation.into_parts();
        conn.publish_prepared_top_level_location_navigation_owner_action(
            session_id, owner, navigation,
        );
    }
}

pub(in crate::domains) async fn emit_top_level_history_traversal_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(traversal) = prepared_outputs
        .and_then(ProtocolOutputPayloads::page_mut)
        .and_then(PagePreparedOutputSlot::take_top_level_history_traversal)
    {
        traverse_session_owner_history_from_renderer_background_events_async(
            conn,
            out,
            session_id,
            traversal.delta,
        )
        .await;
    }
}

pub(crate) async fn navigate_page_owned_top_level_location_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    owner: &crate::conn::TargetPageResidenceIdentity,
    navigation: RendererDocumentSourcedTopLevelLocationNavigation,
) {
    let source_document = navigation.source_document();
    if !conn.target_page_residence_identity_is_current_for_session(session_id, owner) {
        tracing::debug!(
            session_id,
            ?source_document,
            browser_context_id = owner.browser_context_id(),
            target_id = owner.target_id(),
            page_attachment_id = owner.page_attachment_id().get(),
            url = navigation.url(),
            "dropping top-level location navigation produced by a stale Page residence"
        );
        return;
    }
    navigate_session_owner_from_renderer_request_background_events_async(
        conn,
        out,
        session_id,
        navigation.url(),
        navigation.request_method(),
        navigation.request_body(),
        navigation.request_headers(),
        navigation.browser_navigation_kind(),
    )
    .await;
}

pub(crate) async fn navigate_session_owner_from_renderer_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    url: &str,
) {
    navigate_session_owner_from_renderer_request_background_events_async(
        conn,
        out,
        session_id,
        url,
        "GET",
        None,
        &[],
        moli_fetch::BrowserNavigationRequestKind::Navigate,
    )
    .await;
}

async fn navigate_session_owner_from_renderer_request_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    url: &str,
    request_method: &str,
    request_body: Option<&[u8]>,
    request_headers: &[(String, String)],
    browser_navigation_kind: moli_fetch::BrowserNavigationRequestKind,
) {
    let start = navigation::start_session_owner_navigation_from_renderer(
        conn,
        session_id,
        url,
        request_method,
        request_body,
        request_headers,
        browser_navigation_kind,
    );
    let step =
        navigation::finish_started_navigation_command_for_parts(conn, None, session_id, start, &[]);
    complete_renderer_navigation_step_background_events_async(conn, out, step).await;
}

pub(crate) async fn traverse_session_owner_history_from_renderer_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    delta: i64,
) {
    let step =
        navigation::start_session_owner_history_traversal_from_renderer(conn, session_id, delta);
    complete_renderer_navigation_step_background_events_async(conn, out, step).await;
}

async fn complete_renderer_navigation_step_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    mut step: PageCommandTaskStep,
) {
    let mut command_context = CommandDispatchContext::default();
    loop {
        match step {
            PageCommandTaskStep::Complete(plan) => {
                let (_, background_events) = plan.into_command_status_and_background_events();
                out.extend(background_events);
                return;
            }
            PageCommandTaskStep::Pending(pending) => {
                step =
                    complete_pending_page_command(conn, pending.wait().await, &mut command_context)
                        .await;
            }
        }
    }
}

pub(crate) fn emit_page_window_open_background_events(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner_session_id: Option<&str>,
    url: &str,
    window_name: &str,
    window_features: &[String],
    user_gesture: bool,
) {
    if !crate::domains::target::popup_activation_creates_new_target(
        conn,
        owner_session_id,
        window_name,
    ) {
        return;
    }
    for event_session_id in conn.page_event_session_ids_for_session_owner(owner_session_id) {
        if conn.page_domain_enabled_for_session_owner(event_session_id.as_deref()) == Some(true) {
            out.push(BackgroundProtocolEvent::page_window_open(
                event_session_id.as_deref(),
                url,
                window_name,
                window_features,
                user_gesture,
            ));
        }
    }
}

pub(in crate::domains) async fn emit_same_document_navigation_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(navigations) = prepared_outputs
        .and_then(ProtocolOutputPayloads::page_mut)
        .and_then(PagePreparedOutputSlot::take_same_document_navigations)
    {
        navigation::emit_same_document_navigation_background_events_async(
            conn,
            out,
            session_id,
            navigations,
        )
        .await;
    }
}

#[cfg(test)]
mod producer_tests {
    use moli_core::RendererDocumentTitleChanged;
    use moli_core::page::{
        ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentNetworkSnapshot,
        ChildFrameNavigationSnapshot, RENDERER_BACKEND_NODE_ID_START,
        RendererDocumentLifecycleIdentity, RendererDocumentLifecycleSnapshot,
        RendererDocumentSourcedSameDocumentNavigation,
        RendererDocumentSourcedTopLevelLocationNavigation, RendererDocumentToken,
        RendererFrameToken, RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererLifecycleEpoch, RendererLifecycleEventStamp,
        RendererPageCreationArtifacts, RendererPendingDownloadActivation,
        RendererPendingFileChooserActivation, RendererPendingJavaScriptDialog,
        RendererPendingPopupActivation, RendererPendingSameDocumentNavigation,
        RendererWindowDocumentSource, SubresourceResponseBody,
    };
    use serde_json::{Value, json};

    use crate::conn::{BackgroundProtocolEvent, BrowserContext, CdpConnection, CdpTargetFilter};
    use crate::devtools_runtime::{AutomationEvent, NavigationFrameEventKind};
    use crate::domains::activity::{ProtocolOutputPayloads, ProtocolOutputProjectionContext};
    use crate::domains::input::{InputPreparedOutputSlot, InputPreparedOutputs};
    use crate::testing::TestContext;

    fn renderer_document_identity_for_test(
        lifecycle_document_id: u64,
        epoch: u64,
    ) -> RendererDocumentLifecycleIdentity {
        let page_id = moli_core::PageId::new_for_testing(1);
        RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, lifecycle_document_id),
            epoch: RendererLifecycleEpoch(epoch),
        }
    }

    fn bind_renderer_document_for_test(
        conn: &mut CdpConnection,
        session_id: &str,
        frame_id: &str,
        identity: RendererDocumentLifecycleIdentity,
    ) {
        let runtime_slot = conn
            .runtime_session_owner_slot_mut(Some(session_id))
            .expect("test target should expose a runtime owner slot");
        if runtime_slot.page_attachment_id().is_none() {
            runtime_slot.set_page_attachment_id_for_test(identity.document.page_id.as_u64());
        }
        let lifecycle_snapshot = RendererDocumentLifecycleSnapshot {
            frame: identity.frame,
            document: identity.document,
            epoch: identity.epoch,
            started: RendererLifecycleEventStamp {
                sequence: 1,
                timestamp_micros: 1,
            },
            dom_content_loaded: None,
            load: None,
            terminated: None,
        };
        let (binding, initial_events) = conn.bind_renderer_document_lifecycle_for_session_owner(
            Some(session_id),
            RendererPageCreationArtifacts {
                active_document: identity.document,
                active_epoch: identity.epoch,
                lifecycle_snapshot,
                initial_lifecycle_events: Vec::new(),
            },
            None,
            frame_id.to_owned(),
            super::LOADER_ID.to_owned(),
        );
        assert!(binding.is_some(), "test renderer Document should bind");
        assert!(initial_events.is_empty());
    }

    fn page_residence_identity_for_test(
        conn: &mut CdpConnection,
        session_id: &str,
    ) -> crate::conn::TargetPageResidenceIdentity {
        let runtime_slot = conn
            .runtime_session_owner_slot_mut(Some(session_id))
            .expect("test target should expose a runtime owner slot");
        if runtime_slot.page_attachment_id().is_none() {
            runtime_slot.replace_page_attachment_id_for_test();
        }
        conn.target_page_residence_identity_for_session(Some(session_id))
            .expect("test target should expose a Page residence identity")
    }

    fn take_top_level_location_navigation_work_for_test(
        conn: &mut CdpConnection,
    ) -> crate::domains::activity::ProtocolSchedulerWork {
        let [event]: [crate::conn::CdpSchedulerEvent; 1] = conn
            .take_scheduler_events()
            .try_into()
            .expect("prepared navigation should publish one concrete scheduler action");
        let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
            panic!("prepared navigation must not publish a source-shaped scheduler event");
        };
        assert!(work.is_top_level_location_navigation_owner_action());
        work
    }

    fn root_document_attachment_for_test(
        conn: &CdpConnection,
        session_id: &str,
        source_document: RendererDocumentLifecycleIdentity,
    ) -> crate::conn::TargetRootDocumentProtocolAttachmentIdentity {
        conn.target_root_document_protocol_attachment_identity_for_session(
            Some(session_id),
            source_document,
        )
        .expect("test target should expose the exact root Document attachment")
    }

    fn prepared_child_frame_activity_for_test(
        conn: &CdpConnection,
        session_id: &str,
        source_document: RendererDocumentLifecycleIdentity,
        document: super::PagePreparedChildFrameDocumentActivity,
    ) -> super::PagePreparedChildFrameActivity {
        let binding = root_document_attachment_for_test(conn, session_id, source_document);
        super::PagePreparedChildFrameActivity::from_document(binding, document)
    }

    fn default_prepared_child_frame_activity_for_test(
        conn: &CdpConnection,
        session_id: &str,
        source_document: RendererDocumentLifecycleIdentity,
    ) -> super::PagePreparedChildFrameActivity {
        super::PagePreparedOutputs::from_child_frame_activity_for_test(
            root_document_attachment_for_test(conn, session_id, source_document),
        )
        .child_frame_activities
        .pop()
        .expect("default child-frame fixture should contain one activity batch")
    }

    fn javascript_dialog_scope_for_test(
        conn: &CdpConnection,
        session_id: &str,
    ) -> crate::conn::TargetJavaScriptDialogScopeObserver {
        conn.runtime_session_owner_slot(Some(session_id))
            .map(|slot| slot.javascript_dialog_scope_observer())
            .unwrap_or_else(|_| {
                crate::conn::TargetJavaScriptDialogScopeObserver::stale_for_absent_owner_test()
            })
    }

    fn document_sourced_same_document_navigation_for_test(
        source_document: RendererDocumentLifecycleIdentity,
        url: &str,
    ) -> RendererDocumentSourcedSameDocumentNavigation {
        RendererDocumentSourcedSameDocumentNavigation::new(
            source_document,
            RendererPendingSameDocumentNavigation {
                url: url.to_owned(),
                navigation_type: "fragment".to_owned(),
                history_update: moli_core::page::SameDocumentHistoryUpdate::Push,
            },
        )
    }

    fn renderer_javascript_dialog_for_test(
        source_document: RendererDocumentLifecycleIdentity,
        frame_id: &str,
        message: &str,
        completion: Option<RendererJavaScriptDialogCompletion>,
    ) -> RendererPendingJavaScriptDialog {
        RendererPendingJavaScriptDialog::new(
            RendererJavaScriptDialogId::new(1),
            source_document,
            RendererJavaScriptDialogSource::ChildFrame {
                frame_id: frame_id.to_owned(),
                local_window_id: 1,
                document_id: 1,
            },
            "https://example.test/dialog-source".to_owned(),
            "alert".to_owned(),
            message.to_owned(),
            String::new(),
            completion,
        )
    }

    fn renderer_popup_javascript_dialog_for_test(
        source_document: RendererDocumentLifecycleIdentity,
        popup_id: u64,
        popup_document_id: u64,
        message: &str,
        completion: Option<RendererJavaScriptDialogCompletion>,
    ) -> RendererPendingJavaScriptDialog {
        RendererPendingJavaScriptDialog::new(
            RendererJavaScriptDialogId::new(2),
            source_document,
            RendererJavaScriptDialogSource::LightweightPopup {
                popup_id,
                popup_document_id,
            },
            "https://popup.example/dialog-source".to_owned(),
            "alert".to_owned(),
            message.to_owned(),
            String::new(),
            completion,
        )
    }

    fn protocol_messages_from_background_events(
        events: Vec<BackgroundProtocolEvent>,
    ) -> Vec<Value> {
        events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect()
    }

    #[test]
    fn stale_document_title_cannot_overwrite_replacement_target_metadata() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-title-source".into());
        bc.set_active_target_id("TID-title-source");
        bc.attach_active_session("SID-title-source");
        conn.browser_context = Some(bc);

        let predecessor = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-title-source",
            "TID-title-source",
            predecessor,
        );
        assert_eq!(
            conn.apply_renderer_document_title_for_session_owner(
                Some("SID-title-source"),
                &RendererDocumentTitleChanged {
                    source_document: predecessor,
                    title: "predecessor".to_owned(),
                },
            ),
            Some(true)
        );

        let replacement = renderer_document_identity_for_test(2, 2);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-title-source",
            "TID-title-source",
            replacement,
        );
        assert_eq!(
            conn.apply_renderer_document_title_for_session_owner(
                Some("SID-title-source"),
                &RendererDocumentTitleChanged {
                    source_document: replacement,
                    title: "replacement".to_owned(),
                },
            ),
            Some(true)
        );

        assert_eq!(
            conn.apply_renderer_document_title_for_session_owner(
                Some("SID-title-source"),
                &RendererDocumentTitleChanged {
                    source_document: predecessor,
                    title: "late predecessor".to_owned(),
                },
            ),
            None,
            "an old renderer Document must lose authority at replacement commit"
        );
        assert_eq!(
            conn.browser_context
                .as_ref()
                .and_then(|context| context.target_info("TID-title-source"))
                .and_then(|target| target["title"].as_str().map(str::to_owned)),
            Some("replacement".to_owned())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn javascript_dialog_drain_consumes_prepared_dialogs_without_page_readback() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active");
        bc.attach_active_session("SID-1");
        conn.browser_context = Some(bc);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-1");
        let source_document = renderer_document_identity_for_test(1, 1);
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner.clone(),
                    Some("SID-1"),
                    javascript_dialog_scope_for_test(&conn, "SID-1"),
                    "TID-active",
                    vec![renderer_javascript_dialog_for_test(
                        source_document,
                        "FRAME-1",
                        "prepared dialog",
                        None,
                    )],
                ),
            ));

        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-1"),
            Some(&mut prepared),
        )
        .await;

        assert_eq!(out.len(), 1);
        let (message, automation_event) = out.remove(0).into_parts();
        assert_eq!(message["method"], json!("Page.javascriptDialogOpening"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-1"));
        assert_eq!(message["params"]["type"], json!("alert"));
        assert_eq!(message["params"]["message"], json!("prepared dialog"));
        assert_eq!(message["sessionId"], json!("SID-1"));
        let Some(AutomationEvent::PageJavaScriptDialogOpening(event)) = automation_event else {
            panic!("expected typed Page.javascriptDialogOpening sidecar");
        };
        assert_eq!(
            event.frame_id.as_ref().map(|frame_id| frame_id.as_str()),
            Some("FRAME-1")
        );
        assert_eq!(event.dialog_type, "alert");
        assert_eq!(event.message, "prepared dialog");
        assert!(event.has_browser_handler);
        let installed = conn
            .target_page_session_state_for_session(Some("SID-1"))
            .expect("target page session state should exist")
            .javascript_dialog_state
            .pending_dialogs();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].page_owner(), &page_owner);
        assert_eq!(installed[0].source_frame_id(), "FRAME-1");
        assert_eq!(installed[0].message(), "prepared dialog");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_dialog_output_stays_with_its_exact_protocol_attachment() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-dialog-attachment".into());
        browser_context.set_active_target_id("TID-dialog-attachment");
        browser_context.attach_active_session("SID-primary");
        assert!(
            browser_context
                .assign_auxiliary_session_to_target("TID-dialog-attachment", "SID-aux".to_owned(),)
        );
        conn.browser_context = Some(browser_context);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-aux");
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner,
                    Some("SID-aux"),
                    javascript_dialog_scope_for_test(&conn, "SID-aux"),
                    "TID-dialog-attachment",
                    vec![renderer_javascript_dialog_for_test(
                        renderer_document_identity_for_test(1, 1),
                        "FRAME-child",
                        "auxiliary attachment dialog",
                        None,
                    )],
                ),
            ));

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-primary"),
            Some(&mut prepared),
        )
        .await;

        assert_eq!(out.len(), 1);
        let message = out.remove(0).into_protocol_message();
        assert_eq!(message["sessionId"], json!("SID-aux"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-child"));
        assert!(
            conn.target_page_session_state_for_session(Some("SID-primary"))
                .expect("primary session state")
                .javascript_dialog_state
                .is_empty(),
            "drain-time session must not acquire another attachment's dialog"
        );
        assert_eq!(
            conn.target_page_session_state_for_session(Some("SID-aux"))
                .expect("auxiliary session state")
                .javascript_dialog_state
                .pending_dialogs()
                .len(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn detached_source_attachment_dismisses_prepared_child_dialog() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-dialog-detached".into());
        browser_context.set_active_target_id("TID-dialog-detached");
        browser_context.attach_active_session("SID-primary");
        assert!(
            browser_context.assign_auxiliary_session_to_target(
                "TID-dialog-detached",
                "SID-detached".to_owned(),
            )
        );
        conn.browser_context = Some(browser_context);
        let completion = RendererJavaScriptDialogCompletion::pending();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_residence_identity_for_test(&mut conn, "SID-detached"),
                    Some("SID-detached"),
                    javascript_dialog_scope_for_test(&conn, "SID-detached"),
                    "TID-dialog-detached",
                    vec![renderer_javascript_dialog_for_test(
                        renderer_document_identity_for_test(1, 1),
                        "FRAME-detached",
                        "detached attachment dialog",
                        Some(completion.clone()),
                    )],
                ),
            ));
        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .remove_auxiliary_session("SID-detached")
                .as_deref(),
            Some("TID-dialog-detached")
        );

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-primary"),
            Some(&mut prepared),
        )
        .await;

        assert!(out.is_empty());
        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parked_popup_dialog_rejects_a_retired_source_attachment() {
        const POPUP_ID: u64 = 76;

        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-popup-stale-source".into());
        browser_context.set_active_target_id("TID-popup-stale-source");
        browser_context.attach_active_session("SID-primary");
        assert!(
            browser_context.assign_auxiliary_session_to_target(
                "TID-popup-stale-source",
                "SID-source".to_owned(),
            )
        );
        conn.browser_context = Some(browser_context);
        conn.set_auto_attach_owner(None, true, false, CdpTargetFilter::default_auto_attach());
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-source");
        let source_document = renderer_document_identity_for_test(1, 1);
        let completion = RendererJavaScriptDialogCompletion::pending();
        let mut dialog_output =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner.clone(),
                    Some("SID-source"),
                    javascript_dialog_scope_for_test(&conn, "SID-source"),
                    "TID-popup-stale-source",
                    vec![renderer_popup_javascript_dialog_for_test(
                        source_document,
                        POPUP_ID,
                        8,
                        "stale source popup dialog",
                        Some(completion.clone()),
                    )],
                ),
            ));
        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-primary"),
            Some(&mut dialog_output),
        )
        .await;
        assert!(
            out.is_empty(),
            "dialog should be parked before popup creation"
        );

        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .remove_auxiliary_session("SID-source")
                .as_deref(),
            Some("TID-popup-stale-source")
        );
        let mut popup_output =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_popup_activations_for_test(
                    page_owner,
                    vec![RendererPendingPopupActivation::window(
                        source_document,
                        RendererWindowDocumentSource::RootFrame,
                        true,
                        Some(POPUP_ID),
                        "about:blank".to_owned(),
                        "_blank".to_owned(),
                    )],
                ),
            ));
        super::emit_popup_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-primary"),
            Some(&mut popup_output),
        )
        .await;

        let messages = protocol_messages_from_background_events(out);
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == json!("Target.attachedToTarget")),
            "the popup must have a real attachment so source retirement is the rejection reason"
        );
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != json!("Page.javascriptDialogOpening"))
        );
        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lightweight_popup_dialog_waits_for_and_uses_popup_attachment() {
        const POPUP_ID: u64 = 77;

        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-popup-dialog".into());
        browser_context.set_active_target_id("TID-opener");
        browser_context.attach_active_session("SID-opener");
        conn.browser_context = Some(browser_context);
        conn.set_auto_attach_owner(None, true, false, CdpTargetFilter::default_auto_attach());
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-opener");
        let source_dialog_scope = javascript_dialog_scope_for_test(&conn, "SID-opener");
        let source_document = renderer_document_identity_for_test(1, 1);
        let completion = RendererJavaScriptDialogCompletion::pending();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner.clone(),
                    Some("SID-opener"),
                    source_dialog_scope.clone(),
                    "TID-opener",
                    vec![renderer_popup_javascript_dialog_for_test(
                        source_document,
                        POPUP_ID,
                        9,
                        "popup-owned dialog",
                        Some(completion.clone()),
                    )],
                ),
            ));
        prepared.extend_payload(
            super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_popup_activations_for_test(
                    page_owner.clone(),
                    vec![RendererPendingPopupActivation::window(
                        source_document,
                        RendererWindowDocumentSource::RootFrame,
                        true,
                        Some(POPUP_ID),
                        "about:blank".to_owned(),
                        "_blank".to_owned(),
                    )],
                ),
            )
            .into(),
        );

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-opener"),
            Some(&mut prepared),
        )
        .await;
        assert!(
            out.is_empty(),
            "dialog must wait for its popup target instead of falling back to the opener"
        );

        super::emit_popup_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-opener"),
            Some(&mut prepared),
        )
        .await;

        let browser_context = conn
            .browser_context_by_id("BID-popup-dialog")
            .expect("popup browser context");
        let popup_target_id = browser_context
            .target_id_for_popup_id(POPUP_ID)
            .expect("popup id should resolve to its created target")
            .to_owned();
        let popup_session_id = browser_context
            .background_target(&popup_target_id)
            .and_then(|target| target.session_id())
            .expect("auto-attached popup session")
            .to_owned();
        let observed_messages = protocol_messages_from_background_events(out);
        let dialog_event = observed_messages
            .iter()
            .find(|message| message["method"] == json!("Page.javascriptDialogOpening"))
            .unwrap_or_else(|| {
                panic!(
                    "popup dialog opening event; popup_session_id={popup_session_id}; \
                     observed={observed_messages:?}"
                )
            });
        assert_eq!(dialog_event["sessionId"], json!(popup_session_id));
        assert_eq!(
            dialog_event["params"]["frameId"],
            json!(popup_target_id.clone())
        );
        assert!(
            conn.target_page_session_state_for_session(Some("SID-opener"))
                .expect("opener session state")
                .javascript_dialog_state
                .is_empty(),
            "opener session must not own the popup's modal dialog"
        );
        assert_eq!(
            conn.target_page_session_state_for_session(Some(&popup_session_id))
                .expect("popup session state")
                .javascript_dialog_state
                .pending_dialogs()
                .len(),
            1
        );

        let mut later_dialog =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner,
                    Some("SID-opener"),
                    source_dialog_scope,
                    "TID-opener",
                    vec![renderer_popup_javascript_dialog_for_test(
                        source_document,
                        POPUP_ID,
                        9,
                        "later popup-owned dialog",
                        None,
                    )],
                ),
            ));
        let mut later_out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut later_out,
            Some("SID-opener"),
            Some(&mut later_dialog),
        )
        .await;
        let later_messages = protocol_messages_from_background_events(later_out);
        assert!(later_messages.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["sessionId"] == json!(popup_session_id)
                && message["params"]["message"] == json!("later popup-owned dialog")
        }));
        assert_eq!(
            conn.target_page_session_state_for_session(Some(&popup_session_id))
                .expect("popup session state")
                .javascript_dialog_state
                .pending_dialogs()
                .len(),
            2,
            "a later output batch should resolve through the existing popup attachment"
        );
        conn.with_target_devtools_session_state_for_session_mut(Some(&popup_session_id), |state| {
            state.page_session_state.javascript_dialog_state.clear()
        })
        .expect("popup session state should clear");
        assert!(!completion.wait().accepted);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unattached_popup_dialog_is_dismissed_without_opener_fallback() {
        const POPUP_ID: u64 = 78;

        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-popup-no-session".into());
        browser_context.set_active_target_id("TID-opener-no-session");
        browser_context.attach_active_session("SID-opener-no-session");
        conn.browser_context = Some(browser_context);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-opener-no-session");
        let source_document = renderer_document_identity_for_test(1, 1);
        let completion = RendererJavaScriptDialogCompletion::pending();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner.clone(),
                    Some("SID-opener-no-session"),
                    javascript_dialog_scope_for_test(&conn, "SID-opener-no-session"),
                    "TID-opener-no-session",
                    vec![renderer_popup_javascript_dialog_for_test(
                        source_document,
                        POPUP_ID,
                        10,
                        "unattached popup dialog",
                        Some(completion.clone()),
                    )],
                ),
            ));
        prepared.extend_payload(
            super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_popup_activations_for_test(
                    page_owner,
                    vec![RendererPendingPopupActivation::window(
                        source_document,
                        RendererWindowDocumentSource::RootFrame,
                        true,
                        Some(POPUP_ID),
                        "about:blank".to_owned(),
                        "_blank".to_owned(),
                    )],
                ),
            )
            .into(),
        );

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-opener-no-session"),
            Some(&mut prepared),
        )
        .await;
        super::emit_popup_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-opener-no-session"),
            Some(&mut prepared),
        )
        .await;

        let messages = protocol_messages_from_background_events(out);
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != json!("Page.javascriptDialogOpening"))
        );
        assert!(
            conn.target_page_session_state_for_session(Some("SID-opener-no-session"))
                .expect("opener session state")
                .javascript_dialog_state
                .is_empty()
        );
        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }

    #[test]
    fn renderer_document_epoch_change_retires_page_dialog_scope_once() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-dialog-epoch".into());
        bc.set_active_target_id("TID-dialog-epoch");
        bc.attach_active_session("SID-dialog-epoch");
        conn.browser_context = Some(bc);
        let first_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-dialog-epoch",
            "TID-dialog-epoch",
            first_document,
        );
        let observer = conn
            .runtime_session_owner_slot(Some("SID-dialog-epoch"))
            .expect("target Page runtime slot")
            .javascript_dialog_scope_observer();

        bind_renderer_document_for_test(
            &mut conn,
            "SID-dialog-epoch",
            "TID-dialog-epoch",
            first_document,
        );
        assert!(
            conn.runtime_session_owner_slot(Some("SID-dialog-epoch"))
                .expect("target Page runtime slot")
                .observes_javascript_dialog_scope(&observer),
            "rebinding the same exact renderer Document must preserve prepared dialog output"
        );

        bind_renderer_document_for_test(
            &mut conn,
            "SID-dialog-epoch",
            "TID-dialog-epoch",
            renderer_document_identity_for_test(1, 2),
        );
        assert!(
            !conn
                .runtime_session_owner_slot(Some("SID-dialog-epoch"))
                .expect("target Page runtime slot")
                .observes_javascript_dialog_scope(&observer),
            "a new lifecycle epoch for the same Document token must retire old prepared dialogs"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn javascript_dialog_prepared_action_dismisses_replacement_page_output() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-dialog-stale-page".into());
        bc.set_active_target_id("TID-dialog-stale-page");
        bc.attach_active_session("SID-dialog-stale-page");
        conn.browser_context = Some(bc);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-dialog-stale-page");
        let completion = moli_core::page::RendererJavaScriptDialogCompletion::pending();
        let dialog = renderer_javascript_dialog_for_test(
            renderer_document_identity_for_test(1, 1),
            "FRAME-stale-page",
            "stale page dialog",
            Some(completion.clone()),
        );
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner.clone(),
                    Some("SID-dialog-stale-page"),
                    javascript_dialog_scope_for_test(&conn, "SID-dialog-stale-page"),
                    "TID-dialog-stale-page",
                    vec![dialog],
                ),
            ));
        conn.runtime_session_owner_slot_mut(Some("SID-dialog-stale-page"))
            .expect("test target runtime slot")
            .replace_page_attachment_id_for_test();

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-dialog-stale-page"),
            Some(&mut prepared),
        )
        .await;

        assert!(out.is_empty());
        assert!(
            conn.target_page_session_state_for_session(Some("SID-dialog-stale-page"))
                .expect("target page session state")
                .javascript_dialog_state
                .is_empty()
        );
        assert!(
            !completion.finish(true, "late accept".to_owned()),
            "stale apply must already dismiss the renderer completion"
        );
        let result = completion.wait();
        assert!(!result.accepted);
        assert!(result.user_input.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn javascript_dialog_prepared_action_dismisses_retired_dialog_scope() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-dialog-generation".into());
        bc.set_active_target_id("TID-dialog-generation");
        bc.attach_active_session("SID-dialog-generation");
        conn.browser_context = Some(bc);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-dialog-generation");
        let completion = moli_core::page::RendererJavaScriptDialogCompletion::pending();
        let dialog = renderer_javascript_dialog_for_test(
            renderer_document_identity_for_test(1, 1),
            "FRAME-dialog-generation",
            "retired dialog",
            Some(completion.clone()),
        );
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner,
                    Some("SID-dialog-generation"),
                    javascript_dialog_scope_for_test(&conn, "SID-dialog-generation"),
                    "TID-dialog-generation",
                    vec![dialog],
                ),
            ));
        conn.runtime_session_owner_slot_mut(Some("SID-dialog-generation"))
            .expect("target Page runtime slot")
            .retire_javascript_dialog_scope();
        conn.with_target_devtools_session_state_for_session_mut(
            Some("SID-dialog-generation"),
            |state| state.page_session_state.javascript_dialog_state.clear(),
        )
        .expect("target session state should retire its dialog scope");

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-dialog-generation"),
            Some(&mut prepared),
        )
        .await;

        assert!(out.is_empty());
        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn javascript_dialog_projection_uses_captured_url_and_frame() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-dialog-source".into());
        bc.set_active_target_id("TID-dialog-source");
        bc.set_target_url("https://example.test/current-before-capture".to_owned());
        bc.attach_active_session("SID-dialog-source");
        conn.browser_context = Some(bc);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-dialog-source");
        let dialog = RendererPendingJavaScriptDialog::new(
            RendererJavaScriptDialogId::new(9),
            renderer_document_identity_for_test(2, 3),
            RendererJavaScriptDialogSource::ChildFrame {
                frame_id: "FRAME-source".to_owned(),
                local_window_id: 4,
                document_id: 5,
            },
            "https://source.example/dialog".to_owned(),
            "alert".to_owned(),
            "source identity".to_owned(),
            String::new(),
            None,
        );
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner,
                    Some("SID-dialog-source"),
                    javascript_dialog_scope_for_test(&conn, "SID-dialog-source"),
                    "TID-dialog-source",
                    vec![dialog],
                ),
            ));
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_target_url("https://replacement.example/current".to_owned());

        let mut out = Vec::new();
        super::emit_javascript_dialog_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-dialog-source"),
            Some(&mut prepared),
        )
        .await;

        assert_eq!(out.len(), 1);
        let message = out.remove(0).into_protocol_message();
        assert_eq!(message["params"]["frameId"], json!("FRAME-source"));
        assert_eq!(
            message["params"]["url"],
            json!("https://source.example/dialog")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn canonical_activity_drain_order_survives_ordered_typed_event_stream() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        conn.download_behavior
            .set_global("deny".to_owned(), None, true);
        let mut bc = BrowserContext::new("BID-activity-order".into());
        bc.set_active_target_id("TID-activity-order");
        bc.set_target_url("https://example.test/page".to_owned());
        bc.attach_active_session("SID-activity-order");
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-activity-order",
            "TID-activity-order",
            source_document,
        );
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-activity-order");

        let mut prepared =
            ProtocolOutputPayloads::from_slot(InputPreparedOutputSlot::from_outputs(
                InputPreparedOutputs::from_file_chooser_activations_for_test(
                    page_owner.clone(),
                    "TID-activity-order",
                    vec![RendererPendingFileChooserActivation::new(
                        source_document,
                        Some("TID-activity-order".to_owned()),
                        RENDERER_BACKEND_NODE_ID_START + 42,
                        false,
                    )],
                ),
            ));
        prepared.extend_payload(
            InputPreparedOutputSlot::from_outputs(
                InputPreparedOutputs::from_download_activations_for_test(vec![
                    RendererPendingDownloadActivation {
                        url: "https://example.test/report.txt".to_owned(),
                        suggested_filename: Some("report.txt".to_owned()),
                        response: None,
                    },
                ]),
            )
            .into(),
        );
        prepared.extend_payload(
            super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_javascript_dialogs_for_test(
                    page_owner.clone(),
                    Some("SID-activity-order"),
                    javascript_dialog_scope_for_test(&conn, "SID-activity-order"),
                    "TID-activity-order",
                    vec![renderer_javascript_dialog_for_test(
                        source_document,
                        "TID-activity-order",
                        "ordered dialog",
                        None,
                    )],
                ),
            )
            .into(),
        );
        prepared.extend_payload(
            super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_popup_activations_for_test(
                    page_owner.clone(),
                    vec![RendererPendingPopupActivation::window(
                        source_document,
                        RendererWindowDocumentSource::RootFrame,
                        true,
                        None,
                        "data:text/html,%3Cmain%3Eordered-popup%3C/main%3E".to_owned(),
                        "_blank".to_owned(),
                    )],
                ),
            )
            .into(),
        );

        let mut command_context = crate::conn::CommandDispatchContext::default();
        let mut context =
            ProtocolOutputProjectionContext::new(Some("SID-activity-order"), &mut command_context);

        for step in [
            super::PageOutputProjectionStep::FileChooser,
            super::PageOutputProjectionStep::Download,
            super::PageOutputProjectionStep::JavascriptDialog,
            super::PageOutputProjectionStep::Popup,
        ] {
            step.project_async(&mut conn, &mut context, Some(&mut prepared))
                .await;
        }

        let events = context.command.take_protocol_events();
        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        let ordered_methods = parts
            .iter()
            .filter_map(|(message, automation_event)| match automation_event {
                Some(AutomationEvent::BrowserDownloadWillBegin(_)) => {
                    Some("Browser.downloadWillBegin")
                }
                Some(AutomationEvent::BrowserDownloadProgress(_)) => {
                    Some("Browser.downloadProgress")
                }
                _ => message["method"].as_str(),
            })
            .filter(|method| {
                matches!(
                    *method,
                    "Page.fileChooserOpened"
                        | "Browser.downloadWillBegin"
                        | "Browser.downloadProgress"
                        | "Page.javascriptDialogOpening"
                        | "Target.targetCreated"
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            ordered_methods,
            vec![
                "Page.fileChooserOpened",
                "Browser.downloadWillBegin",
                "Browser.downloadProgress",
                "Page.javascriptDialogOpening",
                "Target.targetCreated",
            ],
            "typed activity sidecars must stay in canonical activity order"
        );
        assert!(matches!(
            parts[0].1.as_ref(),
            Some(AutomationEvent::PageFileChooserOpened(event))
                if event.backend_node_id == RENDERER_BACKEND_NODE_ID_START + 42
        ));
        assert!(
            parts.iter().any(|(_, event)| matches!(
                event,
                Some(AutomationEvent::BrowserDownloadWillBegin(download))
                    if download.suggested_filename == "report.txt"
            )),
            "download willBegin should retain its typed sidecar in the ordered stream"
        );
        assert!(
            parts.iter().any(|(_, event)| matches!(
                event,
                Some(AutomationEvent::PageJavaScriptDialogOpening(dialog))
                    if dialog.message == "ordered dialog"
            )),
            "javascript dialog should retain its typed sidecar in the ordered stream"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn later_navigation_drain_order_survives_ordered_typed_event_stream() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-later-activity-order".into());
        bc.set_active_target_id("TID-later-activity-order");
        bc.set_target_url("https://example.test/page".to_owned());
        bc.attach_active_session("SID-later-activity-order");
        bc.devtools_session_state
            .page_session_state
            .page_lifecycle_events = true;
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-later-activity-order",
            "TID-later-activity-order",
            source_document,
        );

        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_child_frame_activity_for_test(
                    root_document_attachment_for_test(
                        &conn,
                        "SID-later-activity-order",
                        source_document,
                    ),
                ),
            ));
        prepared.extend_payload(
            super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_same_document_navigations_for_test(
                    page_residence_identity_for_test(&mut conn, "SID-later-activity-order"),
                    vec![document_sourced_same_document_navigation_for_test(
                        source_document,
                        "https://example.test/page#ordered",
                    )],
                ),
            )
            .into(),
        );
        prepared.extend_payload(
            super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                    page_residence_identity_for_test(&mut conn, "SID-later-activity-order"),
                    Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                        source_document,
                        "data:text/html,%3Cmain%3Eordered-location%3C/main%3E".to_owned(),
                    )),
                ),
            )
            .into(),
        );

        let mut command_context = crate::conn::CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext::new(
            Some("SID-later-activity-order"),
            &mut command_context,
        );

        for step in [
            super::PageOutputProjectionStep::ChildFrameActivity,
            super::PageOutputProjectionStep::SameDocumentNavigation,
            super::PageOutputProjectionStep::TopLevelLocationNavigation,
        ] {
            step.project_async(&mut conn, &mut context, Some(&mut prepared))
                .await;
        }

        let work = take_top_level_location_navigation_work_for_test(&mut conn);
        let (navigation_events, nested_scheduler_events) = conn
            .complete_ready_protocol_scheduler_work_turn(work)
            .await
            .into_protocol_event_parts();
        assert!(
            !nested_scheduler_events.iter().any(|event| {
                matches!(
                    event,
                    crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                        if work.is_top_level_location_navigation_owner_action()
                )
            }),
            "executing the concrete navigation must not republish its own owner action"
        );
        context
            .command
            .protocol_events_mut()
            .extend(navigation_events);

        let events = context.command.take_protocol_events();
        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        let ordered_markers = parts
            .iter()
            .filter_map(|(message, _)| {
                let method = message["method"].as_str()?;
                let params = &message["params"];
                match method {
                    "Page.frameAttached"
                        if params["frameId"] == json!("CHILD-FRAME-1")
                            && params["parentFrameId"] == json!("TID-1") =>
                    {
                        Some("child-frame-completion")
                    }
                    "Page.navigatedWithinDocument"
                        if params["url"] == json!("https://example.test/page#ordered") =>
                    {
                        Some("same-document-navigation")
                    }
                    "Page.frameStartedNavigating"
                        if params["url"]
                            == json!("data:text/html,%3Cmain%3Eordered-location%3C/main%3E") =>
                    {
                        Some("top-level-location-navigation")
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>();

        let child_frame_completion_index = ordered_markers
            .iter()
            .position(|marker| *marker == "child-frame-completion")
            .expect("child-frame completion marker");
        let same_document_navigation_index = ordered_markers
            .iter()
            .position(|marker| *marker == "same-document-navigation")
            .expect("same-document navigation marker");
        let top_level_location_navigation_index = ordered_markers
            .iter()
            .position(|marker| *marker == "top-level-location-navigation")
            .expect("top-level location navigation marker");

        assert!(
            child_frame_completion_index < same_document_navigation_index
                && same_document_navigation_index < top_level_location_navigation_index,
            "typed navigation activity sidecars must not be appended after later raw activity output: {ordered_markers:?}"
        );
        assert!(
            parts.iter().any(|(_, event)| matches!(
                event,
                Some(AutomationEvent::NavigationFrame(navigation))
                    if navigation.kind == NavigationFrameEventKind::Navigated
                        && navigation.frame_id.as_str() == "CHILD-FRAME-1"
                        && navigation.loader_id.as_ref().is_some_and(|loader_id| {
                            loader_id.as_str() == "LOADER-CHILD-FRAME-1"
                        })
                        && navigation.url == "https://example.test/child"
            )),
            "child-frame completion should regain its typed navigation sidecar after out projection"
        );
        assert!(
            parts.iter().any(|(_, event)| matches!(
                event,
                Some(AutomationEvent::SameDocumentNavigation(navigation))
                    if navigation.url == "https://example.test/page#ordered"
            )),
            "same-document navigation should regain its typed sidecar after out projection"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn browser_initiated_child_frame_completion_omits_renderer_request_events() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_lifecycle_events = true;
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        bc.devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let mut background_events = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_child_frame_activity_for_test(
                    root_document_attachment_for_test(&conn, "SID-1", source_document),
                ),
            ));
        let activity = prepared
            .page_mut()
            .and_then(super::PagePreparedOutputSlot::take_child_frame_activity)
            .and_then(|mut activities| activities.pop())
            .expect("test prepared output should carry one child frame activity");

        super::emit_prepared_child_frame_activity(
            &mut conn,
            &mut background_events,
            activity,
            Some("CHILD-FRAME-1"),
        )
        .await;
        let out = protocol_messages_from_background_events(background_events);

        assert!(
            conn.runtime_session_owner_slot(Some("SID-1"))
                .expect("runtime owner slot should exist")
                .loaded_page()
                .is_none(),
            "prepared child-frame completion emission must not require a loaded page"
        );
        assert!(out.iter().any(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!("CHILD-FRAME-1")
                && message["params"]["parentFrameId"] == json!("TID-1")
                && message["sessionId"] == json!("SID-1")
        }));
        assert!(out.iter().any(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!("CHILD-FRAME-1")
                && message["params"]["frame"]["url"] == json!("https://example.test/child")
                && message["sessionId"] == json!("SID-1")
        }));
        assert!(out.iter().any(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!("CHILD-FRAME-1")
        }));
        assert!(out.iter().any(|message| {
            message["method"] == json!("Page.frameStartedNavigating")
                && message["params"]["frameId"] == json!("CHILD-FRAME-1")
        }));
        assert!(
            !out.iter().any(|message| matches!(
                message["method"].as_str(),
                Some(
                    "Page.frameScheduledNavigation"
                        | "Page.frameRequestedNavigation"
                        | "Page.frameClearedScheduledNavigation"
                )
            )),
            "Page.navigate(frameId=child) must not fabricate renderer navigation probes: {out:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_frame_activity_emits_navigation_before_init_before_lifecycle_terminal() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_lifecycle_events = true;
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let mut background_events = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_child_frame_activity_for_test(
                    root_document_attachment_for_test(&conn, "SID-1", source_document),
                ),
            ));
        let activity = prepared
            .page_mut()
            .and_then(super::PagePreparedOutputSlot::take_child_frame_activity)
            .and_then(|mut activities| activities.pop())
            .expect("test prepared output should carry one child frame activity");

        super::emit_prepared_child_frame_activity(
            &mut conn,
            &mut background_events,
            activity,
            None,
        )
        .await;
        let out = protocol_messages_from_background_events(background_events);

        let navigated_index = out
            .iter()
            .position(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["id"] == json!("CHILD-FRAME-1")
            })
            .expect("child frameNavigated should be emitted");
        let init_index = out
            .iter()
            .position(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["frameId"] == json!("CHILD-FRAME-1")
                    && message["params"]["name"] == json!("init")
            })
            .expect("child init lifecycle should be emitted");
        let stopped_index = out
            .iter()
            .position(|message| {
                message["method"] == json!("Page.frameStoppedLoading")
                    && message["params"]["frameId"] == json!("CHILD-FRAME-1")
            })
            .expect("child frameStoppedLoading should be emitted");

        assert!(
            navigated_index < init_index,
            "frameNavigated must precede child init lifecycle output"
        );
        assert!(
            init_index < stopped_index,
            "child lifecycle terminal markers must precede stoppedLoading"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_frame_activity_fans_out_page_events_to_enabled_auxiliary_session() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-child-page-fanout".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-child-page-fanout");
        bc.attach_active_session("SID-primary");
        assert!(bc.assign_auxiliary_session_to_target(
            "TID-child-page-fanout",
            "SID-auxiliary".to_owned(),
        ));
        conn.browser_context = Some(bc);
        conn.with_target_devtools_session_state_for_session_mut(Some("SID-auxiliary"), |state| {
            state.page_session_state.page_domain_enabled = true;
            state.page_session_state.page_lifecycle_events = true;
        })
        .expect("auxiliary session should expose Page state");

        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-primary",
            "TID-child-page-fanout",
            source_document,
        );
        let activity =
            default_prepared_child_frame_activity_for_test(&conn, "SID-primary", source_document);
        let mut background_events = Vec::new();

        super::emit_prepared_child_frame_activity(
            &mut conn,
            &mut background_events,
            activity,
            None,
        )
        .await;
        let out = protocol_messages_from_background_events(background_events);

        for method in [
            "Page.frameAttached",
            "Page.frameNavigated",
            "Page.frameStoppedLoading",
        ] {
            assert!(
                out.iter().any(|message| {
                    message["sessionId"] == json!("SID-auxiliary")
                        && message["method"] == json!(method)
                }),
                "Page-enabled auxiliary session should receive {method}: {out:?}"
            );
            assert!(
                !out.iter().any(|message| {
                    message["sessionId"] == json!("SID-primary")
                        && message["method"] == json!(method)
                }),
                "Page-disabled primary session must not receive {method}: {out:?}"
            );
        }
        assert!(out.iter().any(|message| {
            message["sessionId"] == json!("SID-auxiliary")
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("CHILD-FRAME-1")
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_frame_activity_projects_sandboxed_about_blank_from_document_url() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://top.example/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let mut background_events = Vec::new();
        let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
            12.5,
            vec![super::PagePreparedChildFrameTreeEvent::Attached {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: "TID-1".to_owned(),
            }],
            Vec::new(),
            Vec::new(),
            vec![ChildFrameNavigationSnapshot {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: Some("TID-1".to_owned()),
                loader_id: Some("LID-CHILD-1".to_owned()),
                name: Some("sandboxed-blank".to_owned()),
                url: "about:blank".to_owned(),
                document_open_replacement: false,
                security_origin_inherited: true,
                security_origin_opaque: true,
                document_network: None,
            }],
            "https://top.example".to_owned(),
            "Secure".to_owned(),
        );
        let activity =
            prepared_child_frame_activity_for_test(&conn, "SID-1", source_document, document);

        super::emit_prepared_child_frame_activity(
            &mut conn,
            &mut background_events,
            activity,
            None,
        )
        .await;
        let out = protocol_messages_from_background_events(background_events);

        let navigated = out
            .iter()
            .find(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["id"] == json!("CHILD-FRAME-1")
            })
            .expect("child frame navigation event");
        // Page.Frame projects securityOrigin from DocumentLoader::Url while
        // secureContextType still reflects the live inherited security state.
        assert_eq!(navigated["params"]["frame"]["securityOrigin"], json!("://"));
        assert_eq!(
            navigated["params"]["frame"]["secureContextType"],
            json!("Secure")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_frame_activity_emits_document_network_events_from_prepared_load() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-AUXILIARY".to_owned(),));
        conn.browser_context = Some(bc);
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-1")));
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-AUXILIARY")));
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let mut background_events = Vec::new();
        let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
            12.5,
            vec![super::PagePreparedChildFrameTreeEvent::Attached {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: "TID-1".to_owned(),
            }],
            Vec::new(),
            Vec::new(),
            vec![ChildFrameNavigationSnapshot {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: Some("TID-1".to_owned()),
                loader_id: Some("LID-CHILD-1".to_owned()),
                name: Some("child-frame".to_owned()),
                url: "https://example.test/child".to_owned(),
                document_open_replacement: false,
                security_origin_inherited: false,
                security_origin_opaque: false,
                document_network: Some(ChildFrameDocumentNetworkSnapshot {
                    request_url: "https://example.test/child".to_owned(),
                    request_method: "GET".to_owned(),
                    request_headers: vec![("Accept".to_owned(), "text/html".to_owned())],
                    final_url: "https://example.test/child".to_owned(),
                    status: 200,
                    response_headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
                    encoded_data_length: 3,
                    response_body: Some(SubresourceResponseBody::from_text_and_bytes(
                        "\0\u{fffd}a".to_owned(),
                        vec![0x00, 0xff, b'a'],
                    )),
                    from_cache: true,
                }),
            }],
            "https://example.test".to_owned(),
            "Secure".to_owned(),
        );
        let activity =
            prepared_child_frame_activity_for_test(&conn, "SID-1", source_document, document);

        super::emit_prepared_child_frame_activity(
            &mut conn,
            &mut background_events,
            activity,
            None,
        )
        .await;
        let out = protocol_messages_from_background_events(background_events);

        let request = out
            .iter()
            .find(|message| message["method"] == json!("Network.requestWillBeSent"))
            .expect("child document request event");
        assert_eq!(request["sessionId"], json!("SID-1"));
        assert_eq!(request["params"]["frameId"], json!("CHILD-FRAME-1"));
        assert_eq!(request["params"]["loaderId"], json!("LID-CHILD-1"));
        assert_eq!(
            request["params"]["request"]["url"],
            json!("https://example.test/child")
        );
        assert_eq!(request["params"]["type"], json!("Document"));

        let request_index = out
            .iter()
            .position(|message| message["method"] == json!("Network.requestWillBeSent"))
            .expect("child document request event index");
        let cached_index = out
            .iter()
            .position(|message| message["method"] == json!("Network.requestServedFromCache"))
            .expect("child document cache event");
        let response = out
            .iter()
            .find(|message| message["method"] == json!("Network.responseReceived"))
            .expect("child document response event");
        let response_index = out
            .iter()
            .position(|message| message["method"] == json!("Network.responseReceived"))
            .expect("child document response event index");
        assert!(request_index < cached_index && cached_index < response_index);
        assert_eq!(
            out[cached_index]["params"]["requestId"],
            json!("LID-CHILD-1")
        );
        assert_eq!(response["params"]["frameId"], json!("CHILD-FRAME-1"));
        assert_eq!(response["params"]["loaderId"], json!("LID-CHILD-1"));
        assert_eq!(
            response["params"]["response"]["url"],
            json!("https://example.test/child")
        );
        assert_eq!(response["params"]["response"]["status"], json!(200));
        assert_eq!(response["params"]["response"]["fromDiskCache"], json!(true));

        let finished = out
            .iter()
            .find(|message| message["method"] == json!("Network.loadingFinished"))
            .expect("child document loadingFinished event");
        assert_eq!(finished["params"]["requestId"], json!("LID-CHILD-1"));
        assert_eq!(finished["params"]["encodedDataLength"], json!(3));
        assert!(out.iter().any(|message| {
            message["sessionId"] == json!("SID-AUXILIARY")
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!("LID-CHILD-1")
        }));

        let mut ctx = TestContext::from_conn(conn);
        ctx.process_async(json!({
            "id": 7_501,
            "method": "Network.getResponseBody",
            "sessionId": "SID-1",
            "params": { "requestId": "LID-CHILD-1" }
        }))
        .await;
        ctx.expect_result(
            7_501,
            json!({ "body": "AP9h", "base64Encoded": true }),
            Some("SID-1"),
        );
        ctx.process_async(json!({
            "id": 7_504,
            "method": "Network.getResponseBody",
            "sessionId": "SID-AUXILIARY",
            "params": { "requestId": "LID-CHILD-1" }
        }))
        .await;
        ctx.expect_result(
            7_504,
            json!({ "body": "AP9h", "base64Encoded": true }),
            Some("SID-AUXILIARY"),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_child_document_response_emits_network_without_navigation_or_lifecycle() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_lifecycle_events = true;
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(bc);
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-1")));
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
            19.25,
            Vec::new(),
            Vec::new(),
            vec![ChildFrameDocumentNetworkActivitySnapshot {
                frame_id: "RETIRED-CHILD-FRAME".to_owned(),
                parent_frame_id: Some("TID-1".to_owned()),
                loader_id: "LID-RETIRED-CHILD".to_owned(),
                snapshot: ChildFrameDocumentNetworkSnapshot {
                    request_url: "https://example.test/retired-child".to_owned(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    final_url: "https://example.test/retired-child".to_owned(),
                    status: 200,
                    response_headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
                    encoded_data_length: 21,
                    response_body: Some(SubresourceResponseBody::from_text(
                        "historical child body".to_owned(),
                    )),
                    from_cache: false,
                },
            }],
            Vec::new(),
            "https://example.test".to_owned(),
            "Secure".to_owned(),
        );
        let activity =
            prepared_child_frame_activity_for_test(&conn, "SID-1", source_document, document);
        let mut background_events = Vec::new();

        super::emit_prepared_child_frame_activity(
            &mut conn,
            &mut background_events,
            activity,
            None,
        )
        .await;
        let out = protocol_messages_from_background_events(background_events);

        assert_eq!(
            out.iter()
                .filter(|message| {
                    matches!(
                        message["method"].as_str(),
                        Some(
                            "Network.requestWillBeSent"
                                | "Network.responseReceived"
                                | "Network.dataReceived"
                                | "Network.loadingFinished"
                        )
                    )
                })
                .count(),
            4,
            "historical response should retain its complete Network event family: {out:?}"
        );
        assert_eq!(
            out.iter()
                .filter_map(|message| message["method"].as_str())
                .filter(|method| method.starts_with("Network."))
                .collect::<Vec<_>>(),
            vec![
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.dataReceived",
                "Network.loadingFinished",
            ],
            "historical response must preserve the request/response/data/finish protocol order"
        );
        assert!(
            out.iter().all(|message| {
                !matches!(
                    message["method"].as_str(),
                    Some(
                        "Page.frameNavigated"
                            | "Page.frameStartedLoading"
                            | "Page.frameStoppedLoading"
                            | "Page.frameStartedNavigating"
                            | "Page.frameRequestedNavigation"
                            | "Page.frameScheduledNavigation"
                            | "Page.lifecycleEvent"
                    )
                )
            }),
            "historical Network-only output must not imply a commit or lifecycle transition: {out:?}"
        );
        let request = out
            .iter()
            .find(|message| message["method"] == json!("Network.requestWillBeSent"))
            .expect("historical request event");
        assert_eq!(request["params"]["frameId"], json!("RETIRED-CHILD-FRAME"));
        assert_eq!(request["params"]["loaderId"], json!("LID-RETIRED-CHILD"));

        let mut ctx = TestContext::from_conn(conn);
        ctx.process_async(json!({
            "id": 7_502,
            "method": "Network.getResponseBody",
            "sessionId": "SID-1",
            "params": { "requestId": "LID-RETIRED-CHILD" }
        }))
        .await;
        ctx.expect_result(
            7_502,
            json!({ "body": "historical child body", "base64Encoded": false }),
            Some("SID-1"),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_document_network_without_body_records_known_no_data() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        conn.browser_context = Some(bc);
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-1")));
        let snapshot = ChildFrameDocumentNetworkSnapshot {
            request_url: "https://example.test/legacy-child".to_owned(),
            request_method: "GET".to_owned(),
            request_headers: Vec::new(),
            final_url: "https://example.test/legacy-child".to_owned(),
            status: 200,
            response_headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
            encoded_data_length: 0,
            response_body: None,
            from_cache: false,
        };
        let mut background_events = Vec::new();

        crate::domains::network::emit_child_document_navigation_network_background_events(
            &mut conn,
            &mut background_events,
            Some("SID-1"),
            "CHILD-FRAME-LEGACY",
            "LID-CHILD-LEGACY",
            "LID-CHILD-LEGACY",
            12.5,
            &snapshot,
        );

        let messages = protocol_messages_from_background_events(background_events);
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == json!("Network.loadingFinished"))
        );
        let mut ctx = TestContext::from_conn(conn);
        ctx.process_async(json!({
            "id": 7_503,
            "method": "Network.getResponseBody",
            "sessionId": "SID-1",
            "params": { "requestId": "LID-CHILD-LEGACY" }
        }))
        .await;
        ctx.expect_error(
            7_503,
            -32000,
            "No data found for resource with given identifier",
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_frame_activity_drain_preserves_prepared_attachment_only_token() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let document = super::PagePreparedChildFrameDocumentActivity::from_parts(
            12.5,
            vec![super::PagePreparedChildFrameTreeEvent::Attached {
                frame_id: "CHILD-FRAME-ATTACH-ONLY".to_owned(),
                parent_frame_id: "TID-1".to_owned(),
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            "https://example.test".to_owned(),
            "Secure".to_owned(),
        );
        let outputs = super::PagePreparedOutputs {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: vec![super::PagePreparedChildFrameActivity::from_document(
                root_document_attachment_for_test(&conn, "SID-1", source_document),
                document,
            )],
            same_document_navigations: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        };
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(outputs));
        let mut command_context = crate::conn::CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-1"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::PageOutputProjectionStep::ChildFrameActivity
            .project_async(&mut conn, &mut context, Some(&mut prepared))
            .await;

        let events = context
            .command
            .take_protocol_events()
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|message| message["method"] == json!("Page.frameAttached"))
                .count(),
            1,
            "prepared child-frame completion must not drop attachment-only tokens"
        );
        assert!(events.iter().any(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!("CHILD-FRAME-ATTACH-ONLY")
                && message["params"]["parentFrameId"] == json!("TID-1")
                && message["sessionId"] == json!("SID-1")
        }));
        assert!(
            events
                .iter()
                .all(|message| message["method"] != json!("Page.frameNavigated")),
            "attachment-only child-frame token should not synthesize navigation events"
        );
        assert!(
            conn.runtime_session_owner_slot(Some("SID-1"))
                .expect("runtime owner slot should exist")
                .loaded_page()
                .is_none(),
            "prepared attachment-only emission must not require live page readback"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_child_frame_activity_does_not_follow_replacement_page_residence() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-child-page-owner".into());
        browser_context.set_target_url("https://example.test/page".to_owned());
        browser_context.set_active_target_id("TID-child-page-owner");
        browser_context.attach_active_session("SID-child-page-owner");
        conn.browser_context = Some(browser_context);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-child-page-owner",
            "TID-child-page-owner",
            source_document,
        );
        let activity = default_prepared_child_frame_activity_for_test(
            &conn,
            "SID-child-page-owner",
            source_document,
        );
        conn.runtime_session_owner_slot_mut(Some("SID-child-page-owner"))
            .expect("test runtime owner")
            .replace_page_attachment_id_for_test();

        let mut events = Vec::new();
        super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

        assert!(
            events.is_empty(),
            "retired Page output must not be projected through its replacement attachment"
        );
        assert!(
            !conn.has_attached_child_frame_id("CHILD-FRAME-1"),
            "retired Page output must not mutate the replacement attached-frame registry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_child_frame_activity_does_not_follow_root_document_open_replacement() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-child-root-document".into());
        browser_context.set_target_url("https://example.test/page".to_owned());
        browser_context.set_active_target_id("TID-child-root-document");
        browser_context.attach_active_session("SID-child-root-document");
        conn.browser_context = Some(browser_context);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-child-root-document",
            "TID-child-root-document",
            source_document,
        );
        let activity = default_prepared_child_frame_activity_for_test(
            &conn,
            "SID-child-root-document",
            source_document,
        );
        bind_renderer_document_for_test(
            &mut conn,
            "SID-child-root-document",
            "TID-child-root-document",
            renderer_document_identity_for_test(2, 2),
        );

        let mut events = Vec::new();
        super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

        assert!(
            events.is_empty(),
            "old root Document output must not appear in the document.open replacement"
        );
        assert!(
            !conn.has_attached_child_frame_id("CHILD-FRAME-1"),
            "old root Document output must not mutate the replacement child-frame registry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_child_frame_activity_keeps_root_document_route_until_delivery() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-child-delivery-route".into());
        browser_context.set_target_url("https://example.test/page".to_owned());
        browser_context.set_active_target_id("TID-child-delivery-route");
        browser_context.attach_active_session("SID-child-delivery-route");
        browser_context
            .devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(browser_context);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-child-delivery-route",
            "TID-child-delivery-route",
            source_document,
        );
        let activity = default_prepared_child_frame_activity_for_test(
            &conn,
            "SID-child-delivery-route",
            source_document,
        );

        let mut events = Vec::new();
        super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

        assert!(
            !events.is_empty(),
            "live child activity should produce output"
        );
        assert!(events.iter().all(|event| event.route_is_current(&conn)));

        // Projection and scheduler delivery are separate steps. A replacement
        // root Document may commit between them, so the concrete events must
        // retain their exact Document route instead of inheriting the new
        // target's still-live session.
        bind_renderer_document_for_test(
            &mut conn,
            "SID-child-delivery-route",
            "TID-child-delivery-route",
            renderer_document_identity_for_test(2, 2),
        );

        assert!(
            events.iter().all(|event| !event.route_is_current(&conn)),
            "already-projected child output must not enter the replacement Document's FIFO"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_child_frame_activity_does_not_follow_detached_protocol_session() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-child-session".into());
        browser_context.set_target_url("https://example.test/page".to_owned());
        browser_context.set_active_target_id("TID-child-session");
        browser_context.attach_active_session("SID-child-session");
        conn.browser_context = Some(browser_context);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-child-session",
            "TID-child-session",
            source_document,
        );
        let activity = default_prepared_child_frame_activity_for_test(
            &conn,
            "SID-child-session",
            source_document,
        );
        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("test browser context")
                .detach_active_session()
                .as_deref(),
            Some("SID-child-session")
        );

        let mut events = Vec::new();
        super::emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;

        assert!(
            events.is_empty(),
            "held output must not be routed after its exact protocol attachment detaches"
        );
        assert!(
            !conn.has_attached_child_frame_id("CHILD-FRAME-1"),
            "detached-session output must not mutate target-wide child-frame state"
        );
    }

    #[test]
    fn child_frame_tree_emission_deduplicates_attach_and_removes_owner_state_on_detach() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("about:blank".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        conn.browser_context = Some(bc);
        let child_frame_id = "CHILD-FRAME-1".to_owned();
        let mut emitted = Vec::new();
        super::emit_prepared_child_frame_tree_background_events(
            &mut conn,
            &mut emitted,
            Some("SID-1"),
            vec![super::PagePreparedChildFrameTreeEvent::Attached {
                frame_id: child_frame_id.clone(),
                parent_frame_id: "TID-1".to_owned(),
            }],
        );
        assert_eq!(emitted.len(), 1);
        assert!(
            conn.target_owner_state_for_session(Some("SID-1"))
                .expect("owner state should exist")
                .has_attached_child_frame_id(&child_frame_id),
            "initial child-frame attachment should be recorded before the next prepare"
        );
        super::emit_prepared_child_frame_tree_background_events(
            &mut conn,
            &mut emitted,
            Some("SID-1"),
            vec![
                super::PagePreparedChildFrameTreeEvent::Attached {
                    frame_id: child_frame_id.clone(),
                    parent_frame_id: "TID-1".to_owned(),
                },
                super::PagePreparedChildFrameTreeEvent::Detached {
                    frame_id: child_frame_id.clone(),
                },
            ],
        );

        assert_eq!(
            emitted
                .iter()
                .map(BackgroundProtocolEvent::protocol_method)
                .collect::<Vec<_>>(),
            vec![Some("Page.frameAttached"), Some("Page.frameDetached")],
            "duplicate attach should be suppressed without suppressing the following detach"
        );
        assert!(
            !conn
                .target_owner_state_for_session(Some("SID-1"))
                .expect("owner state should exist")
                .has_attached_child_frame_id(&child_frame_id),
            "detach must remove the child frame from the owner state"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_frame_activity_drain_requires_prepared_output() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("https://example.test/page".to_owned());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        conn.browser_context = Some(bc);
        let mut command_context = crate::conn::CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-1"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::PageOutputProjectionStep::ChildFrameActivity
            .project_async(&mut conn, &mut context, None)
            .await;

        assert!(
            context.command.take_protocol_events().is_empty(),
            "child-frame completion drain should not emit from live page state without prepared output"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn popup_activation_creates_target_and_schedules_navigation_without_page_readback() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active");
        bc.attach_active_session("SID-1");
        conn.browser_context = Some(bc);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-1");
        let source_document = renderer_document_identity_for_test(1, 1);
        let mut out = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_popup_activations_for_test(
                    page_owner,
                    vec![RendererPendingPopupActivation::window(
                        source_document,
                        RendererWindowDocumentSource::RootFrame,
                        true,
                        None,
                        "data:text/html,%3Cmain%3Eprepared-popup%3C/main%3E".to_owned(),
                        "_blank".to_owned(),
                    )],
                ),
            ));

        super::emit_popup_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-1"),
            Some(&mut prepared),
        )
        .await;

        let events = out
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        let (target_created, target_created_sidecar) = events
            .iter()
            .find(|(message, _)| message["method"] == json!("Target.targetCreated"))
            .unwrap_or_else(|| panic!("missing Target.targetCreated event: {events:?}"));
        assert_eq!(
            target_created["params"]["targetInfo"]["url"],
            json!("data:text/html,%3Cmain%3Eprepared-popup%3C/main%3E")
        );
        assert!(matches!(
            target_created_sidecar,
            Some(AutomationEvent::TargetCreated(event))
                if event.url == "data:text/html,%3Cmain%3Eprepared-popup%3C/main%3E"
        ));
        assert!(
            events
                .iter()
                .all(|(message, _)| message["method"] != json!("Page.frameNavigated")),
            "the opener action must not join the popup Page stream; its concrete commit is published by that stream: {events:?}"
        );
        assert_eq!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .background_targets
                .len(),
            1,
            "prepared popup should create the owner popup target without reading a loaded page"
        );
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .background_targets
                .first()
                .and_then(|target| target.loaded_page())
                .is_some_and(|page| moli_url::is_about_blank(page.final_url())),
            "target creation should install only the initial empty Document"
        );
        let scheduler_events = conn.take_scheduler_events();
        assert!(matches!(
            scheduler_events.as_slice(),
            [crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }]
                if work.kind()
                    == crate::domains::activity::ProtocolSchedulerWorkKind::PopupTargetNavigationOwnerAction
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn popup_activation_publishes_automation_lifecycle_without_cdp_discovery() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-automation".into());
        bc.set_active_target_id("TID-opener");
        bc.attach_active_session("SID-opener");
        conn.browser_context = Some(bc);
        let page_owner = page_residence_identity_for_test(&mut conn, "SID-opener");
        let source_document = renderer_document_identity_for_test(1, 1);
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_popup_activations_for_test(
                    page_owner,
                    vec![RendererPendingPopupActivation::window(
                        source_document,
                        RendererWindowDocumentSource::RootFrame,
                        true,
                        None,
                        "data:text/html,%3Cmain%3Eautomation-popup%3C/main%3E".to_owned(),
                        "_blank".to_owned(),
                    )],
                ),
            ));
        let mut out = Vec::new();

        super::emit_popup_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-opener"),
            Some(&mut prepared),
        )
        .await;

        let events = out
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .all(|(message, _)| message["method"] != json!("Target.targetCreated")),
            "CDP discovery must remain the gate for Target.targetCreated: {events:?}"
        );
        assert!(
            events.iter().any(|(_, event)| {
                matches!(
                    event,
                    Some(AutomationEvent::TargetCreated(event))
                        if event.url
                            == "data:text/html,%3Cmain%3Eautomation-popup%3C/main%3E"
                )
            }),
            "popup creation must publish its internal browsing-context lifecycle fact even without CDP discovery: {events:?}"
        );
    }

    #[test]
    #[should_panic(expected = "popup activation must not carry an existing-context special target")]
    fn popup_carrier_rejects_existing_context_special_targets() {
        let _ = RendererPendingPopupActivation::window(
            renderer_document_identity_for_test(1, 1),
            RendererWindowDocumentSource::RootFrame,
            true,
            None,
            "https://example.test/self".to_owned(),
            "_self".to_owned(),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_document_drain_consumes_prepared_navigations_without_page_readback() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.set_target_url("https://example.test/page".to_owned());
        bc.attach_active_session("SID-1");
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "TID-1", source_document);
        let mut out = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_same_document_navigations_for_test(
                    page_residence_identity_for_test(&mut conn, "SID-1"),
                    vec![document_sourced_same_document_navigation_for_test(
                        source_document,
                        "https://example.test/page#prepared",
                    )],
                ),
            ));

        super::emit_same_document_navigation_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-1"),
            Some(&mut prepared),
        )
        .await;

        assert!(
            conn.runtime_session_owner_slot(Some("SID-1"))
                .expect("runtime owner slot should exist")
                .loaded_page()
                .is_none(),
            "prepared same-document navigation emission must not require a loaded page"
        );
        assert_eq!(out.len(), 1);
        assert!(
            out[0].protocol_message().is_none(),
            "same-document navigation should stay typed until wire projection"
        );
        assert_eq!(
            out[0].protocol_method(),
            Some("Page.navigatedWithinDocument")
        );
        assert!(out[0].has_protocol_wire_message());
        let events = out
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0["method"], json!("Page.navigatedWithinDocument"));
        assert_eq!(events[0].0["params"]["frameId"], json!("TID-1"));
        assert_eq!(
            events[0].0["params"]["url"],
            json!("https://example.test/page#prepared")
        );
        assert_eq!(events[0].0["params"]["navigationType"], json!("fragment"));
        assert_eq!(events[0].0["sessionId"], json!("SID-1"));
        assert!(matches!(
            events[0].1.as_ref(),
            Some(AutomationEvent::SameDocumentNavigation(event))
                if event.frame_id.as_str() == "TID-1"
                    && event.url == "https://example.test/page#prepared"
                    && event.navigation_type == "fragment"
        ));
        assert_eq!(
            conn.browser_context.as_ref().unwrap().target_url(),
            "https://example.test/page#prepared",
            "prepared same-document navigation should still update owner URL state"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn document_open_replacement_keeps_same_document_navigation_handoff() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-document-open-same-document".into());
        bc.set_active_target_id("TID-document-open-same-document");
        bc.set_target_url("https://example.test/source".to_owned());
        bc.attach_active_session("SID-document-open-same-document");
        conn.browser_context = Some(bc);

        let source_document = renderer_document_identity_for_test(1, 1);
        let replacement_document = renderer_document_identity_for_test(2, 2);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-document-open-same-document",
            "TID-document-open-same-document",
            source_document,
        );
        let owner = page_residence_identity_for_test(&mut conn, "SID-document-open-same-document");
        bind_renderer_document_for_test(
            &mut conn,
            "SID-document-open-same-document",
            "TID-document-open-same-document",
            replacement_document,
        );
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_same_document_navigations_for_test(
                    owner,
                    vec![document_sourced_same_document_navigation_for_test(
                        source_document,
                        "https://example.test/source#preserved",
                    )],
                ),
            ));
        let mut out = Vec::new();

        super::emit_same_document_navigation_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-document-open-same-document"),
            Some(&mut prepared),
        )
        .await;

        assert_eq!(
            out.len(),
            1,
            "document.open must not erase prior history output"
        );
        assert_eq!(
            conn.browser_context.as_ref().unwrap().target_url(),
            "https://example.test/source#preserved",
            "same-Document history mutation survives replacement of only the Document shell"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_page_residence_same_document_navigation_cannot_mutate_replacement() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-stale-page-same-document".into());
        bc.set_active_target_id("TID-stale-page-same-document");
        bc.set_target_url("https://example.test/replacement".to_owned());
        bc.attach_active_session("SID-stale-page-same-document");
        conn.browser_context = Some(bc);

        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-stale-page-same-document",
            "TID-stale-page-same-document",
            source_document,
        );
        let owner = page_residence_identity_for_test(&mut conn, "SID-stale-page-same-document");
        conn.runtime_session_owner_slot_mut(Some("SID-stale-page-same-document"))
            .expect("test runtime slot should exist")
            .replace_page_attachment_id_for_test();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_same_document_navigations_for_test(
                    owner,
                    vec![document_sourced_same_document_navigation_for_test(
                        source_document,
                        "https://example.test/replacement#stale",
                    )],
                ),
            ));
        let mut out = Vec::new();

        super::emit_same_document_navigation_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-stale-page-same-document"),
            Some(&mut prepared),
        )
        .await;

        assert!(
            out.is_empty(),
            "a retired Page residence must emit no event"
        );
        assert_eq!(
            conn.browser_context.as_ref().unwrap().target_url(),
            "https://example.test/replacement",
            "a retired Page's output must not update replacement target state"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_top_level_location_navigation_waits_for_its_scheduler_turn() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-location".into());
        bc.set_active_target_id("TID-location");
        bc.set_target_url("about:blank".to_owned());
        bc.attach_active_session("SID-location");
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-location", "TID-location", source_document);

        let target_url = "data:text/html,%3Cmain%3Eprepared-location%3C/main%3E".to_owned();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                    page_residence_identity_for_test(&mut conn, "SID-location"),
                    Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                        source_document,
                        target_url.clone(),
                    )),
                ),
            ));

        super::publish_prepared_top_level_location_navigation_owner_action(
            &mut conn,
            Some("SID-location"),
            Some(&mut prepared),
        );

        assert_eq!(
            conn.browser_context.as_ref().unwrap().target_url(),
            "about:blank",
            "capturing prepared output must not execute its owner action"
        );
        assert!(
            !conn.has_pending_document_navigation_for_session_owner(Some("SID-location")),
            "capturing prepared output must not start navigation"
        );

        let work = take_top_level_location_navigation_work_for_test(&mut conn);
        let (events, scheduler_events) = conn
            .complete_ready_protocol_scheduler_work_turn(work)
            .await
            .into_protocol_event_parts();
        assert!(!scheduler_events.iter().any(|event| {
            matches!(
                event,
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.is_top_level_location_navigation_owner_action()
            )
        }));
        let events = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        let (message, automation_event) = events
            .iter()
            .find(|(message, _)| message["method"] == json!("Page.frameStartedNavigating"))
            .expect("prepared top-level location navigation should emit frameStartedNavigating");

        assert_eq!(message["sessionId"], json!("SID-location"));
        assert_eq!(message["params"]["frameId"], json!("TID-location"));
        assert_eq!(message["params"]["loaderId"], json!(super::LOADER_ID));
        assert_eq!(message["params"]["url"], json!(target_url));
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::NavigationFrame(event))
                if event.kind == NavigationFrameEventKind::StartedNavigating
                    && event.frame_id.as_str() == "TID-location"
                    && event.loader_id.as_ref().map(|id| id.as_str()) == Some(super::LOADER_ID)
                    && event.url == target_url
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn document_open_replacement_keeps_requested_top_level_navigation() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-document-open-location".into());
        bc.set_active_target_id("TID-document-open-location");
        bc.set_target_url("https://example.test/source".to_owned());
        bc.attach_active_session("SID-document-open-location");
        conn.browser_context = Some(bc);

        let source_document = renderer_document_identity_for_test(1, 1);
        let replacement_document = renderer_document_identity_for_test(2, 2);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-document-open-location",
            "TID-document-open-location",
            source_document,
        );
        let owner = page_residence_identity_for_test(&mut conn, "SID-document-open-location");
        bind_renderer_document_for_test(
            &mut conn,
            "SID-document-open-location",
            "TID-document-open-location",
            replacement_document,
        );
        let target_url = "data:text/html,%3Cmain%3Epreserved%3C/main%3E".to_owned();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                    owner,
                    Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                        source_document,
                        target_url.clone(),
                    )),
                ),
            ));
        super::publish_prepared_top_level_location_navigation_owner_action(
            &mut conn,
            Some("SID-document-open-location"),
            Some(&mut prepared),
        );
        let work = take_top_level_location_navigation_work_for_test(&mut conn);
        let (out, scheduler_events) = conn
            .complete_ready_protocol_scheduler_work_turn(work)
            .await
            .into_protocol_event_parts();
        assert!(!scheduler_events.iter().any(|event| {
            matches!(
                event,
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.is_top_level_location_navigation_owner_action()
            )
        }));

        assert!(
            out.iter().any(|event| {
                event.protocol_method() == Some("Page.frameStartedNavigating")
                    && event.protocol_message().is_none()
            }),
            "document.open must not cancel a navigation already requested by the same Page"
        );
        // A data: navigation may complete and clear its pending token before
        // this helper returns; the typed frame-start event proves the action
        // was admitted rather than discarded as a stale Document.
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_page_residence_top_level_navigation_cannot_replace_current_page() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-stale-page-location".into());
        bc.set_active_target_id("TID-stale-page-location");
        bc.set_target_url("https://example.test/replacement".to_owned());
        bc.attach_active_session("SID-stale-page-location");
        conn.browser_context = Some(bc);

        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-stale-page-location",
            "TID-stale-page-location",
            source_document,
        );
        let owner = page_residence_identity_for_test(&mut conn, "SID-stale-page-location");
        conn.runtime_session_owner_slot_mut(Some("SID-stale-page-location"))
            .expect("test runtime slot should exist")
            .replace_page_attachment_id_for_test();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::PagePreparedOutputSlot::from_outputs(
                super::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                    owner,
                    Some(RendererDocumentSourcedTopLevelLocationNavigation::new(
                        source_document,
                        "data:text/html,%3Cmain%3Estale%3C/main%3E".to_owned(),
                    )),
                ),
            ));
        super::publish_prepared_top_level_location_navigation_owner_action(
            &mut conn,
            Some("SID-stale-page-location"),
            Some(&mut prepared),
        );
        let work = take_top_level_location_navigation_work_for_test(&mut conn);
        let (out, scheduler_events) = conn
            .complete_ready_protocol_scheduler_work_turn(work)
            .await
            .into_protocol_event_parts();
        assert!(scheduler_events.is_empty());

        assert!(out.is_empty(), "a retired Page must start no navigation");
        assert_eq!(
            conn.browser_context.as_ref().unwrap().target_url(),
            "https://example.test/replacement",
            "a retired Page's action must not navigate its replacement"
        );
        assert!(
            !conn
                .has_pending_document_navigation_for_session_owner(Some("SID-stale-page-location")),
            "discarding the retired Page action must install no navigation token"
        );
    }
}

fn get_frame_tree_command_output_plan(
    output_kind: FrameTreeCommandOutputKind,
    target_id: String,
    target_loader_id: String,
    target_url: String,
    target_unreachable_url: Option<String>,
    target_security_origin: String,
    target_secure_context_type: String,
    target_mime_type: String,
    child_frame_snapshots: Vec<ChildFrameTreeSnapshot>,
    resource_records: &[moli_core::page::SubresourceNetworkRecord],
) -> CommandOutputPlan {
    let frame_tree = frame_tree_payload(
        target_id,
        target_loader_id,
        target_url,
        target_unreachable_url,
        target_security_origin,
        target_secure_context_type,
        target_mime_type,
        child_frame_snapshots,
    );
    let frame_tree = match output_kind {
        FrameTreeCommandOutputKind::FrameTree => frame_tree,
        FrameTreeCommandOutputKind::ResourceTree => {
            resource_tree::attach_frame_resources(frame_tree, resource_records)
        }
    };
    CommandOutputPlan::result(json!({
        "frameTree": frame_tree
    }))
}

fn frame_tree_payload(
    target_id: String,
    target_loader_id: String,
    target_url: String,
    target_unreachable_url: Option<String>,
    target_security_origin: String,
    target_secure_context_type: String,
    target_mime_type: String,
    child_frame_snapshots: Vec<ChildFrameTreeSnapshot>,
) -> Value {
    let mut frame_tree = json!({
        "frame": {
            "id": target_id,
            "loaderId": target_loader_id,
            "url": target_url,
            "domainAndRegistry": "",
            "securityOrigin": target_security_origin,
            "mimeType": target_mime_type,
            "adFrameStatus": { "adFrameType": "none" },
            "secureContextType": target_secure_context_type,
            "crossOriginIsolatedContextType": "NotIsolated",
            "gatedAPIFeatures": [],
        }
    });
    if let Some(unreachable_url) = target_unreachable_url {
        frame_tree["frame"]["unreachableUrl"] = json!(unreachable_url);
    }
    let child_frames = child_frame_snapshots
        .into_iter()
        .map(|frame| {
            build_child_frame_tree_payload(
                &frame,
                &target_id,
                &target_security_origin,
                &target_secure_context_type,
            )
        })
        .collect::<Vec<_>>();
    if !child_frames.is_empty() {
        frame_tree["childFrames"] = Value::Array(child_frames);
    }
    frame_tree
}

fn build_child_frame_tree_payload(
    frame: &moli_core::page::ChildFrameTreeSnapshot,
    parent_frame_id: &str,
    inherited_security_origin: &str,
    inherited_secure_context_type: &str,
) -> Value {
    let (security_origin, secure_context_type) = child_frame_security_identity(
        &frame.url,
        frame.security_origin_inherited,
        frame.security_origin_opaque,
        inherited_security_origin,
        inherited_secure_context_type,
    );
    let name = frame
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .or(frame.owner_element_id.as_deref())
        .unwrap_or("");
    let frame_payload = json!({
        "id": frame.frame_id,
        "parentId": parent_frame_id,
        "loaderId": if frame.loader_id.is_empty() { LOADER_ID } else { &frame.loader_id },
        "name": name,
        "url": frame.url,
        "domainAndRegistry": "",
        "securityOrigin": security_origin.clone(),
        "mimeType": "text/html",
        "adFrameStatus": { "adFrameType": "none" },
        "secureContextType": secure_context_type.clone(),
        "crossOriginIsolatedContextType": "NotIsolated",
        "gatedAPIFeatures": [],
    });
    let mut payload = json!({ "frame": frame_payload });
    if !frame.child_frames.is_empty() {
        payload["childFrames"] = Value::Array(
            frame
                .child_frames
                .iter()
                .map(|child| {
                    build_child_frame_tree_payload(
                        child,
                        &frame.frame_id,
                        &security_origin,
                        &secure_context_type,
                    )
                })
                .collect(),
        );
    }
    payload
}

pub(crate) fn child_frame_security_identity(
    url: &str,
    security_origin_inherited: bool,
    _security_origin_opaque: bool,
    _inherited_security_origin: &str,
    inherited_secure_context_type: &str,
) -> (String, String) {
    let parsed_url = Url::parse(url).ok();
    // Blink's Page.Frame projection constructs this field from the
    // DocumentLoader URL, not from the document's live SecurityOrigin.
    let security_origin = parsed_url
        .as_ref()
        .map(|url| {
            if url.scheme() == "about" {
                "://".to_owned()
            } else {
                moli_url::origin_ascii_serialization(url)
            }
        })
        .unwrap_or_else(|| "null".to_owned());
    let inherited_secure_context =
        security_origin_inherited || parsed_url.as_ref().is_some_and(moli_url::is_about_blank);
    let secure_context_type = if inherited_secure_context {
        inherited_secure_context_type.to_owned()
    } else if parsed_url
        .as_ref()
        .is_some_and(moli_url::is_potentially_trustworthy_url)
    {
        "Secure".to_owned()
    } else {
        "InsecureScheme".to_owned()
    };
    (security_origin, secure_context_type)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotClip {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotParams {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    quality: Option<i32>,
    #[serde(default)]
    clip: Option<ScreenshotClip>,
    #[serde(default)]
    from_surface: Option<bool>,
    #[serde(default)]
    capture_beyond_viewport: Option<bool>,
    #[serde(default)]
    optimize_for_speed: Option<bool>,
}

fn unsupported_cdp_screenshot_option(option: &str) -> CommandOutputPlan {
    CommandOutputPlan::error(
        -32000,
        format!("Page.captureScreenshot option '{option}' is not supported."),
    )
}

fn build_cdp_capture_screenshot_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsCaptureScreenshotCommand, CommandOutputPlan> {
    let params: ScreenshotParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        Ok(None) => ScreenshotParams {
            format: None,
            quality: None,
            clip: None,
            from_surface: None,
            capture_beyond_viewport: None,
            optimize_for_speed: None,
        },
        Err(e) => return Err(CommandOutputPlan::error(-32602, e)),
    };
    match params.format.as_deref() {
        None | Some("png" | "jpeg") => {}
        Some("webp") => {
            return Err(unsupported_cdp_screenshot_option("format"));
        }
        Some(_) => {
            return Err(CommandOutputPlan::error(-32602, "Invalid image format"));
        }
    }
    let quality = params
        .quality
        .map(|quality| {
            u8::try_from(quality)
                .ok()
                .filter(|quality| *quality <= 100)
                .ok_or_else(|| {
                    CommandOutputPlan::error(
                        -32602,
                        "Page.captureScreenshot quality must be between 0 and 100.",
                    )
                })
        })
        .transpose()?;
    if let Some(clip) = params.clip.as_ref()
        && (!clip.x.is_finite()
            || !clip.y.is_finite()
            || !clip.width.is_finite()
            || !clip.height.is_finite()
            || !clip.scale.is_finite()
            || clip.width <= 0.0
            || clip.height <= 0.0
            || clip.scale <= 0.0)
    {
        return Err(CommandOutputPlan::error(
            -32602,
            "Page.captureScreenshot clip must have a finite origin and positive finite width, height, and scale.",
        ));
    }
    if params.from_surface == Some(false) {
        return Err(unsupported_cdp_screenshot_option("fromSurface"));
    }
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsCaptureScreenshotCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        format: params.format,
        quality,
        clip: params.clip.map(|clip| {
            DevToolsCaptureScreenshotClip::Box(DevToolsScreenshotClip {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
                scale: clip.scale,
            })
        }),
        capture_beyond_viewport: params.capture_beyond_viewport.unwrap_or(false),
        optimize_for_speed: params.optimize_for_speed.unwrap_or(false),
    })
}

fn devtools_capture_screenshot_error(command: &DevToolsCaptureScreenshotCommand) -> DevToolsError {
    match command.format.as_deref() {
        None | Some("png") => DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE,
        ),
        _ => DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "unsupported screenshot format.",
        ),
    }
}

fn try_start_page_capture_screenshot_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = match build_cdp_capture_screenshot_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return PageCommandTaskStep::Complete(plan),
    };
    start_devtools_page_command(conn, cmd.id, DevToolsCommand::CaptureScreenshot(command))
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapturePageSnapshotParams {
    #[serde(default)]
    format: Option<String>,
}

fn try_start_page_capture_snapshot_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: CapturePageSnapshotParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => CapturePageSnapshotParams::default(),
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32602, error));
        }
    };
    if !matches!(params.format.as_deref(), None | Some("mhtml")) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "unsupported snapshot format.",
        ));
    }
    let page = match conn.loaded_page_mut_for_protocol_access(cmd.session_id) {
        Ok(page) => page,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    match page.start_serialize_html() {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id: cmd.id,
            owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
            kind: Box::new(PendingPageCommandKind::CaptureSnapshot { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to serialize page snapshot: {error}"),
        )),
    }
}

fn build_mhtml_snapshot(url: &str, html: &str) -> String {
    let boundary = "----MultipartBoundary--moli";
    let content_location = sanitize_mhtml_header_value(url);
    let encoded_html = BASE64_STANDARD.encode(html.as_bytes());
    format!(
        concat!(
            "Snapshot-Content-Location: {content_location}\r\n",
            "Subject: \r\n",
            "Date: \r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/related;\r\n",
            "\ttype=\"text/html\";\r\n",
            "\tboundary=\"{boundary}\"\r\n",
            "\r\n",
            "--{boundary}\r\n",
            "Content-Type: text/html\r\n",
            "Content-ID: <frame-1@mhtml.moli>\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "Content-Location: {content_location}\r\n",
            "\r\n",
            "{encoded_html}\r\n",
            "--{boundary}--\r\n"
        ),
        boundary = boundary,
        content_location = content_location,
        encoded_html = encoded_html,
    )
}

fn sanitize_mhtml_header_value(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '\r' | '\n'))
        .collect()
}

fn default_print_to_pdf_params() -> PrintToPdfParams {
    PrintToPdfParams {
        landscape: None,
        display_header_footer: None,
        print_background: None,
        scale: None,
        paper_width: None,
        paper_height: None,
        margin_top: None,
        margin_bottom: None,
        margin_left: None,
        margin_right: None,
        page_ranges: None,
        header_template: None,
        footer_template: None,
        prefer_css_page_size: None,
        transfer_mode: None,
        generate_tagged_pdf: None,
        generate_document_outline: None,
    }
}

fn build_cdp_print_to_pdf_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsPrintToPdfCommand, CommandOutputPlan> {
    let params: PrintToPdfParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => default_print_to_pdf_params(),
        Err(error) => return Err(CommandOutputPlan::error(-32602, error)),
    };
    if params.display_header_footer.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option("displayHeaderFooter"));
    }
    if params.prefer_css_page_size.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option("preferCSSPageSize"));
    }
    if params.generate_tagged_pdf.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option("generateTaggedPDF"));
    }
    if params.generate_document_outline.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option(
            "generateDocumentOutline",
        ));
    }
    let transfer_mode = match params.transfer_mode {
        Some(PrintToPdfTransferMode::ReturnAsBase64) => {
            Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64)
        }
        Some(PrintToPdfTransferMode::ReturnAsStream) => {
            Some(DevToolsPrintToPdfTransferMode::ReturnAsStream)
        }
        None => None,
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsPrintToPdfCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        landscape: params.landscape,
        print_background: params.print_background,
        scale: params.scale,
        paper_width: params.paper_width,
        paper_height: params.paper_height,
        margin_top: params.margin_top,
        margin_bottom: params.margin_bottom,
        margin_left: params.margin_left,
        margin_right: params.margin_right,
        page_ranges: params.page_ranges,
        shrink_to_fit: None,
        transfer_mode,
    })
}

fn unsupported_cdp_print_to_pdf_option(option: &str) -> CommandOutputPlan {
    CommandOutputPlan::error(
        -32000,
        format!("Page.printToPDF option '{option}' is not supported."),
    )
}

fn devtools_print_to_pdf_error(command: &DevToolsPrintToPdfCommand) -> DevToolsError {
    if let Some(message) = print_to_pdf_page_size_error(command) {
        return DevToolsError::new(DevToolsErrorKind::Unsupported, message);
    }
    DevToolsError::new(
        DevToolsErrorKind::Unsupported,
        PRINT_TO_PDF_UNSUPPORTED_MESSAGE,
    )
}

fn complete_devtools_print_to_pdf_command(
    conn: &mut CdpConnection,
    command: DevToolsPrintToPdfCommand,
) -> CommandOutputPlan {
    if let Err(error) = validate_page_capture_target_context(conn, &command.context) {
        return CommandOutputPlan::from_devtools_error(error);
    }
    CommandOutputPlan::from_devtools_error(devtools_print_to_pdf_error(&command))
}

fn print_to_pdf_page_size_error(command: &DevToolsPrintToPdfCommand) -> Option<&'static str> {
    let paper_width = command
        .paper_width
        .unwrap_or(DEFAULT_PRINT_PAGE_WIDTH_INCHES);
    let paper_height = command
        .paper_height
        .unwrap_or(DEFAULT_PRINT_PAGE_HEIGHT_INCHES);
    let margin_left = command.margin_left.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    let margin_right = command.margin_right.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    let margin_top = command.margin_top.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    let margin_bottom = command.margin_bottom.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    if ![
        paper_width,
        paper_height,
        margin_left,
        margin_right,
        margin_top,
        margin_bottom,
    ]
    .into_iter()
    .all(|value| value.is_finite() && value >= 0.0)
    {
        return Some("invalid printToPDF page size or margin");
    }
    if paper_width <= margin_left + margin_right || paper_height <= margin_top + margin_bottom {
        return Some("printToPDF paper size is too small for margins");
    }
    None
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageSetDownloadBehaviorParams {
    behavior: String,
    #[serde(default)]
    download_path: Option<String>,
}

fn current_viewport_surface(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> EmulatedViewportSurface {
    EmulatedViewportSurface::from_metrics(
        conn.target_session_owner_emulated_device_metrics(session_id)
            .as_ref(),
    )
}

async fn execute_devtools_get_layout_metrics_command(
    conn: &mut CdpConnection,
    command: DevToolsGetLayoutMetricsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut command = command;
    let route = if let Some(target_id) = command.context.target_id.as_ref() {
        Some(page_route_for_context_id(conn, target_id.as_str())?)
    } else {
        None
    };
    if route.is_some() {
        command.context.session_id = None;
    }
    let result = if let Some(route) = route {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        execute_devtools_get_layout_metrics_for_current_owner(route_scope.conn_mut(), command).await
    } else {
        execute_devtools_get_layout_metrics_for_current_owner(conn, command).await
    };
    result.map(DevToolsCommandResult::LayoutMetrics)
}

async fn execute_devtools_get_layout_metrics_for_current_owner(
    conn: &mut CdpConnection,
    command: DevToolsGetLayoutMetricsCommand,
) -> Result<DevToolsLayoutMetricsResult, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let fallback = layout_metrics_result_from_surface(current_viewport_surface(conn, session_id));
    let Some(page) = conn
        .runtime_session_owner_slot_mut(session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(fallback);
    };
    let pending = page.start_layout_metrics().map_err(|error| {
        devtools_layout_metrics_error(format!("Failed to start layout metrics: {error}"))
    })?;
    let completed = pending.wait().await.map_err(|error| {
        devtools_layout_metrics_error(format!("Failed to produce layout metrics: {error}"))
    })?;
    let Some(page) = conn
        .runtime_session_owner_slot_mut(session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Err(devtools_layout_metrics_error("NoDocumentLoaded"));
    };
    page.finish_layout_metrics(completed)
        .map(layout_metrics_result_from_renderer)
        .map_err(|error| {
            devtools_layout_metrics_error(format!("Failed to finish layout metrics: {error}"))
        })
}

fn layout_metrics_result_from_surface(
    surface: EmulatedViewportSurface,
) -> DevToolsLayoutMetricsResult {
    DevToolsLayoutMetricsResult {
        layout_viewport_width: surface.inner_width,
        layout_viewport_height: surface.inner_height,
        page_x: 0.0,
        page_y: 0.0,
        content_width: f64::from(surface.inner_width),
        content_height: f64::from(surface.inner_height),
        device_pixel_ratio: surface.device_pixel_ratio,
    }
}

fn layout_metrics_result_from_renderer(
    metrics: RendererLayoutMetrics,
) -> DevToolsLayoutMetricsResult {
    DevToolsLayoutMetricsResult {
        layout_viewport_width: metrics.viewport_width,
        layout_viewport_height: metrics.viewport_height,
        page_x: metrics.page_x,
        page_y: metrics.page_y,
        content_width: metrics.content_width,
        content_height: metrics.content_height,
        device_pixel_ratio: metrics.device_pixel_ratio,
    }
}

fn devtools_layout_metrics_error(message: impl Into<String>) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, message)
}

pub(crate) fn try_start_page_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<PageCommandTaskStep> {
    match cmd.parse_action::<PageAction>() {
        Some(PageAction::Enable) => try_start_page_enable_command(conn, cmd),
        Some(
            PageAction::Disable
            | PageAction::SetLifecycleEventsEnabled
            | PageAction::SetFontFamilies
            | PageAction::SetInterceptFileChooserDialog
            | PageAction::HandleJavaScriptDialog,
        ) => Some(PageCommandTaskStep::Complete(command_output_plan(
            conn, cmd,
        ))),
        Some(PageAction::SetDownloadBehavior) => Some(PageCommandTaskStep::Complete(
            page_set_download_behavior_command_output_plan(conn, cmd),
        )),
        Some(PageAction::SetBypassCsp) => Some(try_start_set_bypass_csp_command(conn, cmd)),
        Some(PageAction::StartScreencast) => Some(PageCommandTaskStep::Complete(
            start_screencast_command(conn, cmd),
        )),
        Some(PageAction::StopScreencast) => Some(PageCommandTaskStep::Complete(
            stop_screencast_command(conn, cmd),
        )),
        Some(PageAction::ScreencastFrameAck) => Some(PageCommandTaskStep::Complete(
            screencast_frame_ack_command(conn, cmd),
        )),
        Some(PageAction::GetNavigationHistory) => Some(PageCommandTaskStep::Complete(
            navigation::get_navigation_history_command_output_plan(conn, cmd),
        )),
        Some(PageAction::ResetNavigationHistory) => Some(
            navigation::try_start_reset_navigation_history_command(conn, cmd),
        ),
        Some(PageAction::BringToFront) => Some(start_bring_to_front_command(conn, cmd)),
        Some(PageAction::CaptureScreenshot) => {
            Some(try_start_page_capture_screenshot_command(conn, cmd))
        }
        Some(PageAction::CaptureSnapshot) => {
            Some(try_start_page_capture_snapshot_command(conn, cmd))
        }
        Some(PageAction::PrintToPdf) => Some(try_start_page_print_to_pdf_command(conn, cmd)),
        Some(PageAction::SetDocumentContent) => {
            Some(try_start_page_set_document_content_command(conn, cmd))
        }
        Some(PageAction::GetFrameTree) => Some(try_start_page_get_frame_tree_command(conn, cmd)),
        Some(PageAction::GetResourceTree) => {
            Some(try_start_page_get_resource_tree_command(conn, cmd))
        }
        Some(PageAction::GetAppManifest) => {
            Some(app_manifest::try_start_get_app_manifest_command(conn, cmd))
        }
        Some(PageAction::SearchInResource) => Some(
            resource_search::try_start_search_in_resource_command(conn, cmd),
        ),
        Some(PageAction::GetLayoutMetrics) => {
            Some(try_start_page_get_layout_metrics_command(conn, cmd))
        }
        Some(PageAction::Navigate) => {
            Some(navigation::try_start_navigate_command_dispatch(conn, cmd))
        }
        Some(PageAction::NavigateToHistoryEntry) => {
            Some(navigation::try_start_navigate_to_history_entry_command_dispatch(conn, cmd))
        }
        Some(PageAction::Reload) => Some(navigation::try_start_reload_command_dispatch(conn, cmd)),
        Some(PageAction::StopLoading) => Some(
            termination::try_start_stop_loading_command_dispatch(conn, cmd),
        ),
        Some(PageAction::Crash) => Some(termination::try_start_crash_command_dispatch(conn, cmd)),
        Some(PageAction::Close) => Some(termination::try_start_close_command_dispatch(conn, cmd)),
        Some(PageAction::AddScriptToEvaluateOnNewDocument) => {
            preload::try_start_add_script_to_evaluate_on_new_document_command(conn, cmd)
        }
        Some(PageAction::RemoveScriptToEvaluateOnNewDocument) => {
            preload::try_start_remove_script_to_evaluate_on_new_document_command(conn, cmd)
        }
        Some(PageAction::CreateIsolatedWorld) => {
            Some(preload::try_start_create_isolated_world_command(conn, cmd))
        }
        None => Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        ))),
    }
}

fn page_set_download_behavior_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: PageSetDownloadBehaviorParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    if !crate::domains::browser::is_valid_download_behavior(params.behavior.as_str()) {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    }

    let session_id = cmd.session_id;
    let Some((browser_context_id, _)) = conn.target_owner_identity_for_session(session_id) else {
        return CommandOutputPlan::error(-32000, "Could not fetch browser context");
    };
    if !conn.has_browser_context_id(browser_context_id.as_str()) {
        return CommandOutputPlan::error(-32000, "Could not fetch browser context");
    }

    conn.download_behavior.set_browser_context_policy(
        browser_context_id,
        params.behavior,
        params.download_path,
    );
    CommandOutputPlan::success()
}

pub(crate) async fn execute_devtools_page_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
    background_command_id: Option<u64>,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    match command {
        DevToolsCommand::GetFrameTree(command) => (
            execute_devtools_get_frame_tree_command_async(conn, command).await,
            Vec::new(),
            None,
        ),
        DevToolsCommand::GetFrameTrees(command) => (
            execute_devtools_get_frame_trees_command_async(conn, command).await,
            Vec::new(),
            None,
        ),
        DevToolsCommand::GetNavigationHistory(command) => (
            navigation::execute_devtools_get_navigation_history_command(conn, command),
            Vec::new(),
            None,
        ),
        DevToolsCommand::GetLayoutMetrics(command) => (
            execute_devtools_get_layout_metrics_command(conn, command).await,
            Vec::new(),
            None,
        ),
        DevToolsCommand::GetJavaScriptDialog(command) => (
            execute_devtools_get_javascript_dialog_command(conn, command),
            Vec::new(),
            None,
        ),
        DevToolsCommand::SetJavaScriptDialogPromptText(command) => (
            execute_devtools_set_javascript_dialog_prompt_text_command(conn, command),
            Vec::new(),
            None,
        ),
        DevToolsCommand::HandleJavaScriptDialog(command) => {
            let (result, events) = execute_devtools_handle_javascript_dialog_command(conn, command);
            (result, events, None)
        }
        DevToolsCommand::CaptureScreenshot(command) => (
            execute_devtools_capture_screenshot_command(conn, command).await,
            Vec::new(),
            None,
        ),
        DevToolsCommand::PrintToPdf(command) => (
            execute_devtools_print_to_pdf_command(conn, command),
            Vec::new(),
            None,
        ),
        command @ (DevToolsCommand::Navigate(_)
        | DevToolsCommand::Reload(_)
        | DevToolsCommand::TraverseHistory(_)) => {
            navigation::execute_devtools_navigation_command_async_with_protocol_events(
                conn,
                command,
                background_command_id,
            )
            .await
        }
        command @ (DevToolsCommand::AddPreloadScript(_)
        | DevToolsCommand::RemovePreloadScript(_)) => {
            preload::execute_devtools_preload_command_async(conn, command).await
        }
        _ => (
            Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            )),
            Vec::new(),
            None,
        ),
    }
}

async fn execute_devtools_get_frame_trees_command_async(
    conn: &mut CdpConnection,
    command: DevToolsGetFrameTreesCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut frame_trees = Vec::new();
    for target_info in devtools_browsing_context_target_infos(conn) {
        let Some(target_id) = target_info.target_id.clone() else {
            continue;
        };
        let frame_tree_command = DevToolsGetFrameTreeCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id),
                ..command.context.clone()
            },
            max_depth: command.max_depth,
        };
        let DevToolsCommandResult::GetFrameTree(frame_tree_result) =
            execute_devtools_get_frame_tree_command_async(conn, frame_tree_command).await?
        else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "UnexpectedFrameTreeResult",
            ));
        };
        frame_trees.push(frame_tree_result);
    }
    Ok(DevToolsCommandResult::GetFrameTrees(
        DevToolsGetFrameTreesResult { frame_trees },
    ))
}

fn devtools_browsing_context_target_infos(conn: &CdpConnection) -> Vec<DevToolsTargetInfo> {
    conn.browser_contexts()
        .flat_map(|browser_context| browser_context.devtools_target_infos())
        .filter(|info| {
            matches!(
                info.kind,
                DevToolsTargetKind::Page
                    | DevToolsTargetKind::Frame
                    | DevToolsTargetKind::ServiceWorker
            )
        })
        .collect()
}

async fn execute_devtools_get_frame_tree_command_async(
    conn: &mut CdpConnection,
    mut command: DevToolsGetFrameTreeCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target_info = command
        .context
        .target_id
        .as_ref()
        .and_then(|target_id| devtools_target_info_for_target_id(conn, target_id.as_str()));
    if matches!(
        target_info.as_ref().map(|info| info.kind),
        Some(DevToolsTargetKind::ServiceWorker)
    ) {
        return devtools_service_worker_frame_tree_result(
            target_info.expect("service worker target info was checked"),
            command.max_depth,
        )
        .map(DevToolsCommandResult::GetFrameTree);
    }
    let route = if let Some(target_id) = command.context.target_id.as_ref() {
        let route = conn
            .target_session_route_for_target_id(target_id.as_str())
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        command.context.session_id = None;
        Some(route)
    } else {
        None
    };
    let max_depth = command.max_depth;
    execute_devtools_get_frame_tree_command_for_current_owner_async(conn, command, route)
        .await
        .map(|frame_tree| {
            DevToolsCommandResult::GetFrameTree(DevToolsGetFrameTreeResult {
                frame_tree,
                target_info,
                max_depth,
            })
        })
}

fn devtools_service_worker_frame_tree_result(
    target_info: DevToolsTargetInfo,
    max_depth: Option<u32>,
) -> Result<DevToolsGetFrameTreeResult, DevToolsError> {
    let Some(target_id) = target_info.target_id.as_ref() else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "MissingServiceWorkerTargetId",
        ));
    };
    Ok(DevToolsGetFrameTreeResult {
        frame_tree: json!({
            "frame": {
                "id": target_id.as_str(),
                "url": target_info.url.as_str(),
            }
        }),
        target_info: Some(target_info),
        max_depth,
    })
}

async fn execute_devtools_get_frame_tree_command_for_current_owner_async(
    conn: &mut CdpConnection,
    command: DevToolsGetFrameTreeCommand,
    route: Option<CdpSessionRoute>,
) -> Result<Value, DevToolsError> {
    if let Some(route) = route {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        devtools_frame_tree_for_current_owner_async(route_scope.conn_mut(), command).await
    } else {
        devtools_frame_tree_for_current_owner_async(conn, command).await
    }
}

async fn devtools_frame_tree_for_current_owner_async(
    conn: &mut CdpConnection,
    command: DevToolsGetFrameTreeCommand,
) -> Result<Value, DevToolsError> {
    let command_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    if command_session_id.is_none() && conn.browser_context.is_none() {
        return Err(devtools_frame_tree_error("BrowserContextNotLoaded"));
    }
    let (target_id, target_url, target_security_origin, target_secure_context_type) = conn
        .target_session_owner_frame_tree_identity(command_session_id)
        .ok_or_else(|| devtools_frame_tree_error("TargetNotLoaded"))?;
    let target_unreachable_url =
        network_error_page_unreachable_url(conn, command_session_id, &target_url);
    let target_loader_id = frame_tree_loader_id_for_current_owner(conn, command_session_id);
    if conn
        .ensure_document_accessible_for_session_owner(command_session_id)
        .is_err()
    {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
        ));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
        ));
    };
    let target_mime_type = main_document_mime_type(page);
    let pending = page.start_child_frame_tree_snapshot().map_err(|error| {
        devtools_frame_tree_error(format!("Failed to snapshot child frame tree: {error}"))
    })?;
    let completed = pending.wait().await.map_err(|error| {
        devtools_frame_tree_error(format!("Failed to snapshot child frame tree: {error}"))
    })?;
    if conn
        .ensure_document_accessible_for_session_owner(command_session_id)
        .is_err()
    {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            target_mime_type,
            Vec::new(),
        ));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            target_mime_type,
            Vec::new(),
        ));
    };
    let child_frames = page
        .finish_child_frame_tree_snapshot(completed)
        .map_err(|error| {
            devtools_frame_tree_error(format!("Failed to snapshot child frame tree: {error}"))
        })?;
    Ok(frame_tree_payload(
        target_id,
        target_loader_id,
        target_url,
        target_unreachable_url,
        target_security_origin,
        target_secure_context_type,
        target_mime_type,
        child_frames,
    ))
}

fn default_document_mime_type() -> String {
    "text/html".to_owned()
}

fn network_error_page_unreachable_url(
    conn: &CdpConnection,
    session_id: Option<&str>,
    document_url: &str,
) -> Option<String> {
    (document_url == NETWORK_ERROR_PAGE_URL)
        .then(|| conn.runtime_session_owner_target_url(session_id))
        .flatten()
}

fn main_document_mime_type(page: &Page) -> String {
    moli_web_mime::effective_response_mime_essence(page.headers(), None)
        .unwrap_or_else(default_document_mime_type)
}

fn devtools_frame_tree_error(message: impl Into<String>) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, message)
}

/// Returns the current committed DocumentLoader identity for CDP serialization.
///
/// Blink's `BuildObjectForFrame()` permits the rare state where a `LocalFrame`
/// has no `DocumentLoader`; `IdentifiersFactory::LoaderId(nullptr)` serializes
/// that absence as an empty string. Reusing a well-known loader ID here would
/// instead claim that the frame belongs to an unrelated document/navigation.
fn frame_tree_loader_id_for_current_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> String {
    conn.target_session_owner_frame_tree_loader_id(session_id)
        .unwrap_or_default()
}

fn devtools_target_info_for_target_id(
    conn: &CdpConnection,
    target_id: &str,
) -> Option<DevToolsTargetInfo> {
    conn.browser_contexts()
        .find_map(|browser_context| browser_context.devtools_target_info(target_id))
}

async fn execute_devtools_capture_screenshot_command(
    conn: &mut CdpConnection,
    command: DevToolsCaptureScreenshotCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    validate_page_capture_target_context(conn, &command.context)?;
    Err(devtools_capture_screenshot_error(&command))
}

fn execute_devtools_get_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsGetJavaScriptDialogCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut command = command;
    let route = if let Some(target_id) = command.context.target_id.as_ref() {
        Some(page_route_for_context_id(conn, target_id.as_str())?)
    } else {
        None
    };
    if route.is_some() {
        command.context.session_id = None;
    }
    let result = if let Some(route) = route {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        finish_devtools_get_javascript_dialog_command(route_scope.conn_mut(), command)
    } else {
        finish_devtools_get_javascript_dialog_command(conn, command)
    };
    result.map(DevToolsCommandResult::JavaScriptDialog)
}

fn execute_devtools_set_javascript_dialog_prompt_text_command(
    conn: &mut CdpConnection,
    command: DevToolsSetJavaScriptDialogPromptTextCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut command = command;
    let route = if let Some(target_id) = command.context.target_id.as_ref() {
        Some(page_route_for_context_id(conn, target_id.as_str())?)
    } else {
        None
    };
    if route.is_some() {
        command.context.session_id = None;
    }
    if let Some(route) = route {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        finish_devtools_set_javascript_dialog_prompt_text_command(route_scope.conn_mut(), command)
    } else {
        finish_devtools_set_javascript_dialog_prompt_text_command(conn, command)
    }
}

fn execute_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<BackgroundProtocolEvent>,
) {
    let mut command = command;
    let route = if let Some(target_id) = command.context.target_id.as_ref() {
        Some(match page_route_for_context_id(conn, target_id.as_str()) {
            Ok(route) => route,
            Err(error) => return (Err(error), Vec::new()),
        })
    } else {
        None
    };
    if route.is_some() {
        command.context.session_id = None;
    }
    let result = if let Some(route) = route {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        finish_devtools_handle_javascript_dialog_command(route_scope.conn_mut(), command)
    } else {
        finish_devtools_handle_javascript_dialog_command(conn, command)
    };
    match result {
        Ok(event) => (Ok(DevToolsCommandResult::Empty), vec![event]),
        Err(error) => (Err(error), Vec::new()),
    }
}

fn execute_devtools_print_to_pdf_command(
    conn: &mut CdpConnection,
    command: DevToolsPrintToPdfCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    validate_page_capture_target_context(conn, &command.context)?;
    Err(devtools_print_to_pdf_error(&command))
}

fn validate_page_capture_target_context(
    conn: &CdpConnection,
    context: &DevToolsCommandContext,
) -> Result<(), DevToolsError> {
    if let Some(target_id) = context.target_id.as_ref() {
        page_capture_route_for_context_id(conn, target_id.as_str())?;
    }
    Ok(())
}

fn page_capture_route_for_context_id(
    conn: &CdpConnection,
    context_id: &str,
) -> Result<CdpSessionRoute, DevToolsError> {
    page_route_for_context_id(conn, context_id)
}

fn page_route_for_context_id(
    conn: &CdpConnection,
    context_id: &str,
) -> Result<CdpSessionRoute, DevToolsError> {
    conn.target_session_route_for_target_id(context_id)
        .or_else(|| conn.target_session_route_for_child_frame_id(context_id))
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))
}

fn try_start_page_enable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<PageCommandTaskStep> {
    let params: EnablePageParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => EnablePageParams::default(),
        Err(error) => {
            return Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602, error,
            )));
        }
    };
    if !conn.set_page_domain_enabled_for_session_owner(cmd.session_id, true) {
        return Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )));
    }
    if let Err(error) = start_set_javascript_dialog_handler_enabled(conn, cmd.session_id, true) {
        return Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("failed to update JavaScript dialog handling: {error}"),
        )));
    }
    if let Some(enabled) = params.enable_file_chooser_opened_event
        && !conn
            .set_page_file_chooser_opened_event_enabled_for_session_owner(cmd.session_id, enabled)
    {
        return Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )));
    }
    if conn.auto_attach_wait_for_debugger_on_start {
        return Some(PageCommandTaskStep::Complete(CommandOutputPlan::success()));
    }
    match conn.runtime_session_owner_slot(cmd.session_id) {
        Ok(slot)
            if slot.has_loaded_page()
                && conn.runtime_session_owner_should_start_initial_document_navigation(
                    cmd.session_id,
                ) =>
        {
            let start = match navigation::start_initial_document_navigation_for_session_owner(
                conn,
                cmd.id,
                cmd.session_id,
                json!({}),
            ) {
                Ok(start) => start,
                Err(plan) => return Some(PageCommandTaskStep::Complete(plan)),
            };
            Some(navigation::finish_started_navigation_command_for_parts(
                conn,
                cmd.id,
                cmd.session_id,
                start,
                &[],
            ))
        }
        Ok(slot) if slot.has_loaded_page() => {
            Some(PageCommandTaskStep::Complete(CommandOutputPlan::success()))
        }
        Ok(_) if !conn.runtime_session_owner_target_is_initial_about_blank(cmd.session_id) => {
            let start = match navigation::start_initial_document_navigation_for_session_owner(
                conn,
                cmd.id,
                cmd.session_id,
                json!({}),
            ) {
                Ok(start) => start,
                Err(plan) => return Some(PageCommandTaskStep::Complete(plan)),
            };
            Some(navigation::finish_started_navigation_command_for_parts(
                conn,
                cmd.id,
                cmd.session_id,
                start,
                &[],
            ))
        }
        Ok(_) => Some(PageCommandTaskStep::Complete(CommandOutputPlan::success())),
        Err(_) if cmd.session_id.is_some() => Some(PageCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "TargetNotLoaded"),
        )),
        Err(_) => Some(PageCommandTaskStep::Complete(CommandOutputPlan::success())),
    }
}

fn try_start_page_get_frame_tree_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = build_cdp_get_frame_tree_command(conn, cmd);
    start_devtools_get_frame_tree_command(
        conn,
        cmd.id,
        command,
        FrameTreeCommandOutputKind::FrameTree,
    )
}

fn try_start_page_get_resource_tree_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = build_cdp_get_frame_tree_command(conn, cmd);
    start_devtools_get_frame_tree_command(
        conn,
        cmd.id,
        command,
        FrameTreeCommandOutputKind::ResourceTree,
    )
}

fn try_start_page_set_document_content_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: SetDocumentContentParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "Invalid parameters",
            ));
        }
    };
    let session_id = cmd.session_id;
    if let Err(message) = conn.ensure_document_accessible_for_session_owner(session_id) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut(session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "No Document instance to set HTML for",
        ));
    };
    match page.start_set_document_content(params.frame_id.into(), params.html) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id: cmd.id,
            owner_scope: CommandOwnerScope::capture(conn, session_id),
            kind: Box::new(PendingPageCommandKind::SetDocumentContent { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to set document content: {error}"),
        )),
    }
}

fn build_cdp_get_frame_tree_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> DevToolsGetFrameTreeCommand {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    DevToolsGetFrameTreeCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        max_depth: None,
    }
}

fn start_devtools_page_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsCommand,
) -> PageCommandTaskStep {
    match command {
        DevToolsCommand::GetFrameTree(command) => start_devtools_get_frame_tree_command(
            conn,
            command_id,
            command,
            FrameTreeCommandOutputKind::FrameTree,
        ),
        DevToolsCommand::GetLayoutMetrics(command) => {
            start_devtools_get_layout_metrics_command(conn, command_id, command)
        }
        DevToolsCommand::GetJavaScriptDialog(command) => PageCommandTaskStep::Complete(
            match finish_devtools_get_javascript_dialog_command(conn, command) {
                Ok(result) => CommandOutputPlan::from_devtools_result(
                    DevToolsCommandResult::JavaScriptDialog(result),
                ),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            },
        ),
        DevToolsCommand::SetJavaScriptDialogPromptText(command) => PageCommandTaskStep::Complete(
            match finish_devtools_set_javascript_dialog_prompt_text_command(conn, command) {
                Ok(result) => CommandOutputPlan::from_devtools_result(result),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            },
        ),
        DevToolsCommand::HandleJavaScriptDialog(command) => PageCommandTaskStep::Complete(
            complete_devtools_handle_javascript_dialog_command(conn, command),
        ),
        DevToolsCommand::CaptureScreenshot(command) => {
            start_devtools_capture_screenshot_command(conn, command_id, command)
        }
        DevToolsCommand::PrintToPdf(command) => {
            start_devtools_print_to_pdf_command(conn, command_id, command)
        }
        _ => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "UnsupportedDevToolsCommand",
        )),
    }
}

fn start_devtools_capture_screenshot_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsCaptureScreenshotCommand,
) -> PageCommandTaskStep {
    if let Err(error) = validate_page_capture_target_context(conn, &command.context) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
    }
    if command.context.protocol != DevToolsProtocol::Cdp {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
            devtools_capture_screenshot_error(&command),
        ));
    }
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE,
        ));
    }

    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let page = match conn.loaded_page_mut_for_protocol_access(session_id) {
        Ok(page) => page,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let format = match command.format.as_deref() {
        None | Some("png") => RendererScreenshotFormat::Png,
        Some("jpeg") => RendererScreenshotFormat::Jpeg,
        Some(_) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "Invalid image format",
            ));
        }
    };
    let region = match command.clip.as_ref() {
        Some(DevToolsCaptureScreenshotClip::Box(clip)) => {
            RendererScreenshotRegion::PageClip(RendererScreenshotClip {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
                scale: clip.scale,
            })
        }
        Some(DevToolsCaptureScreenshotClip::Element(_)) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                devtools_capture_screenshot_error(&command),
            ));
        }
        None if command.capture_beyond_viewport => RendererScreenshotRegion::FullDocument,
        None => RendererScreenshotRegion::Viewport,
    };
    let request = RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Screenshot,
        format,
        quality: command.quality.unwrap_or(80),
        region,
        optimize_for_speed: command.optimize_for_speed,
        max_width: None,
        max_height: None,
    };
    match page.start_capture_screenshot_with_request(request) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::CaptureScreenshot { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to start page screenshot: {error}"),
        )),
    }
}

fn start_devtools_print_to_pdf_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsPrintToPdfCommand,
) -> PageCommandTaskStep {
    if let Err(error) = validate_page_capture_target_context(conn, &command.context) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
    }
    if command.context.protocol != DevToolsProtocol::Cdp {
        return PageCommandTaskStep::Complete(complete_devtools_print_to_pdf_command(
            conn, command,
        ));
    }
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            PRINT_TO_PDF_LAYOUT_DISABLED_MESSAGE,
        ));
    }
    let options = match pdf::RasterPdfOptions::from_command(&command) {
        Ok(options) => options,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                error.code(),
                error.message(),
            ));
        }
    };
    let transfer_mode = command
        .transfer_mode
        .unwrap_or(DevToolsPrintToPdfTransferMode::ReturnAsBase64);
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let page = match conn.loaded_page_mut_for_protocol_access(session_id) {
        Ok(page) => page,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let request = RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Print {
            print_background: command.print_background.unwrap_or(false),
        },
        format: RendererScreenshotFormat::Jpeg,
        quality: 90,
        region: RendererScreenshotRegion::FullDocument,
        optimize_for_speed: false,
        max_width: None,
        max_height: None,
    };
    match page.start_capture_screenshot_with_request(request) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::PrintToPdf {
                pending,
                options,
                transfer_mode,
            }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to start PDF capture: {error}"),
        )),
    }
}

fn build_cdp_get_layout_metrics_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> crate::devtools_runtime::DevToolsGetLayoutMetricsCommand {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    crate::devtools_runtime::DevToolsGetLayoutMetricsCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
    }
}

fn start_devtools_get_frame_tree_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsGetFrameTreeCommand,
    output_kind: FrameTreeCommandOutputKind,
) -> PageCommandTaskStep {
    let command_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    if command_session_id.is_none() && conn.browser_context.is_none() {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let (target_id, target_url, target_security_origin, target_secure_context_type) =
        match conn.target_session_owner_frame_tree_identity(command_session_id) {
            Some(identity) => identity,
            None => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -31998,
                    "TargetNotLoaded",
                ));
            }
        };
    let target_unreachable_url =
        network_error_page_unreachable_url(conn, command_session_id, &target_url);
    let target_loader_id = frame_tree_loader_id_for_current_owner(conn, command_session_id);
    if conn
        .ensure_document_accessible_for_session_owner(command_session_id)
        .is_err()
    {
        return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
            output_kind,
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
            &[],
        ));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
            output_kind,
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
            &[],
        ));
    };
    let target_mime_type = main_document_mime_type(page);
    match page.start_child_frame_tree_snapshot() {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope: CommandOwnerScope::capture(conn, command_session_id),
            kind: Box::new(PendingPageCommandKind::GetFrameTree {
                output_kind,
                target_id,
                target_loader_id,
                target_url,
                target_unreachable_url,
                target_security_origin,
                target_secure_context_type,
                target_mime_type,
                pending,
            }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to snapshot child frame tree: {error}"),
        )),
    }
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::{
        DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand, DevToolsCommand,
        DevToolsCommandContext, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
        DevToolsProtocol, DevToolsScreenshotClip, DevToolsTargetId,
    };
    use serde_json::{Value, json};

    use crate::conn::{CdpConnection, Cmd};

    use super::{
        PageCommandTaskStep, build_cdp_capture_screenshot_command,
        build_cdp_get_frame_tree_command, build_cdp_get_layout_metrics_command,
        build_cdp_handle_javascript_dialog_command, build_cdp_print_to_pdf_command,
        start_devtools_page_command,
    };

    #[test]
    fn cdp_get_frame_tree_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(120),
            "Page.getFrameTree",
            &params,
            Some("SID-page"),
            r#"{"id":120,"method":"Page.getFrameTree"}"#,
        );

        let command = build_cdp_get_frame_tree_command(&conn, &cmd);

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-page")
        );
        assert_eq!(command.context.target_id, None);
        assert_eq!(command.context.browser_context_id, None);
    }

    #[test]
    fn devtools_page_entry_routes_get_frame_tree_command_to_page_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(121),
            "Page.getFrameTree",
            &params,
            None,
            r#"{"id":121,"method":"Page.getFrameTree"}"#,
        );
        let command = build_cdp_get_frame_tree_command(&conn, &cmd);

        let step =
            start_devtools_page_command(&mut conn, cmd.id, DevToolsCommand::GetFrameTree(command));

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("missing browser context should complete through the unified page entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(121));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn cdp_get_layout_metrics_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(122),
            "Page.getLayoutMetrics",
            &params,
            Some("SID-page"),
            r#"{"id":122,"method":"Page.getLayoutMetrics"}"#,
        );

        let command = build_cdp_get_layout_metrics_command(&conn, &cmd);

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-page")
        );
        assert_eq!(command.context.target_id, None);
        assert_eq!(command.context.browser_context_id, None);
    }

    #[test]
    fn devtools_page_entry_routes_get_layout_metrics_command_to_page_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(123),
            "Page.getLayoutMetrics",
            &params,
            None,
            r#"{"id":123,"method":"Page.getLayoutMetrics"}"#,
        );
        let command = build_cdp_get_layout_metrics_command(&conn, &cmd);

        let step = start_devtools_page_command(
            &mut conn,
            cmd.id,
            DevToolsCommand::GetLayoutMetrics(command),
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("missing page should complete through the unified page entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(123));
        assert!(out[0]["result"]["layoutViewport"]["clientWidth"].is_u64());
        assert!(out[0]["result"]["visualViewport"]["clientHeight"].is_u64());
    }

    #[test]
    fn cdp_handle_javascript_dialog_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "accept": false,
            "promptText": "typed text"
        });
        let cmd = Cmd::for_test(
            Some(124),
            "Page.handleJavaScriptDialog",
            &params,
            Some("SID-page"),
            r#"{"id":124,"method":"Page.handleJavaScriptDialog"}"#,
        );

        let command = build_cdp_handle_javascript_dialog_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid handleJavaScriptDialog command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-page")
        );
        assert!(!command.accept);
        assert_eq!(command.prompt_text, "typed text");
    }

    #[test]
    fn devtools_page_entry_routes_handle_javascript_dialog_command_to_page_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "accept": true
        });
        let cmd = Cmd::for_test(
            Some(125),
            "Page.handleJavaScriptDialog",
            &params,
            Some("SID-page"),
            r#"{"id":125,"method":"Page.handleJavaScriptDialog"}"#,
        );
        let command = build_cdp_handle_javascript_dialog_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid handleJavaScriptDialog command");
        };

        let step = start_devtools_page_command(
            &mut conn,
            cmd.id,
            DevToolsCommand::HandleJavaScriptDialog(command),
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("dialog command should complete through the unified page entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(125));
        assert_eq!(out[0]["error"]["code"], json!(-32602));
        assert_eq!(out[0]["error"]["message"], json!("No dialog is showing"));
    }

    #[test]
    fn cdp_capture_screenshot_builds_requested_capture_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "format": "png",
            "quality": 100,
            "fromSurface": true,
            "captureBeyondViewport": false,
            "optimizeForSpeed": false
        });
        let cmd = Cmd::for_test(
            Some(126),
            "Page.captureScreenshot",
            &params,
            Some("SID-page"),
            r#"{"id":126,"method":"Page.captureScreenshot"}"#,
        );

        let command = build_cdp_capture_screenshot_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid captureScreenshot command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-page")
        );
        assert_eq!(command.format.as_deref(), Some("png"));
        assert_eq!(command.quality, Some(100));
        assert_eq!(command.clip, None);
        assert!(!command.capture_beyond_viewport);
        assert!(!command.optimize_for_speed);
    }

    #[test]
    fn cdp_capture_screenshot_preserves_page_clip() {
        let conn = CdpConnection::new();
        let params = json!({
            "format": "png",
            "clip": {
                "x": 0.0,
                "y": 0.0,
                "width": 2.0,
                "height": 3.0,
                "scale": 1.0
            }
        });
        let cmd = Cmd::for_test(
            Some(127),
            "Page.captureScreenshot",
            &params,
            Some("SID-page"),
            r#"{"id":127,"method":"Page.captureScreenshot"}"#,
        );
        let Ok(command) = build_cdp_capture_screenshot_command(&conn, &cmd) else {
            panic!("valid clip should enter the protocol-neutral screenshot command");
        };
        assert_eq!(
            command.clip,
            Some(DevToolsCaptureScreenshotClip::Box(DevToolsScreenshotClip {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 3.0,
                scale: 1.0,
            }))
        );
    }

    #[test]
    fn devtools_page_entry_rejects_unsupported_capture_screenshot_format() {
        let conn = CdpConnection::new();
        let params = json!({
            "format": "webp"
        });
        let cmd = Cmd::for_test(
            Some(128),
            "Page.captureScreenshot",
            &params,
            None,
            r#"{"id":128,"method":"Page.captureScreenshot"}"#,
        );
        let Err(plan) = build_cdp_capture_screenshot_command(&conn, &cmd) else {
            panic!("webp should be rejected at the CDP capability boundary");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(128));
        assert_eq!(out[0]["error"]["code"], json!(-32000));
        assert_eq!(
            out[0]["error"]["message"],
            json!("Page.captureScreenshot option 'format' is not supported.")
        );
    }

    #[test]
    fn cdp_capture_screenshot_rejects_invalid_quality_and_clip() {
        let conn = CdpConnection::new();
        for (id, params, expected_message) in [
            (
                132,
                json!({ "quality": 101 }),
                "Page.captureScreenshot quality must be between 0 and 100.",
            ),
            (
                133,
                json!({
                    "clip": {
                        "x": 0.0,
                        "y": 0.0,
                        "width": 0.0,
                        "height": 3.0,
                        "scale": 1.0
                    }
                }),
                "Page.captureScreenshot clip must have a finite origin and positive finite width, height, and scale.",
            ),
        ] {
            let raw = format!(r#"{{"id":{id},"method":"Page.captureScreenshot"}}"#);
            let cmd = Cmd::for_test(
                Some(id),
                "Page.captureScreenshot",
                &params,
                Some("SID-page"),
                &raw,
            );
            let Err(plan) = build_cdp_capture_screenshot_command(&conn, &cmd) else {
                panic!("invalid screenshot parameters should be rejected");
            };
            let mut out = Vec::new();
            plan.emit_into(&mut out, cmd.id, cmd.session_id);
            assert_eq!(out[0]["error"]["code"], json!(-32602));
            assert_eq!(out[0]["error"]["message"], json!(expected_message));
        }
    }

    #[test]
    fn devtools_page_entry_validates_capture_screenshot_target_before_unsupported() {
        let mut conn = CdpConnection::new();
        let command = DevToolsCaptureScreenshotCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: Some(DevToolsTargetId::from("missing-target")),
                browser_context_id: None,
            },
            format: Some("png".to_owned()),
            quality: None,
            clip: None,
            capture_beyond_viewport: false,
            optimize_for_speed: false,
        };

        let step = start_devtools_page_command(
            &mut conn,
            Some(131),
            DevToolsCommand::CaptureScreenshot(command),
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("screenshot target validation should complete synchronously");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, Some(131), None);
        assert_eq!(out[0]["id"], json!(131));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("NoSuchTarget"));
    }

    #[test]
    fn cdp_print_to_pdf_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "landscape": true,
            "printBackground": true,
            "scale": 1.25,
            "paperWidth": 8.0,
            "paperHeight": 10.0,
            "marginTop": 0.25,
            "marginBottom": 0.5,
            "marginLeft": 0.75,
            "marginRight": 1.0,
            "pageRanges": "1-2,4",
            "transferMode": "ReturnAsStream"
        });
        let cmd = Cmd::for_test(
            Some(129),
            "Page.printToPDF",
            &params,
            Some("SID-page"),
            r#"{"id":129,"method":"Page.printToPDF"}"#,
        );

        let command = build_cdp_print_to_pdf_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid printToPDF command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-page")
        );
        assert_eq!(command.landscape, Some(true));
        assert_eq!(command.print_background, Some(true));
        assert_eq!(command.scale, Some(1.25));
        assert_eq!(command.paper_width, Some(8.0));
        assert_eq!(command.paper_height, Some(10.0));
        assert_eq!(command.margin_top, Some(0.25));
        assert_eq!(command.margin_bottom, Some(0.5));
        assert_eq!(command.margin_left, Some(0.75));
        assert_eq!(command.margin_right, Some(1.0));
        assert_eq!(command.page_ranges, Some("1-2,4".to_owned()));
        assert_eq!(
            command.transfer_mode,
            Some(DevToolsPrintToPdfTransferMode::ReturnAsStream)
        );
    }

    #[test]
    fn devtools_page_entry_reports_layout_disabled_without_placeholder_payload() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(130),
            "Page.printToPDF",
            &params,
            None,
            r#"{"id":130,"method":"Page.printToPDF"}"#,
        );
        let command = build_cdp_print_to_pdf_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("default printToPDF command should build");
        };

        let step =
            start_devtools_page_command(&mut conn, cmd.id, DevToolsCommand::PrintToPdf(command));

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("printToPDF command should complete through the unified page entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(130));
        assert_eq!(out[0]["error"]["code"], json!(-32000));
        assert_eq!(
            out[0]["error"]["message"],
            json!("Page.printToPDF is not supported: renderer layout is disabled.")
        );
    }

    #[test]
    fn devtools_page_entry_validates_print_to_pdf_target_before_unsupported() {
        let mut conn = CdpConnection::new();
        let command = DevToolsPrintToPdfCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: Some(DevToolsTargetId::from("missing-target")),
                browser_context_id: None,
            },
            landscape: None,
            print_background: None,
            scale: None,
            paper_width: None,
            paper_height: None,
            margin_top: None,
            margin_bottom: None,
            margin_left: None,
            margin_right: None,
            page_ranges: None,
            shrink_to_fit: None,
            transfer_mode: None,
        };

        let step =
            start_devtools_page_command(&mut conn, Some(132), DevToolsCommand::PrintToPdf(command));

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("print target validation should complete synchronously");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, Some(132), None);
        assert_eq!(out[0]["id"], json!(132));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("NoSuchTarget"));
    }
}

fn try_start_page_get_layout_metrics_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = build_cdp_get_layout_metrics_command(conn, cmd);
    start_devtools_page_command(conn, cmd.id, DevToolsCommand::GetLayoutMetrics(command))
}

fn start_devtools_get_layout_metrics_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: crate::devtools_runtime::DevToolsGetLayoutMetricsCommand,
) -> PageCommandTaskStep {
    let command_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let fallback =
        layout_metrics_result_from_surface(current_viewport_surface(conn, command_session_id));
    let owner_scope = CommandOwnerScope::capture(conn, command_session_id);
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_result(
            DevToolsCommandResult::LayoutMetrics(fallback),
        ));
    };
    match page.start_layout_metrics() {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::GetLayoutMetrics { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to start layout metrics: {error}"),
        )),
    }
}

fn try_start_page_print_to_pdf_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = match build_cdp_print_to_pdf_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return PageCommandTaskStep::Complete(plan),
    };
    start_devtools_page_command(conn, cmd.id, DevToolsCommand::PrintToPdf(command))
}

pub(crate) async fn complete_pending_page_command(
    conn: &mut CdpConnection,
    completed: CompletedPageCommandDispatch,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let owner_scope = completed.owner_scope.clone();
    let mut route_scope = owner_scope.enter(conn);
    complete_pending_page_command_inner(route_scope.conn_mut(), completed, command_context).await
}

async fn complete_pending_page_command_inner(
    conn: &mut CdpConnection,
    completed: CompletedPageCommandDispatch,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let command_id = completed.command_id;
    let session_id = completed.owner_scope.session_id().map(str::to_owned);
    if let Some(predecessor) = completed.kind.renderer_output_predecessor() {
        command_context.set_renderer_output_predecessor(predecessor);
    }
    let plan = match *completed.kind {
        CompletedPageCommandKind::BringToFront {
            route,
            restore_browser_context_id,
        } => {
            let result = bring_session_route_to_front_async(conn, route).await;
            restore_page_bring_to_front_context_async(conn, restore_browser_context_id.as_deref())
                .await;
            match result {
                Ok(()) => CommandOutputPlan::success(),
                Err(message) => CommandOutputPlan::error(-31998, message),
            }
        }
        CompletedPageCommandKind::AppendDefaultDocumentStartScript {
            identifier,
            completed,
        } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let completed_script = {
                let Some(page) = conn
                    .runtime_session_owner_slot_mut(session_id.as_deref())
                    .ok()
                    .and_then(|slot| slot.loaded_page_mut())
                else {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "NoDocumentLoaded",
                    ));
                };
                page.finish_document_start_script_result_command_turn(completion)
            };
            let (_, output) = match completed_script {
                Ok(completed_script) => completed_script,
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        error.to_string(),
                    ));
                }
            };
            command_context.consume_renderer_command_turn_output(output);
            preload::add_preload_script_result_plan(identifier)
        }
        CompletedPageCommandKind::RemoveDocumentStartScript { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let Some(page) = conn
                .runtime_session_owner_slot_mut(session_id.as_deref())
                .ok()
                .and_then(|slot| slot.loaded_page_mut())
            else {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "NoDocumentLoaded",
                ));
            };
            if let Err(error) =
                page.finish_unit_runtime_page_command(completion, "remove document-start script")
            {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
            CommandOutputPlan::success()
        }
        CompletedPageCommandKind::SearchInResource(completed) => {
            return resource_search::complete_search_in_resource_command(
                conn,
                command_id,
                session_id.as_deref(),
                *completed,
            );
        }
        CompletedPageCommandKind::GetAppManifest(completed) => {
            return app_manifest::complete_get_app_manifest_command(
                conn,
                command_id,
                session_id.as_deref(),
                *completed,
                command_context,
            );
        }
        CompletedPageCommandKind::ResetNavigationHistory { completed } => {
            return navigation::complete_reset_navigation_history_command(
                conn,
                session_id.as_deref(),
                *completed,
            );
        }
        CompletedPageCommandKind::AddScriptToEvaluateOnNewDocument(completed) => {
            return preload::complete_pending_add_script_to_evaluate_on_new_document_command(
                conn,
                command_id,
                session_id.as_deref(),
                completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::SetBypassContentSecurityPolicy { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let Some(page) = conn
                .runtime_session_owner_slot_mut(session_id.as_deref())
                .ok()
                .and_then(|slot| slot.loaded_page_mut())
            else {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "NoDocumentLoaded",
                ));
            };
            if let Err(error) = page.finish_set_bypass_content_security_policy(completion) {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
            CommandOutputPlan::success()
        }
        CompletedPageCommandKind::SetDocumentContent { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let (result, output) = {
                let Some(page) = conn
                    .runtime_session_owner_slot_mut(session_id.as_deref())
                    .ok()
                    .and_then(|slot| slot.loaded_page_mut())
                else {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "No Document instance to set HTML for",
                    ));
                };
                match page.finish_set_document_content_command_turn(completion) {
                    Ok(completed) => completed,
                    Err(error) => {
                        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000,
                            error.to_string(),
                        ));
                    }
                }
            };
            command_context.consume_renderer_command_turn_output(output);
            let mut plan = CommandOutputPlan::default();
            match result {
                RendererSetDocumentContentResult::Updated => plan.push_success(),
                RendererSetDocumentContentResult::FrameNotFound => {
                    plan.push_error(-32000, "No frame for given id found");
                }
                RendererSetDocumentContentResult::DocumentNotFound => {
                    plan.push_error(-32000, "No Document instance to set HTML for");
                }
            }
            plan
        }
        CompletedPageCommandKind::SameDocumentNavigate(completed) => {
            return navigation::complete_pending_same_document_navigate_command(
                conn,
                session_id.as_deref(),
                *completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::GetFrameTree {
            output_kind,
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            target_mime_type,
            completed,
        } => {
            if conn
                .ensure_document_accessible_for_session_owner(session_id.as_deref())
                .is_err()
            {
                return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
                    output_kind,
                    target_id,
                    target_loader_id,
                    target_url,
                    target_unreachable_url,
                    target_security_origin,
                    target_secure_context_type,
                    target_mime_type,
                    Vec::new(),
                    &[],
                ));
            }
            let Some(page) = conn
                .runtime_session_owner_slot_mut(session_id.as_deref())
                .ok()
                .and_then(|slot| slot.loaded_page_mut())
            else {
                return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
                    output_kind,
                    target_id,
                    target_loader_id,
                    target_url,
                    target_unreachable_url,
                    target_security_origin,
                    target_secure_context_type,
                    target_mime_type,
                    Vec::new(),
                    &[],
                ));
            };
            let child_frames = match *completed {
                Ok(completion) => match page.finish_child_frame_tree_snapshot(completion) {
                    Ok(frames) => frames,
                    Err(error) => {
                        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000,
                            format!("Failed to snapshot child frame tree: {error}"),
                        ));
                    }
                },
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to snapshot child frame tree: {message}"),
                    ));
                }
            };
            get_frame_tree_command_output_plan(
                output_kind,
                target_id,
                target_loader_id,
                target_url,
                target_unreachable_url,
                target_security_origin,
                target_secure_context_type,
                target_mime_type,
                child_frames,
                page.subresource_network_records(),
            )
        }
        CompletedPageCommandKind::CaptureSnapshot { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to serialize page snapshot: {message}"),
                    ));
                }
            };
            let Some(page) = conn
                .runtime_session_owner_slot_mut(session_id.as_deref())
                .ok()
                .and_then(|slot| slot.loaded_page_mut())
            else {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "NoDocumentLoaded",
                ));
            };
            let html = match page.finish_serialize_html(completion) {
                Ok(html) => html,
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to serialize page snapshot: {error}"),
                    ));
                }
            };
            let url = page.final_url().as_str().to_owned();
            CommandOutputPlan::result(json!({ "data": build_mhtml_snapshot(&url, &html) }))
        }
        CompletedPageCommandKind::GetLayoutMetrics { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to produce layout metrics: {message}"),
                    ));
                }
            };
            let page = match conn.loaded_page_mut_for_protocol_access(session_id.as_deref()) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match page.finish_layout_metrics(completion) {
                Ok(metrics) => {
                    CommandOutputPlan::from_devtools_result(DevToolsCommandResult::LayoutMetrics(
                        layout_metrics_result_from_renderer(metrics),
                    ))
                }
                Err(error) => CommandOutputPlan::error(
                    -32000,
                    format!("Failed to finish layout metrics: {error}"),
                ),
            }
        }
        CompletedPageCommandKind::CaptureScreenshot { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to capture page screenshot: {message}"),
                    ));
                }
            };
            let page = match conn.loaded_page_mut_for_protocol_access(session_id.as_deref()) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match page.finish_capture_screenshot(completion) {
                Ok(RendererCaptureScreenshotReply::Captured(image)) => {
                    CommandOutputPlan::from_devtools_result(
                        DevToolsCommandResult::CaptureScreenshot(DevToolsCaptureScreenshotResult {
                            mime_type: image.mime_type,
                            width: image.width,
                            height: image.height,
                            bytes: image.bytes,
                        }),
                    )
                }
                Ok(RendererCaptureScreenshotReply::LayoutDisabled) => {
                    CommandOutputPlan::error(-32000, CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE)
                }
                Ok(RendererCaptureScreenshotReply::NoDocument) => {
                    CommandOutputPlan::error(-32000, "NoDocumentLoaded")
                }
                Err(error) => CommandOutputPlan::error(
                    -32000,
                    format!("Failed to capture page screenshot: {error}"),
                ),
            }
        }
        CompletedPageCommandKind::PrintToPdf {
            completed,
            options,
            transfer_mode,
        } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to capture PDF content: {message}"),
                    ));
                }
            };
            let page = match conn.loaded_page_mut_for_protocol_access(session_id.as_deref()) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let image = match page.finish_capture_screenshot(completion) {
                Ok(RendererCaptureScreenshotReply::Captured(image)) => image,
                Ok(RendererCaptureScreenshotReply::LayoutDisabled) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        PRINT_TO_PDF_LAYOUT_DISABLED_MESSAGE,
                    ));
                }
                Ok(RendererCaptureScreenshotReply::NoDocument) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "NoDocumentLoaded",
                    ));
                }
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to capture PDF content: {error}"),
                    ));
                }
            };
            if image.mime_type != "image/jpeg" {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "Printing failed: renderer returned a non-JPEG raster",
                ));
            }
            let pdf = match pdf::build_raster_pdf(
                image.bytes.as_ref(),
                image.width,
                image.height,
                &options,
            ) {
                Ok(pdf) => pdf,
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        error.code(),
                        error.message(),
                    ));
                }
            };
            match transfer_mode {
                DevToolsPrintToPdfTransferMode::ReturnAsBase64 => {
                    CommandOutputPlan::result(json!({
                        "data": BASE64_STANDARD.encode(pdf),
                    }))
                }
                DevToolsPrintToPdfTransferMode::ReturnAsStream => {
                    let stream = match conn.open_io_stream_body_source_for_session_owner(
                        session_id.as_deref(),
                        CapturedBody::from_bytes_spooled(pdf),
                    ) {
                        Ok(stream) => stream,
                        Err(message) => {
                            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                                -32000, message,
                            ));
                        }
                    };
                    CommandOutputPlan::result(json!({
                        "data": "",
                        "stream": stream,
                    }))
                }
            }
        }
        CompletedPageCommandKind::CreateIsolatedWorld(completed) => {
            return preload::complete_pending_create_isolated_world_command(
                conn,
                command_id,
                session_id.as_deref(),
                *completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::ChildFrameNavigate(completed) => {
            return navigation::complete_pending_child_frame_navigate_command(
                conn,
                session_id.as_deref(),
                *completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::Navigate(completed) => {
            return navigation::complete_pending_navigate_load_command(
                conn,
                *completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::TraverseSameDocumentHistory(completed) => {
            return navigation::complete_pending_same_document_history_traversal_command(
                conn,
                command_id,
                session_id.as_deref(),
                *completed,
            );
        }
        CompletedPageCommandKind::ContinueNavigationWithoutRequestPause(completed) => {
            return navigation::complete_pending_continue_navigation_without_request_pause_command(
                conn, *completed,
            )
            .await;
        }
        CompletedPageCommandKind::StopLoading => {
            return termination::complete_stop_loading_command_dispatch(
                conn,
                command_id,
                session_id.as_deref(),
            )
            .await;
        }
        CompletedPageCommandKind::Crash => {
            return termination::complete_crash_command_dispatch(
                conn,
                command_id,
                session_id.as_deref(),
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::Close => {
            return termination::complete_close_command_dispatch(
                conn,
                command_id,
                session_id.as_deref(),
                command_context,
            )
            .await;
        }
    };
    PageCommandTaskStep::Complete(plan)
}

// ────────────────────────────────────────────────────────────────────────────

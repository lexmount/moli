use crate::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsAddPreloadScriptCommand,
    DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand,
    DevToolsCaptureScreenshotResult, DevToolsError, DevToolsErrorKind, DevToolsGetFrameTreeCommand,
    DevToolsGetFrameTreeResult, DevToolsGetFrameTreesCommand, DevToolsGetFrameTreesResult,
    DevToolsGetJavaScriptDialogCommand, DevToolsGetLayoutMetricsCommand,
    DevToolsHandleJavaScriptDialogCommand, DevToolsJavaScriptDialogResult,
    DevToolsLayoutMetricsResult, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
    DevToolsScreenshotClip, DevToolsSetJavaScriptDialogPromptTextCommand, DevToolsTargetInfo,
    DevToolsTargetKind, FrontendProtocol, UserPromptClosedEvent,
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
    CompletedPageCommand, Page, PendingPageCommand, RendererCaptureScreencastFrameReply,
    RendererCaptureScreencastFrameRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererDocumentLifecycleEvent,
    RendererDocumentLifecycleIdentity, RendererDocumentLifecycleMilestone,
    RendererDocumentLifecycleWaitOutcome, RendererDocumentLifecycleWaiter,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererLayoutMetrics,
    RendererPendingTopLevelHistoryTraversal, RendererPendingWindowOpenEvent,
    RendererScreenshotClip, RendererScreenshotFormat, RendererScreenshotPurpose,
    RendererScreenshotRegion, RendererSetDocumentContentResult, RendererVisualStateToken,
};
use moli_core::{RendererDocumentTitleChanged, RendererRuntimeCommandCausalIdentity};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use super::input;
use crate::conn::{
    BackgroundProtocolEvent, CapturedBody, CdpSessionRoute, CommandDispatchContext,
    CommandOwnerScope, NETWORK_ERROR_PAGE_URL, PageLifecycleEventsEnableResult,
    PageScreencastConfig, PageScreencastFormat,
};
use crate::conn::{CdpConnection, Cmd};
pub(crate) use crate::conn::{DEFAULT_LOADER_ID as LOADER_ID, monotonic_timestamp_seconds};
use crate::domains::actions::PageAction;
use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink, ProtocolOutputSlot,
};
use crate::domains::command_output::CommandOutputPlan;

mod app_manifest;
mod child_frame_activity;
mod javascript_dialog;
pub(in crate::domains) use javascript_dialog::emit_javascript_dialog_activity_background_events_async;
use javascript_dialog::*;
mod capture;
mod lifecycle;
mod main_document_commit;
pub(crate) mod native;
mod navigation;
mod output;
use capture::*;
pub(in crate::domains) use output::project_page_output_async;
mod navigation_commit;
mod pdf;
mod popup;
mod preload;
mod prepared_navigation;
mod resource_search;
mod resource_tree;
pub(crate) use resource_tree::child_frame_security_identity;
use resource_tree::*;
mod termination;
#[cfg(test)]
mod tests;

/// Removes renderer-owned Page resources for one DevTools session while its
/// Inspector binding is still usable for cleanup commands.
pub(in crate::domains) async fn dispose_session_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<()> {
    conn.remove_document_start_scripts_for_detached_session_async(session_id)
        .await
}

/// Clears target-visible state owned by the primary Page session after every
/// per-session handler contribution has been removed.
pub(in crate::domains) async fn dispose_primary_session_target_state_async(
    conn: &mut CdpConnection,
    plan: &crate::conn::SessionDisposalPlan,
) -> anyhow::Result<()> {
    let session_id = plan.session_id();
    if let crate::conn::SessionDisposalTarget::PageTarget {
        browser_context_id,
        target_id,
        session_key: moli_page_types::DevToolsSessionKey::Primary,
    } = plan.target()
    {
        let reset_result = match conn.browser_context_by_id_mut(browser_context_id) {
            Some(browser_context) => {
                browser_context
                    .reset_primary_page_session_target_state_async(target_id, session_id)
                    .await
            }
            None => Ok(false),
        };
        reset_result.and_then(|found| {
            anyhow::ensure!(
                found,
                "primary Page target disappeared during session disposal"
            );
            Ok(())
        })?;
    }
    Ok(())
}

fn missing_page_target_error_message(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> &'static str {
    if session_id.is_none() && conn.browser_context.is_some() {
        "TargetNotLoaded"
    } else {
        "BrowserContextNotLoaded"
    }
}

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
    emit_navigation_frame_stop_without_commit_background_events,
    emit_navigation_frame_stopped_loading_background_events,
    emit_navigation_lifecycle_init_background_events,
    emit_navigation_network_idle_background_events,
};
pub(in crate::domains) use main_document_commit::{
    MainDocumentCommitPreparedOutput, append_renderer_main_document_commit_to_output_sink,
    project_main_document_commit_async,
};
#[cfg(test)]
pub(crate) use navigation::emit_prepared_child_frame_tree_background_events;
pub(crate) use navigation::navigation_cookie_access_report;
pub use navigation::{BackgroundNavigationCompletion, PendingNavigationUnload};
pub(crate) use navigation::{
    MaterializedNavigationCompletion,
    complete_materialized_navigation_after_unload_into_buffer_async,
    complete_materialized_navigation_into_buffer_async, emit_prepared_child_frame_activity,
    push_superseded_navigation_result,
    finish_renderer_navigation_into_buffer_async,
};
use prepared_navigation::{
    PagePreparedSameDocumentNavigation, PagePreparedTopLevelLocationNavigation,
};
pub(crate) use termination::{
    PageTargetTerminationOwnerAction, complete_page_target_termination_owner_action_async,
    fail_pending_fetch_state_background_events_async, take_pending_fetch_state,
};

const DEFAULT_PRINT_MARGIN_INCHES: f64 = 1.0 / 2.54;
const DEFAULT_PRINT_PAGE_WIDTH_INCHES: f64 = 8.5;
const DEFAULT_PRINT_PAGE_HEIGHT_INCHES: f64 = 11.0;
const CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE: &str =
    "Page.captureScreenshot is not supported: renderer screenshots are not implemented.";
const CAPTURE_SCREENSHOT_LAYOUT_DISABLED_MESSAGE: &str = "Page.captureScreenshot is not supported: renderer layout is disabled; start Moli with --layout.";
const START_SCREENCAST_LAYOUT_DISABLED_MESSAGE: &str =
    "Page.startScreencast is not supported: renderer layout is disabled.";
const PRINT_TO_PDF_LAYOUT_DISABLED_MESSAGE: &str =
    "Page.printToPDF is not supported: renderer layout is disabled; start Moli with --layout.";
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
    AddScriptToEvaluateOnNewDocument(DevToolsAddPreloadScriptCommand),
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
    BeforeUnloadNavigation(Box<navigation::PendingBeforeUnloadNavigationCommand>),
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
    AddScriptToEvaluateOnNewDocument(DevToolsAddPreloadScriptCommand),
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
    BeforeUnloadNavigation(Box<navigation::CompletedBeforeUnloadNavigationCommand>),
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
            | Self::BeforeUnloadNavigation(_)
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
            PendingPageCommandKind::AddScriptToEvaluateOnNewDocument(command) => {
                CompletedPageCommandKind::AddScriptToEvaluateOnNewDocument(command)
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
            PendingPageCommandKind::BeforeUnloadNavigation(pending) => {
                CompletedPageCommandKind::BeforeUnloadNavigation(Box::new(pending.wait().await))
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

#[derive(Debug, Default, PartialEq)]
pub(crate) struct PagePreparedOutputs {
    javascript_dialogs: Vec<javascript_dialog::PreparedJavaScriptDialog>,
    window_open_events: Vec<popup::PagePreparedWindowOpenEvent>,
    popup_activations: Vec<popup::PagePreparedPopupActivation>,
    auxiliary_window_closes: Vec<(String, moli_core::page::RendererAuxiliaryWindow)>,
    auxiliary_window_navigations: Vec<prepared_navigation::PagePreparedAuxiliaryWindowNavigation>,
    document_title_changes: Vec<RendererDocumentTitleChanged>,
    document_lifecycle_events: Vec<RendererDocumentLifecycleEvent>,
    child_frame_activities: Vec<PagePreparedChildFrameActivity>,
    same_document_navigations: Vec<PagePreparedSameDocumentNavigation>,
    session_history_updates: Vec<(
        crate::conn::TargetPageResidenceIdentity,
        RendererDocumentLifecycleIdentity,
        moli_page_types::SessionHistoryUpdate,
    )>,
    top_level_location_navigation: Option<PagePreparedTopLevelLocationNavigation>,
    top_level_history_traversal: Option<RendererPendingTopLevelHistoryTraversal>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct PagePreparedOutputSlot {
    outputs: PagePreparedOutputs,
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
) -> Result<Vec<BackgroundProtocolEvent>, String> {
    let (browser_context_id, target_id) = match route {
        CdpSessionRoute::Browser => return Ok(Vec::new()),
        CdpSessionRoute::BrowserContext { browser_context_id } => {
            if !conn
                .activate_browser_context_by_id_async(&browser_context_id)
                .await
            {
                return Err("BrowserContextNotLoaded".into());
            }
            return Ok(Vec::new());
        }
        CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            ..
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
        conn.select_browser_focus_for_target(&target_id);
        conn.apply_browser_document_activity_async()
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Vec::new());
    }

    match conn
        .select_page_target_for_connection_async(&target_id)
        .await
    {
        Ok(Some(activation)) => Ok(activation.into_protocol_events()),
        Ok(None) => Err("UnknownTargetId".into()),
        Err(error) => Err(error.to_string()),
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
    pending: PendingPageCommand,
}

pub struct CompletedPageScreencastCapture {
    session_id: Option<String>,
    generation: i32,
    owner_scope: CommandOwnerScope,
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
    Frame {
        event: BackgroundProtocolEvent,
        visual_state: RendererVisualStateToken,
    },
    Unchanged,
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
    let visible = conn
        .page_screencast_visible_for_session_owner(cmd.session_id)
        .unwrap_or(true);
    let mut plan = CommandOutputPlan::default();
    plan.push_background_event(BackgroundProtocolEvent::page_screencast_visibility_changed(
        cmd.session_id,
        visible,
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
        page_screencast_subscription_status_for_owner(
            self,
            &registration.owner_scope,
            registration.generation,
        )
    }

    pub fn start_page_screencast_frame_capture(
        &mut self,
        registration: &PageScreencastRegistration,
        known_visual_state: Option<RendererVisualStateToken>,
    ) -> PageScreencastCaptureStart {
        let session_id = registration.session_id().map(str::to_owned);
        let generation = registration.generation;
        let owner_scope = registration.owner_scope.clone();
        if page_screencast_subscription_status_for_owner(self, &owner_scope, generation)
            != PageScreencastSubscriptionStatus::Ready
        {
            return PageScreencastCaptureStart::Stale;
        }
        let Some(config) = self
            .target_page_session_state_for_owner(&owner_scope)
            .and_then(|state| state.page_screencast.config())
            .cloned()
        else {
            return PageScreencastCaptureStart::Stale;
        };
        let request = RendererCaptureScreencastFrameRequest {
            base_background_color: self.default_background_color_for_owner(&owner_scope),
            format: match config.format() {
                PageScreencastFormat::Png => RendererScreenshotFormat::Png,
                PageScreencastFormat::Jpeg => RendererScreenshotFormat::Jpeg,
            },
            quality: config.quality(),
            optimize_for_speed: true,
            max_width: config.max_width(),
            max_height: config.max_height(),
            known_visual_state,
        };
        let pending = match self.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
            Ok(page) => match page.start_capture_screencast_frame(request) {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::debug!(?error, "failed to start screencast frame capture");
                    return PageScreencastCaptureStart::Retry;
                }
            },
            Err(_) => return PageScreencastCaptureStart::Retry,
        };
        if self.begin_page_screencast_capture_for_owner(&owner_scope, generation) != Some(true) {
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
            completed,
        } = completed;
        let session_id_ref = session_id.as_deref();
        if page_screencast_subscription_status_for_owner(self, &owner_scope, generation)
            != PageScreencastSubscriptionStatus::CaptureInProgress
        {
            return PageScreencastCaptureCompletion::Stale;
        }

        let frame = match completed {
            Ok(completion) => {
                let page = match self.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
                    Ok(page) => page,
                    Err(_) => {
                        let _ = self.complete_page_screencast_capture_for_owner(
                            &owner_scope,
                            generation,
                            false,
                        );
                        return PageScreencastCaptureCompletion::Retry;
                    }
                };
                match page.finish_capture_screencast_frame(*completion) {
                    Ok(RendererCaptureScreencastFrameReply::Captured(frame)) => frame,
                    Ok(RendererCaptureScreencastFrameReply::Unchanged) => {
                        if self.complete_page_screencast_capture_for_owner(
                            &owner_scope,
                            generation,
                            false,
                        ) != Some(true)
                        {
                            return PageScreencastCaptureCompletion::Stale;
                        }
                        return PageScreencastCaptureCompletion::Unchanged;
                    }
                    Ok(
                        RendererCaptureScreencastFrameReply::LayoutDisabled
                        | RendererCaptureScreencastFrameReply::NoDocument,
                    )
                    | Err(_) => {
                        let _ = self.complete_page_screencast_capture_for_owner(
                            &owner_scope,
                            generation,
                            false,
                        );
                        return PageScreencastCaptureCompletion::Retry;
                    }
                }
            }
            Err(_) => {
                let _ = self.complete_page_screencast_capture_for_owner(
                    &owner_scope,
                    generation,
                    false,
                );
                return PageScreencastCaptureCompletion::Retry;
            }
        };

        let visual_state = frame.visual_state;
        if self.complete_page_screencast_capture_for_owner(&owner_scope, generation, true)
            != Some(true)
        {
            return PageScreencastCaptureCompletion::Stale;
        }
        let metadata = crate::conn::PageScreencastFrameMetadata {
            offset_top: 0.0,
            page_scale_factor: 1.0,
            device_width: f64::from(frame.viewport_size.0),
            device_height: f64::from(frame.viewport_size.1),
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        };
        PageScreencastCaptureCompletion::Frame {
            event: BackgroundProtocolEvent::page_screencast_frame(
                session_id_ref,
                BASE64_STANDARD.encode(&frame.image.bytes),
                metadata,
                generation,
            ),
            visual_state,
        }
    }
}

fn page_screencast_subscription_status_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    generation: i32,
) -> PageScreencastSubscriptionStatus {
    let Some(state) = conn.target_page_session_state_for_owner(owner) else {
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

pub(in crate::domains) async fn emit_popup_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    if let Some(popups) = prepared_outputs
        .page_mut()
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
    command_owner: &CommandOwnerScope,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    if let Some(navigation) = prepared_outputs
        .page_mut()
        .and_then(PagePreparedOutputSlot::take_top_level_location_navigation)
    {
        let (owner, navigation) = navigation.into_parts();
        conn.publish_prepared_top_level_location_navigation_owner_action(
            command_owner,
            owner,
            navigation,
        );
    }
}

pub(in crate::domains) async fn emit_top_level_history_traversal_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    if let Some(traversal) = prepared_outputs
        .page_mut()
        .and_then(PagePreparedOutputSlot::take_top_level_history_traversal)
    {
        traverse_session_owner_history_from_renderer_background_events_async(
            conn,
            out,
            owner,
            traversal.delta,
        )
        .await;
    }
}

pub(crate) async fn navigate_page_owned_top_level_location_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    command_owner: &CommandOwnerScope,
    owner: &crate::conn::TargetPageResidenceIdentity,
    navigation: RendererDocumentSourcedTopLevelLocationNavigation,
) {
    let session_id = command_owner.session_id();
    let source_document = navigation.source_document();
    if !conn.target_page_residence_identity_is_current(owner) {
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
    let navigation_history = url::Url::parse(navigation.url())
        .ok()
        .and_then(|url| {
            conn.renderer_navigation_request_for_owner(
                command_owner,
                &url,
                navigation.browser_navigation_kind()
                    == moli_fetch::BrowserNavigationRequestKind::Reload,
                navigation.navigation_history().cloned(),
            )
        })
        .map(|history| {
            history.with_about_document_state(navigation.about_document_state().cloned())
        });
    navigate_command_owner_from_renderer_request_background_events_async(
        conn,
        out,
        command_owner.clone(),
        navigation.url(),
        navigation.request_method(),
        navigation.request_body(),
        navigation.request_headers(),
        navigation.browser_navigation_kind(),
        navigation_history,
        navigation.web_mcp_invocation(),
        Some(match navigation.browser_navigation_kind() {
            moli_fetch::BrowserNavigationRequestKind::Reload => {
                moli_core::page::RendererAuxiliaryNavigationKind::Reload
            }
            moli_fetch::BrowserNavigationRequestKind::Navigate => {
                match navigation.history_mutation() {
                    moli_page_types::NavigationHistoryMutation::Push => {
                        moli_core::page::RendererAuxiliaryNavigationKind::Assign
                    }
                    moli_page_types::NavigationHistoryMutation::Replace => {
                        moli_core::page::RendererAuxiliaryNavigationKind::Replace
                    }
                }
            }
        }),
        None,
        navigation.initial_document_environment().cloned(),
        navigation.navigation_initiator().cloned(),
    )
    .await;
}

pub(crate) async fn navigate_command_owner_from_renderer_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    url: &str,
) {
    let pending_navigation = conn
        .target_owner_identity_for_owner(owner)
        .and_then(|(context_id, target_id)| {
            let target_id = target_id?;
            conn.browser_context_by_id_mut(&context_id)?
                .page_target_mut(&target_id)?
                .owner_state
                .pending_popup_navigation
                .take()
        })
        .filter(|pending| pending.url == url);
    let (document_response, navigation_initiator) = pending_navigation
        .map(|pending| (pending.response, pending.initiator))
        .unwrap_or_default();
    navigate_command_owner_from_renderer_request_background_events_async(
        conn,
        out,
        owner.clone(),
        url,
        "GET",
        None,
        &[],
        moli_fetch::BrowserNavigationRequestKind::Navigate,
        None,
        None,
        None,
        document_response,
        None,
        navigation_initiator,
    )
    .await;
}

pub(in crate::domains) async fn navigate_command_owner_from_renderer_request_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: CommandOwnerScope,
    url: &str,
    request_method: &str,
    request_body: Option<&[u8]>,
    request_headers: &[(String, String)],
    browser_navigation_kind: moli_fetch::BrowserNavigationRequestKind,
    navigation_history: Option<moli_core::RendererNavigationHistoryRequest>,
    web_mcp_invocation: Option<moli_page_types::RendererWebMcpNavigation>,
    auxiliary_navigation: Option<moli_core::page::RendererAuxiliaryNavigationKind>,
    document_response: Option<moli_core::page::RendererAuxiliaryDocumentResponse>,
    initial_document_environment: Option<moli_core::page::RendererCapturedDocumentEnvironment>,
    navigation_initiator: Option<moli_core::page::RendererNavigationInitiator>,
) {
    let start = navigation::start_session_owner_navigation_from_renderer(
        conn,
        &owner,
        url,
        request_method,
        request_body,
        request_headers,
        browser_navigation_kind,
        navigation_history,
        web_mcp_invocation,
        auxiliary_navigation,
        document_response,
        initial_document_environment,
        navigation_initiator,
    );
    let step =
        navigation::finish_started_navigation_command_for_parts(conn, None, owner, start, &[]);
    complete_renderer_navigation_step_background_events_async(conn, out, step).await;
}

pub(crate) async fn traverse_session_owner_history_from_renderer_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    delta: i64,
) {
    let step = navigation::start_session_owner_history_traversal_from_renderer(conn, owner, delta);
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
                // Navigation futures carry the response and renderer setup.
                // Keep them out of each enclosing target-navigation future.
                let completed = Box::pin(pending.wait()).await;
                step = Box::pin(complete_pending_page_command(
                    conn,
                    completed,
                    &mut command_context,
                ))
                .await;
            }
        }
    }
}

pub(crate) fn emit_page_window_open_background_events_for_owner(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    url: &str,
    window_name: &str,
    window_features: &[String],
    user_gesture: bool,
) {
    if !crate::domains::target::popup_activation_creates_new_target_for_owner(
        conn,
        owner,
        window_name,
    ) {
        return;
    }
    for event_session_id in conn.page_event_session_ids_for_owner(owner) {
        let event_owner = event_session_id
            .as_deref()
            .map(CommandOwnerScope::for_session)
            .unwrap_or_else(|| owner.clone());
        if conn.page_domain_enabled_for_owner(&event_owner) == Some(true) {
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
    owner: &CommandOwnerScope,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    if let Some(navigations) = prepared_outputs
        .page_mut()
        .and_then(PagePreparedOutputSlot::take_same_document_navigations)
    {
        navigation::emit_same_document_navigation_background_events_async(
            conn,
            out,
            owner,
            navigations,
        )
        .await;
    }
}

#[cfg(test)]
mod producer_tests;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageSetDownloadBehaviorParams {
    behavior: String,
    #[serde(default)]
    download_path: Option<String>,
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
    command: AutomationCommand,
    background_command_id: Option<u64>,
) -> (
    Result<AutomationResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    match command {
        AutomationCommand::GetFrameTree(command) => (
            execute_devtools_get_frame_tree_command_async(conn, command).await,
            Vec::new(),
            None,
        ),
        AutomationCommand::GetFrameTrees(command) => (
            execute_devtools_get_frame_trees_command_async(conn, command).await,
            Vec::new(),
            None,
        ),
        AutomationCommand::GetNavigationHistory(command) => (
            navigation::execute_devtools_get_navigation_history_command(conn, command),
            Vec::new(),
            None,
        ),
        AutomationCommand::GetLayoutMetrics(command) => (
            execute_devtools_get_layout_metrics_command(conn, command).await,
            Vec::new(),
            None,
        ),
        AutomationCommand::GetJavaScriptDialog(command) => (
            execute_devtools_get_javascript_dialog_command(conn, command),
            Vec::new(),
            None,
        ),
        AutomationCommand::SetJavaScriptDialogPromptText(command) => (
            execute_devtools_set_javascript_dialog_prompt_text_command(conn, command),
            Vec::new(),
            None,
        ),
        AutomationCommand::HandleJavaScriptDialog(command) => {
            let (result, events) = execute_devtools_handle_javascript_dialog_command(conn, command);
            (result, events, None)
        }
        AutomationCommand::CaptureScreenshot(command) => {
            let (result, predecessor) =
                execute_devtools_capture_screenshot_command(conn, command).await;
            (result, Vec::new(), predecessor)
        }
        AutomationCommand::PrintToPdf(command) => (
            execute_devtools_print_to_pdf_command(conn, command),
            Vec::new(),
            None,
        ),
        command @ (AutomationCommand::Navigate(_)
        | AutomationCommand::Reload(_)
        | AutomationCommand::TraverseHistory(_)) => {
            navigation::execute_devtools_navigation_command_async_with_protocol_events(
                conn,
                command,
                background_command_id,
            )
            .await
        }
        command @ (AutomationCommand::AddPreloadScript(_)
        | AutomationCommand::RemovePreloadScript(_)) => {
            preload::execute_devtools_preload_command_async(conn, command).await
        }
        _ => (
            Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedAutomationCommand",
            )),
            Vec::new(),
            None,
        ),
    }
}

fn page_route_for_context_id(
    conn: &CdpConnection,
    context_id: &str,
) -> Result<CdpSessionRoute, DevToolsError> {
    conn.target_session_route_for_target_id(context_id)
        .or_else(|| conn.target_session_route_for_child_frame_id(context_id))
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))
}

fn page_command_owner(
    conn: &CdpConnection,
    context: &AutomationContext,
) -> Result<CommandOwnerScope, DevToolsError> {
    conn.command_owner_scope_for_devtools_context(context)
        .ok_or_else(|| {
            if context.target_id.is_some() {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget")
            } else {
                devtools_frame_tree_error("TargetNotLoaded")
            }
        })
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
    match conn.runtime_session_owner_slot(cmd.session_id) {
        Ok(slot)
            if slot.has_loaded_page()
                && conn.runtime_session_owner_can_start_initial_document_navigation(
                    cmd.session_id,
                ) =>
        {
            let owner = CommandOwnerScope::capture(conn, cmd.session_id);
            let start = match navigation::start_initial_document_navigation_for_session_owner(
                conn,
                cmd.id,
                &owner,
                json!({}),
            ) {
                Ok(start) => start,
                Err(plan) => return Some(PageCommandTaskStep::Complete(plan)),
            };
            Some(navigation::finish_started_navigation_command_for_parts(
                conn,
                cmd.id,
                owner,
                start,
                &[],
            ))
        }
        Ok(slot) if slot.has_loaded_page() => {
            Some(PageCommandTaskStep::Complete(CommandOutputPlan::success()))
        }
        Ok(_) if !conn.runtime_session_owner_target_is_initial_about_blank(cmd.session_id) => {
            let owner = CommandOwnerScope::capture(conn, cmd.session_id);
            let start = match navigation::start_initial_document_navigation_for_session_owner(
                conn,
                cmd.id,
                &owner,
                json!({}),
            ) {
                Ok(start) => start,
                Err(plan) => return Some(PageCommandTaskStep::Complete(plan)),
            };
            Some(navigation::finish_started_navigation_command_for_parts(
                conn,
                cmd.id,
                owner,
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

fn start_devtools_page_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: AutomationCommand,
) -> PageCommandTaskStep {
    match command {
        AutomationCommand::GetFrameTree(command) => start_devtools_get_frame_tree_command(
            conn,
            command_id,
            command,
            FrameTreeCommandOutputKind::FrameTree,
        ),
        AutomationCommand::GetLayoutMetrics(command) => {
            start_devtools_get_layout_metrics_command(conn, command_id, command)
        }
        AutomationCommand::GetJavaScriptDialog(command) => {
            let owner = page_command_owner(conn, &command.context);
            PageCommandTaskStep::Complete(match owner {
                Ok(owner) => match finish_devtools_get_javascript_dialog_command(conn, &owner) {
                    Ok(result) => CommandOutputPlan::from_devtools_result(
                        AutomationResult::JavaScriptDialog(result),
                    ),
                    Err(error) => CommandOutputPlan::from_devtools_error(error),
                },
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            })
        }
        AutomationCommand::SetJavaScriptDialogPromptText(command) => {
            let owner = page_command_owner(conn, &command.context);
            PageCommandTaskStep::Complete(match owner {
                Ok(owner) => match finish_devtools_set_javascript_dialog_prompt_text_command(
                    conn, command, &owner,
                ) {
                    Ok(result) => CommandOutputPlan::from_devtools_result(result),
                    Err(error) => CommandOutputPlan::from_devtools_error(error),
                },
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            })
        }
        AutomationCommand::HandleJavaScriptDialog(command) => PageCommandTaskStep::Complete(
            complete_devtools_handle_javascript_dialog_command(conn, command),
        ),
        AutomationCommand::CaptureScreenshot(command) => {
            start_devtools_capture_screenshot_command(conn, command_id, command)
        }
        AutomationCommand::PrintToPdf(command) => {
            start_devtools_print_to_pdf_command(conn, command_id, command)
        }
        _ => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "UnsupportedAutomationCommand",
        )),
    }
}

#[cfg(test)]
mod protocol_neutral_tests;

pub(crate) async fn complete_pending_page_command(
    conn: &mut CdpConnection,
    completed: CompletedPageCommandDispatch,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let command_id = completed.command_id;
    let owner_scope = completed.owner_scope.clone();
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
                Ok(events) => {
                    let mut plan = CommandOutputPlan::default();
                    plan.extend_background_events(events);
                    plan.push_success();
                    plan
                }
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
                    .runtime_session_owner_slot_mut_for_owner(&owner_scope)
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
                .runtime_session_owner_slot_mut_for_owner(&owner_scope)
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
                &owner_scope,
                *completed,
            );
        }
        CompletedPageCommandKind::GetAppManifest(completed) => {
            return app_manifest::complete_get_app_manifest_command(
                conn,
                command_id,
                &owner_scope,
                *completed,
                command_context,
            );
        }
        CompletedPageCommandKind::ResetNavigationHistory { completed } => {
            return navigation::complete_reset_navigation_history_command(
                conn,
                &owner_scope,
                *completed,
            );
        }
        CompletedPageCommandKind::AddScriptToEvaluateOnNewDocument(command) => {
            return preload::complete_pending_add_script_to_evaluate_on_new_document_command(
                conn,
                &owner_scope,
                command,
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
                .runtime_session_owner_slot_mut_for_owner(&owner_scope)
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
                    .runtime_session_owner_slot_mut_for_owner(&owner_scope)
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
                &owner_scope,
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
                .ensure_document_accessible_for_owner(&owner_scope)
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
                .runtime_session_owner_slot_mut_for_owner(&owner_scope)
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
                .runtime_session_owner_slot_mut_for_owner(&owner_scope)
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
            let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match page.finish_layout_metrics(completion) {
                Ok(metrics) => CommandOutputPlan::from_devtools_result(
                    AutomationResult::LayoutMetrics(layout_metrics_result_from_renderer(metrics)),
                ),
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
            let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match page.finish_capture_screenshot(completion) {
                Ok(RendererCaptureScreenshotReply::Captured(image)) => {
                    CommandOutputPlan::from_devtools_result(AutomationResult::CaptureScreenshot(
                        DevToolsCaptureScreenshotResult {
                            mime_type: image.mime_type,
                            width: image.width,
                            height: image.height,
                            bytes: image.bytes,
                        },
                    ))
                }
                Ok(RendererCaptureScreenshotReply::LayoutDisabled) => {
                    CommandOutputPlan::error(-32000, CAPTURE_SCREENSHOT_LAYOUT_DISABLED_MESSAGE)
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
            let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
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
                    let stream = match conn.open_io_stream_body_source_for_owner(
                        &owner_scope,
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
                &owner_scope,
                *completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::ChildFrameNavigate(completed) => {
            return navigation::complete_pending_child_frame_navigate_command(
                conn,
                &owner_scope,
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
        CompletedPageCommandKind::BeforeUnloadNavigation(completed) => {
            return navigation::complete_pending_beforeunload_navigation_command(
                conn,
                *completed,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::TraverseSameDocumentHistory(completed) => {
            return navigation::complete_pending_same_document_history_traversal_command(
                conn, command_id, *completed,
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
                &owner_scope,
            )
            .await;
        }
        CompletedPageCommandKind::Crash => {
            return termination::complete_crash_command_dispatch(
                conn,
                command_id,
                &owner_scope,
                command_context,
            )
            .await;
        }
        CompletedPageCommandKind::Close => {
            return termination::complete_close_command_dispatch(
                conn,
                command_id,
                &owner_scope,
                command_context,
            )
            .await;
        }
    };
    PageCommandTaskStep::Complete(plan)
}

// ────────────────────────────────────────────────────────────────────────────

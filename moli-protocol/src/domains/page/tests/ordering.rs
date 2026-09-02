use crate::conn::CdpPendingCommandOrdering;
use crate::domains::actions::PageAction;

use super::super::page_action_pending_command_ordering;

#[test]
fn pending_page_ordering_tracks_chromium_handler_completion_contract() {
    use CdpPendingCommandOrdering::{Interleavable, SameSessionResponseBarrier};

    for action in [
        PageAction::Enable,
        PageAction::Disable,
        PageAction::SetLifecycleEventsEnabled,
        PageAction::SetBypassCsp,
        PageAction::SetFontFamilies,
        PageAction::SetInterceptFileChooserDialog,
        PageAction::HandleJavaScriptDialog,
        PageAction::SetDownloadBehavior,
        PageAction::StartScreencast,
        PageAction::StopScreencast,
        PageAction::ScreencastFrameAck,
        PageAction::GetNavigationHistory,
        PageAction::ResetNavigationHistory,
        PageAction::BringToFront,
        PageAction::SetDocumentContent,
        PageAction::GetFrameTree,
        PageAction::GetResourceTree,
        PageAction::GetLayoutMetrics,
        PageAction::NavigateToHistoryEntry,
        PageAction::StopLoading,
        PageAction::Crash,
        PageAction::Close,
        PageAction::AddScriptToEvaluateOnNewDocument,
        PageAction::RemoveScriptToEvaluateOnNewDocument,
    ] {
        assert_eq!(
            page_action_pending_command_ordering(action),
            SameSessionResponseBarrier,
            "Chromium completes {action:?} from a synchronous Response handler"
        );
    }

    for action in [
        PageAction::CaptureScreenshot,
        PageAction::CaptureSnapshot,
        PageAction::PrintToPdf,
        PageAction::GetAppManifest,
        PageAction::SearchInResource,
        PageAction::Navigate,
        PageAction::Reload,
        PageAction::CreateIsolatedWorld,
    ] {
        assert_eq!(
            page_action_pending_command_ordering(action),
            Interleavable,
            "Chromium completes {action:?} through a callback"
        );
    }
}

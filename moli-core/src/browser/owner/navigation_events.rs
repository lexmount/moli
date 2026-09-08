use super::Browser;
use crate::browser::{
    BrowserEvent, NavigationAttempt, NavigationFailureReason, NavigationId, NavigationRequest,
    NavigationSnapshot, WebContentsHandle,
};

impl Browser {
    pub(super) fn pending_navigation(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<NavigationRequest>, String> {
        Ok(
            match self
                .context(handle.context())?
                .navigation_snapshot(handle)?
                .attempt
            {
                Some(NavigationAttempt::Started(request)) => Some(request),
                _ => None,
            },
        )
    }

    pub(super) fn publish_failed_navigations(
        &mut self,
        snapshots: impl IntoIterator<Item = NavigationSnapshot>,
        reason: NavigationFailureReason,
    ) {
        for snapshot in snapshots {
            if let Some(NavigationAttempt::Started(request)) = snapshot.attempt {
                self.events
                    .publish(BrowserEvent::NavigationFailed { request, reason });
            }
        }
    }

    pub(super) fn cancel_navigation(
        &mut self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        reason: NavigationFailureReason,
    ) -> Result<bool, String> {
        let Some(request) = self
            .pending_navigation(handle)?
            .filter(|request| request.navigation == navigation)
        else {
            return Ok(false);
        };
        let canceled = self
            .context_mut(handle.context())?
            .cancel_document_navigation(handle, &navigation, reason)?;
        if canceled {
            self.navigation_work.remove_web_contents(handle);
            self.events
                .publish(BrowserEvent::NavigationFailed { request, reason });
        }
        Ok(canceled)
    }
}

use super::{Browser, BrowserContextHandle, BrowserHandle};
use crate::browser::{
    BrowserEvent, NavigationAttempt, NavigationFailureReason, NavigationId, NavigationRequest,
    NavigationSnapshot, WebContentsHandle,
};

impl Browser {
    pub(super) fn record_native_response(
        &mut self,
        response: crate::browser::NavigationResponseSnapshot,
    ) -> Result<(), String> {
        let request = response.request;
        let contents = self
            .context_mut(request.web_contents.context())?
            .web_contents_mut(request.web_contents)?;
        if !contents.navigation_mut().record_native_response(response)
            || !contents.arm_background_navigation_completion(&request.navigation, None)
        {
            return Err("stale native navigation response".into());
        }
        self.events
            .publish(BrowserEvent::NavigationResponseChanged(request));
        Ok(())
    }

    pub(super) fn complete_native_response(
        &mut self,
        request: NavigationRequest,
        body: Result<crate::browser::CapturedBody, String>,
    ) -> Result<(), String> {
        let contents = self
            .context_mut(request.web_contents.context())?
            .web_contents_mut(request.web_contents)?;
        let completed = contents
            .navigation_mut()
            .complete_native_response(request, body);
        contents.settle_background_navigation_completion(&request.navigation);
        if completed {
            self.events
                .publish(BrowserEvent::NavigationResponseChanged(request));
        }
        Ok(())
    }

    pub(super) fn start_navigation(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<NavigationId, String> {
        let previous = self
            .context(handle.context())?
            .navigation_snapshot(handle)?;
        let navigation = self
            .context_mut(handle.context())?
            .start_document_navigation(handle)?;
        self.navigation_work.remove_web_contents(handle);
        self.publish_failed_navigations([previous], NavigationFailureReason::Superseded);
        let request = self
            .pending_navigation(handle)?
            .expect("admitted navigation owns its reserved Document");
        self.events
            .publish(BrowserEvent::NavigationStarted(request));
        Ok(navigation)
    }

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

impl BrowserHandle {
    pub fn register_navigation_decision_provider(
        &self,
    ) -> Result<crate::browser::NavigationDecisionProvider, String> {
        self.execute(|browser| {
            if browser
                .navigation_decision_provider
                .as_ref()
                .is_some_and(|provider| provider.has_changed().is_ok())
            {
                return Err("Browser already has a navigation decision provider".to_owned());
            }
            let (alive, receiver) = tokio::sync::watch::channel(());
            browser.navigation_decision_provider = Some(receiver);
            Ok(crate::browser::NavigationDecisionProvider { _alive: alive })
        })?
    }
}

impl BrowserContextHandle {
    pub fn navigation_responses(
        &self,
        contents: WebContentsHandle,
    ) -> Result<Vec<crate::browser::NavigationResponseSnapshot>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            Ok(browser
                .context(context)?
                .web_contents(contents)?
                .navigation()
                .response_snapshots())
        })?
    }

    pub fn navigation_decision(
        &self,
        contents: WebContentsHandle,
    ) -> Result<Option<crate::browser::NavigationDecisionSnapshot>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            Ok(browser
                .context(context)?
                .web_contents(contents)?
                .navigation()
                .driver_decision())
        })?
    }

    pub fn native_initial_document_navigation(
        &self,
        contents: WebContentsHandle,
    ) -> Result<Option<NavigationId>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            let navigation = browser
                .context(context)?
                .web_contents(contents)?
                .navigation();
            Ok(navigation
                .pending_document()
                .and_then(|(id, _)| navigation.has_native_initial_document().then_some(id)))
        })?
    }

    pub fn resolve_navigation_decision(
        &self,
        contents: WebContentsHandle,
        permit: crate::browser::web_contents::NavigationInterceptionPermit,
        decision: crate::browser::NavigationDecision,
    ) -> Result<bool, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            let contents = browser.context_mut(context)?.web_contents_mut(contents)?;
            if contents.id() != permit.web_contents() {
                return Ok(false);
            }
            Ok(contents
                .navigation_mut()
                .resolve_driver_decision(permit, decision))
        })?
    }
}

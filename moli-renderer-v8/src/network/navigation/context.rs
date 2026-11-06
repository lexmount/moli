use std::sync::Arc;

use moli_cookie_jar::BrowserCookieFacadeContext;
use url::Url;

use crate::network::{BrowserResourceRuntime, PageNetworkPolicy, RendererResourceTaskRunner};

/// Immutable input transferred from one successful navigation attempt to the
/// resource authority of the committed Document.
///
/// This deliberately stores logical request context, never a host filesystem
/// path or an ambient "current Document" lookup. More Document policy fields
/// are added here as their callers move into exact Document authority.
#[derive(Clone, Debug)]
pub struct DocumentFetchContextSeed {
    requested_url: Url,
    final_url: Url,
    resource_runtime: BrowserResourceRuntime,
    page_network_policy: PageNetworkPolicy,
    browser_site_context: Option<Arc<BrowserCookieFacadeContext>>,
    resource_task_runner: RendererResourceTaskRunner,
    timing: super::NavigationTiming,
}

impl DocumentFetchContextSeed {
    pub(super) fn new(
        requested_url: Url,
        final_url: Url,
        resource_runtime: BrowserResourceRuntime,
        page_network_policy: PageNetworkPolicy,
        browser_site_context: Option<Arc<BrowserCookieFacadeContext>>,
        resource_task_runner: RendererResourceTaskRunner,
        timing: super::NavigationTiming,
    ) -> Self {
        Self {
            requested_url,
            final_url,
            resource_runtime,
            page_network_policy,
            browser_site_context,
            resource_task_runner,
            timing,
        }
    }

    pub fn requested_url(&self) -> &Url {
        &self.requested_url
    }

    pub(crate) fn navigation_timing(&self) -> super::NavigationTiming {
        self.timing.clone()
    }

    pub fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub fn browser_resource_runtime(&self) -> BrowserResourceRuntime {
        self.resource_runtime.clone()
    }

    pub fn page_network_policy(&self) -> PageNetworkPolicy {
        self.page_network_policy.clone()
    }

    #[cfg(test)]
    pub(crate) fn browser_site_context(&self) -> Option<&BrowserCookieFacadeContext> {
        self.browser_site_context.as_deref()
    }

    pub(crate) fn shared_browser_site_context(&self) -> Option<Arc<BrowserCookieFacadeContext>> {
        self.browser_site_context.clone()
    }

    pub(crate) fn resource_task_runner(&self) -> RendererResourceTaskRunner {
        self.resource_task_runner.clone()
    }
}

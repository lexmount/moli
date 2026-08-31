use moli_cookie_jar::{BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides};
#[cfg(test)]
use moli_core::page::Page;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextDocumentCookiePolicySurfaceSnapshot {
    pub(crate) overrides: BrowserCookieFacadeOverrides,
    pub(crate) cookies_enabled_override: Option<bool>,
    pub(crate) browser_context_overrides: BrowserCookieFacadeContextOverrides,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BrowserContextDocumentCookiePolicySurface {
    overrides: BrowserCookieFacadeOverrides,
    generation: u64,
}

impl BrowserContextDocumentCookiePolicySurface {
    pub(crate) fn snapshot(&self) -> BrowserContextDocumentCookiePolicySurfaceSnapshot {
        BrowserContextDocumentCookiePolicySurfaceSnapshot {
            overrides: self.overrides.clone(),
            cookies_enabled_override: self.cookies_enabled_override(),
            browser_context_overrides: self.browser_context_overrides(),
            generation: self.generation,
        }
    }

    #[cfg(test)]
    pub(crate) fn overrides(&self) -> &BrowserCookieFacadeOverrides {
        &self.overrides
    }

    pub(crate) fn cookies_enabled_override(&self) -> Option<bool> {
        self.overrides.cookies_enabled
    }

    pub(crate) fn browser_context_overrides(&self) -> BrowserCookieFacadeContextOverrides {
        self.overrides.browser_context_overrides()
    }

    #[cfg(test)]
    pub(crate) fn set_overrides(&mut self, overrides: &BrowserCookieFacadeOverrides) -> bool {
        if self.overrides == *overrides {
            return false;
        }
        self.overrides = overrides.clone();
        self.generation = self.generation.wrapping_add(1);
        true
    }

    #[cfg(test)]
    pub(crate) fn clear_overrides(&mut self) -> bool {
        self.set_overrides(&BrowserCookieFacadeOverrides::default())
    }

    #[cfg(test)]
    pub(crate) fn set_cookies_enabled_override(&mut self, enabled: bool) -> bool {
        self.set_overrides(&self.overrides.clone().with_cookies_enabled(enabled))
    }

    #[cfg(test)]
    pub(crate) fn clear_cookies_enabled_override(&mut self) -> bool {
        let mut overrides = self.overrides.clone();
        if overrides.cookies_enabled.is_none() {
            return false;
        }
        overrides.cookies_enabled = None;
        self.set_overrides(&overrides)
    }

    #[cfg(test)]
    pub(crate) fn set_browser_context_overrides(
        &mut self,
        overrides: &BrowserCookieFacadeContextOverrides,
    ) -> bool {
        let mut combined = self.overrides.clone();
        combined.site_for_cookies_url = overrides.site_for_cookies_url.clone();
        combined.top_frame_origin_url = overrides.top_frame_origin_url.clone();
        combined.storage_access_status = overrides.storage_access_status;
        self.set_overrides(&combined)
    }

    #[cfg(test)]
    pub(crate) fn clear_browser_context_overrides(&mut self) -> bool {
        let mut overrides = self.overrides.clone();
        let changed = overrides.site_for_cookies_url.is_some()
            || overrides.top_frame_origin_url.is_some()
            || overrides.storage_access_status.is_some();
        if !changed {
            return false;
        }
        overrides.site_for_cookies_url = None;
        overrides.top_frame_origin_url = None;
        overrides.storage_access_status = None;
        self.set_overrides(&overrides)
    }

    #[cfg(test)]
    pub(crate) async fn apply_to_page_async(&self, page: &mut Page) {
        if self.overrides == BrowserCookieFacadeOverrides::default() {
            // See the sync variant above for why empty policy means clearing
            // the live document facade rather than applying empty overrides.
            let _ = page.clear_document_cookie_facade_overrides_async().await;
        } else {
            let _ = page
                .apply_document_cookie_facade_overrides_async(self.overrides())
                .await;
        }
    }
}

use anyhow::Result;

use super::{
    BrowserContext, CdpConnection,
    cookie_owner::{
        BrowserContextCookieBoundaryOperationReport, BrowserContextCookieBoundarySnapshot,
    },
};
use moli_cookie_jar::{
    CookieSiteDataClearPreviewReport, CookieSiteDataClearReport, CookieSiteDataClearScope,
    CookieSiteDataOperation, CookieSiteDataOperationPreviewReport, CookieSiteDataOperationReport,
    CookieSiteDataScope, CookieSiteDataSummary, CookieStorageClearPreviewReport,
    CookieStorageClearReport, CookieStorageClearTarget, CookieStorageStateSnapshot,
};

impl CdpConnection {
    pub(crate) fn cookie_sites(&mut self) -> Result<Vec<String>> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.sites_with_cookies())
    }

    pub(crate) fn cookie_site_data(&mut self) -> Result<Vec<CookieSiteDataSummary>> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.cookie_site_data())
    }

    pub(crate) fn cookie_storage_state_snapshot(&mut self) -> Result<CookieStorageStateSnapshot> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.cookie_storage_state_snapshot())
    }

    pub(crate) fn cookie_storage_state_snapshot_for_sites(
        &mut self,
        sites: &[&str],
    ) -> Result<CookieStorageStateSnapshot> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.cookie_storage_state_snapshot_for_sites(sites))
    }

    pub(crate) fn cookie_boundary_snapshot(
        &self,
    ) -> Result<BrowserContextCookieBoundarySnapshot, String> {
        self.browser_context
            .as_ref()
            .map(BrowserContext::cookie_boundary_snapshot)
            .ok_or_else(|| "BrowserContextNotLoaded".to_owned())
    }

    pub(crate) fn cookie_boundary_snapshot_for_sites(
        &self,
        sites: &[&str],
    ) -> Result<BrowserContextCookieBoundarySnapshot, String> {
        self.browser_context
            .as_ref()
            .map(|bc| bc.cookie_boundary_snapshot_for_sites(sites))
            .ok_or_else(|| "BrowserContextNotLoaded".to_owned())
    }

    pub(crate) fn apply_cookie_boundary_operation(
        &mut self,
        operation: &CookieSiteDataOperation,
    ) -> Result<BrowserContextCookieBoundaryOperationReport, String> {
        let browser_context = self
            .browser_context
            .as_ref()
            .ok_or_else(|| "BrowserContextNotLoaded".to_owned())?;
        browser_context.apply_cookie_boundary_operation(operation)
    }

    pub(crate) fn clear_cookies_for_sites(&mut self, sites: &[&str]) -> Result<usize> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.clear_cookies_for_sites(sites))
    }

    pub(crate) fn clear_cookies_for_sites_with_report(
        &mut self,
        sites: &[&str],
    ) -> Result<CookieSiteDataClearReport> {
        self.clear_cookies_for_sites_with_scope_and_report(sites, CookieSiteDataClearScope::All)
    }

    pub(crate) fn clear_cookies_for_sites_with_scope_and_report(
        &mut self,
        sites: &[&str],
        scope: CookieSiteDataClearScope,
    ) -> Result<CookieSiteDataClearReport> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.clear_cookies_for_sites_with_scope_and_report(sites, scope))
    }

    pub(crate) fn preview_clear_cookies_for_sites(
        &mut self,
        sites: &[&str],
    ) -> Result<CookieSiteDataClearPreviewReport> {
        self.preview_clear_cookies_for_sites_with_scope(sites, CookieSiteDataClearScope::All)
    }

    pub(crate) fn preview_clear_cookies_for_sites_with_scope(
        &mut self,
        sites: &[&str],
        scope: CookieSiteDataClearScope,
    ) -> Result<CookieSiteDataClearPreviewReport> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.preview_clear_cookies_for_sites_with_scope(sites, scope))
    }

    pub(crate) fn preview_clear_cookie_store_with_scope(
        &mut self,
        scope: CookieSiteDataClearScope,
    ) -> Result<CookieStorageClearPreviewReport> {
        self.preview_clear_cookie_storage_with_target_and_scope(
            CookieStorageClearTarget::WholeStore,
            scope,
        )
    }

    pub(crate) fn preview_clear_cookie_storage_with_target_and_scope(
        &mut self,
        target: CookieStorageClearTarget,
        scope: CookieSiteDataClearScope,
    ) -> Result<CookieStorageClearPreviewReport> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.preview_clear_with_scope_and_target(target, scope))
    }

    pub(crate) fn clear_cookie_store_with_scope_and_report(
        &mut self,
        scope: CookieSiteDataClearScope,
    ) -> Result<CookieStorageClearReport> {
        self.clear_cookie_storage_with_target_and_scope_and_report(
            CookieStorageClearTarget::WholeStore,
            scope,
        )
    }

    pub(crate) fn clear_cookie_storage_with_target_and_scope_and_report(
        &mut self,
        target: CookieStorageClearTarget,
        scope: CookieSiteDataClearScope,
    ) -> Result<CookieStorageClearReport> {
        let cookie_store = self.ensure_cookie_store()?;
        let mut cookie_store = cookie_store.lock()?;
        Ok(cookie_store.clear_with_scope_and_target_report(target, scope))
    }
}

impl BrowserContext {
    pub(crate) fn cookie_sites(&self) -> Vec<String> {
        self.with_cookie_store_mut(|store| store.sites_with_cookies())
    }

    pub(crate) fn cookie_site_data(&self) -> Vec<CookieSiteDataSummary> {
        self.with_cookie_store_mut(|store| store.cookie_site_data())
    }

    pub(crate) fn cookie_site_data_with_scope(
        &self,
        scope: CookieSiteDataScope,
    ) -> Vec<CookieSiteDataSummary> {
        self.with_cookie_store_mut(|store| store.cookie_site_data_with_scope(scope))
    }

    pub(crate) fn cookie_storage_state_snapshot(&self) -> CookieStorageStateSnapshot {
        self.with_cookie_store_mut(|store| store.cookie_storage_state_snapshot())
    }

    pub(crate) fn cookie_storage_state_snapshot_for_sites(
        &self,
        sites: &[&str],
    ) -> CookieStorageStateSnapshot {
        self.with_cookie_store_mut(|store| store.cookie_storage_state_snapshot_for_sites(sites))
    }

    pub(crate) fn preview_cookie_site_data_operation(
        &self,
        operation: &CookieSiteDataOperation,
    ) -> Result<CookieSiteDataOperationPreviewReport, String> {
        self.with_cookie_store_mut(|store| store.preview_site_data_operation(operation))
    }

    pub(crate) fn apply_cookie_site_data_operation(
        &self,
        operation: &CookieSiteDataOperation,
    ) -> Result<CookieSiteDataOperationReport, String> {
        self.with_cookie_store_mut(|store| store.apply_site_data_operation(operation))
    }

    pub(crate) fn clear_cookies_for_sites(&self, sites: &[&str]) -> usize {
        self.with_cookie_store_mut(|store| store.clear_cookies_for_sites(sites))
    }

    pub(crate) fn clear_cookies_for_sites_with_report(
        &self,
        sites: &[&str],
    ) -> CookieSiteDataClearReport {
        self.clear_cookies_for_sites_with_scope_and_report(sites, CookieSiteDataClearScope::All)
    }

    pub(crate) fn clear_cookies_for_sites_with_scope_and_report(
        &self,
        sites: &[&str],
        scope: CookieSiteDataClearScope,
    ) -> CookieSiteDataClearReport {
        self.with_cookie_store_mut(|store| {
            store.clear_cookies_for_sites_with_scope_and_report(sites, scope)
        })
    }

    pub(crate) fn preview_clear_cookies_for_sites(
        &self,
        sites: &[&str],
    ) -> CookieSiteDataClearPreviewReport {
        self.preview_clear_cookies_for_sites_with_scope(sites, CookieSiteDataClearScope::All)
    }

    pub(crate) fn preview_clear_cookies_for_sites_with_scope(
        &self,
        sites: &[&str],
        scope: CookieSiteDataClearScope,
    ) -> CookieSiteDataClearPreviewReport {
        self.with_cookie_store_mut(|store| {
            store.preview_clear_cookies_for_sites_with_scope(sites, scope)
        })
    }

    pub(crate) fn preview_clear_cookie_store(&self) -> CookieStorageClearPreviewReport {
        self.preview_clear_cookie_store_with_scope(CookieSiteDataClearScope::All)
    }

    pub(crate) fn preview_clear_cookie_store_with_scope(
        &self,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearPreviewReport {
        self.preview_clear_cookie_storage_with_target_and_scope(
            CookieStorageClearTarget::WholeStore,
            scope,
        )
    }

    pub(crate) fn preview_clear_cookie_storage_with_target_and_scope(
        &self,
        target: CookieStorageClearTarget,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearPreviewReport {
        self.with_cookie_store_mut(|store| store.preview_clear_with_scope_and_target(target, scope))
    }
}

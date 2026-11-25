//! Site-data operations implemented on `BrowserCookieStore`.

use std::collections::{BTreeMap, BTreeSet};

use cookie_store::CookieStore;
use moli_site::{registrable_site_host, site_key_for_host};

use super::BrowserCookieStore;
#[cfg(any(test, feature = "test-support"))]
use crate::{
    CookieSiteDataClearPreviewReport, CookieSiteDataOperation,
    CookieSiteDataOperationPreviewReport, CookieSiteDataOperationReport,
    CookieStorageClearPreviewReport,
};
use crate::{
    CookieSiteDataClearReport, CookieSiteDataClearScope, CookieSiteDataScope,
    CookieSiteDataSummary, CookieStorageClearReport, CookieStorageClearTarget,
    CookieStorageStateDiff, CookieStorageStateSnapshot, NetworkCookieRequestContext,
};

impl BrowserCookieStore {
    /// Clears the response site's unpartitioned cookies and cookies in the request partition.
    pub fn clear_cookies_for_response_site(
        &mut self,
        response_url: &url::Url,
        request_context: &NetworkCookieRequestContext,
    ) -> usize {
        let Some(host) = response_url.host_str() else {
            return 0;
        };
        let site = registrable_site_host(host);
        let partition = crate::model::cookie_partition_key_for_url(
            &request_context.browser_context,
            response_url,
        );
        self.delete_cookies_matching(|cookie| {
            registrable_site_host(&cookie.domain) == site
                && cookie
                    .partition_key
                    .as_ref()
                    .is_none_or(|key| key == &partition)
        })
    }

    /// Clears all cookies from the live store.
    pub fn clear(&mut self) {
        let _ = self.clear_with_scope_and_report(CookieSiteDataClearScope::All);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn preview_clear_with_scope(
        &mut self,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearPreviewReport {
        self.preview_clear_with_scope_and_target(CookieStorageClearTarget::WholeStore, scope)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn preview_clear_with_scope_and_target(
        &mut self,
        target: CookieStorageClearTarget,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearPreviewReport {
        match target {
            CookieStorageClearTarget::WholeStore => {
                self.preview_whole_store_clear_with_scope(scope)
            }
            CookieStorageClearTarget::RegistrableSites(sites) => {
                storage_clear_preview_from_site_preview(
                    self.preview_clear_cookies_for_sites_with_scope(
                        &sites.iter().map(String::as_str).collect::<Vec<_>>(),
                        scope,
                    ),
                )
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn preview_whole_store_clear_with_scope(
        &mut self,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearPreviewReport {
        self.purge_expired();
        let replaced_state = self.cookie_storage_state_snapshot();
        let resulting_state = targeted_clear_resulting_state(&replaced_state, scope);
        CookieStorageClearPreviewReport {
            target: CookieStorageClearTarget::WholeStore,
            scope,
            would_remove_cookie_count: clear_scope_cookie_count(&replaced_state, scope),
            state_diff: diff_storage_state_snapshots(&replaced_state, &resulting_state),
            replaced_state,
            resulting_state,
        }
    }

    /// Clears the whole store for the requested scope and returns before/after state.
    pub fn clear_with_scope_and_report(
        &mut self,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearReport {
        self.clear_with_scope_and_target_report(CookieStorageClearTarget::WholeStore, scope)
    }

    /// Clears either the whole store or selected registrable sites.
    pub fn clear_with_scope_and_target_report(
        &mut self,
        target: CookieStorageClearTarget,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearReport {
        match target {
            CookieStorageClearTarget::WholeStore => {
                self.clear_whole_store_with_scope_and_report(scope)
            }
            CookieStorageClearTarget::RegistrableSites(sites) => {
                storage_clear_report_from_site_report(
                    self.clear_cookies_for_sites_with_scope_and_report(
                        &sites.iter().map(String::as_str).collect::<Vec<_>>(),
                        scope,
                    ),
                )
            }
        }
    }

    fn clear_whole_store_with_scope_and_report(
        &mut self,
        scope: CookieSiteDataClearScope,
    ) -> CookieStorageClearReport {
        self.purge_expired();
        let replaced_state = self.cookie_storage_state_snapshot();
        let cookie_keys = self
            .full_core
            .iter_unexpired()
            .filter(|&cookie| clear_scope_matches_cookie(scope, cookie.is_persistent()))
            .map(|cookie| {
                let cookie = super::super::model::stored_cookie_from_core(cookie);
                (cookie.domain, cookie.path, cookie.name)
            })
            .collect::<Vec<_>>();
        let mut removed = 0;
        for (domain, path, name) in cookie_keys {
            if self.full_core.remove(&domain, &path, &name).is_some() {
                removed += 1;
            }
        }
        if removed > 0 {
            self.bump_document_cookie_generation();
        }
        let resulting_state = self.cookie_storage_state_snapshot();
        CookieStorageClearReport {
            target: CookieStorageClearTarget::WholeStore,
            scope,
            removed_cookie_count: removed,
            state_diff: diff_storage_state_snapshots(&replaced_state, &resulting_state),
            replaced_state,
            resulting_state,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn preview_site_data_operation(
        &mut self,
        operation: &CookieSiteDataOperation,
    ) -> Result<CookieSiteDataOperationPreviewReport, String> {
        match operation {
            CookieSiteDataOperation::Clear { target, scope } => {
                Ok(CookieSiteDataOperationPreviewReport::Clear(
                    self.preview_clear_with_scope_and_target(target.clone(), *scope),
                ))
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn apply_site_data_operation(
        &mut self,
        operation: &CookieSiteDataOperation,
    ) -> Result<CookieSiteDataOperationReport, String> {
        match operation {
            CookieSiteDataOperation::Clear { target, scope } => {
                Ok(CookieSiteDataOperationReport::Clear(
                    self.clear_with_scope_and_target_report(target.clone(), *scope),
                ))
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn sites_with_cookies(&mut self) -> Vec<String> {
        self.cookie_site_data()
            .into_iter()
            .map(|site| site.name)
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cookie_site_data(&mut self) -> Vec<CookieSiteDataSummary> {
        self.cookie_site_data_with_scope(CookieSiteDataScope::Live)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cookie_site_data_with_scope(
        &mut self,
        scope: CookieSiteDataScope,
    ) -> Vec<CookieSiteDataSummary> {
        self.cookie_site_data_with_scope_and_sites(scope, None)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn cookie_site_data_with_scope_and_sites(
        &mut self,
        scope: CookieSiteDataScope,
        wanted_sites: Option<&BTreeSet<String>>,
    ) -> Vec<CookieSiteDataSummary> {
        self.purge_expired();
        Self::cookie_site_data_with_scope_and_sites_from_core(&self.full_core, scope, wanted_sites)
    }

    pub(super) fn cookie_site_data_with_scope_and_sites_from_core(
        full_core: &CookieStore,
        scope: CookieSiteDataScope,
        wanted_sites: Option<&BTreeSet<String>>,
    ) -> Vec<CookieSiteDataSummary> {
        // Keep a browser-boundary site-data seam separate from the canonical
        // core. The fork owns matching/storage rules, while Moli groups
        // accepted cookies by registrable site so future Servo-style site-data
        // management can reuse one stable summary entry point.
        let mut site_counts = BTreeMap::<String, (usize, usize)>::new();
        for cookie in full_core.iter_unexpired() {
            if matches!(scope, CookieSiteDataScope::Persistent) && !cookie.is_persistent() {
                continue;
            }
            let cookie = super::super::model::stored_cookie_from_core(cookie);
            if let Some(site) = site_key_for_host(&cookie.domain) {
                if let Some(wanted_sites) = wanted_sites
                    && !wanted_sites.contains(&site)
                {
                    continue;
                }
                let entry = site_counts.entry(site).or_insert((0, 0));
                entry.0 += 1;
                if cookie.expires.is_some() {
                    entry.1 += 1;
                }
            }
        }
        site_counts
            .into_iter()
            .map(|(name, (cookie_count, persistent_cookie_count))| {
                CookieSiteDataSummary::new(name, cookie_count, persistent_cookie_count)
            })
            .collect()
    }

    pub fn cookie_storage_state_snapshot(&mut self) -> CookieStorageStateSnapshot {
        self.cookie_storage_state_snapshot_for_site_keys(None)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cookie_storage_state_snapshot_for_sites(
        &mut self,
        sites: &[&str],
    ) -> CookieStorageStateSnapshot {
        let wanted_sites = normalized_site_keys(sites);
        self.cookie_storage_state_snapshot_for_site_keys(Some(&wanted_sites))
    }

    pub(super) fn cookie_storage_state_snapshot_for_site_keys(
        &mut self,
        wanted_sites: Option<&BTreeSet<String>>,
    ) -> CookieStorageStateSnapshot {
        self.purge_expired();
        Self::cookie_storage_state_snapshot_from_core(
            &self.full_core,
            wanted_sites,
            Some(self.document_cookie_generation),
        )
    }

    pub(super) fn cookie_storage_state_snapshot_from_core(
        full_core: &CookieStore,
        wanted_sites: Option<&BTreeSet<String>>,
        store_generation: Option<u64>,
    ) -> CookieStorageStateSnapshot {
        let live_site_data = Self::cookie_site_data_with_scope_and_sites_from_core(
            full_core,
            CookieSiteDataScope::Live,
            wanted_sites,
        );
        let persistent_site_data = Self::cookie_site_data_with_scope_and_sites_from_core(
            full_core,
            CookieSiteDataScope::Persistent,
            wanted_sites,
        );
        CookieStorageStateSnapshot {
            store_generation,
            live_cookie_count: live_site_data.iter().map(|site| site.cookie_count).sum(),
            live_site_data,
            persistent_cookie_count: persistent_site_data
                .iter()
                .map(|site| site.cookie_count)
                .sum(),
            persistent_site_data,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn clear_cookies_for_sites(&mut self, sites: &[&str]) -> usize {
        self.clear_cookies_for_sites_with_scope_and_report(sites, CookieSiteDataClearScope::All)
            .removed_cookie_count
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn preview_clear_cookies_for_sites(
        &mut self,
        sites: &[&str],
    ) -> CookieSiteDataClearPreviewReport {
        self.preview_clear_cookies_for_sites_with_scope(sites, CookieSiteDataClearScope::All)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn preview_clear_cookies_for_sites_with_scope(
        &mut self,
        sites: &[&str],
        scope: CookieSiteDataClearScope,
    ) -> CookieSiteDataClearPreviewReport {
        self.purge_expired();
        let wanted_sites = normalized_site_keys(sites);
        let requested_sites = wanted_sites.iter().cloned().collect::<Vec<_>>();
        let replaced_state = self.cookie_storage_state_snapshot_for_site_keys(Some(&wanted_sites));
        let targeted_resulting_state = targeted_clear_resulting_state(&replaced_state, scope);

        // Keep preview as a pure browser-boundary observation seam. Site-data
        // managers need the targeted slice that would be replaced plus the
        // resulting global state, but preview must not mutate the live store
        // or advance document-cookie generations/cache invalidation.
        let current_state = self.cookie_storage_state_snapshot();
        let resulting_state = rebuild_storage_state_snapshot(
            None,
            filter_out_sites(&current_state.live_site_data, &wanted_sites),
            filter_out_sites_by_clear_scope(
                &current_state.persistent_site_data,
                &wanted_sites,
                scope,
            ),
        );
        let resulting_state =
            merge_storage_state_snapshots(resulting_state, &targeted_resulting_state);

        CookieSiteDataClearPreviewReport {
            scope,
            requested_sites,
            would_remove_cookie_count: clear_scope_cookie_count(&replaced_state, scope),
            state_diff: diff_storage_state_snapshots(&replaced_state, &targeted_resulting_state),
            replaced_state,
            resulting_state,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn clear_cookies_for_sites_with_report(
        &mut self,
        sites: &[&str],
    ) -> CookieSiteDataClearReport {
        self.clear_cookies_for_sites_with_scope_and_report(sites, CookieSiteDataClearScope::All)
    }

    pub fn clear_cookies_for_sites_with_scope_and_report(
        &mut self,
        sites: &[&str],
        scope: CookieSiteDataClearScope,
    ) -> CookieSiteDataClearReport {
        self.purge_expired();
        let wanted_sites = normalized_site_keys(sites);
        let requested_sites = wanted_sites.iter().cloned().collect::<Vec<_>>();
        if wanted_sites.is_empty() {
            return CookieSiteDataClearReport {
                scope,
                requested_sites,
                removed_cookie_count: 0,
                state_diff: CookieStorageStateDiff::default(),
                replaced_state: self
                    .cookie_storage_state_snapshot_for_site_keys(Some(&wanted_sites)),
                resulting_state: self
                    .cookie_storage_state_snapshot_for_site_keys(Some(&wanted_sites)),
            };
        }
        let replaced_state = self.cookie_storage_state_snapshot_for_site_keys(Some(&wanted_sites));

        // Servo keeps a first-class site-data seam for cookie clearing keyed
        // by eTLD+1/site identity rather than ad hoc domain filters. Keep the
        // same browser-boundary API here so site-scoped state management has
        // one stable entry point even though the core still owns canonical
        // cookie matching and deletion.
        let cookie_keys = self
            .full_core
            .iter_unexpired()
            .filter_map(|cookie| {
                let is_persistent = cookie.is_persistent();
                let cookie = super::super::model::stored_cookie_from_core(cookie);
                let site = site_key_for_host(&cookie.domain)?;
                (wanted_sites.contains(&site) && clear_scope_matches_cookie(scope, is_persistent))
                    .then_some((cookie.domain, cookie.path, cookie.name))
            })
            .collect::<Vec<_>>();

        let mut removed = 0;
        for (domain, path, name) in cookie_keys {
            if self.full_core.remove(&domain, &path, &name).is_some() {
                removed += 1;
            }
        }
        if removed > 0 {
            self.bump_document_cookie_generation();
        }
        let resulting_state = self.cookie_storage_state_snapshot_for_site_keys(Some(&wanted_sites));
        CookieSiteDataClearReport {
            scope,
            requested_sites,
            removed_cookie_count: removed,
            state_diff: diff_storage_state_snapshots(&replaced_state, &resulting_state),
            replaced_state,
            resulting_state,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn filter_out_sites(
    site_data: &[CookieSiteDataSummary],
    wanted_sites: &BTreeSet<String>,
) -> Vec<CookieSiteDataSummary> {
    site_data
        .iter()
        .filter(|site| !wanted_sites.contains(&site.name))
        .cloned()
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn filter_out_sites_by_clear_scope(
    site_data: &[CookieSiteDataSummary],
    wanted_sites: &BTreeSet<String>,
    scope: CookieSiteDataClearScope,
) -> Vec<CookieSiteDataSummary> {
    if matches!(
        scope,
        CookieSiteDataClearScope::Persistent | CookieSiteDataClearScope::All
    ) {
        return filter_out_sites(site_data, wanted_sites);
    }
    site_data.to_vec()
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn merge_site_data_summaries(
    base: Vec<CookieSiteDataSummary>,
    replacement: Vec<CookieSiteDataSummary>,
) -> Vec<CookieSiteDataSummary> {
    let mut merged = BTreeMap::new();
    for site in base.into_iter().chain(replacement) {
        merged.insert(site.name.clone(), site);
    }
    merged.into_values().collect()
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn merge_storage_state_snapshots(
    base: CookieStorageStateSnapshot,
    replacement: &CookieStorageStateSnapshot,
) -> CookieStorageStateSnapshot {
    rebuild_storage_state_snapshot(
        base.store_generation,
        merge_site_data_summaries(base.live_site_data, replacement.live_site_data.clone()),
        merge_site_data_summaries(
            base.persistent_site_data,
            replacement.persistent_site_data.clone(),
        ),
    )
}

pub(super) fn diff_storage_state_snapshots(
    before: &CookieStorageStateSnapshot,
    after: &CookieStorageStateSnapshot,
) -> CookieStorageStateDiff {
    CookieStorageStateDiff {
        live_site_changes: diff_site_data_summaries(&before.live_site_data, &after.live_site_data),
        persistent_site_changes: diff_site_data_summaries(
            &before.persistent_site_data,
            &after.persistent_site_data,
        ),
    }
}

fn diff_site_data_summaries(
    before: &[CookieSiteDataSummary],
    after: &[CookieSiteDataSummary],
) -> Vec<super::super::model::CookieSiteDataChange> {
    let before_map = before
        .iter()
        .cloned()
        .map(|site| (site.name.clone(), site))
        .collect::<BTreeMap<_, _>>();
    let after_map = after
        .iter()
        .cloned()
        .map(|site| (site.name.clone(), site))
        .collect::<BTreeMap<_, _>>();
    let names = before_map
        .keys()
        .chain(after_map.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    names
        .into_iter()
        .filter_map(|name| {
            let before = before_map.get(&name).cloned();
            let after = after_map.get(&name).cloned();
            (before != after).then_some(super::super::model::CookieSiteDataChange {
                name,
                before,
                after,
            })
        })
        .collect()
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) fn clear_scope_matches_cookie(
    scope: CookieSiteDataClearScope,
    _is_persistent: bool,
) -> bool {
    matches!(scope, CookieSiteDataClearScope::All)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn clear_scope_matches_cookie(
    scope: CookieSiteDataClearScope,
    is_persistent: bool,
) -> bool {
    match scope {
        CookieSiteDataClearScope::All => true,
        CookieSiteDataClearScope::Persistent => is_persistent,
        CookieSiteDataClearScope::Session => !is_persistent,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn clear_scope_cookie_count(
    state: &CookieStorageStateSnapshot,
    scope: CookieSiteDataClearScope,
) -> usize {
    match scope {
        CookieSiteDataClearScope::All => state.live_cookie_count,
        CookieSiteDataClearScope::Persistent => state.persistent_cookie_count,
        CookieSiteDataClearScope::Session => state
            .live_cookie_count
            .saturating_sub(state.persistent_cookie_count),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn targeted_clear_resulting_state(
    replaced_state: &CookieStorageStateSnapshot,
    scope: CookieSiteDataClearScope,
) -> CookieStorageStateSnapshot {
    match scope {
        CookieSiteDataClearScope::All => {
            rebuild_storage_state_snapshot(None, Vec::new(), Vec::new())
        }
        CookieSiteDataClearScope::Persistent => rebuild_storage_state_snapshot(
            None,
            replaced_state
                .live_site_data
                .iter()
                .filter(|&site| site.session_cookie_count > 0)
                .map(|site| {
                    CookieSiteDataSummary::new(site.name.clone(), site.session_cookie_count, 0)
                })
                .collect(),
            Vec::new(),
        ),
        CookieSiteDataClearScope::Session => rebuild_storage_state_snapshot(
            None,
            replaced_state
                .live_site_data
                .iter()
                .filter(|&site| site.persistent_cookie_count > 0)
                .map(|site| {
                    CookieSiteDataSummary::new(
                        site.name.clone(),
                        site.persistent_cookie_count,
                        site.persistent_cookie_count,
                    )
                })
                .collect(),
            replaced_state.persistent_site_data.clone(),
        ),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn rebuild_storage_state_snapshot(
    store_generation: Option<u64>,
    live_site_data: Vec<CookieSiteDataSummary>,
    persistent_site_data: Vec<CookieSiteDataSummary>,
) -> CookieStorageStateSnapshot {
    CookieStorageStateSnapshot {
        store_generation,
        live_cookie_count: live_site_data.iter().map(|site| site.cookie_count).sum(),
        live_site_data,
        persistent_cookie_count: persistent_site_data
            .iter()
            .map(|site| site.cookie_count)
            .sum(),
        persistent_site_data,
    }
}

pub(super) fn normalized_site_keys(sites: &[&str]) -> BTreeSet<String> {
    sites
        .iter()
        .filter_map(|site| site_key_for_host(site))
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
fn storage_clear_preview_from_site_preview(
    preview: CookieSiteDataClearPreviewReport,
) -> CookieStorageClearPreviewReport {
    CookieStorageClearPreviewReport {
        target: CookieStorageClearTarget::RegistrableSites(preview.requested_sites.clone()),
        scope: preview.scope,
        would_remove_cookie_count: preview.would_remove_cookie_count,
        replaced_state: preview.replaced_state,
        resulting_state: preview.resulting_state,
        state_diff: preview.state_diff,
    }
}

fn storage_clear_report_from_site_report(
    report: CookieSiteDataClearReport,
) -> CookieStorageClearReport {
    CookieStorageClearReport {
        target: CookieStorageClearTarget::RegistrableSites(report.requested_sites.clone()),
        scope: report.scope,
        removed_cookie_count: report.removed_cookie_count,
        replaced_state: report.replaced_state,
        resulting_state: report.resulting_state,
        state_diff: report.state_diff,
    }
}

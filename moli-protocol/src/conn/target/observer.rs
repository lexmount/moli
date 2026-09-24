use std::collections::{HashMap, HashSet};

use crate::conn::{BackgroundProtocolEvent, CdpTargetFilter};
use crate::devtools_runtime::{
    AutomationEvent, DevToolsTargetFilterEntry, DevToolsTargetId, DevToolsTargetInfo,
    TargetLifecycleEvent,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetHandlerState {
    discover_filter: Option<CdpTargetFilter>,
    reported_hosts: HashSet<String>,
}

impl TargetHandlerState {
    fn set_discover_targets(&mut self, filter: CdpTargetFilter) {
        self.discover_filter = Some(filter);
    }

    fn clear_discover_targets(&mut self) {
        self.discover_filter = None;
        self.reported_hosts.clear();
    }

    fn discover_filter(&self) -> Option<&CdpTargetFilter> {
        self.discover_filter.as_ref()
    }

    fn take_unreported_matching_targets(
        &mut self,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<DevToolsTargetInfo> {
        let Some(filter) = self.discover_filter.as_ref() else {
            return Vec::new();
        };
        target_infos
            .into_iter()
            .filter(|target_info| filter.matches(target_info.kind.as_cdp_type()))
            .filter(|target_info| {
                let Some(target_id) = target_info.target_id.as_ref() else {
                    return false;
                };
                self.reported_hosts.insert(target_id.as_str().to_owned())
            })
            .collect()
    }

    fn target_info_changed_events(
        &self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
        auto_attached_target_ids: &HashSet<String>,
    ) -> Vec<BackgroundProtocolEvent> {
        target_infos
            .into_iter()
            .filter(|target_info| {
                target_info.target_id.as_ref().is_some_and(|target_id| {
                    self.reported_hosts.contains(target_id.as_str())
                        || auto_attached_target_ids.contains(target_id.as_str())
                })
            })
            .map(|target_info| {
                BackgroundProtocolEvent::target_info_changed(owner_session_id, target_info)
            })
            .collect()
    }

    fn target_destroyed_events(
        &mut self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<BackgroundProtocolEvent> {
        target_infos
            .into_iter()
            .filter(|target_info| {
                let Some(target_id) = target_info.target_id.as_ref() else {
                    return false;
                };
                self.reported_hosts.remove(target_id.as_str())
            })
            .filter_map(|target_info| target_destroyed_event(owner_session_id, target_info))
            .collect()
    }

    fn target_crashed_event(
        &self,
        owner_session_id: Option<&str>,
        target_id: &str,
        status: &str,
        error_code: i32,
    ) -> Option<BackgroundProtocolEvent> {
        self.reported_hosts.contains(target_id).then(|| {
            BackgroundProtocolEvent::target_crashed(owner_session_id, target_id, status, error_code)
        })
    }

    #[cfg(test)]
    fn has_reported_host(&self, target_id: &str) -> bool {
        self.reported_hosts.contains(target_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetHandlerStore {
    handlers: HashMap<Option<String>, TargetHandlerState>,
}

impl TargetHandlerStore {
    pub(crate) fn ensure_owner(&mut self, owner_session_id: Option<&str>) {
        let _ = self.handler_mut(owner_session_id);
    }

    pub(crate) fn set_discover_targets(
        &mut self,
        owner_session_id: Option<&str>,
        filter: CdpTargetFilter,
    ) {
        self.handler_mut(owner_session_id)
            .set_discover_targets(filter);
    }

    pub(crate) fn clear_discover_targets(&mut self, owner_session_id: Option<&str>) {
        self.handler_mut(owner_session_id).clear_discover_targets();
    }

    pub(crate) fn remove_owner(&mut self, owner_session_id: Option<&str>) {
        self.handlers.remove(&owner_session_id.map(str::to_owned));
    }

    pub(crate) fn discover_filter_entries(
        &self,
        owner_session_id: Option<&str>,
    ) -> Option<Vec<DevToolsTargetFilterEntry>> {
        self.handler(owner_session_id)?
            .discover_filter()
            .map(CdpTargetFilter::to_devtools_entries)
    }

    pub(crate) fn take_unreported_matching_targets(
        &mut self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<DevToolsTargetInfo> {
        self.handler_mut(owner_session_id)
            .take_unreported_matching_targets(target_infos)
    }

    pub(crate) fn target_created_events(
        &mut self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<BackgroundProtocolEvent> {
        self.take_unreported_matching_targets(owner_session_id, target_infos)
            .into_iter()
            .filter_map(|target_info| target_created_event(owner_session_id, target_info))
            .collect()
    }

    pub(crate) fn target_info_changed_events(
        &self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
        auto_attached_target_ids: &HashSet<String>,
    ) -> Vec<BackgroundProtocolEvent> {
        self.handler(owner_session_id)
            .map(|handler| {
                handler.target_info_changed_events(
                    owner_session_id,
                    target_infos,
                    auto_attached_target_ids,
                )
            })
            .unwrap_or_default()
    }

    pub(crate) fn target_destroyed_events(
        &mut self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<BackgroundProtocolEvent> {
        self.handler_mut(owner_session_id)
            .target_destroyed_events(owner_session_id, target_infos)
    }

    pub(crate) fn target_crashed_event(
        &self,
        owner_session_id: Option<&str>,
        target_id: &str,
        status: &str,
        error_code: i32,
    ) -> Option<BackgroundProtocolEvent> {
        self.handler(owner_session_id).and_then(|handler| {
            handler.target_crashed_event(owner_session_id, target_id, status, error_code)
        })
    }

    pub(crate) fn has_any_discovery(&self) -> bool {
        self.handlers
            .values()
            .any(|handler| handler.discover_filter().is_some())
    }

    pub(crate) fn discovery_owner_session_ids(&self) -> Vec<Option<String>> {
        let mut owner_session_ids = self
            .handlers
            .iter()
            .filter(|(_, handler)| handler.discover_filter().is_some())
            .map(|(owner_session_id, _)| owner_session_id.clone())
            .collect::<Vec<_>>();
        owner_session_ids.sort();
        owner_session_ids
    }

    pub(crate) fn target_info_owner_session_ids(
        &self,
        auto_attached_owner_session_ids: Vec<Option<String>>,
    ) -> Vec<Option<String>> {
        let mut owner_session_ids = self.discovery_owner_session_ids();
        owner_session_ids.extend(
            auto_attached_owner_session_ids
                .into_iter()
                .filter(|owner_session_id| self.handlers.contains_key(owner_session_id)),
        );
        owner_session_ids.sort();
        owner_session_ids.dedup();
        owner_session_ids
    }

    fn handler(&self, owner_session_id: Option<&str>) -> Option<&TargetHandlerState> {
        self.handlers.get(&owner_session_id.map(str::to_owned))
    }

    fn handler_mut(&mut self, owner_session_id: Option<&str>) -> &mut TargetHandlerState {
        self.handlers
            .entry(owner_session_id.map(str::to_owned))
            .or_default()
    }

    #[cfg(test)]
    fn has_reported_host(&self, owner_session_id: Option<&str>, target_id: &str) -> bool {
        self.handler(owner_session_id)
            .is_some_and(|handler| handler.has_reported_host(target_id))
    }
}

fn target_created_event(
    owner_session_id: Option<&str>,
    target_info: DevToolsTargetInfo,
) -> Option<BackgroundProtocolEvent> {
    let lifecycle = target_lifecycle_event(target_info)?;
    Some(BackgroundProtocolEvent::target_created(
        owner_session_id,
        lifecycle,
    ))
}

fn target_destroyed_event(
    owner_session_id: Option<&str>,
    target_info: DevToolsTargetInfo,
) -> Option<BackgroundProtocolEvent> {
    let lifecycle = target_lifecycle_event(target_info)?;
    Some(BackgroundProtocolEvent::target_destroyed(
        owner_session_id,
        lifecycle,
    ))
}

pub(crate) fn with_primary_target_lifecycle_event(
    mut events: Vec<BackgroundProtocolEvent>,
    target_info: DevToolsTargetInfo,
    project: fn(TargetLifecycleEvent) -> AutomationEvent,
) -> Vec<BackgroundProtocolEvent> {
    // Root discovery already carries the primary automation receipt. Other
    // discovery scopes and filters cannot suppress the actual lifecycle fact.
    if events
        .iter()
        .all(|event| event.protocol_session_id().is_some())
        && let Some(lifecycle) = target_lifecycle_event(target_info)
    {
        events.push(BackgroundProtocolEvent::automation_only(project(lifecycle)));
    }
    events
}

fn target_lifecycle_event(target_info: DevToolsTargetInfo) -> Option<TargetLifecycleEvent> {
    let target_id = target_info.target_id.clone()?;
    Some(TargetLifecycleEvent {
        target_id: DevToolsTargetId::from(target_id.as_str()),
        browser_context_id: target_info.browser_context_id.clone(),
        kind: target_info.kind,
        url: target_info.url.clone(),
        target_info: Some(target_info),
    })
}

#[cfg(test)]
mod tests {
    use crate::conn::{CdpTargetFilter, CdpTargetFilterEntry};
    use crate::devtools_runtime::{
        AutomationEvent, DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind,
    };

    use super::TargetHandlerStore;

    fn target_info(target_id: &str, kind: DevToolsTargetKind) -> DevToolsTargetInfo {
        DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(target_id)),
            kind,
            title: String::new(),
            url: "about:blank".to_owned(),
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: None,
            moli_popup_id: None,
        }
    }

    #[test]
    fn target_handler_discover_reports_matching_unreported_hosts() {
        let mut store = TargetHandlerStore::default();
        store.set_discover_targets(
            None,
            CdpTargetFilter::from_entries(vec![CdpTargetFilterEntry {
                exclude: false,
                target_type: Some("page".to_owned()),
            }]),
        );

        let first = store.take_unreported_matching_targets(
            None,
            vec![
                target_info("TAB-TID-page", DevToolsTargetKind::Tab),
                target_info("TID-page", DevToolsTargetKind::Page),
            ],
        );
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0]
                .target_id
                .as_ref()
                .map(|target_id| target_id.as_str()),
            Some("TID-page")
        );
        assert!(store.has_reported_host(None, "TID-page"));

        let second = store.take_unreported_matching_targets(
            None,
            vec![target_info("TID-page", DevToolsTargetKind::Page)],
        );
        assert!(second.is_empty());
    }

    #[test]
    fn target_handler_clear_discover_forgets_reported_hosts() {
        let mut store = TargetHandlerStore::default();
        store.set_discover_targets(None, CdpTargetFilter::default_target_discovery());
        let first = store.take_unreported_matching_targets(
            None,
            vec![target_info("TID-page", DevToolsTargetKind::Page)],
        );
        assert_eq!(first.len(), 1);

        store.clear_discover_targets(None);
        store.set_discover_targets(None, CdpTargetFilter::default_target_discovery());
        let second = store.take_unreported_matching_targets(
            None,
            vec![target_info("TID-page", DevToolsTargetKind::Page)],
        );
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn target_handler_info_changed_allows_auto_attached_unreported_host() {
        let mut store = TargetHandlerStore::default();
        store.ensure_owner(None);
        let auto_attached_target_ids = ["TID-page".to_owned()].into_iter().collect();

        let events = store.target_info_changed_events(
            None,
            vec![target_info("TID-page", DevToolsTargetKind::Page)],
            &auto_attached_target_ids,
        );

        assert_eq!(events.len(), 1);
    }

    #[test]
    fn target_handler_info_owner_ids_include_auto_attach_only_handlers() {
        let mut store = TargetHandlerStore::default();
        store.ensure_owner(Some("SID-auto"));
        store.set_discover_targets(
            Some("SID-discover"),
            CdpTargetFilter::from_entries(vec![CdpTargetFilterEntry {
                exclude: false,
                target_type: Some("page".to_owned()),
            }]),
        );

        assert_eq!(
            store.target_info_owner_session_ids(vec![
                Some("SID-auto".to_owned()),
                Some("SID-missing".to_owned()),
            ]),
            vec![Some("SID-auto".to_owned()), Some("SID-discover".to_owned())]
        );
    }

    #[test]
    fn target_destroyed_automation_events_do_not_require_reported_hosts() {
        let events = [
            target_info("TAB-TID-page", DevToolsTargetKind::Tab),
            target_info("TID-page", DevToolsTargetKind::Page),
        ]
        .into_iter()
        .flat_map(|info| {
            super::with_primary_target_lifecycle_event(
                Vec::new(),
                info,
                AutomationEvent::TargetDestroyed,
            )
        });

        let target_ids = events
            .filter_map(|event| {
                let (_message, automation_event) = event.into_parts();
                let Some(AutomationEvent::TargetDestroyed(event)) = automation_event else {
                    return None;
                };
                Some(event.target_id.into_string())
            })
            .collect::<Vec<_>>();

        assert_eq!(
            target_ids,
            vec!["TAB-TID-page".to_owned(), "TID-page".to_owned()]
        );
    }
}

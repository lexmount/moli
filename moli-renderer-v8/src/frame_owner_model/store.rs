use std::collections::{BTreeMap, HashMap, VecDeque};

use super::ids::FrameOwnerIdAllocator;
use super::lifecycle_tasks::{
    ChildDocumentAsyncClassicScriptLoadDelay, DocumentLinkEventOwner,
    FrameDocumentCompleteLifecycleAction, FrameDocumentDomContentLoadedLifecycleAction,
    FrameDocumentImageLoadEventBinding, FrameDocumentInteractiveLifecycleAction,
    FrameDocumentLifecycleAction, FrameDocumentMediaLoadDelayBinding,
    MainDocumentCompleteLifecycleAction, MainDocumentDomContentLoadedLifecycleAction,
    MainDocumentImageLoadDelayBinding, MainDocumentInteractiveLifecycleAction,
    MainDocumentMediaLoadDelayBinding, MainDocumentScriptLoadDelayKind,
    MainDocumentScriptLoadDelayLease, MainDocumentScriptLoadDelayRelease,
    MainDocumentStyleLoadEventBinding, StylesheetSubresourceLoadDelayBinding,
};
use super::load_event_gate::DocumentLoadGateRelease;
use super::module_clients::{
    FrameDocumentModuleClientReservation, FrameDocumentModuleFetchClientStart,
    FrameDocumentModulepreloadLinkClient,
};
use super::navigation_tasks::{
    FrameLaneNavigationCommitTask, FrameNavigationCommitReservationResult,
};
use super::records::*;
use super::{
    FrameDocumentLoadDeliveryAction, FrameDocumentLoadDeliveryAdmission,
    FrameDocumentLoadDeliveryProgress, FrameDocumentLoadDeliveryTask,
};
use crate::document_runtime::DomHandle;
use moli_fetch::RequestCredentialsMode;
use url::Url;

#[derive(Default, Debug)]
pub(crate) struct FrameOwnerStore {
    ids: FrameOwnerIdAllocator,
    pub(crate) frames: BTreeMap<FrameId, FrameRecord>,
    pub(crate) frame_owner_elements: HashMap<DomHandle, FrameOwnerElementRecord>,
    pub(crate) window_proxies: BTreeMap<WindowProxyId, WindowProxyRecord>,
    pub(crate) scheduler_lanes: BTreeMap<FrameSchedulerLaneId, FrameSchedulerLaneRecord>,
    pub(crate) local_windows: BTreeMap<LocalWindowId, LocalWindowRecord>,
    pub(crate) documents: BTreeMap<DocumentId, DocumentRecord>,
    pub(crate) realms: BTreeMap<FrameRealmId, FrameRealmRecord>,
    frame_ids_by_child_handle: HashMap<DomHandle, FrameId>,
    pending_main_document_owner_transitions: VecDeque<MainDocumentOwnerTransition>,
    pending_child_document_owner_retirements: VecDeque<FrameDocumentOwnerTransition>,
    pending_parent_document_descendant_completions: VecDeque<FrameDocumentDescendantLoadCompletion>,
}

impl FrameOwnerStore {
    fn new_loading_document_lifecycle(
        &mut self,
        load_delivery_kind: DocumentLoadDeliveryKind,
    ) -> DocumentLifecycleRecord {
        let parsing_delay_token = self.ids.document_load_delay_token();
        let domcontentloaded_transition_token = self.ids.document_load_delay_token();
        DocumentLifecycleRecord::loading(
            load_delivery_kind,
            parsing_delay_token,
            domcontentloaded_transition_token,
        )
    }

    fn new_loading_document_lifecycle_for_document_open(
        &mut self,
        load_delivery_kind: DocumentLoadDeliveryKind,
        continuation: Option<super::records::DocumentOpenLoadContinuation>,
    ) -> DocumentLifecycleRecord {
        let parsing_delay_token = self.ids.document_load_delay_token();
        let domcontentloaded_transition_token = self.ids.document_load_delay_token();
        DocumentLifecycleRecord::loading_for_document_open(
            load_delivery_kind,
            parsing_delay_token,
            domcontentloaded_transition_token,
            continuation,
        )
    }

    pub(crate) fn ensure_main_frame(
        &mut self,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
        service_worker_client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    ) -> FrameRealmId {
        let frame_id = main_frame_id();
        let window_proxy_id = main_window_proxy_id();
        let scheduler_lane_id = main_scheduler_lane_id();
        let local_window_id = main_local_window_id();
        let document_id = main_document_id();
        let realm_id = main_frame_realm_id();
        let settings = FrameSettingsObject {
            base_url: base_url.clone(),
            origin,
            referrer_policy: None,
            #[cfg(test)]
            credentials_mode: RequestCredentialsMode::SameOrigin,
            document_policy_container,
            subresource_policy_context,
            service_worker_client_id,
            module_map_owner: ModuleMapOwner::Document(document_id),
        };
        self.frames.insert(
            frame_id.clone(),
            FrameRecord {
                frame_id: frame_id.clone(),
                kind: FrameKind::Main,
                parent_frame_id: None,
                owner_element_handle: None,
                window_proxy_id,
                scheduler_lane_id,
                current_local_window_id: Some(local_window_id),
                current_document_id: Some(document_id),
                parent_document_load: None,
                navigation_load: None,
                lifecycle: FrameLifecycleState::Attached,
            },
        );
        self.window_proxies.insert(
            window_proxy_id,
            WindowProxyRecord {
                id: window_proxy_id,
                frame_id: frame_id.clone(),
                current_local_window_id: Some(local_window_id),
                reachability: WindowProxyReachability::LiveFrame,
            },
        );
        self.scheduler_lanes.insert(
            scheduler_lane_id,
            FrameSchedulerLaneRecord {
                id: scheduler_lane_id,
                frame_id: frame_id.clone(),
                lifecycle: FrameSchedulerLaneLifecycleState::Active,
            },
        );
        self.local_windows.insert(
            local_window_id,
            LocalWindowRecord {
                id: local_window_id,
                frame_id: frame_id.clone(),
                document_id,
                realm_id: Some(realm_id),
                settings,
                lifecycle: LocalWindowLifecycleState::Current,
            },
        );
        let lifecycle_progress =
            self.new_loading_document_lifecycle(DocumentLoadDeliveryKind::Main);
        self.documents.insert(
            document_id,
            DocumentRecord {
                id: document_id,
                local_window_id,
                document_handle,
                url,
                base_url,
                creation_kind: DocumentCreationKind::Navigation,
                lifecycle: DocumentLifecycleState::Current,
                lifecycle_progress,
                active_requests: BTreeMap::new(),
                import_map_registry: Default::default(),
            },
        );
        self.realms.insert(
            realm_id,
            FrameRealmRecord {
                id: realm_id,
                local_window_id,
                document_id,
                inspector_execution_context_id: None,
                lifecycle: FrameRealmLifecycleState::Materialized,
            },
        );
        realm_id
    }

    pub(crate) fn ensure_child_frame(
        &mut self,
        child_handle: DomHandle,
        frame_id: String,
        parent_frame_id: Option<String>,
    ) -> FrameId {
        let frame_id = FrameId(frame_id);
        let parent_frame_id = parent_frame_id.map(FrameId);
        if let Some(previous_frame_id) = self.frame_ids_by_child_handle.get(&child_handle).cloned()
            && previous_frame_id != frame_id
        {
            if let Some(completion) =
                self.release_parent_document_descendant_load_for_frame(&previous_frame_id)
            {
                self.pending_parent_document_descendant_completions
                    .push_back(completion);
            }
            if let Some(previous_frame) = self.frames.get_mut(&previous_frame_id)
                && previous_frame.owner_element_handle == Some(child_handle)
            {
                previous_frame.owner_element_handle = None;
            }
            if let Some(previous_owner) = self.frame_owner_elements.get_mut(&child_handle) {
                previous_owner.content_frame_id = None;
                previous_owner.lifecycle = FrameOwnerElementLifecycleState::Detached;
            }
            self.detach_current_frame_records(&previous_frame_id);
        }
        self.frame_ids_by_child_handle
            .insert(child_handle, frame_id.clone());
        self.frame_owner_elements.insert(
            child_handle,
            FrameOwnerElementRecord {
                owner_handle: child_handle,
                content_frame_id: Some(frame_id.clone()),
                parent_frame_id: parent_frame_id.clone(),
                lifecycle: FrameOwnerElementLifecycleState::Attached,
            },
        );
        if self.frames.contains_key(&frame_id) {
            let previous_owner = self
                .frames
                .get(&frame_id)
                .and_then(|frame| frame.owner_element_handle);
            if let Some(previous_owner) = previous_owner
                && previous_owner != child_handle
            {
                if let Some(completion) =
                    self.release_parent_document_descendant_load_for_frame(&frame_id)
                {
                    self.pending_parent_document_descendant_completions
                        .push_back(completion);
                }
                if self.frame_ids_by_child_handle.get(&previous_owner) == Some(&frame_id) {
                    self.frame_ids_by_child_handle.remove(&previous_owner);
                }
                if let Some(owner_element) = self.frame_owner_elements.get_mut(&previous_owner) {
                    owner_element.content_frame_id = None;
                    owner_element.lifecycle = FrameOwnerElementLifecycleState::Detached;
                }
            }
            if let Some(frame) = self.frames.get_mut(&frame_id) {
                frame.parent_frame_id = parent_frame_id;
                frame.owner_element_handle = Some(child_handle);
                frame.lifecycle = FrameLifecycleState::Attached;
                if let Some(lane) = self.scheduler_lanes.get_mut(&frame.scheduler_lane_id) {
                    lane.lifecycle = FrameSchedulerLaneLifecycleState::Active;
                }
            }
            if let Some(proxy_id) = self
                .frames
                .get(&frame_id)
                .map(|frame| frame.window_proxy_id)
                && let Some(proxy) = self.window_proxies.get_mut(&proxy_id)
            {
                proxy.reachability = WindowProxyReachability::LiveFrame;
            }
            return frame_id;
        }
        let window_proxy_id = self.ids.window_proxy();
        let scheduler_lane_id = self.ids.scheduler_lane();
        self.frames.insert(
            frame_id.clone(),
            FrameRecord {
                frame_id: frame_id.clone(),
                kind: FrameKind::ChildIframe,
                parent_frame_id,
                owner_element_handle: Some(child_handle),
                window_proxy_id,
                scheduler_lane_id,
                current_local_window_id: None,
                current_document_id: None,
                parent_document_load: None,
                navigation_load: None,
                lifecycle: FrameLifecycleState::Attached,
            },
        );
        self.window_proxies.insert(
            window_proxy_id,
            WindowProxyRecord {
                id: window_proxy_id,
                frame_id: frame_id.clone(),
                current_local_window_id: None,
                reachability: WindowProxyReachability::LiveFrame,
            },
        );
        self.scheduler_lanes.insert(
            scheduler_lane_id,
            FrameSchedulerLaneRecord {
                id: scheduler_lane_id,
                frame_id: frame_id.clone(),
                lifecycle: FrameSchedulerLaneLifecycleState::Active,
            },
        );
        frame_id
    }

    pub(crate) fn replace_main_document(
        &mut self,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
    ) -> Option<MainDocumentOwnerTransition> {
        let snapshot = self.current_main_owner_snapshot()?;
        let retired_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );

        let retired_document = self
            .documents
            .get_mut(&snapshot.document_id)
            .expect("validated main owner snapshot must retain its document record");
        let load_continuation = retired_document
            .lifecycle_progress
            .document_open_load_continuation();
        retired_document.lifecycle = DocumentLifecycleState::Replaced;
        retired_document.lifecycle_progress.retire();
        retired_document.active_requests.clear();

        let document_id = self.ids.document();
        let lifecycle_progress = self.new_loading_document_lifecycle_for_document_open(
            DocumentLoadDeliveryKind::Main,
            load_continuation,
        );
        self.documents.insert(
            document_id,
            DocumentRecord {
                id: document_id,
                local_window_id: snapshot.local_window_id,
                document_handle,
                url,
                base_url: base_url.clone(),
                creation_kind: DocumentCreationKind::DocumentOpen,
                lifecycle: DocumentLifecycleState::Current,
                lifecycle_progress,
                active_requests: BTreeMap::new(),
                import_map_registry: Default::default(),
            },
        );

        let local_window = self
            .local_windows
            .get_mut(&snapshot.local_window_id)
            .expect("validated main owner snapshot must retain its LocalWindow record");
        local_window.document_id = document_id;
        local_window.settings.base_url = base_url;
        local_window.settings.module_map_owner = ModuleMapOwner::Document(document_id);

        self.frames
            .get_mut(&snapshot.frame_id)
            .expect("validated main owner snapshot must retain its frame record")
            .current_document_id = Some(document_id);
        if let Some(realm_id) = snapshot.realm_id {
            self.realms
                .get_mut(&realm_id)
                .expect("validated main owner snapshot must retain its realm record")
                .document_id = document_id;
        }

        let transition = MainDocumentOwnerTransition::new(
            retired_owner,
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                document_id,
            ),
        );
        tracing::debug!(
            ?retired_owner,
            current_owner = ?transition.current_owner(),
            "committed and journaled main document owner replacement"
        );
        self.pending_main_document_owner_transitions
            .push_back(transition);
        Some(transition)
    }

    pub(crate) fn take_pending_main_document_owner_transitions(
        &mut self,
    ) -> Vec<MainDocumentOwnerTransition> {
        self.pending_main_document_owner_transitions
            .drain(..)
            .collect()
    }

    fn commit_child_document_with_creation_kind(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        referrer_policy: Option<String>,
        _credentials_mode: RequestCredentialsMode,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
        creation_kind: DocumentCreationKind,
    ) -> Option<(LocalWindowId, DocumentId)> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        let frame = self.frames.get(&frame_id)?;
        if frame.kind != FrameKind::ChildIframe
            || frame.owner_element_handle != Some(child_handle)
            || frame.lifecycle != FrameLifecycleState::Attached
        {
            return None;
        }
        let scheduler_lane = self.scheduler_lanes.get(&frame.scheduler_lane_id)?;
        if scheduler_lane.id != frame.scheduler_lane_id
            || scheduler_lane.frame_id != frame_id
            || scheduler_lane.lifecycle != FrameSchedulerLaneLifecycleState::Active
        {
            return None;
        }
        let window_proxy = self.window_proxies.get(&frame.window_proxy_id)?;
        if window_proxy.frame_id != frame_id
            || window_proxy.reachability != WindowProxyReachability::LiveFrame
        {
            return None;
        }
        let old_local_window_id = frame.current_local_window_id;
        let old_document_id = frame.current_document_id;
        match (old_local_window_id, old_document_id) {
            (Some(local_window_id), Some(document_id)) => {
                let snapshot = self.current_child_owner_snapshot(child_handle)?;
                if snapshot.frame_id != frame_id
                    || snapshot.local_window_id != local_window_id
                    || snapshot.document_id != document_id
                {
                    return None;
                }
            }
            (None, None) if window_proxy.current_local_window_id.is_none() => {}
            _ => return None,
        }
        let window_proxy_id = frame.window_proxy_id;

        if let Some(local_window_id) = old_local_window_id {
            let realm_id = self
                .local_windows
                .get(&local_window_id)
                .expect("validated child frame must retain its current LocalWindow")
                .realm_id;
            let local_window = self
                .local_windows
                .get_mut(&local_window_id)
                .expect("validated child frame must retain its current LocalWindow");
            local_window.lifecycle = LocalWindowLifecycleState::NavigatedAway;
            if let Some(realm_id) = realm_id {
                self.realms
                    .get_mut(&realm_id)
                    .expect("validated child LocalWindow must retain its materialized realm")
                    .lifecycle = FrameRealmLifecycleState::DetachedReachable;
            }
        }
        if let Some(document_id) = old_document_id {
            let document = self
                .documents
                .get_mut(&document_id)
                .expect("validated child frame must retain its current Document");
            document.lifecycle = DocumentLifecycleState::Replaced;
            document.lifecycle_progress.retire();
            document.active_requests.clear();
        }
        let local_window_id = self.ids.local_window();
        let document_id = self.ids.document();
        let parsing_delay_token = self.ids.document_load_delay_token();
        let domcontentloaded_transition_token = self.ids.document_load_delay_token();
        let settings = FrameSettingsObject {
            base_url: base_url.clone(),
            origin,
            referrer_policy,
            #[cfg(test)]
            credentials_mode: _credentials_mode,
            document_policy_container,
            subresource_policy_context,
            service_worker_client_id: None,
            module_map_owner: ModuleMapOwner::Document(document_id),
        };
        self.local_windows.insert(
            local_window_id,
            LocalWindowRecord {
                id: local_window_id,
                frame_id: frame_id.clone(),
                document_id,
                realm_id: None,
                settings,
                lifecycle: LocalWindowLifecycleState::Current,
            },
        );
        self.documents.insert(
            document_id,
            DocumentRecord {
                id: document_id,
                local_window_id,
                document_handle,
                url,
                base_url,
                creation_kind,
                lifecycle: DocumentLifecycleState::Current,
                lifecycle_progress: DocumentLifecycleRecord::loading(
                    DocumentLoadDeliveryKind::Child,
                    parsing_delay_token,
                    domcontentloaded_transition_token,
                ),
                active_requests: BTreeMap::new(),
                import_map_registry: Default::default(),
            },
        );
        let frame = self
            .frames
            .get_mut(&frame_id)
            .expect("validated child frame record must survive its document commit");
        frame.current_local_window_id = Some(local_window_id);
        frame.current_document_id = Some(document_id);
        frame.navigation_load = None;
        frame.lifecycle = FrameLifecycleState::Attached;
        let proxy = self
            .window_proxies
            .get_mut(&window_proxy_id)
            .expect("validated child WindowProxy must survive its document commit");
        proxy.current_local_window_id = Some(local_window_id);
        proxy.reachability = WindowProxyReachability::LiveFrame;
        Some((local_window_id, document_id))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_child_frame_document(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        referrer_policy: Option<String>,
        credentials_mode: RequestCredentialsMode,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
    ) -> Option<FrameDocumentOwnerTransition> {
        if self
            .current_child_document_task_owner(child_handle)
            .is_some()
            || !moli_url::is_about_blank(&url)
        {
            return None;
        }
        let scheduler_lane_id = self
            .current_child_frame_lane_task_owner(child_handle)?
            .scheduler_lane_id;
        let (local_window_id, document_id) = self.commit_child_document_with_creation_kind(
            child_handle,
            document_handle,
            url,
            base_url,
            origin,
            referrer_policy,
            credentials_mode,
            document_policy_container,
            subresource_policy_context,
            DocumentCreationKind::InitialEmpty,
        )?;
        Some(FrameDocumentOwnerTransition::new(
            child_handle,
            None,
            Some(FrameDocumentTaskOwner::new(
                scheduler_lane_id,
                local_window_id,
                document_id,
            )),
        ))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_child_document(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        referrer_policy: Option<String>,
        credentials_mode: RequestCredentialsMode,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
    ) -> Option<(LocalWindowId, DocumentId)> {
        self.commit_child_document_with_creation_kind(
            child_handle,
            document_handle,
            url,
            base_url,
            origin,
            referrer_policy,
            credentials_mode,
            document_policy_container,
            subresource_policy_context,
            DocumentCreationKind::Navigation,
        )
    }

    pub(crate) fn replace_child_document_with_local_window_transition(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        referrer_policy: Option<String>,
        credentials_mode: RequestCredentialsMode,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
        creation_kind: DocumentCreationKind,
        local_window_transition: FrameDocumentLocalWindowTransition,
        expected_current_owner: Option<FrameDocumentTaskOwner>,
    ) -> Option<FrameDocumentOwnerTransition> {
        let retired_owner = self.current_child_document_task_owner(child_handle);
        if retired_owner != expected_current_owner {
            return None;
        }
        let scheduler_lane_id = self
            .current_child_frame_lane_task_owner(child_handle)?
            .scheduler_lane_id;
        if local_window_transition
            == FrameDocumentLocalWindowTransition::ReuseInitialEmptyLocalWindow
        {
            return self.replace_initial_empty_child_document_in_current_local_window(
                child_handle,
                document_handle,
                url,
                base_url,
                origin,
                referrer_policy,
                credentials_mode,
                document_policy_container,
                subresource_policy_context,
                creation_kind,
                retired_owner?,
            );
        }
        let (local_window_id, document_id) = self.commit_child_document_with_creation_kind(
            child_handle,
            document_handle,
            url,
            base_url,
            origin,
            referrer_policy,
            credentials_mode,
            document_policy_container,
            subresource_policy_context,
            creation_kind,
        )?;
        Some(FrameDocumentOwnerTransition::new(
            child_handle,
            retired_owner,
            Some(FrameDocumentTaskOwner::new(
                scheduler_lane_id,
                local_window_id,
                document_id,
            )),
        ))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn replace_child_document(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        referrer_policy: Option<String>,
        credentials_mode: RequestCredentialsMode,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
    ) -> Option<FrameDocumentOwnerTransition> {
        let expected_current_owner = self.current_child_document_task_owner(child_handle);
        self.replace_child_document_with_local_window_transition(
            child_handle,
            document_handle,
            url,
            base_url,
            origin,
            referrer_policy,
            credentials_mode,
            document_policy_container,
            subresource_policy_context,
            DocumentCreationKind::Navigation,
            FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
            expected_current_owner,
        )
    }

    pub(crate) fn child_document_local_window_transition_for_commit(
        &self,
        child_handle: DomHandle,
        expected_current_owner: Option<FrameDocumentTaskOwner>,
        security_origin_allows_reuse: bool,
        new_document_policy: &crate::document_runtime::DocumentPolicyContainer,
    ) -> FrameDocumentLocalWindowTransition {
        if !security_origin_allows_reuse
            || self.current_child_document_task_owner(child_handle) != expected_current_owner
        {
            return FrameDocumentLocalWindowTransition::ReplaceLocalWindow;
        }
        let Some(snapshot) = self.current_child_owner_snapshot(child_handle) else {
            return FrameDocumentLocalWindowTransition::ReplaceLocalWindow;
        };
        let Some(document) = self.documents.get(&snapshot.document_id) else {
            return FrameDocumentLocalWindowTransition::ReplaceLocalWindow;
        };
        let old_policy = &snapshot.settings.document_policy_container;
        if document.creation_kind.is_initial_empty()
            && old_policy.credentialless == new_document_policy.credentialless
            && old_policy.sandbox.forces_opaque_origin
                == new_document_policy.sandbox.forces_opaque_origin
        {
            FrameDocumentLocalWindowTransition::ReuseInitialEmptyLocalWindow
        } else {
            FrameDocumentLocalWindowTransition::ReplaceLocalWindow
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn replace_initial_empty_child_document_in_current_local_window(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
        origin: String,
        referrer_policy: Option<String>,
        _credentials_mode: RequestCredentialsMode,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        subresource_policy_context: crate::types::SubresourcePolicyContext,
        creation_kind: DocumentCreationKind,
        expected_current_owner: FrameDocumentTaskOwner,
    ) -> Option<FrameDocumentOwnerTransition> {
        let snapshot = self.current_child_owner_snapshot(child_handle)?;
        let retired_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        if retired_owner != expected_current_owner
            || !self
                .documents
                .get(&snapshot.document_id)
                .is_some_and(|document| document.creation_kind.is_initial_empty())
        {
            return None;
        }

        let document_id = self.ids.document();
        let lifecycle_progress =
            self.new_loading_document_lifecycle(DocumentLoadDeliveryKind::Child);
        let new_document = DocumentRecord {
            id: document_id,
            local_window_id: snapshot.local_window_id,
            document_handle,
            url,
            base_url: base_url.clone(),
            creation_kind,
            lifecycle: DocumentLifecycleState::Current,
            lifecycle_progress,
            active_requests: BTreeMap::new(),
            import_map_registry: Default::default(),
        };

        let retired_document = self
            .documents
            .get_mut(&snapshot.document_id)
            .expect("validated initial-empty owner must retain its Document");
        retired_document.lifecycle = DocumentLifecycleState::Replaced;
        retired_document.lifecycle_progress.retire();
        retired_document.active_requests.clear();
        self.documents.insert(document_id, new_document);

        let local_window = self
            .local_windows
            .get_mut(&snapshot.local_window_id)
            .expect("validated initial-empty owner must retain its LocalWindow");
        local_window.document_id = document_id;
        local_window.settings = FrameSettingsObject {
            base_url,
            origin,
            referrer_policy,
            #[cfg(test)]
            credentials_mode: _credentials_mode,
            document_policy_container,
            subresource_policy_context,
            service_worker_client_id: None,
            module_map_owner: ModuleMapOwner::Document(document_id),
        };
        let frame = self
            .frames
            .get_mut(&snapshot.frame_id)
            .expect("validated initial-empty owner must retain its frame");
        frame.current_document_id = Some(document_id);
        frame.navigation_load = None;
        if let Some(realm_id) = snapshot.realm_id {
            self.retarget_preserved_realm_to_document(realm_id, document_id);
        }

        let current_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            document_id,
        );
        tracing::debug!(
            ?child_handle,
            ?retired_owner,
            ?current_owner,
            "securely transitioned initial-empty child LocalWindow to committed document"
        );
        Some(FrameDocumentOwnerTransition::new(
            child_handle,
            Some(retired_owner),
            Some(current_owner),
        ))
    }

    /// Validates a child `document.open()` replacement without mutating owner
    /// state. The caller may prepare V8/parser resources before committing.
    pub(crate) fn plan_child_document_open_replacement(
        &self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        url: Url,
        base_url: Url,
    ) -> Option<ChildDocumentOpenReplacementPlan> {
        let snapshot = self.current_child_owner_snapshot(child_handle)?;
        if snapshot.document_handle != document_handle {
            return None;
        }
        Some(ChildDocumentOpenReplacementPlan {
            snapshot,
            document_handle,
            url,
            base_url,
        })
    }

    /// Commits a preflighted child Document replacement without a recoverable
    /// failure path. No page callback runs in this owner-store transaction, so
    /// a stale plan is an internal invariant violation rather than
    /// `DocumentNotFound`.
    pub(crate) fn commit_child_document_open_replacement(
        &mut self,
        plan: ChildDocumentOpenReplacementPlan,
    ) -> FrameDocumentOwnerTransition {
        let ChildDocumentOpenReplacementPlan {
            snapshot,
            document_handle,
            url,
            base_url,
        } = plan;
        let child_handle = snapshot.owner_handle;
        let current = self
            .current_child_owner_snapshot(child_handle)
            .expect("preflighted child document-open replacement must retain its current owner");
        assert_eq!(
            (
                current.scheduler_lane_id,
                current.local_window_id,
                current.document_id,
                current.document_handle,
            ),
            (
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
                snapshot.document_handle,
            ),
            "child document-open owner changed between preflight and commit"
        );
        let retired_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );

        let retired_document = self
            .documents
            .get_mut(&snapshot.document_id)
            .expect("validated child owner snapshot must retain its document record");
        let load_continuation = retired_document
            .lifecycle_progress
            .document_open_load_continuation();
        retired_document.lifecycle = DocumentLifecycleState::Replaced;
        retired_document.lifecycle_progress.retire();
        retired_document.active_requests.clear();

        let document_id = self.ids.document();
        let lifecycle_progress = self.new_loading_document_lifecycle_for_document_open(
            DocumentLoadDeliveryKind::Child,
            load_continuation,
        );
        self.documents.insert(
            document_id,
            DocumentRecord {
                id: document_id,
                local_window_id: snapshot.local_window_id,
                document_handle,
                url,
                base_url: base_url.clone(),
                creation_kind: DocumentCreationKind::DocumentOpen,
                lifecycle: DocumentLifecycleState::Current,
                lifecycle_progress,
                active_requests: BTreeMap::new(),
                import_map_registry: Default::default(),
            },
        );

        let local_window = self
            .local_windows
            .get_mut(&snapshot.local_window_id)
            .expect("validated child owner snapshot must retain its LocalWindow record");
        local_window.document_id = document_id;
        local_window.settings.base_url = base_url;
        local_window.settings.module_map_owner = ModuleMapOwner::Document(document_id);

        self.frames
            .get_mut(&snapshot.frame_id)
            .expect("validated child owner snapshot must retain its frame record")
            .current_document_id = Some(document_id);
        if let Some(realm_id) = snapshot.realm_id {
            self.retarget_preserved_realm_to_document(realm_id, document_id);
        }

        let transition = FrameDocumentOwnerTransition::new(
            child_handle,
            Some(retired_owner),
            Some(FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                document_id,
            )),
        );
        tracing::debug!(
            ?child_handle,
            ?retired_owner,
            current_owner = ?transition.current_owner(),
            "committed child document.open owner replacement in current LocalWindow"
        );
        self.pending_child_document_owner_retirements
            .push_back(transition);
        transition
    }

    pub(crate) fn detach_current_child_document(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentOwnerTransition> {
        let retired_owner = self.current_child_document_task_owner(child_handle);
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle).cloned()?;
        let frame = self.frames.get_mut(&frame_id)?;
        frame.navigation_load = None;
        if let Some(local_window_id) = frame.current_local_window_id.take()
            && let Some(local_window) = self.local_windows.get_mut(&local_window_id)
        {
            local_window.lifecycle = LocalWindowLifecycleState::DetachedReachable;
            if let Some(realm_id) = local_window.realm_id
                && let Some(realm) = self.realms.get_mut(&realm_id)
            {
                realm.lifecycle = FrameRealmLifecycleState::DetachedReachable;
            }
        }
        if let Some(document_id) = frame.current_document_id.take()
            && let Some(document) = self.documents.get_mut(&document_id)
        {
            document.lifecycle = DocumentLifecycleState::Detached;
            document.lifecycle_progress.retire();
            document.active_requests.clear();
        }
        if let Some(proxy) = self.window_proxies.get_mut(&frame.window_proxy_id) {
            proxy.current_local_window_id = None;
        }
        let transition = retired_owner.map(|retired_owner| {
            FrameDocumentOwnerTransition::new(child_handle, Some(retired_owner), None)
        });
        if let Some(transition) = transition {
            self.pending_child_document_owner_retirements
                .push_back(transition);
        }
        transition
    }

    pub(crate) fn take_pending_document_owner_retirements(
        &mut self,
    ) -> Vec<FrameDocumentOwnerTransition> {
        self.pending_child_document_owner_retirements
            .drain(..)
            .collect()
    }

    pub(crate) fn take_pending_parent_document_descendant_completions(
        &mut self,
    ) -> Vec<FrameDocumentDescendantLoadCompletion> {
        self.pending_parent_document_descendant_completions
            .drain(..)
            .collect()
    }

    pub(crate) fn detach_child_frame(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentDescendantLoadCompletion> {
        let _ = self.detach_current_child_document(child_handle);
        let frame_id = self.frame_ids_by_child_handle.remove(&child_handle)?;
        let parent_completion = self.release_parent_document_descendant_load_for_frame(&frame_id);
        if let Some(owner_element) = self.frame_owner_elements.get_mut(&child_handle) {
            owner_element.content_frame_id = None;
            owner_element.lifecycle = FrameOwnerElementLifecycleState::Detached;
        }
        self.detach_current_frame_records(&frame_id);
        parent_completion
    }

    pub(crate) fn ensure_child_realm(&mut self, child_handle: DomHandle) -> Option<FrameRealmId> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?;
        let frame = self.frames.get(frame_id)?;
        let local_window_id = frame.current_local_window_id?;
        let document_id = frame.current_document_id?;
        if let Some(realm_id) = self
            .local_windows
            .get(&local_window_id)
            .and_then(|window| window.realm_id)
            && let Some(realm) = self.realms.get_mut(&realm_id)
            && realm.local_window_id == local_window_id
            && realm.document_id == document_id
            && realm.lifecycle.belongs_to_current_local_window()
        {
            return Some(realm_id);
        }
        let realm_id = self.ids.frame_realm();
        self.realms.insert(
            realm_id,
            FrameRealmRecord {
                id: realm_id,
                local_window_id,
                document_id,
                inspector_execution_context_id: None,
                lifecycle: FrameRealmLifecycleState::Reserved,
            },
        );
        if let Some(local_window) = self.local_windows.get_mut(&local_window_id) {
            local_window.realm_id = Some(realm_id);
        }
        Some(realm_id)
    }

    fn retarget_preserved_realm_to_document(
        &mut self,
        realm_id: FrameRealmId,
        document_id: DocumentId,
    ) {
        let realm = self
            .realms
            .get_mut(&realm_id)
            .expect("a preserved LocalWindow must retain its realm record");
        realm.document_id = document_id;
        if realm.lifecycle == FrameRealmLifecycleState::MaterializationQueued {
            // The durable task names the retired exact Document. Preserve the
            // realm identity, but require the replacement Document to publish
            // its own typed materialization task.
            realm.lifecycle = FrameRealmLifecycleState::Reserved;
            realm.inspector_execution_context_id = None;
        }
    }

    pub(crate) fn request_child_realm_materialization(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<FrameRealmMaterializationRequest> {
        if self.current_child_document_task_owner(child_handle) != Some(owner) {
            return None;
        }
        let realm_id = self.current_reserved_realm_id_for_document_task_owner(owner)?;
        let realm = self.realms.get_mut(&realm_id)?;
        match realm.lifecycle {
            FrameRealmLifecycleState::Reserved => {
                realm.lifecycle = FrameRealmLifecycleState::MaterializationQueued;
                Some(FrameRealmMaterializationRequest::NewlyQueued { realm_id })
            }
            FrameRealmLifecycleState::MaterializationQueued => {
                Some(FrameRealmMaterializationRequest::AlreadyQueued { realm_id })
            }
            FrameRealmLifecycleState::Materialized => {
                Some(FrameRealmMaterializationRequest::AlreadyMaterialized { realm_id })
            }
            FrameRealmLifecycleState::DetachedReachable | FrameRealmLifecycleState::Disposed => {
                None
            }
        }
    }

    pub(crate) fn rollback_child_realm_materialization_request(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        if self.current_child_document_task_owner(child_handle) != Some(owner)
            || self.current_reserved_realm_id_for_document_task_owner(owner) != Some(realm_id)
        {
            return false;
        }
        let Some(realm) = self.realms.get_mut(&realm_id) else {
            return false;
        };
        if realm.lifecycle != FrameRealmLifecycleState::MaterializationQueued {
            return false;
        }
        realm.lifecycle = FrameRealmLifecycleState::Reserved;
        true
    }

    pub(crate) fn retire_child_realm_materialization_request(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        let Some(realm_id) = self.current_reserved_realm_id_for_document_task_owner(owner) else {
            return false;
        };
        self.rollback_child_realm_materialization_request(child_handle, owner, realm_id)
    }

    pub(crate) fn child_realm_materialization_is_queued(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.current_child_document_task_owner(child_handle) == Some(owner)
            && self
                .current_reserved_realm_id_for_document_task_owner(owner)
                .and_then(|realm_id| self.realms.get(&realm_id))
                .is_some_and(|realm| {
                    realm.lifecycle == FrameRealmLifecycleState::MaterializationQueued
                })
    }

    pub(crate) fn has_queued_child_realm_materialization(&self) -> bool {
        self.realms
            .values()
            .any(|realm| realm.lifecycle == FrameRealmLifecycleState::MaterializationQueued)
    }

    pub(crate) fn bind_child_realm_inspector_context(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        inspector_execution_context_id: i64,
    ) -> Option<FrameRealmId> {
        if self.current_child_document_task_owner(child_handle) != Some(owner) {
            return None;
        }
        let realm_id = self.current_reserved_realm_id_for_document_task_owner(owner)?;
        let realm = self.realms.get_mut(&realm_id)?;
        if !matches!(
            realm.lifecycle,
            FrameRealmLifecycleState::MaterializationQueued
                | FrameRealmLifecycleState::Materialized
        ) {
            return None;
        }
        realm.inspector_execution_context_id = Some(inspector_execution_context_id);
        Some(realm_id)
    }

    pub(crate) fn complete_child_realm_materialization(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        if self.current_child_document_task_owner(child_handle) != Some(owner)
            || self.current_reserved_realm_id_for_document_task_owner(owner) != Some(realm_id)
        {
            return false;
        }
        let Some(realm) = self.realms.get_mut(&realm_id) else {
            return false;
        };
        if realm.inspector_execution_context_id.is_none() {
            return false;
        }
        match realm.lifecycle {
            FrameRealmLifecycleState::MaterializationQueued => {
                realm.lifecycle = FrameRealmLifecycleState::Materialized;
                true
            }
            FrameRealmLifecycleState::Materialized => true,
            FrameRealmLifecycleState::Reserved
            | FrameRealmLifecycleState::DetachedReachable
            | FrameRealmLifecycleState::Disposed => false,
        }
    }

    pub(crate) fn fail_child_realm_materialization(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        let Some(realm_id) = self.current_reserved_realm_id_for_document_task_owner(owner) else {
            return false;
        };
        if !self.rollback_child_realm_materialization_request(child_handle, owner, realm_id) {
            return false;
        }
        if let Some(realm) = self.realms.get_mut(&realm_id) {
            realm.inspector_execution_context_id = None;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn clear_child_realm(&mut self, child_handle: DomHandle) {
        let realm_id = self
            .frame_ids_by_child_handle
            .get(&child_handle)
            .and_then(|frame_id| self.frames.get(frame_id))
            .and_then(|frame| frame.current_local_window_id)
            .and_then(|local_window_id| self.local_windows.get(&local_window_id))
            .and_then(|local_window| local_window.realm_id);
        let Some(realm_id) = realm_id else {
            return;
        };
        self.clear_child_realm_if_matches(child_handle, realm_id);
    }

    pub(crate) fn clear_child_realm_if_matches(
        &mut self,
        child_handle: DomHandle,
        expected_realm_id: FrameRealmId,
    ) -> bool {
        // The child handle identifies the stable browsing context, not one
        // LocalWindow. Never let retirement of an older context clear the
        // replacement Document's reserved or materialized realm.
        let Some(frame_id) = self.frame_ids_by_child_handle.get(&child_handle) else {
            return false;
        };
        let Some(local_window_id) = self
            .frames
            .get(frame_id)
            .and_then(|frame| frame.current_local_window_id)
        else {
            return false;
        };
        let realm_id = self
            .local_windows
            .get(&local_window_id)
            .and_then(|local_window| local_window.realm_id);
        if realm_id != Some(expected_realm_id) {
            return false;
        }
        if let Some(realm) = self.realms.get_mut(&expected_realm_id) {
            realm.lifecycle = FrameRealmLifecycleState::Disposed;
        }
        if let Some(local_window) = self.local_windows.get_mut(&local_window_id) {
            local_window.realm_id = None;
        }
        true
    }

    pub(crate) fn set_current_child_service_worker_client_id(
        &mut self,
        child_handle: DomHandle,
        client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    ) -> bool {
        let Some(local_window_id) = self
            .current_child_owner_snapshot(child_handle)
            .map(|snapshot| snapshot.local_window_id)
        else {
            return false;
        };
        let Some(local_window) = self.local_windows.get_mut(&local_window_id) else {
            return false;
        };
        local_window.settings.service_worker_client_id = client_id;
        true
    }

    pub(crate) fn set_current_main_service_worker_client_id(
        &mut self,
        client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    ) -> bool {
        let Some(local_window_id) = self
            .current_main_owner_snapshot()
            .map(|snapshot| snapshot.local_window_id)
        else {
            return false;
        };
        let Some(local_window) = self.local_windows.get_mut(&local_window_id) else {
            return false;
        };
        local_window.settings.service_worker_client_id = client_id;
        true
    }

    pub(crate) fn update_current_child_document_urls(
        &mut self,
        child_handle: DomHandle,
        document_url: Url,
        document_base_url: Url,
    ) -> bool {
        let Some(snapshot) = self.current_child_owner_snapshot(child_handle) else {
            return false;
        };
        let Some(document) = self.documents.get_mut(&snapshot.document_id) else {
            return false;
        };
        if document.local_window_id != snapshot.local_window_id
            || document.document_handle != snapshot.document_handle
            || document.lifecycle != DocumentLifecycleState::Current
        {
            return false;
        }
        document.url = document_url;
        document.base_url = document_base_url.clone();
        let Some(local_window) = self.local_windows.get_mut(&snapshot.local_window_id) else {
            return false;
        };
        if local_window.document_id != snapshot.document_id
            || local_window.lifecycle != LocalWindowLifecycleState::Current
        {
            return false;
        }
        local_window.settings.base_url = document_base_url;
        true
    }

    pub(crate) fn frame_id_for_child_handle(&self, child_handle: DomHandle) -> Option<&FrameId> {
        self.frame_ids_by_child_handle.get(&child_handle)
    }

    #[cfg(test)]
    pub(crate) fn frame_owner_element_for_child_handle(
        &self,
        child_handle: DomHandle,
    ) -> Option<&FrameOwnerElementRecord> {
        self.frame_owner_elements.get(&child_handle)
    }

    pub(crate) fn current_main_owner_snapshot(&self) -> Option<FrameOwnerSnapshot> {
        self.current_frame_owner_snapshot(&main_frame_id())
            .filter(|snapshot| snapshot.kind == FrameKind::Main)
    }

    pub(crate) fn current_main_document_task_owner(&self) -> Option<FrameDocumentTaskOwner> {
        let snapshot = self.current_main_owner_snapshot()?;
        Some(FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        ))
    }

    pub(crate) fn main_document_task_owner_is_current(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.current_main_owner_snapshot().is_some_and(|snapshot| {
            snapshot.scheduler_lane_id == owner.scheduler_lane_id
                && snapshot.local_window_id == owner.local_window_id
                && snapshot.document_id == owner.document_id
        })
    }

    pub(crate) fn finish_current_main_document_parsing(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentInteractiveLifecycleAction> {
        if !self.main_document_task_owner_is_current(owner)
            || !self
                .documents
                .get(&owner.document_id)
                .is_some_and(|document| document.lifecycle_progress.can_finish_parsing())
        {
            return None;
        }
        let delay_token = self.ids.document_load_delay_token();
        let transitioned = self
            .documents
            .get_mut(&owner.document_id)
            .is_some_and(|document| document.lifecycle_progress.finish_parsing(delay_token));
        transitioned.then(|| MainDocumentInteractiveLifecycleAction::new(owner, delay_token))
    }

    pub(crate) fn apply_current_main_document_interactive_transition(
        &mut self,
        action: MainDocumentInteractiveLifecycleAction,
    ) -> bool {
        if !self.main_document_task_owner_is_current(action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .apply_interactive_transition(action.delay_token())
            })
    }

    pub(crate) fn prepare_current_main_document_domcontentloaded_transition(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentDomContentLoadedLifecycleAction> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let delay_token = self
            .documents
            .get_mut(&owner.document_id)?
            .lifecycle_progress
            .prepare_domcontentloaded_transition()?;
        Some(MainDocumentDomContentLoadedLifecycleAction::new(
            owner,
            delay_token,
        ))
    }

    pub(crate) fn current_main_document_domcontentloaded_transition_is_ready(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents.get(&owner.document_id).map(|document| {
            document
                .lifecycle_progress
                .can_prepare_domcontentloaded_transition()
        })
    }

    pub(crate) fn apply_current_main_document_domcontentloaded_transition(
        &mut self,
        action: MainDocumentDomContentLoadedLifecycleAction,
    ) -> bool {
        if !self.main_document_task_owner_is_current(action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .apply_domcontentloaded_transition(action.delay_token())
            })
    }

    pub(crate) fn prepare_current_main_document_complete_transition(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentCompleteLifecycleAction> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let transition_token = self.ids.document_load_delay_token();
        let prepared = self
            .documents
            .get_mut(&owner.document_id)?
            .lifecycle_progress
            .prepare_complete_transition(transition_token);
        prepared.then(|| MainDocumentCompleteLifecycleAction::new(owner, transition_token))
    }

    pub(crate) fn current_main_document_complete_transition_is_ready(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents.get(&owner.document_id).map(|document| {
            document
                .lifecycle_progress
                .can_prepare_complete_transition()
        })
    }

    /// Stop the exact current main Document while keeping its Window and
    /// Document identities live.
    ///
    /// The returned boolean reports whether the observable ready state still
    /// needs to transition to `complete`.  `None` means the supplied owner is
    /// stale or no longer owns lifecycle state.
    pub(crate) fn stop_current_main_document_loading(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents
            .get_mut(&owner.document_id)?
            .lifecycle_progress
            .stop_loading_without_load_event()
    }

    pub(crate) fn apply_current_main_document_complete_transition(
        &mut self,
        action: MainDocumentCompleteLifecycleAction,
    ) -> bool {
        if !self.main_document_task_owner_is_current(action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .apply_complete_transition(action.transition_token())
            })
    }

    pub(crate) fn begin_current_main_document_load_dispatch(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        if !self.main_document_task_owner_is_current(owner) {
            return false;
        }
        self.documents
            .get_mut(&owner.document_id)
            .is_some_and(|document| document.lifecycle_progress.begin_main_load_dispatch())
    }

    pub(crate) fn finish_current_main_document_load_dispatch(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents
            .get_mut(&owner.document_id)
            .and_then(|document| document.lifecycle_progress.finish_main_load_dispatch())
    }

    pub(crate) fn finish_current_main_document_load_after_descendant_completion(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents
            .get_mut(&owner.document_id)
            .and_then(|document| {
                document
                    .lifecycle_progress
                    .finish_main_load_after_descendant_completion()
            })
    }

    pub(crate) fn current_main_document_load_has_dispatched(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.main_document_task_owner_is_current(owner)
            && self
                .documents
                .get(&owner.document_id)
                .is_some_and(|document| document.lifecycle_progress.main_load_has_dispatched())
    }

    pub(crate) fn current_main_document_load_completion_state(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents
            .get(&owner.document_id)?
            .lifecycle_progress
            .main_load_completion_state()
    }

    pub(crate) fn current_frame_owner_snapshot(
        &self,
        frame_id: &FrameId,
    ) -> Option<FrameOwnerSnapshot> {
        let frame = self.frames.get(frame_id)?;
        if frame.lifecycle != FrameLifecycleState::Attached {
            return None;
        }
        let local_window_id = frame.current_local_window_id?;
        let document_id = frame.current_document_id?;

        let scheduler_lane = self.scheduler_lanes.get(&frame.scheduler_lane_id)?;
        if scheduler_lane.id != frame.scheduler_lane_id
            || scheduler_lane.frame_id != *frame_id
            || scheduler_lane.lifecycle != FrameSchedulerLaneLifecycleState::Active
        {
            return None;
        }

        let proxy = self.window_proxies.get(&frame.window_proxy_id)?;
        if proxy.id != frame.window_proxy_id
            || proxy.frame_id != *frame_id
            || proxy.current_local_window_id != Some(local_window_id)
            || proxy.reachability != WindowProxyReachability::LiveFrame
        {
            return None;
        }

        let local_window = self.local_windows.get(&local_window_id)?;
        if local_window.id != local_window_id
            || local_window.frame_id != *frame_id
            || local_window.document_id != document_id
            || local_window.lifecycle != LocalWindowLifecycleState::Current
        {
            return None;
        }

        let document = self.documents.get(&document_id)?;
        if document.id != document_id
            || document.local_window_id != local_window_id
            || document.lifecycle != DocumentLifecycleState::Current
        {
            return None;
        }

        if let Some(realm_id) = local_window.realm_id {
            let realm = self.realms.get(&realm_id)?;
            if realm.id != realm_id
                || realm.local_window_id != local_window_id
                || realm.document_id != document_id
                || !realm.lifecycle.belongs_to_current_local_window()
            {
                return None;
            }
        }

        Some(FrameOwnerSnapshot {
            frame_id: frame_id.clone(),
            kind: frame.kind,
            parent_frame_id: frame.parent_frame_id.clone(),
            owner_element_handle: frame.owner_element_handle,
            #[cfg(test)]
            window_proxy_id: frame.window_proxy_id,
            scheduler_lane_id: frame.scheduler_lane_id,
            local_window_id,
            document_id,
            document_handle: document.document_handle,
            document_url: document.url.clone(),
            document_base_url: document.base_url.clone(),
            realm_id: local_window.realm_id,
            settings: local_window.settings.clone(),
        })
    }

    pub(crate) fn current_child_owner_snapshot(
        &self,
        child_handle: DomHandle,
    ) -> Option<ChildFrameOwnerSnapshot> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        let owner_element = self.frame_owner_elements.get(&child_handle)?;
        if owner_element.owner_handle != child_handle
            || owner_element.content_frame_id.as_ref() != Some(&frame_id)
            || owner_element.lifecycle != FrameOwnerElementLifecycleState::Attached
        {
            return None;
        }

        let owner = self.current_frame_owner_snapshot(&frame_id)?;
        if owner.kind != FrameKind::ChildIframe
            || owner.owner_element_handle != Some(child_handle)
            || owner.frame_id != frame_id
            || owner_element.parent_frame_id != owner.parent_frame_id
        {
            return None;
        }

        Some(ChildFrameOwnerSnapshot {
            owner_handle: child_handle,
            frame_id: owner.frame_id,
            parent_frame_id: owner.parent_frame_id,
            scheduler_lane_id: owner.scheduler_lane_id,
            local_window_id: owner.local_window_id,
            document_id: owner.document_id,
            document_handle: owner.document_handle,
            document_url: owner.document_url,
            document_base_url: owner.document_base_url,
            realm_id: owner.realm_id,
            settings: owner.settings,
        })
    }

    pub(crate) fn current_child_document_owner(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentOwner> {
        self.current_child_document_task_owner(child_handle)
            .map(FrameDocumentTaskOwner::document_owner)
    }

    pub(crate) fn child_document_owner_is_current(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
    ) -> bool {
        self.current_child_document_owner(child_handle)
            .is_some_and(|current| current == owner)
    }

    pub(crate) fn current_frame_document_task_owner(
        &self,
        frame_id: &FrameId,
    ) -> Option<FrameDocumentTaskOwner> {
        let snapshot = self.current_frame_owner_snapshot(frame_id)?;
        Some(FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        ))
    }

    pub(crate) fn current_child_document_task_owner(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentTaskOwner> {
        let snapshot = self.current_child_owner_snapshot(child_handle)?;
        Some(FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        ))
    }

    pub(crate) fn current_child_document_creation_kind(
        &self,
        child_handle: DomHandle,
    ) -> Option<DocumentCreationKind> {
        let snapshot = self.current_child_owner_snapshot(child_handle)?;
        self.documents
            .get(&snapshot.document_id)
            .map(|document| document.creation_kind)
    }

    pub(crate) fn current_child_document_task_owner_reserved_realm(
        &self,
        child_handle: DomHandle,
    ) -> Option<(FrameDocumentTaskOwner, FrameRealmId)> {
        let owner = self.current_child_document_task_owner(child_handle)?;
        let realm_id = self.current_reserved_realm_id_for_document_task_owner(owner)?;
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        Some((owner, realm_id))
    }

    pub(crate) fn current_child_document_task_owner_materialized_realm(
        &self,
        child_handle: DomHandle,
    ) -> Option<(FrameDocumentTaskOwner, FrameRealmId)> {
        let owner = self.current_child_document_task_owner(child_handle)?;
        let realm_id = self.current_materialized_realm_id_for_document_task_owner(owner)?;
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        Some((owner, realm_id))
    }

    pub(crate) fn child_document_task_owner_is_current(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.current_child_document_task_owner(child_handle)
            .is_some_and(|current| current == owner)
    }

    pub(crate) fn finish_current_child_document_parsing(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
    ) -> Option<FrameDocumentInteractiveLifecycleAction> {
        let task_owner = self.current_child_document_task_owner(child_handle)?;
        if task_owner.document_owner() != owner
            || !self
                .documents
                .get(&owner.document_id)
                .is_some_and(|document| document.lifecycle_progress.can_finish_parsing())
        {
            return None;
        }
        let delay_token = self.ids.document_load_delay_token();
        let transitioned = self
            .documents
            .get_mut(&owner.document_id)
            .is_some_and(|document| document.lifecycle_progress.finish_parsing(delay_token));
        transitioned.then(|| {
            FrameDocumentInteractiveLifecycleAction::new(child_handle, task_owner, delay_token)
        })
    }

    pub(crate) fn apply_current_child_document_interactive_transition(
        &mut self,
        action: FrameDocumentInteractiveLifecycleAction,
    ) -> bool {
        if !self.child_document_task_owner_is_current(action.child_handle(), action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .apply_interactive_transition(action.delay_token())
            })
    }

    pub(crate) fn prepare_current_child_document_domcontentloaded_transition(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<FrameDocumentDomContentLoadedLifecycleAction> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        let delay_token = self
            .documents
            .get_mut(&owner.document_id)?
            .lifecycle_progress
            .prepare_domcontentloaded_transition()?;
        Some(FrameDocumentDomContentLoadedLifecycleAction::new(
            child_handle,
            owner,
            delay_token,
        ))
    }

    pub(crate) fn apply_current_child_document_domcontentloaded_transition(
        &mut self,
        action: FrameDocumentDomContentLoadedLifecycleAction,
    ) -> bool {
        if !self.child_document_task_owner_is_current(action.child_handle(), action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .apply_domcontentloaded_transition(action.delay_token())
            })
    }

    pub(crate) fn prepare_current_child_document_complete_transition(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<FrameDocumentCompleteLifecycleAction> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        let transition_token = self.ids.document_load_delay_token();
        let prepared = self
            .documents
            .get_mut(&owner.document_id)?
            .lifecycle_progress
            .prepare_complete_transition(transition_token);
        prepared.then(|| {
            FrameDocumentCompleteLifecycleAction::new(child_handle, owner, transition_token)
        })
    }

    pub(crate) fn cancel_current_child_document_complete_transition(
        &mut self,
        action: FrameDocumentCompleteLifecycleAction,
    ) -> bool {
        if !self.child_document_task_owner_is_current(action.child_handle(), action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .cancel_complete_transition(action.transition_token())
            })
    }

    pub(crate) fn apply_current_child_document_complete_transition(
        &mut self,
        action: FrameDocumentCompleteLifecycleAction,
    ) -> bool {
        if !self.child_document_task_owner_is_current(action.child_handle(), action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .apply_complete_transition(action.transition_token())
            })
    }

    pub(crate) fn current_child_document_lifecycle_action_is_pending(
        &self,
        action: FrameDocumentLifecycleAction,
    ) -> bool {
        if !self.child_document_task_owner_is_current(action.child_handle(), action.owner()) {
            return false;
        }
        let Some(lifecycle) = self
            .documents
            .get(&action.owner().document_id)
            .map(|document| &document.lifecycle_progress)
        else {
            return false;
        };
        match action {
            FrameDocumentLifecycleAction::Interactive(action) => {
                lifecycle.interactive_transition_is_pending(action.delay_token())
            }
            FrameDocumentLifecycleAction::DomContentLoaded(action) => {
                lifecycle.domcontentloaded_transition_is_pending(action.delay_token())
            }
            FrameDocumentLifecycleAction::Complete(action) => {
                lifecycle.complete_transition_is_pending(action.transition_token())
            }
        }
    }

    pub(crate) fn complete_current_child_initial_empty_document(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentLoadDeliveryTask> {
        let owner = self.current_child_document_task_owner(child_handle)?;
        let document = self.documents.get_mut(&owner.document_id)?;
        if !document.creation_kind.is_initial_empty() {
            return None;
        }
        let completed = document
            .lifecycle_progress
            .complete_initial_empty_document();
        completed.then_some(FrameDocumentLoadDeliveryTask {
            child_handle,
            owner,
        })
    }

    pub(crate) fn suppress_current_child_initial_empty_load_delivery(
        &mut self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> bool {
        if !self.child_document_task_owner_is_current(task.child_handle, task.owner) {
            return false;
        }
        self.documents
            .get_mut(&task.owner.document_id)
            .filter(|document| document.creation_kind.is_initial_empty())
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .suppress_initial_empty_load_delivery()
            })
    }

    pub(crate) fn begin_current_child_document_unload(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<super::FrameDocumentUnloadLifecycleAction> {
        let owner = self.current_child_document_task_owner(child_handle)?;
        let began = self
            .documents
            .get_mut(&owner.document_id)?
            .lifecycle_progress
            .begin_child_unload_dispatch();
        began.then(|| super::FrameDocumentUnloadLifecycleAction::new(child_handle, owner))
    }

    pub(crate) fn finish_current_child_document_unload(
        &mut self,
        action: super::FrameDocumentUnloadLifecycleAction,
    ) -> bool {
        if !self.child_document_task_owner_is_current(action.child_handle(), action.owner()) {
            return false;
        }
        self.documents
            .get_mut(&action.owner().document_id)
            .filter(|document| document.local_window_id == action.owner().local_window_id)
            .is_some_and(|document| document.lifecycle_progress.finish_child_unload_dispatch())
    }

    pub(crate) fn begin_current_child_document_load_delivery(
        &mut self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> Option<FrameDocumentLoadDeliveryAction> {
        if !self.child_document_task_owner_is_current(task.child_handle, task.owner) {
            return None;
        }
        let phase = self
            .documents
            .get_mut(&task.owner.document_id)
            .and_then(|document| {
                document
                    .lifecycle_progress
                    .begin_child_load_delivery_phase()
            })?;
        Some(FrameDocumentLoadDeliveryAction::new(task, phase))
    }

    pub(crate) fn current_child_document_load_delivery_is_ready(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.child_document_task_owner_is_current(child_handle, owner)
            && self
                .documents
                .get(&owner.document_id)
                .is_some_and(|document| document.lifecycle_progress.load_delivery_is_ready())
    }

    pub(crate) fn reserve_current_child_document_load_delivery_task(
        &mut self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> Option<FrameDocumentLoadDeliveryAdmission> {
        if !self.child_document_task_owner_is_current(task.child_handle, task.owner) {
            return None;
        }
        let admission_id = self.ids.document_load_delivery_admission();
        let reserved = self
            .documents
            .get_mut(&task.owner.document_id)?
            .lifecycle_progress
            .reserve_child_load_delivery_task(admission_id);
        reserved.then(|| FrameDocumentLoadDeliveryAdmission::new(task, admission_id))
    }

    pub(crate) fn current_child_document_load_delivery_task_is_reserved(
        &self,
        admission: FrameDocumentLoadDeliveryAdmission,
    ) -> bool {
        let task = admission.task();
        self.child_document_task_owner_is_current(task.child_handle, task.owner)
            && self
                .documents
                .get(&task.owner.document_id)
                .is_some_and(|document| {
                    document
                        .lifecycle_progress
                        .child_load_delivery_task_is_reserved(admission.admission_id())
                })
    }

    pub(crate) fn release_current_child_document_load_delivery_task_reservation(
        &mut self,
        admission: FrameDocumentLoadDeliveryAdmission,
    ) -> bool {
        let task = admission.task();
        if !self.child_document_task_owner_is_current(task.child_handle, task.owner) {
            return false;
        }
        self.documents
            .get_mut(&task.owner.document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .release_child_load_delivery_task_reservation(admission.admission_id())
            })
    }

    /// Retire the current exact Document's outstanding HostLoad admission.
    ///
    /// Navigation admission uses this before publishing its own task. The old
    /// stable task remains in FIFO order, but its token can no longer authorize
    /// delivery and therefore settles stale without touching any later
    /// reservation.
    pub(crate) fn retire_current_child_document_load_delivery_task_reservation(
        &mut self,
        child_handle: DomHandle,
    ) -> bool {
        let Some(owner) = self.current_child_document_task_owner(child_handle) else {
            return false;
        };
        self.documents
            .get_mut(&owner.document_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .retire_child_load_delivery_task_reservation()
            })
    }

    pub(crate) fn has_pending_current_child_document_lifecycle(&self) -> bool {
        self.frame_ids_by_child_handle.values().any(|frame_id| {
            let Some(frame) = self.frames.get(frame_id) else {
                return false;
            };
            if frame.kind != FrameKind::ChildIframe
                || frame.lifecycle != FrameLifecycleState::Attached
            {
                return false;
            }
            let Some(document_id) = frame.current_document_id else {
                return false;
            };
            self.documents.get(&document_id).is_some_and(|document| {
                document.lifecycle == DocumentLifecycleState::Current
                    && document.lifecycle_progress.child_load_delivery_is_pending()
            })
        })
    }

    pub(crate) fn abort_child_document_load_delivery(
        &mut self,
        action: FrameDocumentLoadDeliveryAction,
    ) -> bool {
        self.documents
            .get_mut(&action.owner().document_id)
            .filter(|document| document.local_window_id == action.owner().local_window_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .abort_child_load_delivery_phase(action.phase())
            })
    }

    pub(crate) fn finish_current_child_document_load_delivery(
        &mut self,
        action: FrameDocumentLoadDeliveryAction,
    ) -> Option<FrameDocumentLoadDeliveryProgress> {
        let task = action.task();
        if !self.child_document_task_owner_is_current(task.child_handle, task.owner) {
            return None;
        }
        let snapshot = self.current_child_owner_snapshot(task.child_handle)?;
        if snapshot.scheduler_lane_id != task.owner.scheduler_lane_id
            || snapshot.local_window_id != task.owner.local_window_id
            || snapshot.document_id != task.owner.document_id
        {
            return None;
        }
        let lifecycle = &mut self
            .documents
            .get_mut(&task.owner.document_id)?
            .lifecycle_progress;
        let finished = lifecycle.finish_child_load_delivery_phase(action.phase())?;
        if !finished {
            if lifecycle.has_incomplete_child_frames() {
                return Some(FrameDocumentLoadDeliveryProgress::AwaitingDescendantCompletion(task));
            }
            return Some(FrameDocumentLoadDeliveryProgress::Continue(task));
        }
        let frame_id = self
            .frame_ids_by_child_handle
            .get(&task.child_handle)
            .cloned();
        let parent_descendant_completion = frame_id
            .as_ref()
            .and_then(|frame_id| self.release_parent_document_descendant_load_for_frame(frame_id));
        Some(FrameDocumentLoadDeliveryProgress::Finished(
            FrameDocumentLoadDispatchFinish {
                child_handle: task.child_handle,
                owner: task.owner,
                frame_id: snapshot.frame_id,
                parent_frame_id: snapshot.parent_frame_id,
                document_url: snapshot.document_url,
                parent_descendant_completion,
            },
        ))
    }

    pub(crate) fn current_child_document_has_load_delay_tokens(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        self.documents
            .get(&owner.document_id)
            .map(|document| document.lifecycle_progress.has_load_delay_tokens())
    }

    pub(crate) fn current_child_document_has_incomplete_descendants(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        self.documents
            .get(&owner.document_id)
            .map(|document| document.lifecycle_progress.has_incomplete_child_frames())
    }

    pub(crate) fn ensure_current_child_navigation_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentNavigationLoadBinding> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        let current = self
            .frames
            .get(&frame_id)
            .and_then(|frame| frame.navigation_load)
            .map(|state| state.binding);
        if let Some(current) = current
            && self.child_document_task_owner_is_current(child_handle, current.owner())
            && current.document_load_delay_token().is_none_or(|token| {
                self.documents
                    .get(&current.owner().document_id)
                    .is_some_and(|document| {
                        document
                            .lifecycle_progress
                            .owns_load_delay(token, DocumentLoadDelayReason::Navigation)
                    })
            })
        {
            return Some(current);
        }
        self.replace_current_child_navigation_load(child_handle)
    }

    pub(crate) fn replace_current_child_navigation_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentNavigationLoadBinding> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        if let Some(previous) = self
            .frames
            .get_mut(&frame_id)
            .and_then(|frame| frame.navigation_load.take())
            .map(|state| state.binding)
            && let Some(token) = previous.document_load_delay_token()
        {
            let _ = self.release_document_load_delay(
                previous.owner(),
                token,
                DocumentLoadDelayReason::Navigation,
            );
        }
        let owner = self.current_child_document_task_owner(child_handle)?;
        let document_load_delay_token = self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::Navigation,
        );
        if document_load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        let binding = FrameDocumentNavigationLoadBinding::new(
            owner,
            self.ids.frame_navigation(),
            document_load_delay_token,
        );
        self.frames.get_mut(&frame_id)?.navigation_load =
            Some(FrameNavigationLoadState::unqueued(binding));
        Some(binding)
    }

    /// Reserve exactly one stable Page task for the current child navigation
    /// generation. Duplicate producer observations do not create duplicate
    /// scheduler work; replacing `navigation_load` invalidates this
    /// reservation and gives the next task a distinct generation.
    pub(crate) fn reserve_current_child_navigation_commit_task(
        &mut self,
        task: FrameLaneNavigationCommitTask,
    ) -> FrameNavigationCommitReservationResult {
        if !self.child_frame_lane_task_owner_is_current(task.child_handle, task.owner) {
            return FrameNavigationCommitReservationResult::NotCurrent;
        }
        let Some(frame_id) = self
            .frame_ids_by_child_handle
            .get(&task.child_handle)
            .cloned()
        else {
            return FrameNavigationCommitReservationResult::NotCurrent;
        };
        let Some(frame) = self.frames.get_mut(&frame_id) else {
            return FrameNavigationCommitReservationResult::NotCurrent;
        };
        let Some(state) = frame.navigation_load.as_mut() else {
            return FrameNavigationCommitReservationResult::NotCurrent;
        };
        if state.binding != task.navigation_load {
            return FrameNavigationCommitReservationResult::NotCurrent;
        }
        if state.commit_task_queued {
            return FrameNavigationCommitReservationResult::AlreadyReserved;
        }
        state.commit_task_queued = true;
        FrameNavigationCommitReservationResult::Reserved
    }

    pub(crate) fn current_child_navigation_commit_task(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameLaneNavigationCommitTask> {
        let owner = self.current_child_frame_lane_task_owner(child_handle)?;
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?;
        let frame = self.frames.get(frame_id)?;
        let state = frame.navigation_load?;
        state
            .commit_task_queued
            .then_some(FrameLaneNavigationCommitTask {
                child_handle,
                owner,
                navigation_load: state.binding,
            })
    }

    pub(crate) fn claim_current_child_navigation_commit_task(
        &mut self,
        expected: FrameLaneNavigationCommitTask,
    ) -> bool {
        if self.current_child_navigation_commit_task(expected.child_handle) != Some(expected) {
            return false;
        }
        let Some(frame_id) = self
            .frame_ids_by_child_handle
            .get(&expected.child_handle)
            .cloned()
        else {
            return false;
        };
        let Some(frame) = self.frames.get_mut(&frame_id) else {
            return false;
        };
        let Some(state) = frame.navigation_load.as_mut() else {
            return false;
        };
        state.commit_task_queued = false;
        true
    }

    /// Retire a reservation only when it still names this exact generation.
    /// An old Page task must not clear the reservation of a replacement
    /// navigation that reused the same child handle.
    pub(crate) fn retire_child_navigation_commit_task(
        &mut self,
        expected: FrameLaneNavigationCommitTask,
    ) -> bool {
        let Some(frame_id) = self
            .frame_ids_by_child_handle
            .get(&expected.child_handle)
            .cloned()
        else {
            return false;
        };
        let Some(frame) = self.frames.get_mut(&frame_id) else {
            return false;
        };
        let Some(state) = frame.navigation_load.as_mut() else {
            return false;
        };
        if state.binding != expected.navigation_load || !state.commit_task_queued {
            return false;
        }
        state.commit_task_queued = false;
        true
    }

    pub(crate) fn settle_current_child_navigation_load(
        &mut self,
        child_handle: DomHandle,
        expected: FrameDocumentNavigationLoadBinding,
    ) -> Option<FrameDocumentTaskOwner> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        let current = self.frames.get(&frame_id)?.navigation_load?.binding;
        if current != expected {
            return None;
        }
        if let Some(token) = expected.document_load_delay_token()
            && !self
                .documents
                .get(&expected.owner().document_id)
                .is_some_and(|document| {
                    document
                        .lifecycle_progress
                        .owns_load_delay(token, DocumentLoadDelayReason::Navigation)
                })
        {
            return None;
        }
        self.frames.get_mut(&frame_id)?.navigation_load = None;
        if let Some(token) = expected.document_load_delay_token() {
            let released = self.release_document_load_delay(
                expected.owner(),
                token,
                DocumentLoadDelayReason::Navigation,
            );
            debug_assert!(released);
        }
        Some(expected.owner())
    }

    pub(crate) fn current_child_navigation_load(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentNavigationLoadBinding> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?;
        self.frames
            .get(frame_id)?
            .navigation_load
            .map(|state| state.binding)
    }

    pub(crate) fn current_child_frame_load_is_pending(&self, child_handle: DomHandle) -> bool {
        let Some(frame_id) = self.frame_ids_by_child_handle.get(&child_handle) else {
            return false;
        };
        let Some(frame) = self.frames.get(frame_id) else {
            return false;
        };
        if frame.navigation_load.is_some() {
            return true;
        }
        frame
            .current_document_id
            .and_then(|document_id| self.documents.get(&document_id))
            .is_some_and(|document| {
                document.lifecycle == DocumentLifecycleState::Current
                    && document.lifecycle_progress.child_load_delivery_is_pending()
            })
    }

    pub(crate) fn begin_child_frame_parent_document_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentDescendantLoadCompletion> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        let parent_owner = self.current_parent_document_task_owner_for_child_frame(&frame_id);
        let current_binding = self
            .frames
            .get(&frame_id)
            .and_then(|frame| frame.parent_document_load.clone());
        if let (Some(parent_owner), Some(binding)) = (parent_owner, current_binding.as_ref())
            && binding.parent_owner == parent_owner
            && binding.child_frame_id == frame_id
            && self
                .documents
                .get(&parent_owner.document_id)
                .filter(|document| document.local_window_id == parent_owner.local_window_id)
                .is_some_and(|document| {
                    document
                        .lifecycle_progress
                        .descendant_is_incomplete(&frame_id)
                })
        {
            return None;
        }

        let released_parent = self.release_parent_document_descendant_load_for_frame(&frame_id);
        let Some(parent_owner) = parent_owner else {
            return released_parent;
        };
        let marked = self
            .documents
            .get_mut(&parent_owner.document_id)
            .filter(|document| document.local_window_id == parent_owner.local_window_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .mark_descendant_incomplete(frame_id.clone())
            });
        if marked && let Some(frame) = self.frames.get_mut(&frame_id) {
            frame.parent_document_load = Some(FrameParentDocumentLoadBinding {
                parent_owner,
                child_frame_id: frame_id,
            });
        }
        released_parent
    }

    pub(crate) fn rebind_active_child_frame_parent_document_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentDescendantLoadCompletion> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?;
        self.frames
            .get(frame_id)
            .and_then(|frame| frame.parent_document_load.as_ref())?;
        self.begin_child_frame_parent_document_load(child_handle)
    }

    pub(crate) fn cancel_child_frame_parent_document_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentDescendantLoadCompletion> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        self.release_parent_document_descendant_load_for_frame(&frame_id)
    }

    fn current_parent_document_task_owner_for_child_frame(
        &self,
        child_frame_id: &FrameId,
    ) -> Option<FrameDocumentTaskOwner> {
        let parent_frame_id = self.frames.get(child_frame_id)?.parent_frame_id.as_ref()?;
        self.current_frame_document_task_owner(parent_frame_id)
    }

    pub(crate) fn current_frame_owner_document_target(
        &self,
        child_handle: DomHandle,
    ) -> Option<super::records::FrameOwnerDocumentTarget> {
        let child_frame_id = self.frame_ids_by_child_handle.get(&child_handle)?;
        let owner = self.current_parent_document_task_owner_for_child_frame(child_frame_id)?;
        let parent_frame_id = self.frames.get(child_frame_id)?.parent_frame_id.as_ref()?;
        let parent_frame = self.frames.get(parent_frame_id)?;
        let parent = match parent_frame.kind {
            FrameKind::Main => FrameDocumentDescendantLoadParent::MainDocument,
            FrameKind::ChildIframe => {
                FrameDocumentDescendantLoadParent::ChildDocument(parent_frame.owner_element_handle?)
            }
        };
        Some(super::records::FrameOwnerDocumentTarget { parent, owner })
    }

    fn release_parent_document_descendant_load_for_frame(
        &mut self,
        frame_id: &FrameId,
    ) -> Option<FrameDocumentDescendantLoadCompletion> {
        let binding = self.frames.get_mut(frame_id)?.parent_document_load.take()?;
        let released = self
            .documents
            .get_mut(&binding.parent_owner.document_id)
            .filter(|document| document.local_window_id == binding.parent_owner.local_window_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .mark_descendant_complete(&binding.child_frame_id)
            });
        if !released {
            return None;
        }
        let parent_frame_id = self
            .scheduler_lanes
            .get(&binding.parent_owner.scheduler_lane_id)
            .map(|lane| lane.frame_id.clone());
        let parent = parent_frame_id.as_ref().and_then(|parent_frame_id| {
            if self.current_frame_document_task_owner(parent_frame_id) != Some(binding.parent_owner)
            {
                return None;
            }
            let frame = self.frames.get(parent_frame_id)?;
            match frame.kind {
                FrameKind::Main => Some(FrameDocumentDescendantLoadParent::MainDocument),
                FrameKind::ChildIframe => frame
                    .owner_element_handle
                    .map(FrameDocumentDescendantLoadParent::ChildDocument),
            }
        })?;
        Some(FrameDocumentDescendantLoadCompletion {
            parent,
            parent_owner: binding.parent_owner,
            child_frame_id: binding.child_frame_id,
        })
    }

    pub(crate) fn acquire_current_child_async_classic_script_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ChildDocumentAsyncClassicScriptLoadDelay> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        let load_delay_token =
            self.acquire_document_load_delay(owner, DocumentLoadDelayReason::AsyncClassicScript);
        if let Some(token) = load_delay_token {
            return Some(ChildDocumentAsyncClassicScriptLoadDelay::Pending(token));
        }
        self.documents
            .get(&owner.document_id)
            .filter(|document| document.local_window_id == owner.local_window_id)
            .is_some_and(|document| document.lifecycle_progress.is_complete())
            .then_some(ChildDocumentAsyncClassicScriptLoadDelay::AlreadyUnblocked)
    }

    pub(crate) fn acquire_current_child_blocking_stylesheet_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<DocumentLoadDelayTokenId> {
        self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::BlockingStylesheet,
        )
    }

    pub(crate) fn acquire_current_child_parser_deferred_script_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<DocumentLoadDelayTokenId> {
        self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::ParserDeferredScript,
        )
    }

    pub(crate) fn acquire_current_main_parser_deferred_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<DocumentLoadDelayTokenId> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.acquire_document_load_delay(owner, DocumentLoadDelayReason::ParserDeferredScript)
    }

    pub(crate) fn acquire_current_main_document_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        kind: MainDocumentScriptLoadDelayKind,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let reason = match kind {
            MainDocumentScriptLoadDelayKind::Classic => DocumentLoadDelayReason::AsyncClassicScript,
            MainDocumentScriptLoadDelayKind::Module => DocumentLoadDelayReason::AsyncModuleScript,
        };
        let load_delay_token = self.acquire_document_load_delay(owner, reason);
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(MainDocumentScriptLoadDelayLease::new(
            owner,
            kind,
            load_delay_token,
        ))
    }

    pub(crate) fn release_main_document_script_load_delay(
        &mut self,
        binding: MainDocumentScriptLoadDelayLease,
    ) -> MainDocumentScriptLoadDelayRelease {
        if !self.main_document_task_owner_is_current(binding.owner()) {
            return MainDocumentScriptLoadDelayRelease::NotOwned;
        }
        let Some(token) = binding.load_delay_token() else {
            return MainDocumentScriptLoadDelayRelease::AlreadyUnblocked;
        };
        let reason = match binding.kind() {
            MainDocumentScriptLoadDelayKind::Classic => DocumentLoadDelayReason::AsyncClassicScript,
            MainDocumentScriptLoadDelayKind::Module => DocumentLoadDelayReason::AsyncModuleScript,
        };
        let Some(document) = self.documents.get_mut(&binding.owner().document_id) else {
            return MainDocumentScriptLoadDelayRelease::NotOwned;
        };
        match document
            .lifecycle_progress
            .release_window_load_delay(token, reason)
        {
            DocumentLoadGateRelease::NotOwned => MainDocumentScriptLoadDelayRelease::NotOwned,
            DocumentLoadGateRelease::StillBlocked => {
                MainDocumentScriptLoadDelayRelease::StillBlocked
            }
            DocumentLoadGateRelease::BecameUnblocked => {
                MainDocumentScriptLoadDelayRelease::BecameUnblocked
            }
        }
    }

    pub(crate) fn accept_current_main_style_load_event(
        &mut self,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let load_delay_token =
            self.acquire_document_load_delay(owner, DocumentLoadDelayReason::StyleLoadEvent);
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(MainDocumentStyleLoadEventBinding::new(
            owner,
            element,
            load_delay_token,
        ))
    }

    pub(crate) fn accept_current_main_modulepreload_event_owner(
        &self,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
    ) -> Option<DocumentLinkEventOwner> {
        self.main_document_task_owner_is_current(owner)
            .then(|| DocumentLinkEventOwner::new(owner, element))
    }

    pub(crate) fn main_style_load_event_is_current(
        &self,
        binding: MainDocumentStyleLoadEventBinding,
    ) -> bool {
        self.main_document_task_owner_is_current(binding.owner())
    }

    pub(crate) fn settle_main_style_load_event(
        &mut self,
        binding: MainDocumentStyleLoadEventBinding,
    ) -> bool {
        if !self.main_style_load_event_is_current(binding) {
            return false;
        }
        let Some(token) = binding.load_delay_token() else {
            return true;
        };
        self.release_document_load_delay(
            binding.owner(),
            token,
            DocumentLoadDelayReason::StyleLoadEvent,
        )
    }

    pub(crate) fn accept_current_main_stylesheet_subresource_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<StylesheetSubresourceLoadDelayBinding> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let load_delay_token =
            self.acquire_document_load_delay(owner, DocumentLoadDelayReason::StylesheetSubresource);
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(StylesheetSubresourceLoadDelayBinding::main(
            owner,
            load_delay_token,
        ))
    }

    pub(crate) fn accept_current_child_stylesheet_subresource_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<StylesheetSubresourceLoadDelayBinding> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        let load_delay_token = self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::StylesheetSubresource,
        );
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(StylesheetSubresourceLoadDelayBinding::child(
            child_handle,
            owner,
            load_delay_token,
        ))
    }

    pub(crate) fn stylesheet_subresource_load_delay_is_current(
        &self,
        binding: StylesheetSubresourceLoadDelayBinding,
    ) -> bool {
        match binding.child_handle() {
            Some(child_handle) => {
                self.child_document_task_owner_is_current(child_handle, binding.owner())
            }
            None => self.main_document_task_owner_is_current(binding.owner()),
        }
    }

    pub(crate) fn settle_stylesheet_subresource_load_delay(
        &mut self,
        binding: StylesheetSubresourceLoadDelayBinding,
    ) -> bool {
        if !self.stylesheet_subresource_load_delay_is_current(binding) {
            return false;
        }
        let Some(token) = binding.load_delay_token() else {
            return true;
        };
        self.release_document_load_delay(
            binding.owner(),
            token,
            DocumentLoadDelayReason::StylesheetSubresource,
        )
    }

    pub(crate) fn accept_current_main_image_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
    ) -> Option<MainDocumentImageLoadDelayBinding> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let load_delay_token =
            self.acquire_document_load_delay(owner, DocumentLoadDelayReason::Image);
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(MainDocumentImageLoadDelayBinding::new(
            owner,
            element,
            load_delay_token,
        ))
    }

    pub(crate) fn main_image_load_delay_is_current(
        &self,
        binding: MainDocumentImageLoadDelayBinding,
    ) -> bool {
        self.main_document_task_owner_is_current(binding.owner())
    }

    pub(crate) fn settle_main_image_load_delay(
        &mut self,
        binding: MainDocumentImageLoadDelayBinding,
    ) -> bool {
        if !self.main_image_load_delay_is_current(binding) {
            return false;
        }
        let Some(token) = binding.load_delay_token() else {
            return true;
        };
        self.release_document_load_delay(binding.owner(), token, DocumentLoadDelayReason::Image)
    }

    pub(crate) fn accept_current_main_media_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
    ) -> Option<MainDocumentMediaLoadDelayBinding> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        let load_delay_token =
            self.acquire_document_load_delay(owner, DocumentLoadDelayReason::Media);
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(MainDocumentMediaLoadDelayBinding::new(
            owner,
            element,
            load_delay_token,
        ))
    }

    pub(crate) fn main_media_load_delay_is_current(
        &self,
        binding: MainDocumentMediaLoadDelayBinding,
    ) -> bool {
        self.main_document_task_owner_is_current(binding.owner())
    }

    pub(crate) fn settle_main_media_load_delay(
        &mut self,
        binding: MainDocumentMediaLoadDelayBinding,
    ) -> bool {
        if !self.main_media_load_delay_is_current(binding) {
            return false;
        }
        let Some(token) = binding.load_delay_token() else {
            return true;
        };
        self.release_document_load_delay(binding.owner(), token, DocumentLoadDelayReason::Media)
    }

    pub(crate) fn current_main_document_has_async_script_load_delay(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents.get(&owner.document_id).map(|document| {
            document
                .lifecycle_progress
                .has_load_delay_reason(DocumentLoadDelayReason::AsyncClassicScript)
                || document
                    .lifecycle_progress
                    .has_load_delay_reason(DocumentLoadDelayReason::AsyncModuleScript)
        })
    }

    #[cfg(test)]
    pub(crate) fn current_main_document_has_style_load_event_delay(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents.get(&owner.document_id).map(|document| {
            document
                .lifecycle_progress
                .has_load_delay_reason(DocumentLoadDelayReason::StyleLoadEvent)
        })
    }

    #[cfg(test)]
    pub(crate) fn current_main_document_has_parser_deferred_script_load_delay(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        if !self.main_document_task_owner_is_current(owner) {
            return None;
        }
        self.documents.get(&owner.document_id).map(|document| {
            document
                .lifecycle_progress
                .has_load_delay_reason(DocumentLoadDelayReason::ParserDeferredScript)
        })
    }

    pub(crate) fn acquire_current_child_async_module_script_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<DocumentLoadDelayTokenId> {
        self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::AsyncModuleScript,
        )
    }

    pub(crate) fn accept_current_child_image_load_event(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
    ) -> Option<FrameDocumentImageLoadEventBinding> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        let load_delay_token = self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::Image,
        );
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(FrameDocumentImageLoadEventBinding::new(
            child_handle,
            owner,
            element,
            load_delay_token,
        ))
    }

    pub(crate) fn child_image_load_event_binding_is_current(
        &self,
        binding: FrameDocumentImageLoadEventBinding,
    ) -> bool {
        self.child_document_task_owner_is_current(binding.child_handle(), binding.owner())
    }

    pub(crate) fn settle_child_image_load_event_binding(
        &mut self,
        binding: FrameDocumentImageLoadEventBinding,
    ) -> bool {
        let Some(token) = binding.load_delay_token() else {
            return self.child_image_load_event_binding_is_current(binding);
        };
        self.release_document_load_delay(binding.owner(), token, DocumentLoadDelayReason::Image)
    }

    pub(crate) fn accept_current_child_media_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
    ) -> Option<FrameDocumentMediaLoadDelayBinding> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        let load_delay_token = self.acquire_current_child_document_load_delay(
            child_handle,
            owner,
            DocumentLoadDelayReason::Media,
        );
        if load_delay_token.is_none()
            && !self
                .documents
                .get(&owner.document_id)
                .filter(|document| document.local_window_id == owner.local_window_id)
                .is_some_and(|document| document.lifecycle_progress.is_complete())
        {
            return None;
        }
        Some(FrameDocumentMediaLoadDelayBinding::new(
            child_handle,
            owner,
            element,
            load_delay_token,
        ))
    }

    pub(crate) fn child_media_load_delay_is_current(
        &self,
        binding: FrameDocumentMediaLoadDelayBinding,
    ) -> bool {
        self.child_document_task_owner_is_current(binding.child_handle(), binding.owner())
    }

    pub(crate) fn settle_child_media_load_delay(
        &mut self,
        binding: FrameDocumentMediaLoadDelayBinding,
    ) -> bool {
        let Some(token) = binding.load_delay_token() else {
            return self.child_media_load_delay_is_current(binding);
        };
        self.release_document_load_delay(binding.owner(), token, DocumentLoadDelayReason::Media)
    }

    pub(crate) fn accept_current_child_modulepreload_link(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        link_handle: DomHandle,
    ) -> Option<FrameDocumentModulepreloadLinkClient> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        Some(FrameDocumentModulepreloadLinkClient::new(
            child_handle,
            owner,
            link_handle,
        ))
    }

    fn acquire_current_child_document_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        reason: DocumentLoadDelayReason,
    ) -> Option<DocumentLoadDelayTokenId> {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return None;
        }
        self.acquire_document_load_delay(owner, reason)
    }

    fn acquire_document_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        reason: DocumentLoadDelayReason,
    ) -> Option<DocumentLoadDelayTokenId> {
        let token = self.ids.document_load_delay_token();
        let acquired = self
            .documents
            .get_mut(&owner.document_id)
            .filter(|document| document.local_window_id == owner.local_window_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .acquire_load_delay(token, reason)
            });
        acquired.then_some(token)
    }

    pub(crate) fn release_async_classic_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.release_document_load_delay(owner, token, DocumentLoadDelayReason::AsyncClassicScript)
    }

    pub(crate) fn release_blocking_stylesheet_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.release_document_load_delay(owner, token, DocumentLoadDelayReason::BlockingStylesheet)
    }

    pub(crate) fn release_parser_deferred_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.release_document_load_delay(
            owner,
            token,
            DocumentLoadDelayReason::ParserDeferredScript,
        )
    }

    pub(crate) fn release_async_module_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.release_document_load_delay(owner, token, DocumentLoadDelayReason::AsyncModuleScript)
    }

    fn release_document_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        self.documents
            .get_mut(&owner.document_id)
            .filter(|document| document.local_window_id == owner.local_window_id)
            .is_some_and(|document| {
                document
                    .lifecycle_progress
                    .release_load_delay(token, reason)
            })
    }

    pub(crate) fn release_all_document_script_load_delays(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> usize {
        self.documents
            .get_mut(&owner.document_id)
            .filter(|document| document.local_window_id == owner.local_window_id)
            .map_or(0, |document| {
                document
                    .lifecycle_progress
                    .release_all_document_script_delays()
            })
    }

    pub(crate) fn current_child_document_allows_deferred_script_execution(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.child_document_task_owner_is_current(child_handle, owner)
            && self
                .documents
                .get(&owner.document_id)
                .is_some_and(|document| {
                    document
                        .lifecycle_progress
                        .allows_deferred_script_execution()
                })
    }

    pub(crate) fn child_document_task_owner_realm_currentness(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentTaskRealmCurrentness {
        if !self.child_document_task_owner_is_current(child_handle, owner) {
            return FrameDocumentTaskRealmCurrentness::StaleOwner;
        }
        self.document_task_owner_realm_currentness(owner, realm_id)
    }

    pub(crate) fn register_current_child_document_import_map(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        source: &str,
        base_url: &Url,
    ) -> Result<(), String> {
        let owner = self
            .current_child_document_owner(child_handle)
            .ok_or_else(|| "child import map has no current Document owner".to_owned())?;
        let document = self
            .documents
            .get_mut(&owner.document_id)
            .filter(|document| {
                document.local_window_id == owner.local_window_id
                    && document.lifecycle == DocumentLifecycleState::Current
                    && document.document_handle == document_handle
            })
            .ok_or_else(|| {
                "child import map no longer belongs to the current Document".to_owned()
            })?;
        document
            .import_map_registry
            .register_import_map(source, base_url)
    }

    pub(crate) fn resolve_frame_document_module_specifier(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        specifier: &str,
        base_url: &Url,
    ) -> Result<Url, String> {
        if !matches!(
            self.frame_document_owner_realm_currentness(owner, realm_id),
            FrameDocumentTaskRealmCurrentness::Current { .. }
        ) {
            return Err(format!(
                "child module specifier `{specifier}` lost its current Document/realm owner"
            ));
        }
        self.documents
            .get_mut(&owner.document_id)
            .filter(|document| document.local_window_id == owner.local_window_id)
            .ok_or_else(|| "child module settings object is unavailable".to_owned())?
            .import_map_registry
            .resolve_module_specifier(specifier, base_url)
    }

    pub(crate) fn resolve_frame_document_module_integrity(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        url: &Url,
    ) -> Option<String> {
        if !matches!(
            self.frame_document_owner_realm_currentness(owner, realm_id),
            FrameDocumentTaskRealmCurrentness::Current { .. }
        ) {
            return None;
        }
        self.documents
            .get(&owner.document_id)
            .filter(|document| document.local_window_id == owner.local_window_id)
            .and_then(|document| document.import_map_registry.resolve_module_integrity(url))
    }

    pub(crate) fn document_task_owner_is_current(&self, owner: FrameDocumentTaskOwner) -> bool {
        let document_owner = owner.document_owner();
        if !self.frame_document_owner_is_current(document_owner) {
            return false;
        }
        let Some(local_window) = self.local_windows.get(&document_owner.local_window_id) else {
            return false;
        };
        self.frames
            .get(&local_window.frame_id)
            .is_some_and(|frame| frame.scheduler_lane_id == owner.scheduler_lane_id)
    }

    pub(crate) fn current_reserved_realm_id_for_document_task_owner(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<FrameRealmId> {
        if !self.document_task_owner_is_current(owner) {
            return None;
        }
        let realm_id = self
            .local_windows
            .get(&owner.local_window_id)
            .and_then(|local_window| local_window.realm_id)?;
        self.realms
            .get(&realm_id)
            .is_some_and(|realm| realm.lifecycle.belongs_to_current_local_window())
            .then_some(realm_id)
    }

    pub(crate) fn current_materialized_realm_id_for_document_task_owner(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<FrameRealmId> {
        let realm_id = self.current_reserved_realm_id_for_document_task_owner(owner)?;
        self.realms
            .get(&realm_id)
            .is_some_and(|realm| realm.lifecycle == FrameRealmLifecycleState::Materialized)
            .then_some(realm_id)
    }

    pub(crate) fn document_task_owner_realm_currentness(
        &self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentTaskRealmCurrentness {
        if !self.document_task_owner_is_current(owner) {
            return FrameDocumentTaskRealmCurrentness::StaleOwner;
        }
        let Some(current_realm_id) = self
            .local_windows
            .get(&owner.local_window_id)
            .and_then(|local_window| local_window.realm_id)
        else {
            return FrameDocumentTaskRealmCurrentness::MissingRealm { owner };
        };
        if current_realm_id != realm_id {
            return FrameDocumentTaskRealmCurrentness::StaleRealm {
                owner,
                current_realm_id,
            };
        }
        let Some(realm) = self.realms.get(&current_realm_id) else {
            return FrameDocumentTaskRealmCurrentness::MissingRealm { owner };
        };
        if matches!(
            realm.lifecycle,
            FrameRealmLifecycleState::Reserved | FrameRealmLifecycleState::MaterializationQueued
        ) {
            return FrameDocumentTaskRealmCurrentness::PendingRealm {
                owner,
                realm_id: current_realm_id,
            };
        }
        if realm.lifecycle != FrameRealmLifecycleState::Materialized {
            return FrameDocumentTaskRealmCurrentness::MissingRealm { owner };
        }
        FrameDocumentTaskRealmCurrentness::Current {
            owner,
            realm_id: current_realm_id,
        }
    }

    pub(crate) fn frame_document_owner_is_current(&self, owner: FrameDocumentOwner) -> bool {
        let Some(document) = self.documents.get(&owner.document_id) else {
            return false;
        };
        if document.local_window_id != owner.local_window_id
            || document.lifecycle != DocumentLifecycleState::Current
        {
            return false;
        }
        let Some(local_window) = self.local_windows.get(&owner.local_window_id) else {
            return false;
        };
        if local_window.document_id != owner.document_id
            || local_window.lifecycle != LocalWindowLifecycleState::Current
        {
            return false;
        }
        let Some(frame) = self.frames.get(&local_window.frame_id) else {
            return false;
        };
        frame.current_local_window_id == Some(owner.local_window_id)
            && frame.current_document_id == Some(owner.document_id)
            && frame.lifecycle == FrameLifecycleState::Attached
    }

    pub(crate) fn begin_reserved_current_child_module_fetch_client(
        &mut self,
        reservation: FrameDocumentModuleClientReservation,
        request_kind: FrameRequestKind,
    ) -> Option<FrameDocumentModuleFetchClientStart> {
        let owner = reservation.owner();
        if !self.frame_document_owner_is_current(owner) {
            return None;
        }
        let request_id = self.begin_document_request(owner.document_id, request_kind)?;
        Some(FrameDocumentModuleFetchClientStart::new(
            owner,
            request_id,
            request_kind,
            reservation.key().clone(),
            reservation.registration(),
        ))
    }

    pub(crate) fn settle_current_document_module_fetch_request(
        &mut self,
        owner: FrameDocumentOwner,
        request_id: FrameRequestId,
        request_kind: FrameRequestKind,
    ) -> bool {
        if !self.document_request_is_current(owner.document_id, request_id, request_kind) {
            return false;
        }
        self.finish_document_request(owner.document_id, request_id)
    }

    pub(crate) fn current_document_task_owner_for_document_owner(
        &self,
        owner: FrameDocumentOwner,
    ) -> Option<FrameDocumentTaskOwner> {
        if !self.frame_document_owner_is_current(owner) {
            return None;
        }
        let local_window = self.local_windows.get(&owner.local_window_id)?;
        let frame = self.frames.get(&local_window.frame_id)?;
        Some(FrameDocumentTaskOwner::new(
            frame.scheduler_lane_id,
            owner.local_window_id,
            owner.document_id,
        ))
    }

    pub(crate) fn current_document_task_owner_for_execution_context(
        &self,
        local_window_id: LocalWindowId,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentTaskOwner> {
        let local_window = self.local_windows.get(&local_window_id)?;
        if local_window.lifecycle != LocalWindowLifecycleState::Current
            || local_window.realm_id != Some(realm_id)
        {
            return None;
        }
        let frame = self.frames.get(&local_window.frame_id)?;
        if frame.lifecycle != FrameLifecycleState::Attached
            || frame.current_local_window_id != Some(local_window_id)
            || frame.current_document_id != Some(local_window.document_id)
        {
            return None;
        }
        let owner = FrameDocumentTaskOwner::new(
            frame.scheduler_lane_id,
            local_window_id,
            local_window.document_id,
        );
        (self.current_materialized_realm_id_for_document_task_owner(owner) == Some(realm_id))
            .then_some(owner)
    }

    pub(crate) fn frame_document_owner_realm_currentness(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentTaskRealmCurrentness {
        let Some(task_owner) = self.current_document_task_owner_for_document_owner(owner) else {
            return FrameDocumentTaskRealmCurrentness::StaleOwner;
        };
        self.document_task_owner_realm_currentness(task_owner, realm_id)
    }

    pub(crate) fn current_frame_lane_task_owner(
        &self,
        frame_id: &FrameId,
    ) -> Option<FrameLaneTaskOwner> {
        let frame = self.frames.get(frame_id)?;
        if frame.lifecycle != FrameLifecycleState::Attached {
            return None;
        }
        let scheduler_lane = self.scheduler_lanes.get(&frame.scheduler_lane_id)?;
        if scheduler_lane.id != frame.scheduler_lane_id
            || scheduler_lane.frame_id != *frame_id
            || scheduler_lane.lifecycle != FrameSchedulerLaneLifecycleState::Active
        {
            return None;
        }
        let proxy = self.window_proxies.get(&frame.window_proxy_id)?;
        if proxy.frame_id != *frame_id || proxy.reachability != WindowProxyReachability::LiveFrame {
            return None;
        }
        Some(FrameLaneTaskOwner::new(frame.scheduler_lane_id))
    }

    pub(crate) fn current_child_frame_lane_task_owner(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameLaneTaskOwner> {
        let frame_id = self.frame_ids_by_child_handle.get(&child_handle)?.clone();
        let owner_element = self.frame_owner_elements.get(&child_handle)?;
        if owner_element.owner_handle != child_handle
            || owner_element.content_frame_id.as_ref() != Some(&frame_id)
            || owner_element.lifecycle != FrameOwnerElementLifecycleState::Attached
        {
            return None;
        }
        let frame = self.frames.get(&frame_id)?;
        if frame.kind != FrameKind::ChildIframe
            || frame.owner_element_handle != Some(child_handle)
            || frame.frame_id != frame_id
        {
            return None;
        }
        self.current_frame_lane_task_owner(&frame_id)
    }

    pub(crate) fn child_frame_lane_task_owner_is_current(
        &self,
        child_handle: DomHandle,
        owner: FrameLaneTaskOwner,
    ) -> bool {
        self.current_child_frame_lane_task_owner(child_handle)
            .is_some_and(|current| current == owner)
    }

    pub(crate) fn current_child_owner_snapshot_for_realm(
        &self,
        realm_id: FrameRealmId,
    ) -> Option<ChildFrameOwnerSnapshot> {
        let owner = self.current_frame_owner_snapshot_for_realm(realm_id)?;
        if owner.kind != FrameKind::ChildIframe {
            return None;
        }
        let child_handle = owner.owner_element_handle?;
        let snapshot = self.current_child_owner_snapshot(child_handle)?;
        (snapshot.frame_id == owner.frame_id
            && snapshot.local_window_id == owner.local_window_id
            && snapshot.document_id == owner.document_id
            && snapshot.realm_id == Some(realm_id))
        .then_some(snapshot)
    }

    pub(crate) fn current_frame_owner_snapshot_for_realm(
        &self,
        realm_id: FrameRealmId,
    ) -> Option<FrameOwnerSnapshot> {
        let realm = self.realms.get(&realm_id)?;
        if realm.lifecycle != FrameRealmLifecycleState::Materialized {
            return None;
        }
        let local_window = self.local_windows.get(&realm.local_window_id)?;
        if local_window.realm_id != Some(realm_id) || local_window.document_id != realm.document_id
        {
            return None;
        }
        let owner = self.current_frame_owner_snapshot(&local_window.frame_id)?;
        (owner.local_window_id == realm.local_window_id
            && owner.document_id == realm.document_id
            && owner.realm_id == Some(realm_id))
        .then_some(owner)
    }

    pub(crate) fn begin_child_frame_request(
        &mut self,
        child_handle: DomHandle,
        kind: FrameRequestKind,
    ) -> Option<(DocumentId, FrameRequestId)> {
        let document_id = self.current_child_owner_snapshot(child_handle)?.document_id;
        let request_id = self.begin_document_request(document_id, kind)?;
        Some((document_id, request_id))
    }

    pub(crate) fn begin_child_document_request(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<(DocumentId, FrameRequestId)> {
        self.begin_child_frame_request(child_handle, FrameRequestKind::DocumentNavigation)
    }

    pub(crate) fn begin_document_request(
        &mut self,
        document_id: DocumentId,
        kind: FrameRequestKind,
    ) -> Option<FrameRequestId> {
        if !self.document_is_current(document_id) {
            return None;
        }
        let request_id = self.ids.frame_request();
        let document = self.documents.get_mut(&document_id)?;
        document.active_requests.insert(
            request_id,
            FrameRequestRecord {
                id: request_id,
                document_id,
                kind,
            },
        );
        Some(request_id)
    }

    pub(crate) fn document_request_is_current(
        &self,
        document_id: DocumentId,
        request_id: FrameRequestId,
        kind: FrameRequestKind,
    ) -> bool {
        let Some(document) = self.documents.get(&document_id) else {
            return false;
        };
        if document.lifecycle != DocumentLifecycleState::Current
            || !self.document_is_current(document_id)
        {
            return false;
        }
        document
            .active_requests
            .get(&request_id)
            .is_some_and(|request| {
                request.id == request_id
                    && request.document_id == document_id
                    && request.kind == kind
            })
    }

    pub(crate) fn finish_document_request(
        &mut self,
        document_id: DocumentId,
        request_id: FrameRequestId,
    ) -> bool {
        self.documents
            .get_mut(&document_id)
            .and_then(|document| document.active_requests.remove(&request_id))
            .is_some()
    }

    pub(crate) fn current_realm_id_for_frame_script_job(
        &self,
        job: &FrameScriptJob,
    ) -> Option<FrameRealmId> {
        let owner = self.current_frame_owner_snapshot(&job.frame_id)?;
        if owner.local_window_id != job.local_window_id || owner.document_id != job.document_id {
            return None;
        }
        self.current_materialized_realm_id_for_document_task_owner(FrameDocumentTaskOwner::new(
            owner.scheduler_lane_id,
            owner.local_window_id,
            owner.document_id,
        ))
    }

    pub(crate) fn frame_script_job_owner_is_current(&self, job: &FrameScriptJob) -> bool {
        self.current_frame_owner_snapshot(&job.frame_id)
            .is_some_and(|owner| {
                owner.local_window_id == job.local_window_id && owner.document_id == job.document_id
            })
    }

    pub(crate) fn child_handle_for_frame_script_job(
        &self,
        job: &FrameScriptJob,
    ) -> Option<DomHandle> {
        if !self.frame_script_job_owner_is_current(job) {
            return None;
        }
        let frame = self.frames.get(&job.frame_id)?;
        if frame.kind != FrameKind::ChildIframe {
            return None;
        }
        let child_handle = frame.owner_element_handle?;
        let owner_element = self.frame_owner_elements.get(&child_handle)?;
        (self.frame_ids_by_child_handle.get(&child_handle) == Some(&job.frame_id)
            && owner_element.content_frame_id.as_ref() == Some(&job.frame_id)
            && owner_element.lifecycle == FrameOwnerElementLifecycleState::Attached)
            .then_some(child_handle)
    }

    pub(crate) fn frame_source_script_job(
        &self,
        frame_id: &FrameId,
        kind: FrameScriptJobKind,
        source: String,
    ) -> Option<FrameScriptJob> {
        self.frame_script_job(
            frame_id,
            kind,
            None,
            None,
            None,
            None,
            FrameScriptSource::SourceText(source),
        )
    }

    pub(crate) fn frame_classic_source_script_job(
        &self,
        frame_id: &FrameId,
        kind: FrameScriptJobKind,
        current_script: Option<DomHandle>,
        script_url: Url,
        script_base_url: Url,
        script_nonce: Option<String>,
        source: String,
    ) -> Option<FrameScriptJob> {
        if !matches!(
            kind,
            FrameScriptJobKind::ParserClassic
                | FrameScriptJobKind::ExternalClassic
                | FrameScriptJobKind::DynamicClassic
        ) {
            return None;
        }
        self.frame_script_job(
            frame_id,
            kind,
            current_script,
            Some(script_url),
            Some(script_base_url),
            script_nonce,
            FrameScriptSource::SourceText(source),
        )
    }

    #[cfg(test)]
    pub(crate) fn frame_function_constructor_script_job(
        &self,
        frame_id: &FrameId,
        parameters: Vec<String>,
        body: String,
    ) -> Option<FrameScriptJob> {
        self.frame_script_job(
            frame_id,
            FrameScriptJobKind::FunctionConstructor,
            None,
            None,
            None,
            None,
            FrameScriptSource::FunctionConstructor(FrameFunctionConstructorSource {
                parameters,
                body,
            }),
        )
    }

    pub(crate) fn child_source_script_job(
        &self,
        child_handle: DomHandle,
        kind: FrameScriptJobKind,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        self.frame_source_script_job(&owner.frame_id, kind, source)
    }

    pub(crate) fn child_parser_classic_script_job(
        &self,
        child_handle: DomHandle,
        current_script: Option<DomHandle>,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        self.frame_classic_source_script_job(
            &owner.frame_id,
            FrameScriptJobKind::ParserClassic,
            current_script,
            owner.document_url,
            owner.document_base_url,
            None,
            source,
        )
    }

    #[cfg(test)]
    pub(crate) fn child_parser_classic_script_job_for_owner(
        &self,
        child_handle: DomHandle,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
        current_script: Option<DomHandle>,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        if owner.local_window_id != local_window_id || owner.document_id != document_id {
            return None;
        }
        self.frame_classic_source_script_job_for_owner(
            &owner.frame_id,
            local_window_id,
            document_id,
            FrameScriptJobKind::ParserClassic,
            current_script,
            owner.document_url,
            owner.document_base_url,
            None,
            source,
        )
    }

    pub(crate) fn child_prepared_classic_script_job_for_owner(
        &self,
        child_handle: DomHandle,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
        kind: FrameScriptJobKind,
        current_script: Option<DomHandle>,
        script_url: Url,
        script_base_url: Url,
        script_nonce: Option<String>,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        if owner.local_window_id != local_window_id || owner.document_id != document_id {
            return None;
        }
        self.frame_classic_source_script_job_for_owner(
            &owner.frame_id,
            local_window_id,
            document_id,
            kind,
            current_script,
            script_url,
            script_base_url,
            script_nonce,
            source,
        )
    }

    #[cfg(test)]
    pub(crate) fn child_external_classic_script_job(
        &self,
        child_handle: DomHandle,
        current_script: Option<DomHandle>,
        script_url: Url,
        script_base_url: Url,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        self.frame_classic_source_script_job(
            &owner.frame_id,
            FrameScriptJobKind::ExternalClassic,
            current_script,
            script_url,
            script_base_url,
            None,
            source,
        )
    }

    pub(crate) fn child_external_classic_script_job_for_owner(
        &self,
        child_handle: DomHandle,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
        current_script: Option<DomHandle>,
        script_url: Url,
        script_base_url: Url,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        if owner.local_window_id != local_window_id || owner.document_id != document_id {
            return None;
        }
        self.frame_classic_source_script_job_for_owner(
            &owner.frame_id,
            local_window_id,
            document_id,
            FrameScriptJobKind::ExternalClassic,
            current_script,
            script_url,
            script_base_url,
            None,
            source,
        )
    }

    pub(crate) fn child_dynamic_classic_script_job_for_owner(
        &self,
        child_handle: DomHandle,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
        current_script: Option<DomHandle>,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        if owner.local_window_id != local_window_id || owner.document_id != document_id {
            return None;
        }
        self.frame_classic_source_script_job_for_owner(
            &owner.frame_id,
            local_window_id,
            document_id,
            FrameScriptJobKind::DynamicClassic,
            current_script,
            owner.document_url,
            owner.document_base_url,
            None,
            source,
        )
    }

    pub(crate) fn child_javascript_url_script_job_for_owner(
        &self,
        child_handle: DomHandle,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
        script_url: Url,
        source: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        if owner.local_window_id != local_window_id || owner.document_id != document_id {
            return None;
        }
        self.frame_script_job(
            &owner.frame_id,
            FrameScriptJobKind::JavascriptUrl,
            None,
            Some(script_url),
            Some(owner.document_base_url),
            None,
            FrameScriptSource::SourceText(source),
        )
    }

    #[cfg(test)]
    pub(crate) fn child_function_constructor_script_job(
        &self,
        child_handle: DomHandle,
        parameters: Vec<String>,
        body: String,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_child_owner_snapshot(child_handle)?;
        self.frame_function_constructor_script_job(&owner.frame_id, parameters, body)
    }

    fn frame_script_job(
        &self,
        frame_id: &FrameId,
        kind: FrameScriptJobKind,
        current_script: Option<DomHandle>,
        script_url: Option<Url>,
        script_base_url: Option<Url>,
        script_nonce: Option<String>,
        source: FrameScriptSource,
    ) -> Option<FrameScriptJob> {
        let owner = self.current_frame_owner_snapshot(frame_id)?;
        Some(FrameScriptJob {
            frame_id: owner.frame_id,
            local_window_id: owner.local_window_id,
            document_id: owner.document_id,
            current_script,
            kind,
            source,
            script_url: script_url.unwrap_or_else(|| owner.document_url.clone()),
            base_url: script_base_url.unwrap_or(owner.document_base_url),
            script_nonce,
            script_integrity: None,
            #[cfg(test)]
            credentials_mode: owner.settings.credentials_mode,
            referrer_policy: owner.settings.referrer_policy,
        })
    }

    fn frame_classic_source_script_job_for_owner(
        &self,
        frame_id: &FrameId,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
        kind: FrameScriptJobKind,
        current_script: Option<DomHandle>,
        script_url: Url,
        script_base_url: Url,
        script_nonce: Option<String>,
        source: String,
    ) -> Option<FrameScriptJob> {
        if !matches!(
            kind,
            FrameScriptJobKind::ParserClassic
                | FrameScriptJobKind::ExternalClassic
                | FrameScriptJobKind::DynamicClassic
        ) {
            return None;
        }
        let owner = self.current_frame_owner_snapshot(frame_id)?;
        if owner.local_window_id != local_window_id || owner.document_id != document_id {
            return None;
        }
        Some(FrameScriptJob {
            frame_id: owner.frame_id,
            local_window_id,
            document_id,
            current_script,
            kind,
            source: FrameScriptSource::SourceText(source),
            script_url,
            base_url: script_base_url,
            script_nonce,
            script_integrity: None,
            #[cfg(test)]
            credentials_mode: owner.settings.credentials_mode,
            referrer_policy: owner.settings.referrer_policy,
        })
    }

    pub(crate) fn document_is_current(&self, document_id: DocumentId) -> bool {
        let Some(document) = self.documents.get(&document_id) else {
            return false;
        };
        let Some(frame) = self
            .frames
            .get(&document_frame_id(document, &self.local_windows))
        else {
            return false;
        };
        frame.current_document_id == Some(document_id)
    }

    fn detach_current_frame_records(&mut self, frame_id: &FrameId) {
        let Some(frame) = self.frames.get_mut(frame_id) else {
            return;
        };
        frame.navigation_load = None;
        frame.lifecycle = FrameLifecycleState::Detached;
        if let Some(lane) = self.scheduler_lanes.get_mut(&frame.scheduler_lane_id) {
            lane.lifecycle = FrameSchedulerLaneLifecycleState::Detached;
        }
        if let Some(local_window_id) = frame.current_local_window_id.take()
            && let Some(local_window) = self.local_windows.get_mut(&local_window_id)
        {
            local_window.lifecycle = LocalWindowLifecycleState::DetachedReachable;
            if let Some(realm_id) = local_window.realm_id
                && let Some(realm) = self.realms.get_mut(&realm_id)
            {
                realm.lifecycle = FrameRealmLifecycleState::DetachedReachable;
            }
        }
        if let Some(document_id) = frame.current_document_id.take()
            && let Some(document) = self.documents.get_mut(&document_id)
        {
            document.lifecycle = DocumentLifecycleState::Detached;
            document.lifecycle_progress.retire();
            document.active_requests.clear();
        }
        if let Some(proxy) = self.window_proxies.get_mut(&frame.window_proxy_id) {
            proxy.current_local_window_id = None;
            proxy.reachability = WindowProxyReachability::DetachedReachable;
        }
    }
}

fn main_frame_id() -> FrameId {
    FrameId("main".to_owned())
}

fn main_window_proxy_id() -> WindowProxyId {
    WindowProxyId(0)
}

fn main_scheduler_lane_id() -> FrameSchedulerLaneId {
    FrameSchedulerLaneId(0)
}

fn main_local_window_id() -> LocalWindowId {
    LocalWindowId(0)
}

fn main_document_id() -> DocumentId {
    DocumentId(0)
}

fn main_frame_realm_id() -> FrameRealmId {
    FrameRealmId(0)
}

fn document_frame_id(
    document: &DocumentRecord,
    local_windows: &BTreeMap<LocalWindowId, LocalWindowRecord>,
) -> FrameId {
    local_windows
        .get(&document.local_window_id)
        .map(|window| window.frame_id.clone())
        .unwrap_or_else(|| FrameId("<missing-frame>".to_owned()))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;

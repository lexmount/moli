use super::*;

impl PagePreparedOutputs {
    pub(crate) fn from_renderer_javascript_dialog(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        dialog: moli_core::page::RendererPendingJavaScriptDialog,
    ) -> Self {
        let Some(source_attachment) =
            conn.target_page_protocol_attachment_identity_for_owner(owner)
        else {
            let _ = dialog.finish(false, String::new());
            return Self::default();
        };
        let Some((root_frame_id, _, _, _)) =
            conn.target_session_owner_frame_tree_identity_for_owner(owner)
        else {
            let _ = dialog.finish(false, String::new());
            return Self::default();
        };
        let Ok(runtime_slot) = conn.runtime_session_owner_slot_for_owner(owner) else {
            let _ = dialog.finish(false, String::new());
            return Self::default();
        };
        Self {
            javascript_dialogs: vec![crate::conn::TargetPreparedJavaScriptDialog::capture(
                source_attachment,
                runtime_slot.javascript_dialog_scope_observer(),
                &root_frame_id,
                dialog,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_popup_activation(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        activation: moli_core::page::RendererPendingPopupActivation,
    ) -> Self {
        let Some(page_owner) = conn.target_page_residence_identity_for_owner(owner) else {
            return Self::default();
        };
        Self {
            popup_activations: vec![popup::PagePreparedPopupActivation::new(
                page_owner, activation,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_window_open_event(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        event: RendererPendingWindowOpenEvent,
    ) -> Self {
        Self {
            window_open_events: vec![popup::PagePreparedWindowOpenEvent::new(
                conn.subscribed_page_event_session_ids_for_owner(owner),
                event,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_session_history_update(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        source_document: RendererDocumentLifecycleIdentity,
        update: moli_page_types::SessionHistoryUpdate,
    ) -> Self {
        let Some(residence) = conn.target_page_residence_identity_for_owner(owner) else {
            return Self::default();
        };
        Self {
            session_history_updates: vec![(residence, source_document, update)],
            ..Self::default()
        }
    }

    pub(in crate::domains) fn append_to_session_history_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.session_history_updates.is_empty() {
            sink.push_produced_slot(ProtocolOutputSlot::SessionHistoryUpdate);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(crate) fn from_renderer_same_document_navigation(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        navigation: RendererDocumentSourcedSameDocumentNavigation,
    ) -> Self {
        let Some(page_owner) = conn.target_page_residence_identity_for_owner(owner) else {
            return Self::default();
        };
        Self {
            same_document_navigations: vec![PagePreparedSameDocumentNavigation::new(
                page_owner, navigation,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_top_level_location_navigation(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        navigation: RendererDocumentSourcedTopLevelLocationNavigation,
    ) -> Self {
        let Some(page_owner) = conn.target_page_residence_identity_for_owner(owner) else {
            return Self::default();
        };
        Self {
            top_level_location_navigation: Some(PagePreparedTopLevelLocationNavigation::new(
                page_owner, navigation,
            )),
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_top_level_history_traversal(
        traversal: RendererPendingTopLevelHistoryTraversal,
    ) -> Self {
        Self {
            top_level_history_traversal: Some(traversal),
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_tree_event(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameTreeEventSnapshot,
    ) -> Self {
        let Some(binding) = conn
            .target_root_document_protocol_attachment_identity_for_owner(owner, source_document)
        else {
            return Self::default();
        };
        let Some((root_frame_id, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity_for_owner(owner)
        else {
            return Self::default();
        };
        let event = match event {
            ChildFrameTreeEventSnapshot::Attached(attachment) => {
                PagePreparedChildFrameTreeEvent::Attached {
                    frame_id: attachment.frame_id,
                    parent_frame_id: attachment.parent_frame_id.unwrap_or(root_frame_id),
                }
            }
            ChildFrameTreeEventSnapshot::Detached(detachment) => {
                PagePreparedChildFrameTreeEvent::Detached {
                    frame_id: detachment.frame_id,
                }
            }
        };
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            vec![event],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_document_opened(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        source_document: RendererDocumentLifecycleIdentity,
        mut event: ChildFrameDocumentOpenedSnapshot,
    ) -> Self {
        let Some(binding) = conn
            .target_root_document_protocol_attachment_identity_for_owner(owner, source_document)
        else {
            return Self::default();
        };
        let Some((root_frame_id, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity_for_owner(owner)
        else {
            return Self::default();
        };
        if event.parent_frame_id.is_none() {
            event.parent_frame_id = Some(root_frame_id);
        }
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            Vec::new(),
            vec![event],
            Vec::new(),
            Vec::new(),
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_document_network(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameDocumentNetworkActivitySnapshot,
    ) -> Self {
        let Some(binding) = conn
            .target_root_document_protocol_attachment_identity_for_owner(owner, source_document)
        else {
            return Self::default();
        };
        let Some((_, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity_for_owner(owner)
        else {
            return Self::default();
        };
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            Vec::new(),
            Vec::new(),
            vec![event],
            Vec::new(),
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(crate) fn from_renderer_child_frame_load(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        source_document: RendererDocumentLifecycleIdentity,
        mut event: ChildFrameNavigationSnapshot,
    ) -> Self {
        let Some(binding) = conn
            .target_root_document_protocol_attachment_identity_for_owner(owner, source_document)
        else {
            return Self::default();
        };
        let Some((root_frame_id, _, security_origin, secure_context_type)) =
            conn.target_session_owner_frame_tree_identity_for_owner(owner)
        else {
            return Self::default();
        };
        if event.parent_frame_id.is_none() {
            event.parent_frame_id = Some(root_frame_id);
        }
        let document = PagePreparedChildFrameDocumentActivity::from_parts(
            monotonic_timestamp_seconds(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![event],
            security_origin,
            secure_context_type,
        );
        Self {
            child_frame_activities: vec![PagePreparedChildFrameActivity::from_document(
                binding, document,
            )],
            ..Self::default()
        }
    }

    pub(super) fn has_child_frame_activity(&self) -> bool {
        !self.child_frame_activities.is_empty()
    }

    pub(super) fn push_child_frame_activity(&mut self, activity: PagePreparedChildFrameActivity) {
        self.child_frame_activities.push(activity);
    }

    pub(crate) fn from_renderer_document_lifecycle_event(
        event: moli_core::page::RendererDocumentLifecycleEvent,
    ) -> Self {
        Self {
            document_lifecycle_events: vec![event],
            ..Default::default()
        }
    }

    pub(crate) fn from_renderer_document_title_change(
        change: RendererDocumentTitleChanged,
    ) -> Self {
        Self {
            document_title_changes: vec![change],
            ..Default::default()
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.javascript_dialogs.extend(other.javascript_dialogs);
        self.window_open_events.extend(other.window_open_events);
        self.popup_activations.extend(other.popup_activations);
        self.document_title_changes
            .extend(other.document_title_changes);
        self.document_lifecycle_events
            .extend(other.document_lifecycle_events);
        for activity in other.child_frame_activities {
            self.push_child_frame_activity(activity);
        }
        self.same_document_navigations
            .extend(other.same_document_navigations);
        if self.top_level_location_navigation.is_none() {
            self.top_level_location_navigation = other.top_level_location_navigation;
        }
        if self.top_level_history_traversal.is_none() {
            self.top_level_history_traversal = other.top_level_history_traversal;
        }
    }

    pub(in crate::domains) fn append_to_javascript_dialog_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.javascript_dialogs.is_empty() {
            sink.push_produced_slot(SLOT_JAVASCRIPT_DIALOG);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_popup_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.popup_activations.is_empty() {
            sink.push_produced_slot(SLOT_POPUP);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_window_open_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.window_open_events.is_empty() {
            sink.push_produced_slot(SLOT_WINDOW_OPEN);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_document_lifecycle_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.document_lifecycle_events.is_empty() {
            sink.push_produced_slot(SLOT_DOCUMENT_LIFECYCLE);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_document_title_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.document_title_changes.is_empty() {
            sink.push_produced_slot(SLOT_DOCUMENT_TITLE_CHANGED);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_child_frame_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.has_child_frame_activity() {
            sink.push_produced_slot(SLOT_CHILD_FRAME_ACTIVITY);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_same_document_navigation_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.same_document_navigations.is_empty() {
            sink.push_produced_slot(SLOT_SAME_DOCUMENT_NAVIGATION);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_top_level_location_navigation_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.top_level_location_navigation.is_some() {
            sink.push_produced_slot(SLOT_TOP_LEVEL_LOCATION_NAVIGATION);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(in crate::domains) fn append_to_top_level_history_traversal_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if self.top_level_history_traversal.is_some() {
            sink.push_produced_slot(SLOT_TOP_LEVEL_HISTORY_TRAVERSAL);
            sink.push_prepared_payload(PagePreparedOutputSlot::from_outputs(self).into());
        }
    }

    #[cfg(test)]
    pub(crate) fn from_javascript_dialogs_for_test(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        source_session_id: Option<&str>,
        dialog_scope: crate::conn::TargetJavaScriptDialogScopeObserver,
        root_frame_id: &str,
        dialogs: Vec<moli_core::page::RendererPendingJavaScriptDialog>,
    ) -> Self {
        Self {
            javascript_dialogs: dialogs
                .into_iter()
                .map(|dialog| {
                    javascript_dialog::capture_for_test(
                        page_owner.clone(),
                        source_session_id,
                        dialog_scope.clone(),
                        root_frame_id,
                        dialog,
                    )
                })
                .collect(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: Vec::new(),
            session_history_updates: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_popup_activations_for_test(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        activations: Vec<moli_core::page::RendererPendingPopupActivation>,
    ) -> Self {
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: activations
                .into_iter()
                .map(|activation| {
                    popup::PagePreparedPopupActivation::from_renderer_for_test(
                        page_owner.clone(),
                        activation,
                    )
                })
                .collect(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: Vec::new(),
            session_history_updates: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(super) fn child_frame_document_activity_for_test() -> PagePreparedChildFrameDocumentActivity
    {
        PagePreparedChildFrameDocumentActivity::from_parts(
            12.5,
            vec![PagePreparedChildFrameTreeEvent::Attached {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: "TID-1".to_owned(),
            }],
            Vec::new(),
            Vec::new(),
            vec![ChildFrameNavigationSnapshot {
                frame_id: "CHILD-FRAME-1".to_owned(),
                parent_frame_id: None,
                loader_id: Some("LOADER-CHILD-FRAME-1".to_owned()),
                name: Some("child-frame".to_owned()),
                url: "https://example.test/child".to_owned(),
                document_open_replacement: false,
                security_origin_inherited: false,
                security_origin_opaque: false,
                document_network: None,
            }],
            "https://example.test".to_owned(),
            "Secure".to_owned(),
        )
    }

    #[cfg(test)]
    pub(crate) fn from_child_frame_activity_for_test(
        binding: crate::conn::TargetRootDocumentProtocolAttachmentIdentity,
    ) -> Self {
        let activity = PagePreparedChildFrameActivity::from_document(
            binding,
            Self::child_frame_document_activity_for_test(),
        );
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: vec![activity],
            same_document_navigations: Vec::new(),
            session_history_updates: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_same_document_navigations_for_test(
        owner: crate::conn::TargetPageResidenceIdentity,
        navigations: Vec<RendererDocumentSourcedSameDocumentNavigation>,
    ) -> Self {
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: navigations
                .into_iter()
                .map(|navigation| {
                    PagePreparedSameDocumentNavigation::new(owner.clone(), navigation)
                })
                .collect(),
            session_history_updates: Vec::new(),
            top_level_location_navigation: None,
            top_level_history_traversal: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_top_level_location_navigation_for_test(
        owner: crate::conn::TargetPageResidenceIdentity,
        navigation: Option<RendererDocumentSourcedTopLevelLocationNavigation>,
    ) -> Self {
        Self {
            javascript_dialogs: Vec::new(),
            window_open_events: Vec::new(),
            popup_activations: Vec::new(),
            document_title_changes: Vec::new(),
            document_lifecycle_events: Vec::new(),
            child_frame_activities: Vec::new(),
            same_document_navigations: Vec::new(),
            session_history_updates: Vec::new(),
            top_level_location_navigation: navigation
                .map(|navigation| PagePreparedTopLevelLocationNavigation::new(owner, navigation)),
            top_level_history_traversal: None,
        }
    }
}

impl PagePreparedOutputSlot {
    pub(crate) fn from_outputs(outputs: PagePreparedOutputs) -> Self {
        Self { outputs }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    pub(super) fn take_javascript_dialogs(
        &mut self,
    ) -> Option<Vec<javascript_dialog::PreparedJavaScriptDialog>> {
        (!self.outputs.javascript_dialogs.is_empty())
            .then(|| std::mem::take(&mut self.outputs.javascript_dialogs))
    }

    pub(crate) fn take_popup_activations(
        &mut self,
    ) -> Option<Vec<popup::PagePreparedPopupActivation>> {
        (!self.outputs.popup_activations.is_empty())
            .then(|| std::mem::take(&mut self.outputs.popup_activations))
    }

    pub(crate) fn take_window_open_events(
        &mut self,
    ) -> Option<Vec<popup::PagePreparedWindowOpenEvent>> {
        (!self.outputs.window_open_events.is_empty())
            .then(|| std::mem::take(&mut self.outputs.window_open_events))
    }

    pub(crate) fn take_document_lifecycle_events(
        &mut self,
    ) -> Option<Vec<RendererDocumentLifecycleEvent>> {
        (!self.outputs.document_lifecycle_events.is_empty())
            .then(|| std::mem::take(&mut self.outputs.document_lifecycle_events))
    }

    pub(super) fn take_document_title_changes(
        &mut self,
    ) -> Option<Vec<RendererDocumentTitleChanged>> {
        (!self.outputs.document_title_changes.is_empty())
            .then(|| std::mem::take(&mut self.outputs.document_title_changes))
    }

    pub(crate) fn take_child_frame_activity(
        &mut self,
    ) -> Option<Vec<PagePreparedChildFrameActivity>> {
        (!self.outputs.child_frame_activities.is_empty())
            .then(|| std::mem::take(&mut self.outputs.child_frame_activities))
    }

    pub(super) fn take_same_document_navigations(
        &mut self,
    ) -> Option<Vec<PagePreparedSameDocumentNavigation>> {
        (!self.outputs.same_document_navigations.is_empty())
            .then(|| std::mem::take(&mut self.outputs.same_document_navigations))
    }

    pub(super) fn take_top_level_location_navigation(
        &mut self,
    ) -> Option<PagePreparedTopLevelLocationNavigation> {
        self.outputs.top_level_location_navigation.take()
    }

    pub(in crate::domains) fn top_level_location_navigation_runtime_command_cause(
        &self,
    ) -> Option<&RendererRuntimeCommandCausalIdentity> {
        self.outputs
            .top_level_location_navigation
            .as_ref()
            .and_then(PagePreparedTopLevelLocationNavigation::runtime_command_cause)
    }

    pub(in crate::domains) fn take_top_level_location_navigation_for_runtime_command(
        &mut self,
        cause: &RendererRuntimeCommandCausalIdentity,
    ) -> Option<Self> {
        let navigation = (self.top_level_location_navigation_runtime_command_cause()
            == Some(cause))
        .then(|| self.outputs.top_level_location_navigation.take())
        .flatten()?;
        Some(Self {
            outputs: PagePreparedOutputs {
                top_level_location_navigation: Some(navigation),
                ..Default::default()
            },
        })
    }

    pub(crate) fn take_top_level_history_traversal(
        &mut self,
    ) -> Option<RendererPendingTopLevelHistoryTraversal> {
        self.outputs.top_level_history_traversal.take()
    }
}

impl PageOutputProjectionStep {
    pub(super) async fn project_async(
        self,
        conn: &mut CdpConnection,
        context: &mut ProtocolOutputProjectionContext<'_>,
        prepared_outputs: Option<&mut ProtocolOutputPayloads>,
    ) {
        let owner = context.owner().clone();
        match self {
            PageOutputProjectionStep::Download => {
                let mut events = Vec::new();
                input::emit_download_activity_background_events_async(
                    conn,
                    &mut events,
                    &owner,
                    prepared_outputs,
                    context.command,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::DocumentLifecycle => {
                if let Some(renderer_events) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_document_lifecycle_events)
                {
                    let (binding, accepted) = conn
                        .ingest_renderer_document_lifecycle_events_for_owner(
                            &owner,
                            renderer_events,
                        );
                    if let Some(binding) = binding {
                        let mut events = Vec::new();
                        emit_bound_renderer_document_lifecycle_background_events(
                            conn,
                            &mut events,
                            &owner,
                            &binding,
                            &accepted,
                        );
                        context.command.protocol_events_mut().extend(events);
                    }
                }
            }
            PageOutputProjectionStep::DocumentTitleChanged => {
                if let Some(changes) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_document_title_changes)
                {
                    let mut events = Vec::new();
                    for change in changes {
                        if conn
                            .apply_renderer_document_title_for_owner(&owner, &change)
                            .unwrap_or(false)
                        {
                            crate::domains::target::emit_target_info_changed_for_owner_background_event(
                                conn,
                                &mut events,
                                &owner,
                            );
                        }
                    }
                    context.command.protocol_events_mut().extend(events);
                }
            }
            PageOutputProjectionStep::FileChooser => {
                let mut events = Vec::new();
                input::emit_file_chooser_activity_background_events_async(
                    conn,
                    &mut events,
                    &owner,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::JavascriptDialog => {
                let mut events = Vec::new();
                emit_javascript_dialog_activity_background_events_async(
                    conn,
                    &mut events,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::WindowOpen => {
                if let Some(events) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_window_open_events)
                {
                    let mut protocol_events = Vec::new();
                    popup::emit_window_open_events(&mut protocol_events, events);
                    context
                        .command
                        .protocol_events_mut()
                        .extend(protocol_events);
                }
            }
            PageOutputProjectionStep::Popup => {
                let mut events = Vec::new();
                emit_popup_activity_background_events_async(conn, &mut events, prepared_outputs)
                    .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::ChildFrameActivity => {
                if let Some(activities) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::page_mut)
                    .and_then(PagePreparedOutputSlot::take_child_frame_activity)
                {
                    let mut events = Vec::new();
                    for activity in activities {
                        emit_prepared_child_frame_activity(conn, &mut events, activity, None).await;
                    }
                    context.command.protocol_events_mut().extend(events);
                }
            }
            PageOutputProjectionStep::SessionHistoryUpdate => {
                if let Some(slot) = prepared_outputs.and_then(ProtocolOutputPayloads::page_mut) {
                    for (residence, _source_document, update) in
                        std::mem::take(&mut slot.outputs.session_history_updates)
                    {
                        // document.open() does not undo committed history. Page
                        // residence, not current Document identity, is authority.
                        if conn.target_page_residence_identity_is_current(&residence) {
                            conn.record_session_history_update_for_owner(&owner, &update);
                        }
                    }
                }
            }
            PageOutputProjectionStep::SameDocumentNavigation => {
                let mut events = Vec::new();
                emit_same_document_navigation_activity_background_events_async(
                    conn,
                    &mut events,
                    &owner,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
            PageOutputProjectionStep::TopLevelLocationNavigation => {
                publish_prepared_top_level_location_navigation_owner_action(
                    conn,
                    &owner,
                    prepared_outputs,
                );
            }
            PageOutputProjectionStep::TopLevelHistoryTraversal => {
                let mut events = Vec::new();
                emit_top_level_history_traversal_activity_background_events_async(
                    conn,
                    &mut events,
                    &owner,
                    prepared_outputs,
                )
                .await;
                context.command.protocol_events_mut().extend(events);
            }
        }
    }
}

pub(in crate::domains) async fn project_page_output_async(
    output: ProtocolOutputSlot,
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    let step = match output {
        ProtocolOutputSlot::Download => PageOutputProjectionStep::Download,
        ProtocolOutputSlot::FileChooser => PageOutputProjectionStep::FileChooser,
        ProtocolOutputSlot::JavascriptDialog => PageOutputProjectionStep::JavascriptDialog,
        ProtocolOutputSlot::WindowOpen => PageOutputProjectionStep::WindowOpen,
        ProtocolOutputSlot::Popup => PageOutputProjectionStep::Popup,
        ProtocolOutputSlot::DocumentTitleChanged => PageOutputProjectionStep::DocumentTitleChanged,
        ProtocolOutputSlot::DocumentLifecycle => PageOutputProjectionStep::DocumentLifecycle,
        ProtocolOutputSlot::ChildFrameActivity => PageOutputProjectionStep::ChildFrameActivity,
        ProtocolOutputSlot::SessionHistoryUpdate => PageOutputProjectionStep::SessionHistoryUpdate,
        ProtocolOutputSlot::SameDocumentNavigation => {
            PageOutputProjectionStep::SameDocumentNavigation
        }
        ProtocolOutputSlot::TopLevelLocationNavigation => {
            PageOutputProjectionStep::TopLevelLocationNavigation
        }
        ProtocolOutputSlot::TopLevelHistoryTraversal => {
            PageOutputProjectionStep::TopLevelHistoryTraversal
        }
        _ => panic!("non-Page output routed through the Page projector: {output:?}"),
    };
    step.project_async(conn, context, prepared_outputs).await;
}

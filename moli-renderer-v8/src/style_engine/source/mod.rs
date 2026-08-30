use std::collections::HashSet;

pub(super) mod adopted;
pub(super) mod imports;
pub(super) mod inline;
pub(super) mod linked;
mod shared_cache;
pub(super) mod store;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind},
};

use self::adopted::AdoptedStyleSheetInstallation;
use self::store::StyloStylesheetSource;
use super::{
    MoliStyleEngine, source_dirty::StyleSourceDirtyReason, source_id::StyleScopeId,
    source_owner::stylesheet_source_base_url, source_scope_plan::StyleSourceScopeCleanupPlan,
};

impl MoliStyleEngine {
    #[cfg(test)]
    pub(crate) fn inline_style_base_url_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.world_for_document(document)
            .inline_style_metadata
            .len()
    }

    #[cfg(test)]
    pub(crate) fn adopted_style_sheet_source_owner_counts_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> (usize, usize) {
        let world = self.world_for_document(document);
        (
            world
                .adopted_style_sheet_sources
                .borrow()
                .document_source_owner_count(),
            world
                .adopted_style_sheet_sources
                .borrow()
                .shadow_root_source_owner_count(),
        )
    }

    #[cfg(test)]
    pub(crate) fn document_adopted_style_sheet_source_ids_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<super::StyleSourceId> {
        self.world_for_document(document)
            .adopted_style_sheet_sources
            .borrow()
            .document_source_ids(document)
    }

    #[cfg(test)]
    pub(crate) fn shadow_root_adopted_style_sheet_source_ids_for_test(
        &self,
        host: &DomHost,
        root: DomHandle,
    ) -> Vec<super::StyleSourceId> {
        self.owner_document_world(host, root)
            .map(|world| {
                world
                    .adopted_style_sheet_sources
                    .borrow()
                    .shadow_root_source_ids(root)
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn linked_stylesheet_owner_registry_counts_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> (usize, usize) {
        self.world_for_document(document)
            .linked_stylesheet_sources
            .borrow()
            .counts_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_document_adopted_style_sheet_sources(
        &mut self,
        document: DomHandle,
        sources: Vec<StyloStylesheetSource>,
    ) {
        let world = self.world_for_document(document);
        let previous_source_ids = world
            .adopted_style_sheet_sources
            .borrow()
            .document_source_ids(document);
        if world
            .adopted_style_sheet_sources
            .borrow_mut()
            .set_document_sources(document, sources)
        {
            let next_source_ids = world
                .adopted_style_sheet_sources
                .borrow()
                .document_source_ids(document);
            self.invalidate_document_stylesheet_set_for_document(
                document,
                previous_source_ids.into_iter().chain(next_source_ids),
            );
        }
    }

    pub(crate) fn set_document_adopted_style_sheet_installations(
        &mut self,
        document: DomHandle,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) {
        let world = self.world_for_document(document);
        let previous_source_ids = world
            .adopted_style_sheet_sources
            .borrow()
            .document_source_ids(document);
        if world
            .adopted_style_sheet_sources
            .borrow_mut()
            .set_document_installations(document, installations)
        {
            let next_source_ids = world
                .adopted_style_sheet_sources
                .borrow()
                .document_source_ids(document);
            self.invalidate_document_stylesheet_set_for_document(
                document,
                previous_source_ids.into_iter().chain(next_source_ids),
            );
        }
    }

    pub(crate) fn adopted_style_sheet_sources_for_document(
        &self,
        document: DomHandle,
    ) -> Vec<StyloStylesheetSource> {
        self.world_for_document(document)
            .adopted_style_sheet_sources
            .borrow()
            .document_sources_or_empty_owned(document)
    }

    #[cfg(test)]
    pub(crate) fn document_adopted_style_sheet_tracks_document_for_test(
        &self,
        document: DomHandle,
    ) -> bool {
        self.world_for_document(document)
            .adopted_style_sheet_sources
            .borrow()
            .tracks_document(document)
    }

    pub(crate) fn set_owner_style_sheet_text_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        css_text: String,
    ) {
        let parser_base = stylesheet_source_base_url(host, owner);
        self.set_owner_style_sheet_source_with_parser_base(host, owner, css_text, parser_base);
    }

    pub(crate) fn sync_owner_style_sheet_text_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        css_text: String,
    ) {
        if self.owner_document_world(host, owner).is_some_and(|world| {
            world
                .owner_style_sheet_sources
                .borrow()
                .cssom_source_is_authoritative(owner)
        }) {
            return;
        }
        self.set_owner_style_sheet_text_with_host(host, owner, css_text);
    }

    fn process_owner_style_sheet_text_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        css_text: String,
    ) {
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return;
        };
        let previous_linked_documents = self.linked_stylesheet_owner_documents(owner);
        self.remove_linked_stylesheet_owner_from_documents(host, owner, previous_linked_documents);
        let previous_documents = self.move_owner_style_sheet_source_to_document(document, owner);
        let parser_base = stylesheet_source_base_url(host, owner);
        self.world_for_document(document)
            .owner_style_sheet_sources
            .borrow_mut()
            .replace_processed_source(owner, css_text, parser_base);
        self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
        for previous_document in previous_documents {
            self.mark_document_stylesheet_set_dirty(previous_document);
        }
    }

    fn set_owner_style_sheet_source_with_parser_base(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        css_text: String,
        parser_base: url::Url,
    ) {
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return;
        };
        let previous_documents = self.move_owner_style_sheet_source_to_document(document, owner);
        let world = self.world_for_document(document);
        if world
            .owner_style_sheet_sources
            .borrow_mut()
            .set_source(owner, css_text, parser_base)
        {
            let previous_linked_documents = self.linked_stylesheet_owner_documents(owner);
            self.remove_linked_stylesheet_owner_from_documents(
                host,
                owner,
                previous_linked_documents,
            );
            self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
        }
        for previous_document in previous_documents {
            self.mark_document_stylesheet_set_dirty(previous_document);
        }
    }

    pub(crate) fn owner_style_sheet_text_with_host(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<String> {
        self.owner_document_world(host, owner)?
            .owner_style_sheet_sources
            .borrow()
            .text_owned(owner)
    }

    pub(crate) fn owner_style_sheet_source_with_host(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<StyloStylesheetSource> {
        self.owner_document_world(host, owner)?
            .owner_style_sheet_sources
            .borrow()
            .source(owner)
    }

    pub(crate) fn install_owner_live_stylesheet_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        stylesheet: crate::live_stylesheet::LiveStylesheetRef,
    ) -> bool {
        let Some(world) = self.owner_document_world(host, owner) else {
            return false;
        };
        let installed = world
            .owner_style_sheet_sources
            .borrow_mut()
            .install_live_stylesheet(owner, stylesheet);
        if installed {
            self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
        }
        installed
    }

    pub(crate) fn refresh_owner_live_stylesheet_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let Some(world) = self.owner_document_world(host, owner) else {
            return false;
        };
        let refreshed = world
            .owner_style_sheet_sources
            .borrow_mut()
            .refresh_live_stylesheet(owner, stylesheet_id);
        if refreshed {
            self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
        }
        refreshed
    }

    pub(crate) fn mark_owner_live_stylesheet_cssom_authoritative_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
    ) -> bool {
        let Some(world) = self.owner_document_world(host, owner) else {
            return false;
        };
        world
            .owner_style_sheet_sources
            .borrow_mut()
            .mark_cssom_authoritative(owner);
        true
    }

    pub(crate) fn owner_live_stylesheet_with_host(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        self.owner_document_world(host, owner)?
            .owner_style_sheet_sources
            .borrow()
            .live_stylesheet(owner)
    }

    pub(crate) fn owner_style_sheet_processing_source(
        &self,
        owner: DomHandle,
    ) -> Option<std::sync::Arc<super::OwnerStyleSheetSource>> {
        let document = self
            .owner_stylesheet_source_documents
            .borrow()
            .get(&owner)
            .copied()?;
        self.world_for_document(document)
            .owner_style_sheet_sources
            .borrow()
            .processing_source(owner)
    }

    pub(crate) fn set_owner_style_sheet_csp_suppressed_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        suppressed: bool,
    ) -> bool {
        let Some(world) = self.owner_document_world(host, owner) else {
            return false;
        };
        let changed = world
            .owner_style_sheet_sources
            .borrow_mut()
            .set_csp_suppressed(owner, suppressed);
        if changed {
            self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
        }
        changed
    }

    #[cfg(test)]
    pub(crate) fn record_stylesheet_source_for_url_for_document_for_test(
        &mut self,
        document: DomHandle,
        url: &url::Url,
        source: StyloStylesheetSource,
    ) {
        self.world_for_document(document)
            .linked_stylesheet_sources
            .borrow_mut()
            .record_source_for_url(url, source);
    }

    #[cfg(test)]
    pub(crate) fn install_linked_stylesheet_source_for_owners_for_test(
        &mut self,
        host: &DomHost,
        url: &url::Url,
        source: StyloStylesheetSource,
        owners: &[DomHandle],
    ) {
        if owners.is_empty() {
            return;
        }
        for &owner in owners {
            self.install_linked_stylesheet_source_with_host(host, owner, url, source.clone());
        }
    }

    #[cfg(test)]
    pub(crate) fn install_linked_stylesheet_source_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        request_url: &url::Url,
        source: StyloStylesheetSource,
    ) {
        self.install_linked_stylesheet_source_and_live_owner_with_host(
            host,
            owner,
            request_url,
            source,
            None,
        );
    }

    pub(crate) fn install_linked_stylesheet_source_and_live_owner_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        request_url: &url::Url,
        source: StyloStylesheetSource,
        live_stylesheet: Option<crate::live_stylesheet::LiveStylesheetRef>,
    ) {
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return;
        };
        let previous_documents = self.linked_stylesheet_owner_documents(owner);
        for previous_document in previous_documents
            .iter()
            .copied()
            .filter(|previous| *previous != document)
        {
            self.remove_linked_stylesheet_owner_from_document(previous_document, owner);
        }
        let world = self.world_for_document(document);
        let sheet_url = source.sheet_url().clone();
        let origin_clean = source.origin_clean();
        let (binding_changed, source_changed, live_source_changed) = {
            let mut sources = world.linked_stylesheet_sources.borrow_mut();
            let binding_changed = sources.bind_owner(owner, request_url.clone());
            let source_changed = sources.record_source_for_url(request_url, source).changed();
            let live_source_changed =
                sources.set_owner_live_stylesheet(owner, live_stylesheet, &sheet_url, origin_clean);
            (binding_changed, source_changed, live_source_changed)
        };
        self.linked_stylesheet_owner_documents
            .borrow_mut()
            .insert(owner, document);
        if binding_changed
            || source_changed
            || live_source_changed
            || previous_documents != [document]
        {
            self.invalidate_linked_stylesheet_owner_lifecycle_change(
                host,
                owner,
                previous_documents,
                Some(document),
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn install_cached_linked_stylesheet_source_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        request_url: &url::Url,
    ) -> bool {
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return false;
        };
        let Some(source) = self
            .world_for_document(document)
            .linked_stylesheet_sources
            .borrow()
            .source_for_url(request_url)
        else {
            return false;
        };
        self.install_linked_stylesheet_source_with_host(host, owner, request_url, source);
        true
    }

    pub(crate) fn cached_linked_stylesheet_source_with_host(
        &self,
        host: &DomHost,
        owner: DomHandle,
        request_url: &url::Url,
    ) -> Option<StyloStylesheetSource> {
        let document = owner_document_for_source_owner(host, owner)?;
        self.world_for_document(document)
            .linked_stylesheet_sources
            .borrow()
            .source_for_url(request_url)
    }

    #[cfg(test)]
    pub(crate) fn install_recorded_linked_stylesheet_source_for_test(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        request_url: &url::Url,
    ) -> bool {
        self.install_cached_linked_stylesheet_source_with_host(host, owner, request_url)
    }

    pub(crate) fn apply_stylesheet_owner_changes_with_host(
        &mut self,
        host: &DomHost,
        changes: &[DomStylesheetOwnerChange],
    ) {
        for change in changes {
            let owner = change.owner();
            if host.is_inline_style_sheet_owner(owner) {
                let should_sync = match change.kind() {
                    DomStylesheetOwnerChangeKind::Registered
                    | DomStylesheetOwnerChangeKind::Contents => host.is_connected(owner),
                    DomStylesheetOwnerChangeKind::OwnerDocumentChanged => true,
                    DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected } => *connected,
                    DomStylesheetOwnerChangeKind::Unregistered
                    | DomStylesheetOwnerChangeKind::Attribute { .. } => false,
                };
                if should_sync {
                    self.process_owner_style_sheet_text_with_host(
                        host,
                        owner,
                        host.text_content(owner).unwrap_or_default(),
                    );
                } else if matches!(
                    change.kind(),
                    DomStylesheetOwnerChangeKind::Attribute {
                        namespace: None,
                        local_name,
                    } if local_name == "type"
                ) {
                    // The type attribute changes whether the already-parsed
                    // owner source participates in its TreeScope. The source
                    // text and identity stay stable, but retained worlds must
                    // refresh their active stylesheet projection immediately.
                    self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
                } else if matches!(
                    change.kind(),
                    DomStylesheetOwnerChangeKind::Unregistered
                        | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: false }
                ) {
                    self.remove_owner_style_sheet_source(owner);
                    let previous_documents = self.linked_stylesheet_owner_documents(owner);
                    self.remove_linked_stylesheet_owner_from_documents(
                        host,
                        owner,
                        previous_documents,
                    );
                }
                continue;
            }
            if !host.is_html_element_named(owner, "link") {
                continue;
            }
            if matches!(
                change.kind(),
                DomStylesheetOwnerChangeKind::Attribute {
                    namespace: None,
                    local_name,
                } if local_name == "title"
            ) {
                // Title changes sheet-set eligibility, not the fetched resource.
                // Preserve the source binding while invalidating style results.
                let previous_documents = self.linked_stylesheet_owner_documents(owner);
                self.invalidate_linked_stylesheet_owner_lifecycle_change(
                    host,
                    owner,
                    previous_documents,
                    owner_document_for_source_owner(host, owner),
                );
                continue;
            }
            if let DomStylesheetOwnerChangeKind::Attribute {
                namespace,
                local_name,
            } = change.kind()
                && (namespace.is_some()
                    || !crate::document_runtime::attribute_reprocesses_connected_stylesheet(
                        local_name,
                    ))
            {
                continue;
            }
            let previous_documents = self.linked_stylesheet_owner_documents(owner);
            self.remove_linked_stylesheet_owner_from_documents(host, owner, previous_documents);
        }
    }

    #[cfg(test)]
    pub(crate) fn stylesheet_text_for_url_for_document_for_test(
        &self,
        document: DomHandle,
        url: &url::Url,
    ) -> Option<String> {
        self.stylesheet_source_for_url_for_document_for_test(document, url)
            .map(|source| source.serialized_css_text().to_string())
    }

    #[cfg(test)]
    pub(crate) fn stylesheet_source_for_url_for_document_for_test(
        &self,
        document: DomHandle,
        url: &url::Url,
    ) -> Option<StyloStylesheetSource> {
        self.world_for_document(document)
            .linked_stylesheet_sources
            .borrow()
            .source_for_url(url)
    }

    pub(crate) fn linked_stylesheet_source_for_owner_with_host(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<StyloStylesheetSource> {
        self.owner_document_world(host, owner)?
            .linked_stylesheet_sources
            .borrow()
            .source_for_owner(host, owner)
    }

    pub(crate) fn linked_live_stylesheet_with_host(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        self.owner_document_world(host, owner)?
            .linked_stylesheet_sources
            .borrow()
            .live_stylesheet(owner)
    }

    pub(crate) fn refresh_linked_live_stylesheet_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return false;
        };
        let refreshed = self
            .world_for_document(document)
            .linked_stylesheet_sources
            .borrow_mut()
            .refresh_live_stylesheet(owner, stylesheet_id);
        if refreshed {
            self.invalidate_linked_stylesheet_owner_lifecycle_change(
                host,
                owner,
                vec![document],
                Some(document),
            );
        }
        refreshed
    }

    pub(crate) fn ensure_inline_style_base_url_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
        base_url: url::Url,
    ) {
        let Some(document) = owner_document_for_source_owner(host, handle) else {
            return;
        };
        self.move_inline_style_metadata_to_document(handle, document);
        let world = self.world_for_document(document);
        world
            .inline_style_metadata
            .ensure_base_url(handle, base_url);
        self.inline_style_metadata_documents
            .borrow_mut()
            .insert(handle, document);
    }

    pub(crate) fn set_inline_style_base_url_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
        base_url: url::Url,
    ) {
        let Some(document) = owner_document_for_source_owner(host, handle) else {
            return;
        };
        self.move_inline_style_metadata_to_document(handle, document);
        self.world_for_document(document)
            .inline_style_metadata
            .set_base_url(handle, base_url);
        self.inline_style_metadata_documents
            .borrow_mut()
            .insert(handle, document);
    }

    pub(crate) fn inline_style_base_url_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) -> Option<url::Url> {
        self.owner_document_world(host, handle)?
            .inline_style_metadata
            .base_url(handle)
    }

    pub(crate) fn clear_inline_style_base_url_with_host(&self, host: &DomHost, handle: DomHandle) {
        let indexed_document = self
            .inline_style_metadata_documents
            .borrow()
            .get(&handle)
            .copied();
        if let Some(document) = indexed_document {
            let world = self.world_for_document(document);
            world.inline_style_metadata.clear_base_url(handle);
            self.remove_inline_style_metadata_document_if_empty(document, handle);
        }
        if let Some(world) = self.owner_document_world(host, handle) {
            world.inline_style_metadata.clear_base_url(handle);
            self.remove_inline_style_metadata_document_if_empty(world.document, handle);
        }
    }

    pub(crate) fn set_inline_style_resolution_text_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
        text: String,
    ) {
        let Some(document) = owner_document_for_source_owner(host, handle) else {
            return;
        };
        self.move_inline_style_metadata_to_document(handle, document);
        self.world_for_document(document)
            .inline_style_metadata
            .set_resolution_text(handle, text);
        self.inline_style_metadata_documents
            .borrow_mut()
            .insert(handle, document);
    }

    pub(crate) fn clear_inline_style_resolution_text_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) {
        let indexed_document = self
            .inline_style_metadata_documents
            .borrow()
            .get(&handle)
            .copied();
        if let Some(document) = indexed_document {
            let world = self.world_for_document(document);
            world.inline_style_metadata.clear_resolution_text(handle);
            self.remove_inline_style_metadata_document_if_empty(document, handle);
        }
        if let Some(world) = self.owner_document_world(host, handle) {
            world.inline_style_metadata.clear_resolution_text(handle);
            self.remove_inline_style_metadata_document_if_empty(world.document, handle);
        }
    }

    pub(crate) fn set_inline_style_csp_state_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
        state: super::InlineStyleCspState,
    ) -> bool {
        let Some(document) = owner_document_for_source_owner(host, handle) else {
            return false;
        };
        self.move_inline_style_metadata_to_document(handle, document);
        let changed = self
            .world_for_document(document)
            .inline_style_metadata
            .set_csp_state(handle, state);
        if state == super::InlineStyleCspState::Unchecked {
            self.remove_inline_style_metadata_document_if_empty(document, handle);
        } else {
            self.inline_style_metadata_documents
                .borrow_mut()
                .insert(handle, document);
        }
        changed
    }

    pub(crate) fn inline_style_csp_state_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) -> super::InlineStyleCspState {
        // This is a metadata lookup performed while resolving CSSOM property
        // names. In particular, a held declaration from a retired iframe may
        // reach it; a read must not recreate that Document's heavyweight world.
        self.active_owner_document_world(host, handle)
            .map(|world| world.inline_style_metadata.csp_state(handle))
            .unwrap_or_default()
    }

    pub(crate) fn migrate_inline_style_metadata_subtree_with_host(
        &self,
        host: &DomHost,
        root: DomHandle,
    ) {
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            self.migrate_inline_style_metadata_to_current_owner_document_with_host(host, handle);
            let mut child = host.first_child(handle);
            while let Some(current) = child {
                stack.push(current);
                child = host.next_sibling(current);
            }
            if let Some(shadow_root) = host.shadow_root_handle(handle) {
                stack.push(shadow_root);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_shadow_root_adopted_style_sheet_sources_with_host(
        &mut self,
        host: &DomHost,
        root: DomHandle,
        sources: Vec<StyloStylesheetSource>,
    ) {
        let Some(world) = self.owner_document_world(host, root) else {
            return;
        };
        let previous_source_ids = world
            .adopted_style_sheet_sources
            .borrow()
            .shadow_root_source_ids(root);
        if world
            .adopted_style_sheet_sources
            .borrow_mut()
            .set_shadow_root_sources(root, sources)
        {
            let next_source_ids = world
                .adopted_style_sheet_sources
                .borrow()
                .shadow_root_source_ids(root);
            self.invalidate_shadow_root_stylesheet_set_for_owner_with_host(
                host,
                root,
                previous_source_ids.into_iter().chain(next_source_ids),
            );
        }
    }

    pub(crate) fn set_shadow_root_adopted_style_sheet_installations_with_host(
        &mut self,
        host: &DomHost,
        root: DomHandle,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) {
        let Some(world) = self.owner_document_world(host, root) else {
            return;
        };
        let previous_source_ids = world
            .adopted_style_sheet_sources
            .borrow()
            .shadow_root_source_ids(root);
        if world
            .adopted_style_sheet_sources
            .borrow_mut()
            .set_shadow_root_installations(root, installations)
        {
            let next_source_ids = world
                .adopted_style_sheet_sources
                .borrow()
                .shadow_root_source_ids(root);
            self.invalidate_shadow_root_stylesheet_set_for_owner_with_host(
                host,
                root,
                previous_source_ids.into_iter().chain(next_source_ids),
            );
        }
    }

    pub(crate) fn shadow_root_adopted_style_sheet_sources_with_host(
        &self,
        host: &DomHost,
        root: DomHandle,
    ) -> Vec<StyloStylesheetSource> {
        let Some(world) = self.owner_document_world(host, root) else {
            return Vec::new();
        };
        world
            .adopted_style_sheet_sources
            .borrow()
            .shadow_root_sources_or_empty_owned(root)
    }

    #[cfg(test)]
    pub(crate) fn shadow_root_adopted_style_sheet_tracks_root_for_document_for_test(
        &self,
        document: DomHandle,
        root: DomHandle,
    ) -> bool {
        self.world_for_document(document)
            .adopted_style_sheet_sources
            .borrow()
            .tracks_shadow_root(root)
    }

    pub(crate) fn mark_document_stylesheet_set_dirty(&mut self, document: DomHandle) {
        let world = self.world_for_document(document);
        world.document_state.bump_source_set_generation();
        world.document_state.record_source_dirty_scope(
            StyleScopeId::Document(document),
            StyleSourceDirtyReason::DocumentStyleSheets,
            std::iter::empty(),
            [document],
        );
    }

    fn invalidate_document_stylesheet_set_for_document(
        &mut self,
        document: DomHandle,
        source_ids: impl IntoIterator<Item = super::StyleSourceId>,
    ) {
        let plan = StyleSourceScopeCleanupPlan::document_adopted_stylesheets(document, source_ids);
        self.apply_source_scope_cleanup_plan(plan);
    }

    fn invalidate_owner_stylesheet_set_for_owner_with_host(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
    ) {
        let plan = StyleSourceScopeCleanupPlan::owner_stylesheet(host, owner);
        self.apply_source_scope_cleanup_plan(plan);
    }

    fn invalidate_linked_stylesheet_set_for_owner_handles_with_host(
        &mut self,
        host: &DomHost,
        owners: &[DomHandle],
    ) {
        let plan =
            StyleSourceScopeCleanupPlan::linked_stylesheet_owners(host, owners.iter().copied());
        self.apply_source_scope_cleanup_plan(plan);
    }

    fn invalidate_shadow_root_stylesheet_set_for_owner_with_host(
        &mut self,
        host: &DomHost,
        root: DomHandle,
        source_ids: impl IntoIterator<Item = super::StyleSourceId>,
    ) {
        let plan =
            StyleSourceScopeCleanupPlan::shadow_root_adopted_stylesheets(host, root, source_ids);
        self.apply_source_scope_cleanup_plan(plan);
    }

    fn apply_source_scope_cleanup_plan(&mut self, plan: StyleSourceScopeCleanupPlan) {
        for document in plan.full_documents() {
            self.mark_document_stylesheet_set_dirty(document);
        }
        let mut source_set_generation_bumped_documents = HashSet::new();
        for entry in plan.scoped_entries_by_document() {
            let document = entry.document();
            let world = self.world_for_document(document);
            if source_set_generation_bumped_documents.insert(document) {
                world.document_state.bump_source_set_generation();
            }
            let roots = entry.roots().clone();
            world.document_state.record_source_dirty_scope(
                entry.scope_id(),
                entry.reason(),
                entry.source_ids(),
                roots.iter().copied(),
            );
        }
    }

    fn remove_linked_stylesheet_owner_from_document(
        &self,
        document: DomHandle,
        owner: DomHandle,
    ) -> bool {
        if self
            .linked_stylesheet_owner_documents
            .borrow()
            .get(&owner)
            .copied()
            != Some(document)
        {
            return false;
        }
        self.linked_stylesheet_owner_documents
            .borrow_mut()
            .remove(&owner);
        self.world_for_document(document)
            .linked_stylesheet_sources
            .borrow_mut()
            .remove_owner(owner)
    }

    fn move_owner_style_sheet_source_to_document(
        &self,
        document: DomHandle,
        owner: DomHandle,
    ) -> Vec<DomHandle> {
        let mut documents = Vec::new();
        let previous_document = self
            .owner_stylesheet_source_documents
            .borrow()
            .get(&owner)
            .copied();
        if let Some(previous_document) = previous_document
            && previous_document != document
            && self
                .world_for_document(previous_document)
                .owner_style_sheet_sources
                .borrow_mut()
                .remove_owner(owner)
        {
            documents.push(previous_document);
        }
        self.owner_stylesheet_source_documents
            .borrow_mut()
            .insert(owner, document);
        documents
    }

    fn remove_owner_style_sheet_source(&mut self, owner: DomHandle) {
        let Some(document) = self
            .owner_stylesheet_source_documents
            .borrow_mut()
            .remove(&owner)
        else {
            return;
        };
        if self
            .world_for_document(document)
            .owner_style_sheet_sources
            .borrow_mut()
            .remove_owner(owner)
        {
            self.mark_document_stylesheet_set_dirty(document);
        }
    }

    fn migrate_inline_style_metadata_to_current_owner_document_with_host(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) {
        let Some(document) = owner_document_for_source_owner(host, handle) else {
            return;
        };
        self.move_inline_style_metadata_to_document(handle, document);
    }

    fn move_inline_style_metadata_to_document(&self, handle: DomHandle, document: DomHandle) {
        let previous_document = self
            .inline_style_metadata_documents
            .borrow()
            .get(&handle)
            .copied();
        if let Some(previous_document) = previous_document
            && previous_document != document
        {
            let metadata = self
                .world_for_document(previous_document)
                .inline_style_metadata
                .take(handle);
            if let Some(metadata) = metadata {
                self.world_for_document(document)
                    .inline_style_metadata
                    .merge_missing(handle, metadata);
                self.inline_style_metadata_documents
                    .borrow_mut()
                    .insert(handle, document);
            } else {
                self.inline_style_metadata_documents
                    .borrow_mut()
                    .remove(&handle);
            }
        }
    }

    fn remove_inline_style_metadata_document_if_empty(
        &self,
        document: DomHandle,
        handle: DomHandle,
    ) {
        if self
            .world_for_document(document)
            .inline_style_metadata
            .has_metadata(handle)
        {
            return;
        }
        let indexed_document = self
            .inline_style_metadata_documents
            .borrow()
            .get(&handle)
            .copied();
        if indexed_document == Some(document) {
            self.inline_style_metadata_documents
                .borrow_mut()
                .remove(&handle);
        }
    }

    fn remove_linked_stylesheet_owner_from_documents(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        documents: Vec<DomHandle>,
    ) {
        if documents.is_empty() {
            return;
        }
        for document in &documents {
            self.remove_linked_stylesheet_owner_from_document(*document, owner);
        }
        self.invalidate_linked_stylesheet_owner_lifecycle_change(
            host,
            owner,
            documents,
            owner_document_for_source_owner(host, owner),
        );
    }

    fn linked_stylesheet_owner_documents(&self, owner: DomHandle) -> Vec<DomHandle> {
        self.linked_stylesheet_owner_documents
            .borrow()
            .get(&owner)
            .copied()
            .into_iter()
            .collect()
    }

    fn invalidate_linked_stylesheet_owner_lifecycle_change(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        previous_documents: Vec<DomHandle>,
        scoped_cleanup_document: Option<DomHandle>,
    ) {
        if scoped_cleanup_document.is_some() {
            self.invalidate_linked_stylesheet_set_for_owner_handles_with_host(host, &[owner]);
        }
        for document in previous_documents {
            if Some(document) == scoped_cleanup_document {
                continue;
            }
            self.mark_document_stylesheet_set_dirty(document);
        }
    }
}

fn owner_document_for_source_owner(host: &DomHost, owner: DomHandle) -> Option<DomHandle> {
    host.owner_document_handle(owner)
}

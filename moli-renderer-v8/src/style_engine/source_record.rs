use crate::{
    document_runtime::DomHandle,
    dom::native::DomHost,
    protocol_types::EmulatedMediaOverrides,
    style_engine::{StyleViewport, StyloStyleEnvironment},
};
use std::sync::Arc;

use moli_selector::{
    StyloSourceDependencyInvalidationBatchSource, StyloSourceDependencySummary,
    StyloStyleSourceScope as StyleSourceScope, StyloStylesheetSourceScopeFallbackInput,
    stylo_stylesheet_source_scope_fallback_roots,
};

use super::{
    StyleInvalidationSourceTarget,
    source::store::StyloStylesheetSource,
    source_id::{StyleScopeId, StyleSourceId, StyleSourceKind},
    source_owner::{
        linked_stylesheet_media_matches_for_stylesheet_source,
        stylesheet_owner_is_stylesheet_source_enabled,
    },
};

pub(super) struct MatchingStyleDependencySource {
    target: StyleInvalidationSourceTarget,
    dependency_summary: Arc<StyloSourceDependencySummary>,
    fallback_roots: Vec<DomHandle>,
}

pub(super) struct RetainedStylesheetSourceRecord<'a> {
    id: StyleSourceId,
    source: RetainedStylesheetSourceRecordSource<'a>,
}

enum RetainedStylesheetSourceRecordSource<'a> {
    OwnerStyleSheet(&'a StyloStylesheetSource),
    Registered(&'a StyloStylesheetSource),
}

impl MatchingStyleDependencySource {
    #[cfg(test)]
    pub(super) fn new_for_test(
        id: StyleSourceId,
        dependency_summary: &StyloSourceDependencySummary,
        fallback_roots: Vec<DomHandle>,
    ) -> Self {
        Self {
            target: StyleInvalidationSourceTarget::stylesheet(id),
            dependency_summary: Arc::new(dependency_summary.clone()),
            fallback_roots,
        }
    }

    pub(super) fn user_agent(
        host: &DomHost,
        document: DomHandle,
        dependency_summary: Arc<StyloSourceDependencySummary>,
        source_scope: &StyleSourceScope,
    ) -> Option<Self> {
        let fallback_roots = stylo_stylesheet_source_scope_fallback_roots(
            host,
            StyloStylesheetSourceScopeFallbackInput::DocumentAdopted { document },
            source_scope,
        );
        if fallback_roots.is_empty() {
            return None;
        }
        Some(Self {
            target: StyleInvalidationSourceTarget::user_agent(document),
            dependency_summary,
            fallback_roots,
        })
    }

    pub(super) fn target(&self) -> &StyleInvalidationSourceTarget {
        &self.target
    }

    pub(super) fn stylo_batch_source<'s>(
        &'s self,
        cause_fallback_roots: &'s [DomHandle],
    ) -> StyloSourceDependencyInvalidationBatchSource<'s> {
        StyloSourceDependencyInvalidationBatchSource::new(
            self.dependency_summary.as_ref(),
            &self.fallback_roots,
            cause_fallback_roots,
        )
    }

    #[cfg(test)]
    pub(super) fn into_target_and_fallback_roots_for_test(
        self,
    ) -> (StyleInvalidationSourceTarget, Vec<DomHandle>) {
        (self.target, self.fallback_roots)
    }

    #[cfg(test)]
    pub(super) fn dependency_summary_matches_for_test(
        &self,
        predicate: impl Fn(&StyloSourceDependencySummary) -> bool,
    ) -> bool {
        predicate(self.dependency_summary.as_ref())
    }
}

impl<'a> RetainedStylesheetSourceRecord<'a> {
    pub(super) fn owner_style_sheet(id: StyleSourceId, source: &'a StyloStylesheetSource) -> Self {
        Self {
            id,
            source: RetainedStylesheetSourceRecordSource::OwnerStyleSheet(source),
        }
    }

    pub(super) fn registered(id: StyleSourceId, source: &'a StyloStylesheetSource) -> Self {
        Self {
            id,
            source: RetainedStylesheetSourceRecordSource::Registered(source),
        }
    }

    pub(super) fn id(&self) -> &StyleSourceId {
        &self.id
    }

    fn dependency_summary(&self) -> Arc<StyloSourceDependencySummary> {
        self.source().source_dependency_summary()
    }

    fn source(&self) -> &StyloStylesheetSource {
        match &self.source {
            RetainedStylesheetSourceRecordSource::OwnerStyleSheet(source)
            | RetainedStylesheetSourceRecordSource::Registered(source) => source,
        }
    }

    fn effective_media_text(&self, host: &DomHost, owner: DomHandle) -> String {
        if self.source().has_authoritative_runtime_media() {
            return self.source().media_text().to_owned();
        }
        host.get_attribute(owner, "media").unwrap_or_default()
    }

    pub(super) fn is_dependency_source_enabled(
        &self,
        host: &DomHost,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) -> bool {
        let environment_for_owner = |owner| {
            StyloStyleEnvironment::from_emulated_media(emulated_media)
                .for_document(host, host.owner_document_handle(owner))
        };
        match self.id.kind {
            StyleSourceKind::OwnerStyleSheet { owner } => {
                stylesheet_owner_is_stylesheet_source_enabled(
                    host,
                    owner,
                    &self.effective_media_text(host, owner),
                    environment_for_owner(owner),
                    viewport,
                )
            }
            StyleSourceKind::LinkedStyleSheet { owner } => {
                linked_stylesheet_media_matches_for_stylesheet_source(
                    &self.effective_media_text(host, owner),
                    environment_for_owner(owner),
                    viewport,
                )
            }
            StyleSourceKind::DocumentAdoptedStyleSheet { .. }
            | StyleSourceKind::ShadowRootAdoptedStyleSheet { .. } => true,
        }
    }

    fn fallback_roots(&self, host: &DomHost, source_scope: &StyleSourceScope) -> Vec<DomHandle> {
        stylo_stylesheet_source_scope_fallback_roots(
            host,
            self.source_scope_fallback_input(),
            source_scope,
        )
    }

    fn source_scope_fallback_input(&self) -> StyloStylesheetSourceScopeFallbackInput {
        match self.id.kind {
            StyleSourceKind::OwnerStyleSheet { owner }
            | StyleSourceKind::LinkedStyleSheet { owner } => {
                StyloStylesheetSourceScopeFallbackInput::StylesheetOwner { owner }
            }
            StyleSourceKind::DocumentAdoptedStyleSheet { .. } => match self.id.scope_id {
                StyleScopeId::Document(document) => {
                    StyloStylesheetSourceScopeFallbackInput::DocumentAdopted { document }
                }
                StyleScopeId::ShadowRoot(_) => StyloStylesheetSourceScopeFallbackInput::Unscoped,
            },
            StyleSourceKind::ShadowRootAdoptedStyleSheet { .. } => match self.id.scope_id {
                StyleScopeId::ShadowRoot(root) => {
                    StyloStylesheetSourceScopeFallbackInput::ShadowRootAdopted { root }
                }
                StyleScopeId::Document(_) => StyloStylesheetSourceScopeFallbackInput::Unscoped,
            },
        }
    }

    pub(super) fn matching_dependency_source(
        &self,
        host: &DomHost,
        source_scope: &StyleSourceScope,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) -> Option<MatchingStyleDependencySource> {
        if !self.is_dependency_source_enabled(host, emulated_media, viewport) {
            return None;
        }
        let fallback_roots = self.fallback_roots(host, source_scope);
        if fallback_roots.is_empty() {
            return None;
        }
        Some(MatchingStyleDependencySource {
            target: StyleInvalidationSourceTarget::stylesheet(self.id.clone()),
            dependency_summary: self.dependency_summary(),
            fallback_roots,
        })
    }

    pub(super) fn to_stylo_source(&self) -> StyloStylesheetSource {
        let source = match &self.source {
            RetainedStylesheetSourceRecordSource::OwnerStyleSheet(source) => (*source).clone(),
            RetainedStylesheetSourceRecordSource::Registered(source) => (*source).clone(),
        };
        source.with_source_id(Some(self.id.clone()))
    }
}

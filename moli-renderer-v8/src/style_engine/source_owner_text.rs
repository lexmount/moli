use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::document_runtime::DomHandle;

use super::source::store::{OwnerStyleSheetSource, StyloStylesheetSource};

#[derive(Debug)]
struct InstalledOwnerStyleSheet {
    processing_source: Arc<OwnerStyleSheetSource>,
    cascade_source: StyloStylesheetSource,
    live_stylesheet: Option<crate::live_stylesheet::LiveStylesheetRef>,
}

impl InstalledOwnerStyleSheet {
    fn from_processing_source(processing_source: Arc<OwnerStyleSheetSource>) -> Self {
        Self {
            cascade_source: processing_source.source().clone(),
            processing_source,
            live_stylesheet: None,
        }
    }

    fn install_live_stylesheet(&mut self, stylesheet: crate::live_stylesheet::LiveStylesheetRef) {
        self.cascade_source = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        self.live_stylesheet = Some(stylesheet);
    }

    fn refresh_live_stylesheet(
        &mut self,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let Some(stylesheet) = self
            .live_stylesheet
            .as_ref()
            .filter(|stylesheet| stylesheet.id() == stylesheet_id)
        else {
            return false;
        };
        self.cascade_source = StyloStylesheetSource::from_live_stylesheet(stylesheet);
        true
    }
}

#[derive(Debug, Default)]
pub(super) struct OwnerStyleSheetSources {
    sources_by_owner: HashMap<DomHandle, InstalledOwnerStyleSheet>,
    // Projection of DocumentRuntime's CSP disposition. This set only suppresses
    // retained/cascade sources; it is not queried for request or event policy.
    csp_suppressed_owners: HashSet<DomHandle>,
}

impl OwnerStyleSheetSources {
    pub(super) fn clear_all(&mut self) {
        self.sources_by_owner.clear();
        self.csp_suppressed_owners.clear();
    }

    #[cfg(test)]
    pub(super) fn set_source(
        &mut self,
        owner: DomHandle,
        css_text: String,
        parser_base: url::Url,
    ) -> bool {
        if self.sources_by_owner.get(&owner).is_some_and(|existing| {
            existing
                .processing_source
                .matches_processing_input(&css_text, &parser_base)
        }) {
            return false;
        }
        let processing_source = Arc::new(OwnerStyleSheetSource::new(owner, css_text, parser_base));
        self.sources_by_owner.insert(
            owner,
            InstalledOwnerStyleSheet::from_processing_source(processing_source),
        );
        true
    }

    pub(super) fn replace_processed_source(
        &mut self,
        owner: DomHandle,
        css_text: String,
        parser_base: url::Url,
    ) {
        let processing_source = Arc::new(OwnerStyleSheetSource::new(owner, css_text, parser_base));
        self.sources_by_owner.insert(
            owner,
            InstalledOwnerStyleSheet::from_processing_source(processing_source),
        );
    }

    pub(super) fn install_live_stylesheet(
        &mut self,
        owner: DomHandle,
        stylesheet: crate::live_stylesheet::LiveStylesheetRef,
    ) -> bool {
        let Some(installed) = self.sources_by_owner.get_mut(&owner) else {
            return false;
        };
        installed.install_live_stylesheet(stylesheet);
        true
    }

    pub(super) fn refresh_live_stylesheet(
        &mut self,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let Some(installed) = self.sources_by_owner.get_mut(&owner) else {
            return false;
        };
        installed.refresh_live_stylesheet(stylesheet_id)
    }

    pub(super) fn live_stylesheet(
        &self,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        if self.is_csp_suppressed(owner) {
            return None;
        }
        self.sources_by_owner
            .get(&owner)
            .and_then(|installed| installed.live_stylesheet.clone())
    }

    pub(super) fn remove_owner(&mut self, owner: DomHandle) -> bool {
        self.csp_suppressed_owners.remove(&owner);
        self.sources_by_owner.remove(&owner).is_some()
    }

    pub(super) fn set_csp_suppressed(&mut self, owner: DomHandle, suppressed: bool) -> bool {
        if suppressed {
            self.csp_suppressed_owners.insert(owner)
        } else {
            self.csp_suppressed_owners.remove(&owner)
        }
    }

    pub(super) fn is_csp_suppressed(&self, owner: DomHandle) -> bool {
        self.csp_suppressed_owners.contains(&owner)
    }

    #[cfg(any(test, feature = "devtools"))]
    pub(super) fn text(&self, owner: DomHandle) -> Option<&str> {
        self.sources_by_owner
            .get(&owner)
            .map(|installed| installed.processing_source.css_text())
    }

    #[cfg(any(test, feature = "devtools"))]
    pub(super) fn text_owned(&self, owner: DomHandle) -> Option<String> {
        self.text(owner).map(str::to_owned)
    }

    pub(super) fn source(&self, owner: DomHandle) -> Option<StyloStylesheetSource> {
        if self.is_csp_suppressed(owner) {
            return None;
        }
        self.sources_by_owner
            .get(&owner)
            .map(|installed| installed.cascade_source.clone())
    }

    pub(super) fn processing_source(&self, owner: DomHandle) -> Option<Arc<OwnerStyleSheetSource>> {
        if self.is_csp_suppressed(owner) {
            return None;
        }
        self.sources_by_owner
            .get(&owner)
            .map(|installed| Arc::clone(&installed.processing_source))
    }
}

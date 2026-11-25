use moli_dom::native::InlineStyleAttribute;
use std::{
    cell::{Ref, RefCell},
    collections::HashMap,
    sync::Arc,
};

use crate::{css_style::CssInlineStyleDeclarationState, document_runtime::DomHandle};

pub(in crate::style_engine) struct InlineCssomDeclaration {
    pub(in crate::style_engine) state: CssInlineStyleDeclarationState,
    pub(in crate::style_engine) source: Option<Arc<InlineStyleAttribute>>,
}

impl crate::style_engine::MoliStyleEngine {
    pub(crate) fn cssom_inline_style(
        &self,
        handle: DomHandle,
    ) -> Option<Ref<'_, CssInlineStyleDeclarationState>> {
        Ref::filter_map(self.cssom_inline_styles.borrow(), |states| {
            states.get(&handle).map(|entry| &entry.state)
        })
        .ok()
    }

    pub(crate) fn set_cssom_inline_style(
        &self,
        handle: DomHandle,
        state: CssInlineStyleDeclarationState,
        source: Option<Arc<InlineStyleAttribute>>,
    ) {
        self.cssom_inline_styles
            .borrow_mut()
            .insert(handle, InlineCssomDeclaration { state, source });
    }

    pub(crate) fn take_cssom_inline_style(
        &self,
        handle: DomHandle,
    ) -> Option<CssInlineStyleDeclarationState> {
        self.cssom_inline_styles
            .borrow_mut()
            .remove(&handle)
            .map(|inline| inline.state)
    }

    pub(crate) fn clear_cssom_inline_styles(&self) {
        self.cssom_inline_styles.borrow_mut().clear();
    }
}

#[derive(Debug, Default)]
pub(in crate::style_engine) struct InlineStyleMetadataStore {
    metadata_by_handle: RefCell<HashMap<DomHandle, InlineStyleMetadata>>,
}

#[derive(Clone, Debug, Default)]
pub(in crate::style_engine) struct InlineStyleMetadata {
    base_url: Option<url::Url>,
    resolution_text: Option<Arc<InlineStyleAttribute>>,
    csp_state: InlineStyleCspState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum InlineStyleCspState {
    #[default]
    Unchecked,
    AllowedAttribute,
    BlockedAttribute,
    Cssom,
}

impl InlineStyleMetadataStore {
    pub(in crate::style_engine) fn ensure_base_url(&self, handle: DomHandle, base_url: url::Url) {
        self.metadata_by_handle
            .borrow_mut()
            .entry(handle)
            .or_default()
            .base_url
            .get_or_insert(base_url);
    }

    pub(in crate::style_engine) fn set_base_url(&self, handle: DomHandle, base_url: url::Url) {
        self.metadata_by_handle
            .borrow_mut()
            .entry(handle)
            .or_default()
            .base_url = Some(base_url);
    }

    pub(in crate::style_engine) fn clear_base_url(&self, handle: DomHandle) {
        self.update(handle, |metadata| {
            metadata.base_url = None;
        });
    }

    pub(in crate::style_engine) fn base_url(&self, handle: DomHandle) -> Option<url::Url> {
        self.metadata_by_handle
            .borrow()
            .get(&handle)
            .and_then(|metadata| metadata.base_url.clone())
    }

    pub(in crate::style_engine) fn set_resolution_text(&self, handle: DomHandle, text: String) {
        let mut entries = self.metadata_by_handle.borrow_mut();
        let metadata = entries.entry(handle).or_default();
        if metadata
            .resolution_text
            .as_ref()
            .map(|source| source.as_str())
            != Some(text.as_str())
        {
            metadata.resolution_text = Some(Arc::new(InlineStyleAttribute::new(text)));
        }
    }

    pub(in crate::style_engine) fn clear_resolution_text(&self, handle: DomHandle) {
        self.update(handle, |metadata| {
            metadata.resolution_text = None;
        });
    }

    pub(in crate::style_engine) fn resolution_text(
        &self,
        handle: DomHandle,
    ) -> Option<Arc<InlineStyleAttribute>> {
        self.metadata_by_handle
            .borrow()
            .get(&handle)
            .and_then(|metadata| metadata.resolution_text.clone())
    }

    pub(in crate::style_engine) fn set_csp_state(
        &self,
        handle: DomHandle,
        state: InlineStyleCspState,
    ) -> bool {
        let mut metadata_by_handle = self.metadata_by_handle.borrow_mut();
        let current = metadata_by_handle
            .get(&handle)
            .map(|metadata| metadata.csp_state)
            .unwrap_or_default();
        if current == state {
            return false;
        }
        if state == InlineStyleCspState::Unchecked {
            if let Some(metadata) = metadata_by_handle.get_mut(&handle) {
                metadata.csp_state = state;
                if metadata.is_empty() {
                    metadata_by_handle.remove(&handle);
                }
            }
        } else {
            metadata_by_handle.entry(handle).or_default().csp_state = state;
        }
        true
    }

    pub(in crate::style_engine) fn csp_state(&self, handle: DomHandle) -> InlineStyleCspState {
        self.metadata_by_handle
            .borrow()
            .get(&handle)
            .map(|metadata| metadata.csp_state)
            .unwrap_or_default()
    }

    pub(in crate::style_engine) fn take(&self, handle: DomHandle) -> Option<InlineStyleMetadata> {
        self.metadata_by_handle.borrow_mut().remove(&handle)
    }

    pub(in crate::style_engine) fn merge_missing(
        &self,
        handle: DomHandle,
        incoming: InlineStyleMetadata,
    ) {
        let mut metadata_by_handle = self.metadata_by_handle.borrow_mut();
        let metadata = metadata_by_handle.entry(handle).or_default();
        if metadata.base_url.is_none() {
            metadata.base_url = incoming.base_url;
        }
        if metadata.resolution_text.is_none() {
            metadata.resolution_text = incoming.resolution_text;
        }
        if metadata.csp_state == InlineStyleCspState::Unchecked {
            metadata.csp_state = incoming.csp_state;
        }
        if metadata.is_empty() {
            metadata_by_handle.remove(&handle);
        }
    }

    pub(in crate::style_engine) fn has_metadata(&self, handle: DomHandle) -> bool {
        self.metadata_by_handle.borrow().contains_key(&handle)
    }

    pub(in crate::style_engine) fn clear_all(&self) {
        self.metadata_by_handle.borrow_mut().clear();
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn len(&self) -> usize {
        self.metadata_by_handle.borrow().len()
    }

    fn update(&self, handle: DomHandle, callback: impl FnOnce(&mut InlineStyleMetadata)) {
        let mut metadata_by_handle = self.metadata_by_handle.borrow_mut();
        let Some(metadata) = metadata_by_handle.get_mut(&handle) else {
            return;
        };
        callback(metadata);
        if metadata.is_empty() {
            metadata_by_handle.remove(&handle);
        }
    }
}

impl InlineStyleMetadata {
    fn is_empty(&self) -> bool {
        self.base_url.is_none()
            && self.resolution_text.is_none()
            && self.csp_state == InlineStyleCspState::Unchecked
    }
}

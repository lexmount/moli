use super::*;
use crate::native::html_serialization::{
    HtmlSerializationTarget, HtmlSerializedShadowRoot, escape_html_attribute_into_string,
    serialize_html_with_shadow_root_provider,
};
use crate::native::{
    HtmlEmbeddedDocumentProvider, HtmlSerializationError, HtmlSerializationPolicy,
};

pub type ShadowRootRegistryAttributePolicy<'a> =
    dyn Fn(DomHandle, DomHandle, &ShadowRootInit) -> bool + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowRootInclusion<'a> {
    None,
    SerializableOrExplicit {
        serializable: bool,
        explicit: &'a [DomHandle],
    },
    AllAuthor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowRootMarkupProfile {
    WebApi,
    Inspector,
}

impl DomHost {
    pub fn get_html(
        &self,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        serializable: bool,
        explicit: &[DomHandle],
    ) -> Option<String> {
        self.get_html_with_shadow_root_registry_attribute_policy(
            handle,
            scripting,
            serializable,
            explicit,
            None,
        )
    }

    /// One read-only traversal, with directly streamed live embedded Documents.
    /// No DOM copy, generated node, JavaScript execution or frame HTML cache.
    pub fn get_html_for_export(
        &self,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        policy: Option<&dyn HtmlSerializationPolicy>,
        embedded: Option<&dyn HtmlEmbeddedDocumentProvider>,
        max_bytes: Option<usize>,
    ) -> Result<String, HtmlSerializationError> {
        self.serialize_html_with_shadow_roots(
            &self.dom,
            handle,
            HtmlSerializationTarget::ChildrenOnly,
            scripting,
            ShadowRootInclusion::AllAuthor,
            ShadowRootMarkupProfile::WebApi,
            None,
            policy,
            embedded,
            max_bytes,
        )?
        .ok_or(HtmlSerializationError::InvalidNode { node: handle })
    }

    /// Serializes an existing export copy with the live host's root bindings.
    /// Kept only until page-level exports switch to the read-only engine.
    pub fn get_html_from_copy(
        &self,
        dom: &NativeDom,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        explicit: &[DomHandle],
        include_adopted_style_sheets: bool,
    ) -> Option<String> {
        struct OmitStyleSheets;
        impl HtmlSerializationPolicy for OmitStyleSheets {
            fn include_shadow_adopted_style_sheets(&self) -> bool {
                false
            }
        }
        self.serialize_html_with_shadow_roots(
            dom,
            handle,
            HtmlSerializationTarget::ChildrenOnly,
            scripting,
            ShadowRootInclusion::SerializableOrExplicit {
                serializable: false,
                explicit,
            },
            ShadowRootMarkupProfile::WebApi,
            None,
            (!include_adopted_style_sheets)
                .then_some(&OmitStyleSheets as &dyn HtmlSerializationPolicy),
            None,
            None,
        )
        .ok()
        .flatten()
    }

    pub fn get_html_with_shadow_root_registry_attribute_policy(
        &self,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        serializable: bool,
        explicit: &[DomHandle],
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            &self.dom,
            handle,
            HtmlSerializationTarget::ChildrenOnly,
            scripting,
            ShadowRootInclusion::SerializableOrExplicit {
                serializable,
                explicit,
            },
            ShadowRootMarkupProfile::WebApi,
            registry,
            None,
            None,
            None,
        )
        .ok()
        .flatten()
    }

    pub fn outer_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        inclusion: ShadowRootInclusion<'_>,
        profile: ShadowRootMarkupProfile,
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            &self.dom,
            handle,
            HtmlSerializationTarget::IncludeNode,
            scripting,
            inclusion,
            profile,
            registry,
            None,
            None,
            None,
        )
        .ok()
        .flatten()
    }

    fn serialize_html_with_shadow_roots(
        &self,
        dom: &NativeDom,
        handle: DomHandle,
        target: HtmlSerializationTarget,
        scripting: &dyn Fn(DomHandle) -> bool,
        inclusion: ShadowRootInclusion<'_>,
        profile: ShadowRootMarkupProfile,
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
        policy: Option<&dyn HtmlSerializationPolicy>,
        embedded: Option<&dyn HtmlEmbeddedDocumentProvider>,
        max_bytes: Option<usize>,
    ) -> Result<Option<String>, HtmlSerializationError> {
        let include_style_sheets = policy.is_none_or(|p| p.include_shadow_adopted_style_sheets());
        let provider = |host| {
            let roots = self.shadow_roots_by_host.borrow();
            let state = roots.get(&host)?;
            let included = match inclusion {
                ShadowRootInclusion::None => false,
                ShadowRootInclusion::SerializableOrExplicit {
                    serializable,
                    explicit,
                } => serializable && state.init.serializable() || explicit.contains(&state.handle),
                // Only author roots enter this native registry; UA trees are renderer projections.
                ShadowRootInclusion::AllAuthor => {
                    profile == ShadowRootMarkupProfile::Inspector
                        || dom
                            .node(host)
                            .and_then(Node::owner_document)
                            .and_then(|document| dom.node(document))
                            .and_then(Node::as_document)
                            .is_none_or(|document| !document.is_inert_template_document())
                }
            };
            if !included {
                return None;
            }
            let mut template_start = String::new();
            self.write_shadow_root_template_start(
                host,
                state.handle,
                &state.init,
                profile,
                &mut template_start,
                registry,
                include_style_sheets,
            );
            Some(HtmlSerializedShadowRoot::new(state.handle, template_start))
        };
        let provider = (!self.shadow_roots_by_host.borrow().is_empty()
            && inclusion != ShadowRootInclusion::None)
            .then_some(&provider);
        serialize_html_with_shadow_root_provider(
            dom, handle, target, scripting, provider, policy, embedded, max_bytes,
        )
    }

    fn write_shadow_root_template_start(
        &self,
        host: DomHandle,
        shadow_root: DomHandle,
        init: &ShadowRootInit,
        markup_profile: ShadowRootMarkupProfile,
        out: &mut String,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
        include_adopted_style_sheets: bool,
    ) {
        out.push_str("<template shadowrootmode=\"");
        escape_html_attribute_into_string(init.mode(), out);
        out.push('"');
        if init.delegates_focus() {
            out.push_str(" shadowrootdelegatesfocus=\"\"");
        }
        if init.serializable() {
            out.push_str(" shadowrootserializable=\"\"");
        }
        if markup_profile == ShadowRootMarkupProfile::WebApi && init.slot_assignment() != "named" {
            out.push_str(" shadowrootslotassignment=\"");
            escape_html_attribute_into_string(init.slot_assignment(), out);
            out.push('"');
        }
        if init.clonable() {
            out.push_str(" shadowrootclonable=\"\"");
        }
        let serialize_registry_attribute = registry_attribute_policy
            .map(|policy| policy(host, shadow_root, init))
            .unwrap_or_else(|| init.null_custom_element_registry());
        if serialize_registry_attribute {
            out.push_str(" shadowrootcustomelementregistry=\"\"");
        }
        if markup_profile == ShadowRootMarkupProfile::WebApi
            && let Some(reference_target) = init.reference_target()
        {
            out.push_str(" shadowrootreferencetarget=\"");
            escape_html_attribute_into_string(reference_target, out);
            out.push('"');
        }
        if include_adopted_style_sheets
            && markup_profile == ShadowRootMarkupProfile::WebApi
            && let Some(adopted_style_sheets) = init.adopted_style_sheets()
        {
            out.push_str(" shadowrootadoptedstylesheets=\"");
            escape_html_attribute_into_string(adopted_style_sheets, out);
            out.push('"');
        }
        out.push('>');
    }
}

#[cfg(test)]
mod tests;

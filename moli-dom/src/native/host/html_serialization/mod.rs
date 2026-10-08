use super::*;
use crate::native::html_serialization::{
    HtmlSerializationOptions, HtmlSerializationTarget, HtmlSerializedShadowRoot,
    HtmlShadowRootProvider, serialize_html_utf16, serialize_html_with_options,
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
    /// Serializes DOMString Web API results without repairing surrogates.
    pub fn get_html_utf16(
        &self,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        serializable: bool,
        explicit: &[DomHandle],
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<Vec<u16>> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationOptions::new(HtmlSerializationTarget::ChildrenOnly, scripting),
            ShadowRootInclusion::SerializableOrExplicit {
                serializable,
                explicit,
            },
            ShadowRootMarkupProfile::WebApi,
            registry,
            |dom, node, options| {
                serialize_html_utf16(
                    dom,
                    node,
                    options.target,
                    options.scripting_enabled_for_node,
                    options.shadow_root_provider,
                )
            },
        )
        .ok()
        .flatten()
    }

    /// Includes the element itself in a lossless DOMString serialization.
    pub fn outer_html_utf16_with_shadow_roots(
        &self,
        handle: DomHandle,
        scripting: &dyn Fn(DomHandle) -> bool,
        inclusion: ShadowRootInclusion<'_>,
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<Vec<u16>> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationOptions::new(HtmlSerializationTarget::IncludeNode, scripting),
            inclusion,
            ShadowRootMarkupProfile::WebApi,
            registry,
            |dom, node, options| {
                serialize_html_utf16(
                    dom,
                    node,
                    options.target,
                    options.scripting_enabled_for_node,
                    options.shadow_root_provider,
                )
            },
        )
        .ok()
        .flatten()
    }

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
        let mut options =
            HtmlSerializationOptions::new(HtmlSerializationTarget::ChildrenOnly, scripting);
        options.policy = policy;
        options.embedded_documents = embedded;
        options.max_bytes = max_bytes;
        self.serialize_html_with_shadow_roots(
            handle,
            options,
            ShadowRootInclusion::AllAuthor,
            ShadowRootMarkupProfile::WebApi,
            None,
            serialize_html_with_options,
        )?
        .ok_or(HtmlSerializationError::InvalidNode { node: handle })
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
            handle,
            HtmlSerializationOptions::new(HtmlSerializationTarget::ChildrenOnly, scripting),
            ShadowRootInclusion::SerializableOrExplicit {
                serializable,
                explicit,
            },
            ShadowRootMarkupProfile::WebApi,
            registry,
            serialize_html_with_options,
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
            handle,
            HtmlSerializationOptions::new(HtmlSerializationTarget::IncludeNode, scripting),
            inclusion,
            profile,
            registry,
            serialize_html_with_options,
        )
        .ok()
        .flatten()
    }

    fn serialize_html_with_shadow_roots<T>(
        &self,
        handle: DomHandle,
        options: HtmlSerializationOptions<'_>,
        inclusion: ShadowRootInclusion<'_>,
        profile: ShadowRootMarkupProfile,
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
        serialize: impl for<'a> FnOnce(
            &'a NativeDom,
            DomHandle,
            HtmlSerializationOptions<'a>,
        ) -> Result<Option<T>, HtmlSerializationError>,
    ) -> Result<Option<T>, HtmlSerializationError> {
        let include_style_sheets = options
            .policy
            .is_none_or(|p| p.include_shadow_adopted_style_sheets());
        // Serialization is synchronous and does not run author code. Borrow
        // the existing host index once, including detached author roots.
        let roots = self.shadow_roots_by_host.borrow();
        let provider = |host| {
            let state = roots.get(&host)?;
            let included = match inclusion {
                ShadowRootInclusion::None => false,
                ShadowRootInclusion::SerializableOrExplicit {
                    serializable,
                    explicit,
                } => serializable && state.init.serializable() || explicit.contains(&state.handle),
                // Only author roots enter this native registry; UA trees are renderer projections.
                ShadowRootInclusion::AllAuthor => true,
            };
            if !included {
                return None;
            }
            Some(HtmlSerializedShadowRoot {
                root: state.handle,
                init: &state.init,
                web_api: profile == ShadowRootMarkupProfile::WebApi,
                registry_attribute: registry.map_or_else(
                    || state.init.null_custom_element_registry(),
                    |policy| policy(host, state.handle, &state.init),
                ),
                include_style_sheets,
            })
        };
        let provider = (!roots.is_empty() && inclusion != ShadowRootInclusion::None)
            .then_some(&provider as &dyn HtmlShadowRootProvider<'_>);
        serialize(
            &self.dom,
            handle,
            HtmlSerializationOptions {
                shadow_root_provider: provider,
                ..options
            },
        )
    }
}

#[cfg(test)]
mod tests;

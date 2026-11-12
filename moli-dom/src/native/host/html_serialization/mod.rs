use super::*;
use crate::native::html_serialization::{
    HtmlSerializationOptions, HtmlSerializationTarget, HtmlSerializedShadowRoot,
    HtmlShadowRootProvider, escape_html_attribute_into_string, serialize_html_with_options,
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
        )
        .ok()
        .flatten()
    }

    fn serialize_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        options: HtmlSerializationOptions<'_>,
        inclusion: ShadowRootInclusion<'_>,
        profile: ShadowRootMarkupProfile,
        registry: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Result<Option<String>, HtmlSerializationError> {
        let include_style_sheets = options
            .policy
            .is_none_or(|p| p.include_shadow_adopted_style_sheets());
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
                ShadowRootInclusion::AllAuthor => true,
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
            .then_some(&provider as &dyn HtmlShadowRootProvider);
        serialize_html_with_options(
            &self.dom,
            handle,
            HtmlSerializationOptions {
                shadow_root_provider: provider,
                ..options
            },
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

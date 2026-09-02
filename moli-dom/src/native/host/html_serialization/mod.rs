use super::*;
use crate::native::html_serialization::{
    HtmlSerializationTarget, HtmlSerializedShadowRoot, escape_html_attribute_into_string,
    serialize_html_with_shadow_root_provider,
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
    AllAuthorForInspector,
}

impl ShadowRootInclusion<'_> {
    fn markup_profile(self) -> ShadowRootMarkupProfile {
        match self {
            Self::AllAuthorForInspector => ShadowRootMarkupProfile::Inspector,
            Self::None | Self::SerializableOrExplicit { .. } => ShadowRootMarkupProfile::WebApi,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadowRootMarkupProfile {
    WebApi,
    Inspector,
}

impl DomHost {
    pub fn get_html(
        &self,
        handle: DomHandle,
        scripting_enabled_for_node: &dyn Fn(DomHandle) -> bool,
        serializable_shadow_roots: bool,
        explicit_shadow_roots: &[DomHandle],
    ) -> Option<String> {
        self.get_html_with_shadow_root_registry_attribute_policy(
            handle,
            scripting_enabled_for_node,
            serializable_shadow_roots,
            explicit_shadow_roots,
            None,
        )
    }

    pub fn get_html_with_shadow_root_registry_attribute_policy(
        &self,
        handle: DomHandle,
        scripting_enabled_for_node: &dyn Fn(DomHandle) -> bool,
        serializable_shadow_roots: bool,
        explicit_shadow_roots: &[DomHandle],
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationTarget::ChildrenOnly,
            scripting_enabled_for_node,
            ShadowRootInclusion::SerializableOrExplicit {
                serializable: serializable_shadow_roots,
                explicit: explicit_shadow_roots,
            },
            registry_attribute_policy,
        )
    }

    pub fn outer_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        scripting_enabled_for_node: &dyn Fn(DomHandle) -> bool,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        self.serialize_html_with_shadow_roots(
            handle,
            HtmlSerializationTarget::IncludeNode,
            scripting_enabled_for_node,
            shadow_root_inclusion,
            registry_attribute_policy,
        )
    }

    fn serialize_html_with_shadow_roots(
        &self,
        handle: DomHandle,
        target: HtmlSerializationTarget,
        scripting_enabled_for_node: &dyn Fn(DomHandle) -> bool,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
    ) -> Option<String> {
        let shadow_root_provider = |host| {
            let (shadow_root, init) =
                self.serialized_shadow_root_for_host(host, shadow_root_inclusion)?;
            let mut template_start = String::new();
            self.write_shadow_root_template_start(
                host,
                shadow_root,
                &init,
                shadow_root_inclusion.markup_profile(),
                &mut template_start,
                registry_attribute_policy,
            );
            Some(HtmlSerializedShadowRoot::new(shadow_root, template_start))
        };
        serialize_html_with_shadow_root_provider(
            &self.dom,
            handle,
            target,
            scripting_enabled_for_node,
            &shadow_root_provider,
        )
    }

    fn serialized_shadow_root_for_host(
        &self,
        host: DomHandle,
        shadow_root_inclusion: ShadowRootInclusion<'_>,
    ) -> Option<(DomHandle, ShadowRootInit)> {
        let state = self.shadow_roots_by_host.borrow().get(&host)?.clone();
        let included = match shadow_root_inclusion {
            ShadowRootInclusion::None => false,
            ShadowRootInclusion::SerializableOrExplicit {
                serializable,
                explicit,
            } => serializable && state.init.serializable() || explicit.contains(&state.handle),
            // DomHost stores author shadow roots. Generated user-agent trees are
            // Inspector projections owned by the renderer and never enter this map.
            ShadowRootInclusion::AllAuthorForInspector => true,
        };
        if included {
            Some((state.handle, state.init))
        } else {
            None
        }
    }

    fn write_shadow_root_template_start(
        &self,
        host: DomHandle,
        shadow_root: DomHandle,
        init: &ShadowRootInit,
        markup_profile: ShadowRootMarkupProfile,
        out: &mut String,
        registry_attribute_policy: Option<&ShadowRootRegistryAttributePolicy<'_>>,
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
        if markup_profile == ShadowRootMarkupProfile::WebApi
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

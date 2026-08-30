use super::{BridgeDescriptor, InstallGroups, RuntimeInstallGroups, SpecializedTemplateInstaller};
use crate::web_api_interfaces;

const fn descriptor(
    interface: moli_webapi_declare::WebApiInterfaceDescriptor,
    install_groups: InstallGroups,
) -> BridgeDescriptor {
    specialized_descriptor(
        interface,
        install_groups,
        SpecializedTemplateInstaller::None,
        NONE_RUNTIME_INSTALL_GROUPS,
    )
}

const fn specialized_descriptor(
    interface: moli_webapi_declare::WebApiInterfaceDescriptor,
    install_groups: InstallGroups,
    specialized_template_installer: SpecializedTemplateInstaller,
    runtime_install_groups: RuntimeInstallGroups,
) -> BridgeDescriptor {
    BridgeDescriptor {
        interface,
        install_groups,
        specialized_template_installer,
        runtime_install_groups,
    }
}

const CHARACTER_DATA_GROUPS: InstallGroups = InstallGroups {
    character_data_api: true,
    markup_container_api: false,
    document_methods: false,
};

const BASE_GROUPS: InstallGroups = InstallGroups {
    character_data_api: false,
    markup_container_api: false,
    document_methods: false,
};

const MARKUP_CONTAINER_GROUPS: InstallGroups = InstallGroups {
    markup_container_api: true,
    character_data_api: false,
    document_methods: false,
};

const DOCUMENT_GROUPS: InstallGroups = InstallGroups {
    character_data_api: false,
    markup_container_api: false,
    document_methods: true,
};

const ELEMENT_GROUPS: InstallGroups = InstallGroups {
    character_data_api: false,
    markup_container_api: true,
    document_methods: false,
};

const HTML_ELEMENT_GROUPS: InstallGroups = InstallGroups {
    character_data_api: false,
    markup_container_api: true,
    document_methods: false,
};

const NONE_RUNTIME_INSTALL_GROUPS: RuntimeInstallGroups = RuntimeInstallGroups {
    svg_geometry_path_length: false,
    svg_rect_animated_lengths: false,
    svg_text_positioning_lists: false,
    svg_pattern_transform: false,
    svg_gradient_transform: false,
};

const SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS: RuntimeInstallGroups = RuntimeInstallGroups {
    svg_geometry_path_length: true,
    svg_rect_animated_lengths: false,
    svg_text_positioning_lists: false,
    svg_pattern_transform: false,
    svg_gradient_transform: false,
};

const NODE_BRIDGE_DESCRIPTORS: &[BridgeDescriptor] = &[
    descriptor(web_api_interfaces::Node::DESCRIPTOR, BASE_GROUPS),
    descriptor(web_api_interfaces::Document::DESCRIPTOR, DOCUMENT_GROUPS),
    descriptor(
        web_api_interfaces::HTMLDocument::DESCRIPTOR,
        DOCUMENT_GROUPS,
    ),
    descriptor(web_api_interfaces::XMLDocument::DESCRIPTOR, DOCUMENT_GROUPS),
    descriptor(
        web_api_interfaces::DocumentFragment::DESCRIPTOR,
        MARKUP_CONTAINER_GROUPS,
    ),
    descriptor(web_api_interfaces::DocumentType::DESCRIPTOR, BASE_GROUPS),
    specialized_descriptor(
        web_api_interfaces::ShadowRoot::DESCRIPTOR,
        MARKUP_CONTAINER_GROUPS,
        SpecializedTemplateInstaller::ShadowRoot,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(web_api_interfaces::Element::DESCRIPTOR, ELEMENT_GROUPS),
    descriptor(web_api_interfaces::SVGElement::DESCRIPTOR, ELEMENT_GROUPS),
    descriptor(
        web_api_interfaces::SVGGraphicsElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGGeometryElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(web_api_interfaces::SVGAElement::DESCRIPTOR, ELEMENT_GROUPS),
    descriptor(web_api_interfaces::SVGClipPathElement::DESCRIPTOR, ELEMENT_GROUPS),
    specialized_descriptor(
        web_api_interfaces::SVGCircleElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(web_api_interfaces::SVGFilterElement::DESCRIPTOR, ELEMENT_GROUPS),
    descriptor(
        web_api_interfaces::SVGDefsElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGDescElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGEllipseElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGFEConvolveMatrixElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGForeignObjectElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(web_api_interfaces::SVGGElement::DESCRIPTOR, ELEMENT_GROUPS),
    descriptor(
        web_api_interfaces::SVGImageElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGLineElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGGradientElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGLinearGradientElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: false,
            svg_pattern_transform: false,
            svg_gradient_transform: true,
        },
    ),
    descriptor(
        web_api_interfaces::SVGMetadataElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGScriptElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGStyleElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(web_api_interfaces::SVGMaskElement::DESCRIPTOR, ELEMENT_GROUPS),
    descriptor(web_api_interfaces::SVGMarkerElement::DESCRIPTOR, ELEMENT_GROUPS),
    specialized_descriptor(
        web_api_interfaces::SVGPathElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGPatternElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: false,
            svg_pattern_transform: true,
            svg_gradient_transform: false,
        },
    ),
    specialized_descriptor(
        web_api_interfaces::SVGPolygonElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGPolylineElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGRadialGradientElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: false,
            svg_pattern_transform: false,
            svg_gradient_transform: true,
        },
    ),
    descriptor(
        web_api_interfaces::SVGSVGElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGSymbolElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGTextContentElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGTextPositioningElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::SVGTextElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: true,
            svg_pattern_transform: false,
            svg_gradient_transform: false,
        },
    ),
    specialized_descriptor(
        web_api_interfaces::SVGTSpanElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: true,
            svg_pattern_transform: false,
            svg_gradient_transform: false,
        },
    ),
    descriptor(
        web_api_interfaces::SVGTitleElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::SVGUseElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(web_api_interfaces::SVGViewElement::DESCRIPTOR, ELEMENT_GROUPS),
    specialized_descriptor(
        web_api_interfaces::SVGRectElement::DESCRIPTOR,
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: true,
            svg_rect_animated_lengths: true,
            svg_text_positioning_lists: false,
            svg_pattern_transform: false,
            svg_gradient_transform: false,
        },
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: false,
            svg_pattern_transform: false,
            svg_gradient_transform: false,
        },
    ),
    descriptor(
        web_api_interfaces::HTMLUnknownElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLAnchorElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlAnchorElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLAreaElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLBaseElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLHtmlElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLHeadElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLBodyElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlBodyElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLBRElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLMediaElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlMediaElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLPictureElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLAudioElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlAudioElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLButtonElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlButtonElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLDetailsElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlDetailsElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLDialogElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlDialogElement,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: false,
            svg_pattern_transform: false,
            svg_gradient_transform: false,
        },
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLCanvasElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlCanvasElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLDataElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLDataListElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlDataListElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLDivElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLDirectoryElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLDListElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLEmbedElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLIFrameElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlIFrameElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLImageElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlImageElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLFontElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLFrameElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLFrameSetElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLHeadingElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLHRElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLLIElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLiElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLOListElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOListElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLOptGroupElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOptGroupElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLQuoteElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlQuoteElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLScriptElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlScriptElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLStyleElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlStyleElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTitleElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTitleElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTemplateElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTemplateElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTableCellElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableCellElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTimeElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTimeElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLInputElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlInputElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLSelectElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlSelectElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLOptionElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOptionElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTrackElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTrackElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTextAreaElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTextAreaElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLVideoElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlVideoElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLFieldSetElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlFieldSetElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLFormElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        // Form elements cannot safely reuse the plain HTMLElement template. The runtime exposes
        // form-specific surface such as branding, collection helpers, and readonly association
        // getters that downstream code probes via `Object.prototype.toString.call(...)`,
        // prototype checks, and specialized methods. Leaving this as a generic HTMLElement
        // descriptor causes the object to present the wrong brand (`[object HTMLElement]`) even
        // when the underlying DOM node is a real `<form>`, which is exactly the regression the
        // upstream select/form fixtures caught.
        SpecializedTemplateInstaller::HtmlFormElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLLegendElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLegendElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLLabelElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLabelElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLLinkElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLinkElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLMapElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLMarqueeElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLMenuElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLMetaElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlMetaElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLMeterElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlMeterElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLModElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLObjectElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlObjectElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLOutputElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOutputElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLParagraphElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLParamElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLPreElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLProgressElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlProgressElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLSlotElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        RuntimeInstallGroups {
            svg_geometry_path_length: false,
            svg_rect_animated_lengths: false,
            svg_text_positioning_lists: false,
            svg_pattern_transform: false,
            svg_gradient_transform: false,
        },
    ),
    descriptor(
        web_api_interfaces::HTMLSourceElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLSpanElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLTableCaptionElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLTableColElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTableElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTableRowElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableRowElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        web_api_interfaces::HTMLTableSectionElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableSectionElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLTitleElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::HTMLUListElement::DESCRIPTOR,
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        web_api_interfaces::MathMLElement::DESCRIPTOR,
        ELEMENT_GROUPS,
    ),
    descriptor(web_api_interfaces::Text::DESCRIPTOR, CHARACTER_DATA_GROUPS),
    descriptor(
        web_api_interfaces::Comment::DESCRIPTOR,
        CHARACTER_DATA_GROUPS,
    ),
    descriptor(
        web_api_interfaces::ProcessingInstruction::DESCRIPTOR,
        CHARACTER_DATA_GROUPS,
    ),
    descriptor(
        web_api_interfaces::CDATASection::DESCRIPTOR,
        CHARACTER_DATA_GROUPS,
    ),
];

pub(crate) fn node_bridge_descriptors() -> &'static [BridgeDescriptor] {
    NODE_BRIDGE_DESCRIPTORS
}

pub(crate) fn node_bridge_descriptor(name: &str) -> Option<&'static BridgeDescriptor> {
    NODE_BRIDGE_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.interface.name() == name)
}

#[cfg(test)]
mod tests {
    use super::{SpecializedTemplateInstaller, node_bridge_descriptor};

    #[test]
    fn descriptor_maps_specialized_template_installers() {
        assert_eq!(
            node_bridge_descriptor("ShadowRoot")
                .unwrap()
                .specialized_template_installer,
            SpecializedTemplateInstaller::ShadowRoot
        );
        assert_eq!(
            node_bridge_descriptor("HTMLAnchorElement")
                .unwrap()
                .specialized_template_installer,
            SpecializedTemplateInstaller::HtmlAnchorElement
        );
        assert_eq!(
            node_bridge_descriptor("HTMLDialogElement")
                .unwrap()
                .specialized_template_installer,
            SpecializedTemplateInstaller::HtmlDialogElement
        );
    }

    #[test]
    fn descriptor_maps_runtime_install_groups() {
        let html_element = node_bridge_descriptor("HTMLElement").unwrap();
        assert!(
            !html_element
                .runtime_install_groups
                .svg_rect_animated_lengths
        );
        assert!(
            !html_element
                .runtime_install_groups
                .svg_text_positioning_lists
        );

        let rect = node_bridge_descriptor("SVGRectElement").unwrap();
        assert!(rect.runtime_install_groups.svg_rect_animated_lengths);
        assert!(!rect.runtime_install_groups.svg_text_positioning_lists);

        let text = node_bridge_descriptor("SVGTextElement").unwrap();
        assert!(text.runtime_install_groups.svg_text_positioning_lists);
        assert!(!text.runtime_install_groups.svg_rect_animated_lengths);

        let pattern = node_bridge_descriptor("SVGPatternElement").unwrap();
        assert!(pattern.runtime_install_groups.svg_pattern_transform);
        assert!(!pattern.runtime_install_groups.svg_gradient_transform);
        assert!(!pattern.runtime_install_groups.svg_rect_animated_lengths);
        assert!(!pattern.runtime_install_groups.svg_text_positioning_lists);

        let linear_gradient = node_bridge_descriptor("SVGLinearGradientElement").unwrap();
        assert!(
            linear_gradient
                .runtime_install_groups
                .svg_gradient_transform
        );
        assert!(!linear_gradient.runtime_install_groups.svg_pattern_transform);
        assert!(
            !linear_gradient
                .runtime_install_groups
                .svg_rect_animated_lengths
        );
        assert!(
            !linear_gradient
                .runtime_install_groups
                .svg_text_positioning_lists
        );

        let radial_gradient = node_bridge_descriptor("SVGRadialGradientElement").unwrap();
        assert!(
            radial_gradient
                .runtime_install_groups
                .svg_gradient_transform
        );
        assert!(!radial_gradient.runtime_install_groups.svg_pattern_transform);
        assert!(
            !radial_gradient
                .runtime_install_groups
                .svg_rect_animated_lengths
        );
        assert!(
            !radial_gradient
                .runtime_install_groups
                .svg_text_positioning_lists
        );

        let dialog = node_bridge_descriptor("HTMLDialogElement").unwrap();
        assert!(!dialog.runtime_install_groups.svg_rect_animated_lengths);
        assert!(!dialog.runtime_install_groups.svg_text_positioning_lists);

        let slot = node_bridge_descriptor("HTMLSlotElement").unwrap();
        assert!(!slot.runtime_install_groups.svg_rect_animated_lengths);
        assert!(!slot.runtime_install_groups.svg_text_positioning_lists);
    }
}

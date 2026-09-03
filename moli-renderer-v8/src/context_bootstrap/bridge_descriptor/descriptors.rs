use super::{BridgeDescriptor, InstallGroups, RuntimeInstallGroups, SpecializedTemplateInstaller};

const fn descriptor(
    prototype_name: &'static str,
    parent_constructor: Option<&'static str>,
    install_groups: InstallGroups,
) -> BridgeDescriptor {
    specialized_descriptor(
        prototype_name,
        parent_constructor,
        install_groups,
        SpecializedTemplateInstaller::None,
        NONE_RUNTIME_INSTALL_GROUPS,
    )
}

const fn specialized_descriptor(
    prototype_name: &'static str,
    parent_constructor: Option<&'static str>,
    install_groups: InstallGroups,
    specialized_template_installer: SpecializedTemplateInstaller,
    runtime_install_groups: RuntimeInstallGroups,
) -> BridgeDescriptor {
    BridgeDescriptor {
        prototype_name,
        constructor_name: prototype_name,
        parent_constructor,
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
    descriptor("Node", Some("EventTarget"), BASE_GROUPS),
    descriptor("Document", Some("Node"), DOCUMENT_GROUPS),
    descriptor("HTMLDocument", Some("Document"), DOCUMENT_GROUPS),
    descriptor("XMLDocument", Some("Document"), DOCUMENT_GROUPS),
    descriptor("DocumentFragment", Some("Node"), MARKUP_CONTAINER_GROUPS),
    descriptor("DocumentType", Some("Node"), BASE_GROUPS),
    specialized_descriptor(
        "ShadowRoot",
        Some("DocumentFragment"),
        MARKUP_CONTAINER_GROUPS,
        SpecializedTemplateInstaller::ShadowRoot,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("Element", Some("Node"), ELEMENT_GROUPS),
    descriptor("SVGElement", Some("Element"), ELEMENT_GROUPS),
    descriptor("SVGGraphicsElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGGeometryElement",
        Some("SVGGraphicsElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGAElement", Some("SVGGraphicsElement"), ELEMENT_GROUPS),
    specialized_descriptor(
        "SVGCircleElement",
        Some("SVGGeometryElement"),
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("SVGClipPathElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGDefsElement", Some("SVGGraphicsElement"), ELEMENT_GROUPS),
    descriptor("SVGDescElement", Some("SVGElement"), ELEMENT_GROUPS),
    specialized_descriptor(
        "SVGEllipseElement",
        Some("SVGGeometryElement"),
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("SVGFilterElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGComponentTransferFunctionElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEFuncAElement",
        Some("SVGComponentTransferFunctionElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEFuncBElement",
        Some("SVGComponentTransferFunctionElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEFuncGElement",
        Some("SVGComponentTransferFunctionElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEFuncRElement",
        Some("SVGComponentTransferFunctionElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGFEBlendElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGFEColorMatrixElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGFECompositeElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGFEConvolveMatrixElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEDiffuseLightingElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEDisplacementMapElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGFEDistantLightElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGFEDropShadowElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGFEGaussianBlurElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGFEMorphologyElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGFEOffsetElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGFEPointLightElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGFESpecularLightingElement",
        Some("SVGElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGFESpotLightElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGFETurbulenceElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGForeignObjectElement",
        Some("SVGGraphicsElement"),
        ELEMENT_GROUPS,
    ),
    descriptor("SVGGElement", Some("SVGGraphicsElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGImageElement",
        Some("SVGGraphicsElement"),
        ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        "SVGLineElement",
        Some("SVGGeometryElement"),
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("SVGGradientElement", Some("SVGElement"), ELEMENT_GROUPS),
    specialized_descriptor(
        "SVGLinearGradientElement",
        Some("SVGGradientElement"),
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
    descriptor("SVGMarkerElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGMaskElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGMetadataElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGScriptElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGStopElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGStyleElement", Some("SVGElement"), ELEMENT_GROUPS),
    specialized_descriptor(
        "SVGPathElement",
        Some("SVGGeometryElement"),
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "SVGPatternElement",
        Some("SVGElement"),
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
        "SVGPolygonElement",
        Some("SVGGeometryElement"),
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "SVGPolylineElement",
        Some("SVGGeometryElement"),
        ELEMENT_GROUPS,
        SpecializedTemplateInstaller::None,
        SVG_GEOMETRY_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "SVGRadialGradientElement",
        Some("SVGGradientElement"),
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
    descriptor("SVGSVGElement", Some("SVGGraphicsElement"), ELEMENT_GROUPS),
    descriptor(
        "SVGSymbolElement",
        Some("SVGGraphicsElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGTextContentElement",
        Some("SVGGraphicsElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGTextPathElement",
        Some("SVGTextContentElement"),
        ELEMENT_GROUPS,
    ),
    descriptor(
        "SVGTextPositioningElement",
        Some("SVGTextContentElement"),
        ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        "SVGTextElement",
        Some("SVGTextPositioningElement"),
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
        "SVGTSpanElement",
        Some("SVGTextPositioningElement"),
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
    descriptor("SVGTitleElement", Some("SVGElement"), ELEMENT_GROUPS),
    descriptor("SVGUseElement", Some("SVGGraphicsElement"), ELEMENT_GROUPS),
    descriptor("SVGViewElement", Some("SVGElement"), ELEMENT_GROUPS),
    specialized_descriptor(
        "SVGRectElement",
        Some("SVGGeometryElement"),
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
        "HTMLElement",
        Some("Element"),
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
        "HTMLUnknownElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        "HTMLAnchorElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlAnchorElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLAreaElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLBaseElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLHtmlElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLHeadElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLBodyElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlBodyElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLBRElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLMediaElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlMediaElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        "HTMLPictureElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        "HTMLAudioElement",
        Some("HTMLMediaElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlAudioElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLButtonElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlButtonElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLDetailsElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlDetailsElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLDialogElement",
        Some("HTMLElement"),
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
        "HTMLCanvasElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlCanvasElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLDataElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLDataListElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlDataListElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLDivElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor(
        "HTMLDirectoryElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor("HTMLDListElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLEmbedElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLIFrameElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlIFrameElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLImageElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlImageElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLFontElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLFrameElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor(
        "HTMLFrameSetElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        "HTMLHeadingElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor("HTMLHRElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLLIElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLiElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLOListElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOListElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLOptGroupElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOptGroupElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLQuoteElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlQuoteElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLScriptElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlScriptElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLStyleElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlStyleElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTitleElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTitleElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTemplateElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTemplateElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTableCellElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableCellElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTimeElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTimeElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLInputElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlInputElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLSelectElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlSelectElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLOptionElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOptionElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTrackElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTrackElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTextAreaElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTextAreaElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLVideoElement",
        Some("HTMLMediaElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlVideoElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLFieldSetElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlFieldSetElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLFormElement",
        Some("HTMLElement"),
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
        "HTMLLegendElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLegendElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLLabelElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLabelElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLLinkElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlLinkElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLMapElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor(
        "HTMLMarqueeElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor("HTMLMenuElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLMetaElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlMetaElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLMeterElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlMeterElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLModElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLObjectElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlObjectElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLOutputElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlOutputElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor(
        "HTMLParagraphElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor("HTMLParamElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLPreElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    specialized_descriptor(
        "HTMLProgressElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlProgressElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLSlotElement",
        Some("HTMLElement"),
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
        "HTMLSourceElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor("HTMLSpanElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor(
        "HTMLTableCaptionElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    descriptor(
        "HTMLTableColElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTableElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTableRowElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableRowElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    specialized_descriptor(
        "HTMLTableSectionElement",
        Some("HTMLElement"),
        HTML_ELEMENT_GROUPS,
        SpecializedTemplateInstaller::HtmlTableSectionElement,
        NONE_RUNTIME_INSTALL_GROUPS,
    ),
    descriptor("HTMLTitleElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("HTMLUListElement", Some("HTMLElement"), HTML_ELEMENT_GROUPS),
    descriptor("MathMLElement", Some("Element"), ELEMENT_GROUPS),
    descriptor("Text", Some("CharacterData"), CHARACTER_DATA_GROUPS),
    descriptor("Comment", Some("CharacterData"), CHARACTER_DATA_GROUPS),
    descriptor(
        "ProcessingInstruction",
        Some("CharacterData"),
        CHARACTER_DATA_GROUPS,
    ),
    descriptor("CDATASection", Some("Text"), CHARACTER_DATA_GROUPS),
];

pub(crate) fn node_bridge_descriptors() -> &'static [BridgeDescriptor] {
    NODE_BRIDGE_DESCRIPTORS
}

pub(crate) fn node_bridge_descriptor(name: &str) -> Option<&'static BridgeDescriptor> {
    NODE_BRIDGE_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.prototype_name == name)
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

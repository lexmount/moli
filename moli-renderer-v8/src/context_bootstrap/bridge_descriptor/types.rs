#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WrapperKind {
    Window,
    Node,
    ClassList,
    Dataset,
    Style,
    ComputedStyle,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InstallGroups {
    pub(crate) character_data_api: bool,
    pub(crate) markup_container_api: bool,
    pub(crate) document_methods: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SpecializedTemplateInstaller {
    #[default]
    None,
    ShadowRoot,
    HtmlAnchorElement,
    HtmlScriptElement,
    HtmlInputElement,
    HtmlButtonElement,
    HtmlMediaElement,
    HtmlAudioElement,
    HtmlVideoElement,
    HtmlStyleElement,
    HtmlTitleElement,
    HtmlTemplateElement,
    HtmlDataListElement,
    HtmlFormElement,
    HtmlFieldSetElement,
    HtmlLegendElement,
    HtmlLabelElement,
    HtmlLinkElement,
    HtmlLiElement,
    HtmlOListElement,
    HtmlOptGroupElement,
    HtmlQuoteElement,
    HtmlTableElement,
    HtmlTableCellElement,
    HtmlTableRowElement,
    HtmlTableSectionElement,
    HtmlTextAreaElement,
    HtmlOptionElement,
    HtmlSelectElement,
    HtmlBodyElement,
    HtmlDetailsElement,
    HtmlDialogElement,
    HtmlCanvasElement,
    HtmlIFrameElement,
    HtmlTrackElement,
    HtmlImageElement,
    HtmlTimeElement,
    HtmlMetaElement,
    HtmlMeterElement,
    HtmlObjectElement,
    HtmlOutputElement,
    HtmlProgressElement,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RuntimeInstallGroups {
    pub(crate) svg_geometry_path_length: bool,
    pub(crate) svg_rect_animated_lengths: bool,
    pub(crate) svg_text_positioning_lists: bool,
    pub(crate) svg_pattern_transform: bool,
    pub(crate) svg_gradient_transform: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BridgeDescriptor {
    pub(crate) interface: moli_webapi_declare::WebApiInterfaceDescriptor,
    pub(crate) install_groups: InstallGroups,
    pub(crate) specialized_template_installer: SpecializedTemplateInstaller,
    pub(crate) runtime_install_groups: RuntimeInstallGroups,
}

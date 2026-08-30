use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::util::v8_string;
use crate::webidl;

use super::super::node::{node_is_element, node_runtime_and_handle_from_object_or_detached};
use super::{
    element_attribute, element_has_attribute,
    remove_live_element_attribute_appending_to_current_reaction_queue,
    set_live_element_attribute_appending_to_current_reaction_queue,
};

macro_rules! impl_reflection_callback_data {
    ($ty:ty) => {
        impl<'s> moli_webapi_declare::WebApiValue<'s> for $ty {
            fn to_v8_value(
                &self,
                scope: &mut v8::PinScope<'s, '_>,
            ) -> Option<v8::Local<'s, v8::Value>> {
                Some(v8::Integer::new_from_unsigned(scope, *self as u32).into())
            }
        }

        impl<'s> moli_webapi_declare::WebApiTemplateValue<'s> for $ty {
            fn to_v8_template_value(
                &self,
                scope: &mut v8::PinScope<'s, '_, ()>,
            ) -> Option<v8::Local<'s, v8::Value>> {
                Some(v8::Integer::new_from_unsigned(scope, *self as u32).into())
            }
        }
    };
}

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum ElementReflectionInterface {
    HtmlAnchorElement,
    HtmlAreaElement,
    HtmlButtonElement,
    HtmlDetailsElement,
    HtmlDivElement,
    HtmlEmbedElement,
    HtmlFieldSetElement,
    HtmlFormElement,
    HtmlFrameElement,
    HtmlHeadingElement,
    HtmlHrElement,
    HtmlIFrameElement,
    HtmlImageElement,
    HtmlInputElement,
    HtmlLegendElement,
    HtmlLinkElement,
    HtmlMapElement,
    HtmlMetaElement,
    HtmlObjectElement,
    HtmlOutputElement,
    HtmlParagraphElement,
    HtmlParamElement,
    HtmlSelectElement,
    HtmlTableCaptionElement,
    HtmlTableCellElement,
    HtmlTableColElement,
    HtmlTableElement,
    HtmlTableRowElement,
    HtmlTableSectionElement,
    HtmlTextAreaElement,
    Count,
}

impl ElementReflectionInterface {
    const ALL: [Self; Self::Count as usize] = [
        Self::HtmlAnchorElement,
        Self::HtmlAreaElement,
        Self::HtmlButtonElement,
        Self::HtmlDetailsElement,
        Self::HtmlDivElement,
        Self::HtmlEmbedElement,
        Self::HtmlFieldSetElement,
        Self::HtmlFormElement,
        Self::HtmlFrameElement,
        Self::HtmlHeadingElement,
        Self::HtmlHrElement,
        Self::HtmlIFrameElement,
        Self::HtmlImageElement,
        Self::HtmlInputElement,
        Self::HtmlLegendElement,
        Self::HtmlLinkElement,
        Self::HtmlMapElement,
        Self::HtmlMetaElement,
        Self::HtmlObjectElement,
        Self::HtmlOutputElement,
        Self::HtmlParagraphElement,
        Self::HtmlParamElement,
        Self::HtmlSelectElement,
        Self::HtmlTableCaptionElement,
        Self::HtmlTableCellElement,
        Self::HtmlTableColElement,
        Self::HtmlTableElement,
        Self::HtmlTableRowElement,
        Self::HtmlTableSectionElement,
        Self::HtmlTextAreaElement,
    ];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::HtmlAnchorElement => "HTMLAnchorElement",
            Self::HtmlAreaElement => "HTMLAreaElement",
            Self::HtmlButtonElement => "HTMLButtonElement",
            Self::HtmlDetailsElement => "HTMLDetailsElement",
            Self::HtmlDivElement => "HTMLDivElement",
            Self::HtmlEmbedElement => "HTMLEmbedElement",
            Self::HtmlFieldSetElement => "HTMLFieldSetElement",
            Self::HtmlFormElement => "HTMLFormElement",
            Self::HtmlFrameElement => "HTMLFrameElement",
            Self::HtmlHeadingElement => "HTMLHeadingElement",
            Self::HtmlHrElement => "HTMLHRElement",
            Self::HtmlIFrameElement => "HTMLIFrameElement",
            Self::HtmlImageElement => "HTMLImageElement",
            Self::HtmlInputElement => "HTMLInputElement",
            Self::HtmlLegendElement => "HTMLLegendElement",
            Self::HtmlLinkElement => "HTMLLinkElement",
            Self::HtmlMapElement => "HTMLMapElement",
            Self::HtmlMetaElement => "HTMLMetaElement",
            Self::HtmlObjectElement => "HTMLObjectElement",
            Self::HtmlOutputElement => "HTMLOutputElement",
            Self::HtmlParagraphElement => "HTMLParagraphElement",
            Self::HtmlParamElement => "HTMLParamElement",
            Self::HtmlSelectElement => "HTMLSelectElement",
            Self::HtmlTableCaptionElement => "HTMLTableCaptionElement",
            Self::HtmlTableCellElement => "HTMLTableCellElement",
            Self::HtmlTableColElement => "HTMLTableColElement",
            Self::HtmlTableElement => "HTMLTableElement",
            Self::HtmlTableRowElement => "HTMLTableRowElement",
            Self::HtmlTableSectionElement => "HTMLTableSectionElement",
            Self::HtmlTextAreaElement => "HTMLTextAreaElement",
            Self::Count => panic!("ElementReflectionInterface::Count is not an interface"),
        }
    }

    pub(super) fn from_callback_data(
        scope: &mut v8::PinScope<'_, '_>,
        data: v8::Local<'_, v8::Value>,
    ) -> Option<Self> {
        Self::ALL.get(data.uint32_value(scope)? as usize).copied()
    }
}

const _: () = {
    assert!(ElementReflectionInterface::ALL.len() == ElementReflectionInterface::Count as usize);
    let mut index = 0;
    while index < ElementReflectionInterface::ALL.len() {
        assert!(ElementReflectionInterface::ALL[index] as usize == index);
        index += 1;
    }
};

impl_reflection_callback_data!(ElementReflectionInterface);

// Reflection declarations pass these compact discriminants as V8 callback data.
// Every table row repeats its enum key, and the const checks below keep key and
// descriptor order aligned without generated wrapper functions or raw pointers.
#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum DomStringReflection {
    AnchorCharset,
    AnchorCoords,
    AnchorDownload,
    AnchorHreflang,
    AnchorReferrerPolicy,
    AnchorRev,
    AnchorShape,
    AreaAlt,
    AreaCoords,
    AreaDownload,
    AreaReferrerPolicy,
    AreaShape,
    BrClear,
    DataValue,
    EmbedHeight,
    EmbedType,
    EmbedWidth,
    FontSize,
    FrameFrameBorder,
    FrameScrolling,
    HrColor,
    HrSize,
    HrWidth,
    HtmlTimeDateTime,
    HtmlVersion,
    IframeFrameBorder,
    IframeHeight,
    IframeLoading,
    IframeReferrerPolicy,
    IframeScrolling,
    IframeWidth,
    ImageAlt,
    ImageFetchPriority,
    ImageReferrerPolicy,
    ImageSizes,
    ImageUseMap,
    InputUseMap,
    LinkAs,
    LinkCharset,
    LinkFetchPriority,
    LinkHreflang,
    LinkMedia,
    LinkReferrerPolicy,
    LiType,
    MarqueeBgColor,
    MarqueeHeight,
    MarqueeWidth,
    MetaMedia,
    ModDateTime,
    ObjectArchive,
    ObjectCode,
    ObjectCodeType,
    ObjectHeight,
    ObjectStandby,
    ObjectType,
    ObjectUseMap,
    ObjectWidth,
    OptgroupLabel,
    ParamType,
    ParamValue,
    ParamValueType,
    ScriptCharset,
    ScriptFetchPriority,
    ScriptReferrerPolicy,
    SourceMedia,
    SourceSizes,
    SourceType,
    StyleMedia,
    StyleType,
    TableBorder,
    TableCellAbbr,
    TableCellAxis,
    TableCellCh,
    TableCellChOff,
    TableCellHeaders,
    TableCellHeight,
    TableCellScope,
    TableCellVAlign,
    TableCellWidth,
    TableColCh,
    TableColChOff,
    TableColVAlign,
    TableColWidth,
    TableRowCh,
    TableRowChOff,
    TableRowVAlign,
    TableSectionCh,
    TableSectionChOff,
    TableSectionVAlign,
    TableWidth,
    TrackLabel,
    UlType,
    Count,
}

pub(super) struct DomStringReflectionDescriptor {
    pub(super) interface: &'static str,
    /// Exact HTML local name when the shared callback owns receiver brand checks.
    ///
    /// Older table entries still use family-specific getters, so their shared
    /// setter keeps the existing receiver path and leaves this unset.
    pub(super) html_local_name: Option<&'static str>,
    pub(super) attribute: &'static str,
    pub(super) member: &'static str,
}

impl DomStringReflectionDescriptor {
    const fn new(interface: &'static str, attribute: &'static str, member: &'static str) -> Self {
        Self {
            interface,
            html_local_name: None,
            attribute,
            member,
        }
    }

    const fn new_html_element(
        interface: &'static str,
        local_name: &'static str,
        attribute: &'static str,
        member: &'static str,
    ) -> Self {
        Self {
            interface,
            html_local_name: Some(local_name),
            attribute,
            member,
        }
    }
}

type ReflectedAttributeDescriptor = DomStringReflectionDescriptor;

const DOM_STRING_REFLECTION_DESCRIPTORS: &[(DomStringReflection, DomStringReflectionDescriptor)] =
    &[
        (
            DomStringReflection::AnchorCharset,
            DomStringReflectionDescriptor::new("HTMLAnchorElement", "charset", "charset"),
        ),
        (
            DomStringReflection::AnchorCoords,
            DomStringReflectionDescriptor::new("HTMLAnchorElement", "coords", "coords"),
        ),
        (
            DomStringReflection::AnchorDownload,
            DomStringReflectionDescriptor::new("HTMLAnchorElement", "download", "download"),
        ),
        (
            DomStringReflection::AnchorHreflang,
            DomStringReflectionDescriptor::new("HTMLAnchorElement", "hreflang", "hreflang"),
        ),
        (
            DomStringReflection::AnchorReferrerPolicy,
            DomStringReflectionDescriptor::new(
                "HTMLAnchorElement",
                "referrerpolicy",
                "referrerPolicy",
            ),
        ),
        (
            DomStringReflection::AnchorRev,
            DomStringReflectionDescriptor::new_html_element("HTMLAnchorElement", "a", "rev", "rev"),
        ),
        (
            DomStringReflection::AnchorShape,
            DomStringReflectionDescriptor::new("HTMLAnchorElement", "shape", "shape"),
        ),
        (
            DomStringReflection::AreaAlt,
            DomStringReflectionDescriptor::new("HTMLAreaElement", "alt", "alt"),
        ),
        (
            DomStringReflection::AreaCoords,
            DomStringReflectionDescriptor::new("HTMLAreaElement", "coords", "coords"),
        ),
        (
            DomStringReflection::AreaDownload,
            DomStringReflectionDescriptor::new("HTMLAreaElement", "download", "download"),
        ),
        (
            DomStringReflection::AreaReferrerPolicy,
            DomStringReflectionDescriptor::new(
                "HTMLAreaElement",
                "referrerpolicy",
                "referrerPolicy",
            ),
        ),
        (
            DomStringReflection::AreaShape,
            DomStringReflectionDescriptor::new("HTMLAreaElement", "shape", "shape"),
        ),
        (
            DomStringReflection::BrClear,
            DomStringReflectionDescriptor::new_html_element(
                "HTMLBRElement",
                "br",
                "clear",
                "clear",
            ),
        ),
        (
            DomStringReflection::DataValue,
            DomStringReflectionDescriptor::new("HTMLDataElement", "value", "value"),
        ),
        (
            DomStringReflection::EmbedHeight,
            DomStringReflectionDescriptor::new("HTMLEmbedElement", "height", "height"),
        ),
        (
            DomStringReflection::EmbedType,
            DomStringReflectionDescriptor::new("HTMLEmbedElement", "type", "type"),
        ),
        (
            DomStringReflection::EmbedWidth,
            DomStringReflectionDescriptor::new("HTMLEmbedElement", "width", "width"),
        ),
        (
            DomStringReflection::FontSize,
            DomStringReflectionDescriptor::new("HTMLFontElement", "size", "size"),
        ),
        (
            DomStringReflection::FrameFrameBorder,
            DomStringReflectionDescriptor::new("HTMLFrameElement", "frameborder", "frameBorder"),
        ),
        (
            DomStringReflection::FrameScrolling,
            DomStringReflectionDescriptor::new("HTMLFrameElement", "scrolling", "scrolling"),
        ),
        (
            DomStringReflection::HrColor,
            DomStringReflectionDescriptor::new("HTMLHRElement", "color", "color"),
        ),
        (
            DomStringReflection::HrSize,
            DomStringReflectionDescriptor::new("HTMLHRElement", "size", "size"),
        ),
        (
            DomStringReflection::HrWidth,
            DomStringReflectionDescriptor::new("HTMLHRElement", "width", "width"),
        ),
        (
            DomStringReflection::HtmlTimeDateTime,
            DomStringReflectionDescriptor::new("HTMLTimeElement", "datetime", "dateTime"),
        ),
        (
            DomStringReflection::HtmlVersion,
            DomStringReflectionDescriptor::new("HTMLHtmlElement", "version", "version"),
        ),
        (
            DomStringReflection::IframeFrameBorder,
            DomStringReflectionDescriptor::new("HTMLIFrameElement", "frameborder", "frameBorder"),
        ),
        (
            DomStringReflection::IframeHeight,
            DomStringReflectionDescriptor::new("HTMLIFrameElement", "height", "height"),
        ),
        (
            DomStringReflection::IframeLoading,
            DomStringReflectionDescriptor::new("HTMLIFrameElement", "loading", "loading"),
        ),
        (
            DomStringReflection::IframeReferrerPolicy,
            DomStringReflectionDescriptor::new(
                "HTMLIFrameElement",
                "referrerpolicy",
                "referrerPolicy",
            ),
        ),
        (
            DomStringReflection::IframeScrolling,
            DomStringReflectionDescriptor::new("HTMLIFrameElement", "scrolling", "scrolling"),
        ),
        (
            DomStringReflection::IframeWidth,
            DomStringReflectionDescriptor::new("HTMLIFrameElement", "width", "width"),
        ),
        (
            DomStringReflection::ImageAlt,
            DomStringReflectionDescriptor::new("HTMLImageElement", "alt", "alt"),
        ),
        (
            DomStringReflection::ImageFetchPriority,
            DomStringReflectionDescriptor::new_html_element(
                "HTMLImageElement",
                "img",
                "fetchpriority",
                "fetchPriority",
            ),
        ),
        (
            DomStringReflection::ImageReferrerPolicy,
            DomStringReflectionDescriptor::new(
                "HTMLImageElement",
                "referrerpolicy",
                "referrerPolicy",
            ),
        ),
        (
            DomStringReflection::ImageSizes,
            DomStringReflectionDescriptor::new("HTMLImageElement", "sizes", "sizes"),
        ),
        (
            DomStringReflection::ImageUseMap,
            DomStringReflectionDescriptor::new("HTMLImageElement", "usemap", "useMap"),
        ),
        (
            DomStringReflection::InputUseMap,
            DomStringReflectionDescriptor::new("HTMLInputElement", "usemap", "useMap"),
        ),
        (
            DomStringReflection::LinkAs,
            DomStringReflectionDescriptor::new("HTMLLinkElement", "as", "as"),
        ),
        (
            DomStringReflection::LinkCharset,
            DomStringReflectionDescriptor::new("HTMLLinkElement", "charset", "charset"),
        ),
        (
            DomStringReflection::LinkFetchPriority,
            DomStringReflectionDescriptor::new_html_element(
                "HTMLLinkElement",
                "link",
                "fetchpriority",
                "fetchPriority",
            ),
        ),
        (
            DomStringReflection::LinkHreflang,
            DomStringReflectionDescriptor::new("HTMLLinkElement", "hreflang", "hreflang"),
        ),
        (
            DomStringReflection::LinkMedia,
            DomStringReflectionDescriptor::new("HTMLLinkElement", "media", "media"),
        ),
        (
            DomStringReflection::LinkReferrerPolicy,
            DomStringReflectionDescriptor::new(
                "HTMLLinkElement",
                "referrerpolicy",
                "referrerPolicy",
            ),
        ),
        (
            DomStringReflection::LiType,
            DomStringReflectionDescriptor::new("HTMLLIElement", "type", "type"),
        ),
        (
            DomStringReflection::MarqueeBgColor,
            DomStringReflectionDescriptor::new("HTMLMarqueeElement", "bgcolor", "bgColor"),
        ),
        (
            DomStringReflection::MarqueeHeight,
            DomStringReflectionDescriptor::new("HTMLMarqueeElement", "height", "height"),
        ),
        (
            DomStringReflection::MarqueeWidth,
            DomStringReflectionDescriptor::new("HTMLMarqueeElement", "width", "width"),
        ),
        (
            DomStringReflection::MetaMedia,
            DomStringReflectionDescriptor::new("HTMLMetaElement", "media", "media"),
        ),
        (
            DomStringReflection::ModDateTime,
            DomStringReflectionDescriptor::new("HTMLModElement", "datetime", "dateTime"),
        ),
        (
            DomStringReflection::ObjectArchive,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "archive", "archive"),
        ),
        (
            DomStringReflection::ObjectCode,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "code", "code"),
        ),
        (
            DomStringReflection::ObjectCodeType,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "codetype", "codeType"),
        ),
        (
            DomStringReflection::ObjectHeight,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "height", "height"),
        ),
        (
            DomStringReflection::ObjectStandby,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "standby", "standby"),
        ),
        (
            DomStringReflection::ObjectType,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "type", "type"),
        ),
        (
            DomStringReflection::ObjectUseMap,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "usemap", "useMap"),
        ),
        (
            DomStringReflection::ObjectWidth,
            DomStringReflectionDescriptor::new("HTMLObjectElement", "width", "width"),
        ),
        (
            DomStringReflection::OptgroupLabel,
            DomStringReflectionDescriptor::new("HTMLOptGroupElement", "label", "label"),
        ),
        (
            DomStringReflection::ParamType,
            DomStringReflectionDescriptor::new("HTMLParamElement", "type", "type"),
        ),
        (
            DomStringReflection::ParamValue,
            DomStringReflectionDescriptor::new("HTMLParamElement", "value", "value"),
        ),
        (
            DomStringReflection::ParamValueType,
            DomStringReflectionDescriptor::new("HTMLParamElement", "valuetype", "valueType"),
        ),
        (
            DomStringReflection::ScriptCharset,
            DomStringReflectionDescriptor::new("HTMLScriptElement", "charset", "charset"),
        ),
        (
            DomStringReflection::ScriptFetchPriority,
            DomStringReflectionDescriptor::new_html_element(
                "HTMLScriptElement",
                "script",
                "fetchpriority",
                "fetchPriority",
            ),
        ),
        (
            DomStringReflection::ScriptReferrerPolicy,
            DomStringReflectionDescriptor::new(
                "HTMLScriptElement",
                "referrerpolicy",
                "referrerPolicy",
            ),
        ),
        (
            DomStringReflection::SourceMedia,
            DomStringReflectionDescriptor::new("HTMLSourceElement", "media", "media"),
        ),
        (
            DomStringReflection::SourceSizes,
            DomStringReflectionDescriptor::new("HTMLSourceElement", "sizes", "sizes"),
        ),
        (
            DomStringReflection::SourceType,
            DomStringReflectionDescriptor::new("HTMLSourceElement", "type", "type"),
        ),
        (
            DomStringReflection::StyleMedia,
            DomStringReflectionDescriptor::new("HTMLStyleElement", "media", "media"),
        ),
        (
            DomStringReflection::StyleType,
            DomStringReflectionDescriptor::new_html_element(
                "HTMLStyleElement",
                "style",
                "type",
                "type",
            ),
        ),
        (
            DomStringReflection::TableBorder,
            DomStringReflectionDescriptor::new("HTMLTableElement", "border", "border"),
        ),
        (
            DomStringReflection::TableCellAbbr,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "abbr", "abbr"),
        ),
        (
            DomStringReflection::TableCellAxis,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "axis", "axis"),
        ),
        (
            DomStringReflection::TableCellCh,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "char", "ch"),
        ),
        (
            DomStringReflection::TableCellChOff,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "charoff", "chOff"),
        ),
        (
            DomStringReflection::TableCellHeaders,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "headers", "headers"),
        ),
        (
            DomStringReflection::TableCellHeight,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "height", "height"),
        ),
        (
            DomStringReflection::TableCellScope,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "scope", "scope"),
        ),
        (
            DomStringReflection::TableCellVAlign,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "valign", "vAlign"),
        ),
        (
            DomStringReflection::TableCellWidth,
            DomStringReflectionDescriptor::new("HTMLTableCellElement", "width", "width"),
        ),
        (
            DomStringReflection::TableColCh,
            DomStringReflectionDescriptor::new("HTMLTableColElement", "char", "ch"),
        ),
        (
            DomStringReflection::TableColChOff,
            DomStringReflectionDescriptor::new("HTMLTableColElement", "charoff", "chOff"),
        ),
        (
            DomStringReflection::TableColVAlign,
            DomStringReflectionDescriptor::new("HTMLTableColElement", "valign", "vAlign"),
        ),
        (
            DomStringReflection::TableColWidth,
            DomStringReflectionDescriptor::new("HTMLTableColElement", "width", "width"),
        ),
        (
            DomStringReflection::TableRowCh,
            DomStringReflectionDescriptor::new("HTMLTableRowElement", "char", "ch"),
        ),
        (
            DomStringReflection::TableRowChOff,
            DomStringReflectionDescriptor::new("HTMLTableRowElement", "charoff", "chOff"),
        ),
        (
            DomStringReflection::TableRowVAlign,
            DomStringReflectionDescriptor::new("HTMLTableRowElement", "valign", "vAlign"),
        ),
        (
            DomStringReflection::TableSectionCh,
            DomStringReflectionDescriptor::new("HTMLTableSectionElement", "char", "ch"),
        ),
        (
            DomStringReflection::TableSectionChOff,
            DomStringReflectionDescriptor::new("HTMLTableSectionElement", "charoff", "chOff"),
        ),
        (
            DomStringReflection::TableSectionVAlign,
            DomStringReflectionDescriptor::new("HTMLTableSectionElement", "valign", "vAlign"),
        ),
        (
            DomStringReflection::TableWidth,
            DomStringReflectionDescriptor::new("HTMLTableElement", "width", "width"),
        ),
        (
            DomStringReflection::TrackLabel,
            DomStringReflectionDescriptor::new("HTMLTrackElement", "label", "label"),
        ),
        (
            DomStringReflection::UlType,
            DomStringReflectionDescriptor::new("HTMLUListElement", "type", "type"),
        ),
    ];

const _: () = {
    assert!(DOM_STRING_REFLECTION_DESCRIPTORS.len() == DomStringReflection::Count as usize);
    let mut index = 0;
    while index < DOM_STRING_REFLECTION_DESCRIPTORS.len() {
        assert!(DOM_STRING_REFLECTION_DESCRIPTORS[index].0 as usize == index);
        index += 1;
    }
};

impl DomStringReflection {
    pub(super) fn descriptor_from_callback_data(
        scope: &mut v8::PinScope<'_, '_>,
        data: v8::Local<'_, v8::Value>,
    ) -> Option<&'static DomStringReflectionDescriptor> {
        DOM_STRING_REFLECTION_DESCRIPTORS
            .get(data.uint32_value(scope)? as usize)
            .map(|(_, descriptor)| descriptor)
    }
}

impl_reflection_callback_data!(DomStringReflection);

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum UsvStringReflection {
    AnchorPing,
    AreaPing,
    FrameLongDesc,
    IframeLongDesc,
    ModCite,
    ObjectCodeBase,
    ObjectData,
    QuoteCite,
    Count,
}

const USV_STRING_REFLECTION_DESCRIPTORS: &[(UsvStringReflection, ReflectedAttributeDescriptor)] = &[
    (
        UsvStringReflection::AnchorPing,
        ReflectedAttributeDescriptor::new("HTMLAnchorElement", "ping", "ping"),
    ),
    (
        UsvStringReflection::AreaPing,
        ReflectedAttributeDescriptor::new("HTMLAreaElement", "ping", "ping"),
    ),
    (
        UsvStringReflection::FrameLongDesc,
        ReflectedAttributeDescriptor::new("HTMLFrameElement", "longdesc", "longDesc"),
    ),
    (
        UsvStringReflection::IframeLongDesc,
        ReflectedAttributeDescriptor::new("HTMLIFrameElement", "longdesc", "longDesc"),
    ),
    (
        UsvStringReflection::ModCite,
        ReflectedAttributeDescriptor::new("HTMLModElement", "cite", "cite"),
    ),
    (
        UsvStringReflection::ObjectCodeBase,
        ReflectedAttributeDescriptor::new("HTMLObjectElement", "codebase", "codeBase"),
    ),
    (
        UsvStringReflection::ObjectData,
        ReflectedAttributeDescriptor::new("HTMLObjectElement", "data", "data"),
    ),
    (
        UsvStringReflection::QuoteCite,
        ReflectedAttributeDescriptor::new("HTMLQuoteElement", "cite", "cite"),
    ),
];

const _: () = {
    assert!(USV_STRING_REFLECTION_DESCRIPTORS.len() == UsvStringReflection::Count as usize);
    let mut index = 0;
    while index < USV_STRING_REFLECTION_DESCRIPTORS.len() {
        assert!(USV_STRING_REFLECTION_DESCRIPTORS[index].0 as usize == index);
        index += 1;
    }
};

impl UsvStringReflection {
    pub(super) fn descriptor_from_callback_data(
        scope: &mut v8::PinScope<'_, '_>,
        data: v8::Local<'_, v8::Value>,
    ) -> Option<&'static ReflectedAttributeDescriptor> {
        USV_STRING_REFLECTION_DESCRIPTORS
            .get(data.uint32_value(scope)? as usize)
            .map(|(_, descriptor)| descriptor)
    }
}

impl_reflection_callback_data!(UsvStringReflection);

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum NullToEmptyDomStringReflection {
    BodyBgColor,
    FontColor,
    FrameMarginHeight,
    FrameMarginWidth,
    IframeMarginHeight,
    IframeMarginWidth,
    ImageBorder,
    ObjectBorder,
    TableBgColor,
    TableCellBgColor,
    TableRowBgColor,
    Count,
}

const NULL_TO_EMPTY_DOM_STRING_REFLECTION_DESCRIPTORS: &[(
    NullToEmptyDomStringReflection,
    ReflectedAttributeDescriptor,
)] = &[
    (
        NullToEmptyDomStringReflection::BodyBgColor,
        ReflectedAttributeDescriptor::new("HTMLBodyElement", "bgcolor", "bgColor"),
    ),
    (
        NullToEmptyDomStringReflection::FontColor,
        ReflectedAttributeDescriptor::new("HTMLFontElement", "color", "color"),
    ),
    (
        NullToEmptyDomStringReflection::FrameMarginHeight,
        ReflectedAttributeDescriptor::new("HTMLFrameElement", "marginheight", "marginHeight"),
    ),
    (
        NullToEmptyDomStringReflection::FrameMarginWidth,
        ReflectedAttributeDescriptor::new("HTMLFrameElement", "marginwidth", "marginWidth"),
    ),
    (
        NullToEmptyDomStringReflection::IframeMarginHeight,
        ReflectedAttributeDescriptor::new("HTMLIFrameElement", "marginheight", "marginHeight"),
    ),
    (
        NullToEmptyDomStringReflection::IframeMarginWidth,
        ReflectedAttributeDescriptor::new("HTMLIFrameElement", "marginwidth", "marginWidth"),
    ),
    (
        NullToEmptyDomStringReflection::ImageBorder,
        ReflectedAttributeDescriptor::new("HTMLImageElement", "border", "border"),
    ),
    (
        NullToEmptyDomStringReflection::ObjectBorder,
        ReflectedAttributeDescriptor::new("HTMLObjectElement", "border", "border"),
    ),
    (
        NullToEmptyDomStringReflection::TableBgColor,
        ReflectedAttributeDescriptor::new("HTMLTableElement", "bgcolor", "bgColor"),
    ),
    (
        NullToEmptyDomStringReflection::TableCellBgColor,
        ReflectedAttributeDescriptor::new("HTMLTableCellElement", "bgcolor", "bgColor"),
    ),
    (
        NullToEmptyDomStringReflection::TableRowBgColor,
        ReflectedAttributeDescriptor::new("HTMLTableRowElement", "bgcolor", "bgColor"),
    ),
];

const _: () = {
    assert!(
        NULL_TO_EMPTY_DOM_STRING_REFLECTION_DESCRIPTORS.len()
            == NullToEmptyDomStringReflection::Count as usize
    );
    let mut index = 0;
    while index < NULL_TO_EMPTY_DOM_STRING_REFLECTION_DESCRIPTORS.len() {
        assert!(NULL_TO_EMPTY_DOM_STRING_REFLECTION_DESCRIPTORS[index].0 as usize == index);
        index += 1;
    }
};

impl NullToEmptyDomStringReflection {
    pub(super) fn descriptor_from_callback_data(
        scope: &mut v8::PinScope<'_, '_>,
        data: v8::Local<'_, v8::Value>,
    ) -> Option<&'static ReflectedAttributeDescriptor> {
        NULL_TO_EMPTY_DOM_STRING_REFLECTION_DESCRIPTORS
            .get(data.uint32_value(scope)? as usize)
            .map(|(_, descriptor)| descriptor)
    }
}

impl_reflection_callback_data!(NullToEmptyDomStringReflection);

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum UnsignedLongReflection {
    ImageHspace,
    ImageVspace,
    MarqueeHspace,
    MarqueeVspace,
    ObjectHspace,
    ObjectVspace,
    Count,
}

const UNSIGNED_LONG_REFLECTION_DESCRIPTORS: &[(
    UnsignedLongReflection,
    ReflectedAttributeDescriptor,
)] = &[
    (
        UnsignedLongReflection::ImageHspace,
        ReflectedAttributeDescriptor::new("HTMLImageElement", "hspace", "hspace"),
    ),
    (
        UnsignedLongReflection::ImageVspace,
        ReflectedAttributeDescriptor::new("HTMLImageElement", "vspace", "vspace"),
    ),
    (
        UnsignedLongReflection::MarqueeHspace,
        ReflectedAttributeDescriptor::new("HTMLMarqueeElement", "hspace", "hspace"),
    ),
    (
        UnsignedLongReflection::MarqueeVspace,
        ReflectedAttributeDescriptor::new("HTMLMarqueeElement", "vspace", "vspace"),
    ),
    (
        UnsignedLongReflection::ObjectHspace,
        ReflectedAttributeDescriptor::new("HTMLObjectElement", "hspace", "hspace"),
    ),
    (
        UnsignedLongReflection::ObjectVspace,
        ReflectedAttributeDescriptor::new("HTMLObjectElement", "vspace", "vspace"),
    ),
];

const _: () = {
    assert!(UNSIGNED_LONG_REFLECTION_DESCRIPTORS.len() == UnsignedLongReflection::Count as usize);
    let mut index = 0;
    while index < UNSIGNED_LONG_REFLECTION_DESCRIPTORS.len() {
        assert!(UNSIGNED_LONG_REFLECTION_DESCRIPTORS[index].0 as usize == index);
        index += 1;
    }
};

impl UnsignedLongReflection {
    pub(super) fn descriptor_from_callback_data(
        scope: &mut v8::PinScope<'_, '_>,
        data: v8::Local<'_, v8::Value>,
    ) -> Option<&'static ReflectedAttributeDescriptor> {
        UNSIGNED_LONG_REFLECTION_DESCRIPTORS
            .get(data.uint32_value(scope)? as usize)
            .map(|(_, descriptor)| descriptor)
    }
}

impl_reflection_callback_data!(UnsignedLongReflection);

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum CrossOriginReflection {
    Image,
    Link,
    Script,
    Count,
}

pub(super) struct CrossOriginReflectionDescriptor {
    pub(super) interface: &'static str,
}

const CROSS_ORIGIN_REFLECTION_DESCRIPTORS: &[(
    CrossOriginReflection,
    CrossOriginReflectionDescriptor,
)] = &[
    (
        CrossOriginReflection::Image,
        CrossOriginReflectionDescriptor {
            interface: "HTMLImageElement",
        },
    ),
    (
        CrossOriginReflection::Link,
        CrossOriginReflectionDescriptor {
            interface: "HTMLLinkElement",
        },
    ),
    (
        CrossOriginReflection::Script,
        CrossOriginReflectionDescriptor {
            interface: "HTMLScriptElement",
        },
    ),
];

const _: () = {
    assert!(CROSS_ORIGIN_REFLECTION_DESCRIPTORS.len() == CrossOriginReflection::Count as usize);
    let mut index = 0;
    while index < CROSS_ORIGIN_REFLECTION_DESCRIPTORS.len() {
        assert!(CROSS_ORIGIN_REFLECTION_DESCRIPTORS[index].0 as usize == index);
        index += 1;
    }
};

impl CrossOriginReflection {
    pub(super) fn descriptor_from_callback_data(
        scope: &mut v8::PinScope<'_, '_>,
        data: v8::Local<'_, v8::Value>,
    ) -> Option<&'static CrossOriginReflectionDescriptor> {
        CROSS_ORIGIN_REFLECTION_DESCRIPTORS
            .get(data.uint32_value(scope)? as usize)
            .map(|(_, descriptor)| descriptor)
    }
}

impl_reflection_callback_data!(CrossOriginReflection);

pub(super) fn set_reflected_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::super::JsContextHost,
    handle: DomHandle,
    name: &str,
    value: &str,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = set_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            name,
            value,
        );
    });
}

pub(super) fn set_reflected_style_attribute_with_inline_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::super::JsContextHost,
    handle: DomHandle,
    value: &str,
    inline_base_url: Option<&url::Url>,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_style_attribute_from_cssom_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            value,
        );
        runtime.set_element_inline_style_csp_state(
            handle,
            crate::style_engine::InlineStyleCspState::Cssom,
        );
        if let Some(inline_base_url) = inline_base_url {
            runtime.set_element_inline_style_base_url(handle, inline_base_url.clone());
        }
    });
}

pub(super) fn remove_reflected_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::super::JsContextHost,
    handle: DomHandle,
    name: &str,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = remove_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            name,
        );
    });
}

pub(super) fn set_reflected_boolean_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::super::JsContextHost,
    handle: DomHandle,
    name: &str,
    enabled: bool,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_boolean_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            name,
            enabled,
        );
    });
}

pub(super) fn set_attribute_property_on_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    let Some(value) = value.to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    set_reflected_attribute(scope, runtime_ptr, handle, name, &value);
}

pub(super) fn set_usv_string_attribute_property_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    let Some(value) = property_usv_string_value(scope, value, owner, property) else {
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, &value);
}

pub(super) fn set_dom_string_attribute_property_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    let Some(value) = property_dom_string_value(scope, value, owner, property) else {
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, &value);
}

pub(super) fn set_nullable_dom_string_attribute_property_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    let Some((runtime_ptr, handle)) = element_reflection_receiver_or_throw(scope, object) else {
        return;
    };
    if value.is_null_or_undefined() {
        remove_reflected_attribute(scope, runtime_ptr, handle, name);
        return;
    }
    let Some(value) = property_dom_string_value(scope, value, owner, property) else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, &value);
}

pub(super) fn attribute_property_getter_from_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, name).unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(super) fn nullable_attribute_property_getter_from_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_reflection_receiver_or_throw(scope, object) else {
        return;
    };
    let Some(value) = element_attribute(unsafe { &*runtime_ptr }, handle, name) else {
        rv.set_null();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn element_reflection_receiver_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(*mut super::super::JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        webidl::throw_type_error(scope, "Illegal invocation");
        return None;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        webidl::throw_type_error(scope, "Illegal invocation");
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(super) fn property_string_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    Some(value.to_string(scope)?.to_rust_string_lossy(scope))
}

pub(super) fn property_dom_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<String> {
    match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member(owner, property),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(super) fn property_usv_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<String> {
    match webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::member(owner, property),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(super) fn boolean_attribute_property_getter_from_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        name,
    ));
}

pub(super) fn set_boolean_attribute_property_on_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return;
    }
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, name, value.boolean_value(scope));
}

pub(super) fn parse_non_negative_dimension(value: Option<String>) -> u32 {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

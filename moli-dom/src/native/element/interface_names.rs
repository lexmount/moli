use crate::custom_elements::is_valid_custom_element_name;

const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";

pub fn html_element_interface_name(local_name: &str) -> &'static str {
    if local_name != local_name.to_ascii_lowercase() {
        return "HTMLUnknownElement";
    }
    match local_name {
        "a" => "HTMLAnchorElement",
        "abbr" | "acronym" | "address" | "article" | "aside" | "b" | "basefont" | "bdi" | "bdo"
        | "big" | "center" | "cite" | "code" | "dd" | "dfn" | "dt" | "em" | "figcaption"
        | "figure" | "footer" | "header" | "hgroup" | "i" | "kbd" | "main" | "mark" | "nav"
        | "nobr" | "noembed" | "noframes" | "noscript" | "plaintext" | "rb" | "rp" | "rt"
        | "rtc" | "ruby" | "s" | "samp" | "section" | "small" | "strike" | "strong" | "sub"
        | "summary" | "sup" | "tt" | "u" | "var" | "wbr" => "HTMLElement",
        "applet" | "bgsound" | "blink" | "content" | "decorator" | "element" | "image"
        | "isindex" | "keygen" | "menuitem" | "shadow" | "spacer" => "HTMLUnknownElement",
        "area" => "HTMLAreaElement",
        "audio" => "HTMLAudioElement",
        "base" => "HTMLBaseElement",
        "blockquote" | "q" => "HTMLQuoteElement",
        "body" => "HTMLBodyElement",
        "br" => "HTMLBRElement",
        "button" => "HTMLButtonElement",
        "canvas" => "HTMLCanvasElement",
        "caption" => "HTMLTableCaptionElement",
        "col" | "colgroup" => "HTMLTableColElement",
        "data" => "HTMLDataElement",
        "datalist" => "HTMLDataListElement",
        "del" | "ins" => "HTMLModElement",
        "details" => "HTMLDetailsElement",
        "dialog" => "HTMLDialogElement",
        "dir" => "HTMLDirectoryElement",
        "div" => "HTMLDivElement",
        "dl" => "HTMLDListElement",
        "embed" => "HTMLEmbedElement",
        "fieldset" => "HTMLFieldSetElement",
        "font" => "HTMLFontElement",
        "form" => "HTMLFormElement",
        "frame" => "HTMLFrameElement",
        "frameset" => "HTMLFrameSetElement",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "HTMLHeadingElement",
        "head" => "HTMLHeadElement",
        "hr" => "HTMLHRElement",
        "html" => "HTMLHtmlElement",
        "iframe" => "HTMLIFrameElement",
        "img" => "HTMLImageElement",
        "input" => "HTMLInputElement",
        "label" => "HTMLLabelElement",
        "legend" => "HTMLLegendElement",
        "li" => "HTMLLIElement",
        "link" => "HTMLLinkElement",
        "listing" | "pre" | "xmp" => "HTMLPreElement",
        "map" => "HTMLMapElement",
        "marquee" => "HTMLMarqueeElement",
        "menu" => "HTMLMenuElement",
        "meta" => "HTMLMetaElement",
        "meter" => "HTMLMeterElement",
        "object" => "HTMLObjectElement",
        "ol" => "HTMLOListElement",
        "optgroup" => "HTMLOptGroupElement",
        "option" => "HTMLOptionElement",
        "output" => "HTMLOutputElement",
        "p" => "HTMLParagraphElement",
        "param" => "HTMLParamElement",
        "picture" => "HTMLPictureElement",
        "progress" => "HTMLProgressElement",
        "script" => "HTMLScriptElement",
        "select" => "HTMLSelectElement",
        "slot" => "HTMLSlotElement",
        "source" => "HTMLSourceElement",
        "span" => "HTMLSpanElement",
        "style" => "HTMLStyleElement",
        "table" => "HTMLTableElement",
        "tbody" | "tfoot" | "thead" => "HTMLTableSectionElement",
        "td" | "th" => "HTMLTableCellElement",
        "template" => "HTMLTemplateElement",
        "textarea" => "HTMLTextAreaElement",
        "time" => "HTMLTimeElement",
        "title" => "HTMLTitleElement",
        "track" => "HTMLTrackElement",
        "tr" => "HTMLTableRowElement",
        "ul" => "HTMLUListElement",
        "video" => "HTMLVideoElement",
        _ if is_valid_custom_element_name(local_name) => "HTMLElement",
        _ => "HTMLUnknownElement",
    }
}

pub fn svg_element_interface_name(local_name: &str) -> &'static str {
    match local_name {
        "a" => "SVGAElement",
        "circle" => "SVGCircleElement",
        "clipPath" => "SVGClipPathElement",
        "defs" => "SVGDefsElement",
        "desc" => "SVGDescElement",
        "ellipse" => "SVGEllipseElement",
        "feBlend" => "SVGFEBlendElement",
        "feColorMatrix" => "SVGFEColorMatrixElement",
        "feComposite" => "SVGFECompositeElement",
        "feConvolveMatrix" => "SVGFEConvolveMatrixElement",
        "feDiffuseLighting" => "SVGFEDiffuseLightingElement",
        "feDisplacementMap" => "SVGFEDisplacementMapElement",
        "feDistantLight" => "SVGFEDistantLightElement",
        "feDropShadow" => "SVGFEDropShadowElement",
        "feFuncA" => "SVGFEFuncAElement",
        "feFuncB" => "SVGFEFuncBElement",
        "feFuncG" => "SVGFEFuncGElement",
        "feFuncR" => "SVGFEFuncRElement",
        "feGaussianBlur" => "SVGFEGaussianBlurElement",
        "feMorphology" => "SVGFEMorphologyElement",
        "feOffset" => "SVGFEOffsetElement",
        "fePointLight" => "SVGFEPointLightElement",
        "feSpecularLighting" => "SVGFESpecularLightingElement",
        "feSpotLight" => "SVGFESpotLightElement",
        "feTurbulence" => "SVGFETurbulenceElement",
        "filter" => "SVGFilterElement",
        "foreignObject" => "SVGForeignObjectElement",
        "g" => "SVGGElement",
        "image" => "SVGImageElement",
        "line" => "SVGLineElement",
        "linearGradient" => "SVGLinearGradientElement",
        "marker" => "SVGMarkerElement",
        "mask" => "SVGMaskElement",
        "metadata" => "SVGMetadataElement",
        "path" => "SVGPathElement",
        "pattern" => "SVGPatternElement",
        "polygon" => "SVGPolygonElement",
        "polyline" => "SVGPolylineElement",
        "radialGradient" => "SVGRadialGradientElement",
        "rect" => "SVGRectElement",
        "script" => "SVGScriptElement",
        "stop" => "SVGStopElement",
        "svg" => "SVGSVGElement",
        "style" => "SVGStyleElement",
        "symbol" => "SVGSymbolElement",
        "text" => "SVGTextElement",
        "textPath" => "SVGTextPathElement",
        "title" => "SVGTitleElement",
        "tspan" => "SVGTSpanElement",
        "use" => "SVGUseElement",
        "view" => "SVGViewElement",
        _ => "SVGElement",
    }
}

pub fn mathml_element_interface_name(_local_name: &str) -> &'static str {
    "MathMLElement"
}

pub fn is_mathml_namespace(namespace: &str) -> bool {
    namespace == MATHML_NAMESPACE
}

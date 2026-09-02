use super::*;
use crate::native::DomHost;

fn test_url() -> url::Url {
    url::Url::parse("https://serialization.test/").expect("test URL")
}

#[test]
fn html_serializers_share_the_complete_void_element_set() {
    let mut dom = NativeDom::new_html(test_url());
    let container = dom.create_element("div");
    let mut expected = String::new();
    let mut void_elements = Vec::new();
    for local_name in [
        "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
        "keygen", "link", "meta", "param", "source", "track", "wbr",
    ] {
        let element = dom.create_element(local_name);
        let ignored_child = dom.create_element("span");
        assert!(dom.append_child(element, ignored_child));
        assert_eq!(dom.outer_html(element), Some(format!("<{local_name}>")));
        assert_eq!(dom.inner_html(element).as_deref(), Some(""));
        assert!(dom.append_child(container, element));
        expected.push_str(&format!("<{local_name}>"));
        void_elements.push(element);
    }
    assert_eq!(
        dom.inner_html(container).as_deref(),
        Some(expected.as_str())
    );

    let foreign_param = dom
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "param")
        .expect("SVG element");
    assert_eq!(
        dom.outer_html(foreign_param).as_deref(),
        Some("<param></param>")
    );

    let mut host = DomHost::from_dom(dom);
    let param = host.create_element("param");
    assert!(host.append_child(container, param));
    assert_eq!(
        host.get_html(container, true, false, &[]).as_deref(),
        Some(format!("{expected}<param>").as_str())
    );
    for element in void_elements {
        assert_eq!(
            host.get_html(element, true, false, &[]).as_deref(),
            Some("")
        );
    }
}

#[test]
fn html_text_serialization_uses_the_actual_parent_and_scripting_mode() {
    let mut dom = NativeDom::new_html(test_url());
    let container = dom.create_element("main");
    let mut cases = Vec::new();
    for (local_name, literal_with_scripting, literal_without_scripting) in [
        ("style", true, true),
        ("script", true, true),
        ("xmp", true, true),
        ("iframe", true, true),
        ("noembed", true, true),
        ("noframes", true, true),
        ("plaintext", true, true),
        ("noscript", true, false),
        ("textarea", false, false),
        ("title", false, false),
        ("div", false, false),
    ] {
        let element = dom.create_element(local_name);
        let text = dom.create_text_node("<&");
        assert!(dom.append_child(element, text));
        assert!(dom.append_child(container, element));
        cases.push((
            element,
            local_name,
            literal_with_scripting,
            literal_without_scripting,
        ));
    }

    for &(element, _, literal_with_scripting, literal_without_scripting) in &cases {
        assert_eq!(
            dom.inner_html_with_scripting_enabled(element, true)
                .as_deref(),
            Some(if literal_with_scripting {
                "<&"
            } else {
                "&lt;&amp;"
            })
        );
        assert_eq!(
            dom.inner_html_with_scripting_enabled(element, false)
                .as_deref(),
            Some(if literal_without_scripting {
                "<&"
            } else {
                "&lt;&amp;"
            })
        );
    }

    let host = DomHost::from_dom(dom);
    for &(element, local_name, literal_with_scripting, literal_without_scripting) in &cases {
        for (scripting_enabled, literal) in [
            (true, literal_with_scripting),
            (false, literal_without_scripting),
        ] {
            let contents = if literal { "<&" } else { "&lt;&amp;" };
            assert_eq!(
                host.get_html_with_shadow_root_registry_attribute_policy(
                    element,
                    scripting_enabled,
                    false,
                    &[],
                    None,
                )
                .as_deref(),
                Some(contents),
                "children-only serialization diverged for {local_name}"
            );
            assert_eq!(
                host.outer_html_with_shadow_roots(
                    element,
                    scripting_enabled,
                    crate::native::ShadowRootInclusion::None,
                    None,
                ),
                Some(format!("<{local_name}>{contents}</{local_name}>")),
                "node serialization diverged for {local_name}"
            );
        }
    }
}

#[test]
fn bounded_outer_html_stops_before_exceeding_the_output_limit() {
    let mut dom = NativeDom::new_html(test_url());
    let element = dom.create_element("div");
    let expected = "<div></div>";

    assert_eq!(
        dom.outer_html_with_limit(element, expected.len()),
        Ok(Some(expected.to_owned()))
    );
    assert_eq!(
        dom.outer_html_with_limit(element, expected.len() - 1),
        Err(HtmlSerializationLimitExceeded {
            max_bytes: expected.len() - 1,
        })
    );
}

#[test]
fn html_serializers_apply_attribute_serialized_name_rules() {
    let mut dom = NativeDom::new_html(test_url());
    let container = dom.create_element("section");
    let element = dom
        .create_element_ns(Some("urn:element"), "div")
        .expect("namespaced element");
    assert!(dom.set_attribute_ns(
        element,
        Some("http://www.w3.org/XML/1998/namespace"),
        Some("alternate"),
        "lang",
        "en-us",
    ));
    assert!(dom.set_attribute_ns(
        element,
        Some("http://www.w3.org/2000/xmlns/"),
        None,
        "binding",
        "urn:binding",
    ));
    assert!(dom.set_attribute_ns(
        element,
        Some("http://www.w3.org/2000/xmlns/"),
        None,
        "xmlns",
        "urn:default",
    ));
    assert!(dom.set_attribute_ns(
        element,
        Some("http://www.w3.org/1999/xlink"),
        Some("alternate"),
        "href",
        "target",
    ));
    assert!(dom.set_attribute_ns(element, Some("urn:custom"), Some("p"), "attr", "value",));
    assert!(dom.append_child(container, element));

    let expected = concat!(
        "<div xml:lang=\"en-us\" xmlns:binding=\"urn:binding\" ",
        "xmlns=\"urn:default\" xlink:href=\"target\" p:attr=\"value\"></div>"
    );
    assert_eq!(dom.inner_html(container).as_deref(), Some(expected));

    let host = DomHost::from_dom(dom);
    assert_eq!(
        host.get_html(container, true, false, &[]).as_deref(),
        Some(expected)
    );
}

#[test]
fn html_serializers_escape_adopted_cdata_as_text() {
    const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
    const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

    let mut xml_dom = NativeDom::new_xml(test_url());
    let xml_svg = xml_dom
        .create_element_ns(Some(SVG_NAMESPACE), "svg")
        .expect("XML SVG element");
    assert!(
        xml_dom.set_attribute_ns(xml_svg, Some(XMLNS_NAMESPACE), None, "xmlns", SVG_NAMESPACE,)
    );
    let xml_cdata = xml_dom.create_cdata_section("<img>&");
    assert!(xml_dom.append_child(xml_svg, xml_cdata));
    assert_eq!(
        xml_dom.outer_html(xml_svg).as_deref(),
        Some(r#"<svg xmlns="http://www.w3.org/2000/svg"><![CDATA[<img>&]]></svg>"#)
    );

    let mut html_dom = NativeDom::new_html(test_url());
    let html_svg = html_dom
        .create_element_ns(Some(SVG_NAMESPACE), "svg")
        .expect("HTML-document SVG element");
    assert!(html_dom.set_attribute_ns(
        html_svg,
        Some(XMLNS_NAMESPACE),
        None,
        "xmlns",
        SVG_NAMESPACE,
    ));
    let adopted_cdata = html_dom.create_cdata_section("<img>&");
    assert!(html_dom.append_child(html_svg, adopted_cdata));
    assert_eq!(
        html_dom.outer_html(html_svg).as_deref(),
        Some(r#"<svg xmlns="http://www.w3.org/2000/svg">&lt;img&gt;&amp;</svg>"#)
    );

    let host = DomHost::from_dom(html_dom);
    assert_eq!(
        host.get_html(html_svg, true, false, &[]).as_deref(),
        Some("&lt;img&gt;&amp;")
    );
}

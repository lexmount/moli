use std::collections::HashMap;

use super::{MAX_ENCODED_SVG_BYTES, SvgDecodeError, SvgImage, SvgTextResource};

pub use usvg::{ResolvedStyleProperty as SvgStyleProperty, ResolvedStyleValue as SvgStyleValue};

/// One emitted XML element and its computed presentation inputs. Parent indices
/// address the same input's element list, including non-SVG elements.
pub struct InlineSvgElement {
    pub namespace: String,
    pub local_name: String,
    pub parent: Option<usize>,
    pub style: Vec<SvgStyleValue>,
}

/// An admitted, immutable inline SVG input. Its source indices are independent
/// of authored IDs and are validated against XML before backend conversion.
pub struct InlineSvgInput {
    xml: String,
    elements: Vec<InlineSvgElement>,
    estimated_bytes: usize,
}

impl InlineSvgInput {
    pub fn new(xml: String) -> Result<Self, SvgDecodeError> {
        super::check_encoded_budget(xml.as_bytes())?;
        let estimated_bytes = xml.capacity();
        if estimated_bytes > MAX_ENCODED_SVG_BYTES {
            return Err(SvgDecodeError::InlineInputBudgetExceeded {
                actual: estimated_bytes,
                max: MAX_ENCODED_SVG_BYTES,
            });
        }
        Ok(Self {
            xml,
            elements: Vec::new(),
            estimated_bytes,
        })
    }

    pub fn push_element(&mut self, element: InlineSvgElement) -> Result<(), SvgDecodeError> {
        let index = self.elements.len();
        if element.parent.is_none() != (index == 0)
            || element.parent.is_some_and(|parent| parent >= index)
        {
            return Err(SvgDecodeError::InlineSourceMismatch { element: index });
        }
        let bytes = std::mem::size_of::<InlineSvgElement>()
            .saturating_mul(2) // account for vector capacity growth
            // Source handles, snapshot records and the XML-ID/index table.
            .saturating_add(12 * std::mem::size_of::<usize>())
            .saturating_add(element.namespace.capacity())
            .saturating_add(element.local_name.capacity())
            .saturating_add(
                element
                    .style
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SvgStyleValue>()),
            )
            .saturating_add(
                element
                    .style
                    .iter()
                    .map(SvgStyleValue::estimated_bytes)
                    .sum::<usize>(),
            );
        let actual = self.estimated_bytes.saturating_add(bytes);
        if actual > MAX_ENCODED_SVG_BYTES {
            return Err(SvgDecodeError::InlineInputBudgetExceeded {
                actual,
                max: MAX_ENCODED_SVG_BYTES,
            });
        }
        self.estimated_bytes = actual;
        self.elements.push(element);
        Ok(())
    }
}

/// Painting and canonical numeric text from the same backend layout. Text
/// source IDs are indices into this input's element list, not XML node IDs.
pub struct InlineSvgLayout {
    pub image: SvgImage,
    pub text: Vec<SvgTextResource>,
}

pub fn layout_inline_svg(input: InlineSvgInput) -> Result<InlineSvgLayout, SvgDecodeError> {
    let metadata = super::probe_svg_image(input.xml.as_bytes())?;
    let xml = usvg::roxmltree::Document::parse_with_options(
        &input.xml,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(usvg::Error::ParsingFailed)?;
    let mut indices = HashMap::new();
    let mut styles = Vec::new();
    let mut expected = input.elements.into_iter();
    for (index, node) in xml
        .descendants()
        .filter(|node| node.is_element())
        .enumerate()
    {
        let Some(element) = expected.next() else {
            return Err(SvgDecodeError::InlineSourceMismatch { element: index });
        };
        let parent = node
            .parent()
            .filter(|node| node.is_element())
            .and_then(|node| indices.get(&node.id().get()).copied());
        if node.tag_name().namespace().unwrap_or_default() != element.namespace
            || node.tag_name().name() != element.local_name
            || parent != element.parent
        {
            return Err(SvgDecodeError::InlineSourceMismatch { element: index });
        }
        indices.insert(node.id().get(), index);
        if !element.style.is_empty() {
            if element.namespace != "http://www.w3.org/2000/svg" {
                return Err(SvgDecodeError::InlineSourceMismatch { element: index });
            }
            styles.push(usvg::ResolvedElementStyle {
                source_node_id: node.id().get(),
                values: element.style,
            });
        }
    }
    if expected.next().is_some() {
        return Err(SvgDecodeError::InlineSourceMismatch {
            element: indices.len(),
        });
    }
    let tree = usvg::Tree::from_xmltree_with_resolved_styles(
        &xml,
        &super::svg_options(metadata)?,
        &styles,
    )?;
    let image = super::finish_svg_image(tree, metadata)?;
    let mut text = image.text_resources();
    let source_index = |id| {
        indices
            .get(&id)
            .and_then(|index| u32::try_from(*index).ok())
            .ok_or(SvgDecodeError::InlineSourceMismatch {
                element: indices.len(),
            })
    };
    for resource in &mut text {
        resource.source_node_id = source_index(resource.source_node_id)?;
        for range in &mut resource.sources {
            range.source_node_id = source_index(range.source_node_id)?;
        }
    }
    Ok(InlineSvgLayout { image, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(name: &str, parent: Option<usize>) -> InlineSvgElement {
        InlineSvgElement {
            namespace: "http://www.w3.org/2000/svg".into(),
            local_name: name.into(),
            parent,
            style: vec![],
        }
    }

    #[test]
    fn source_binding_checks_namespaces_names_and_parentage() {
        let xml = "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><g><rect width='2' height='2'/></g></svg>";
        let mut input = InlineSvgInput::new(xml.into()).unwrap();
        input.push_element(element("svg", None)).unwrap();
        input.push_element(element("g", Some(0))).unwrap();
        input.push_element(element("rect", Some(1))).unwrap();
        assert!(layout_inline_svg(input).is_ok());
        for (name, parent, namespace) in [
            ("circle", Some(1), "http://www.w3.org/2000/svg"),
            ("rect", Some(0), "http://www.w3.org/2000/svg"),
            ("rect", Some(1), "urn:foreign"),
        ] {
            let mut input = InlineSvgInput::new(xml.into()).unwrap();
            input.push_element(element("svg", None)).unwrap();
            input.push_element(element("g", Some(0))).unwrap();
            let mut last = element(name, parent);
            last.namespace = namespace.into();
            input.push_element(last).unwrap();
            assert!(matches!(
                layout_inline_svg(input),
                Err(SvgDecodeError::InlineSourceMismatch { element: 2 })
            ));
        }
    }

    #[test]
    fn text_sources_are_input_indices_even_with_duplicate_ids_and_cdata() {
        let xml = "<svg xmlns='http://www.w3.org/2000/svg' width='300' height='100'>\
            <!-- XML node IDs differ from element indices. -->\
            <text id='same' y='40'>A<tspan><![CDATA[BC]]></tspan>D</text>\
            <text id='same' y='80'>EF</text></svg>";
        let mut input = InlineSvgInput::new(xml.into()).unwrap();
        input.push_element(element("svg", None)).unwrap();
        let mut first = element("text", Some(0));
        first
            .style
            .push(SvgStyleValue::parse(SvgStyleProperty::FontSize, "32px").unwrap());
        input.push_element(first).unwrap();
        input.push_element(element("tspan", Some(1))).unwrap();
        input.push_element(element("text", Some(0))).unwrap();
        let output = layout_inline_svg(input).unwrap();
        assert_eq!(
            output.text.len(),
            2,
            "both text nodes must actually be shaped"
        );
        assert_eq!(output.text[0].source_node_id, 1);
        assert_eq!(output.text[1].source_node_id, 3);
        assert!(
            output.text[0]
                .sources
                .iter()
                .any(|source| source.source_node_id == 2 && source.code_units == (1..3))
        );
        let all = 0..4;
        let query = output.text[0].layout.query(std::slice::from_ref(&all));
        assert_eq!(query.number_of_chars(), 4);
        assert!(query.computed_text_length() > 0.0);
        let usvg::Node::Text(painted) = output.image.tree().node_by_id("same").unwrap() else {
            panic!("paint text")
        };
        assert_eq!(painted.chunks()[0].spans()[0].font_size().get(), 32.0);
    }

    #[test]
    fn styles_share_the_input_budget_and_cannot_inject_xml_or_declarations() {
        let mut input = InlineSvgInput::new(" ".repeat(MAX_ENCODED_SVG_BYTES - 1)).unwrap();
        let mut root = element("svg", None);
        root.style
            .push(SvgStyleValue::parse(SvgStyleProperty::FontSize, "32px").unwrap());
        assert!(matches!(
            input.push_element(root),
            Err(SvgDecodeError::InlineInputBudgetExceeded { .. })
        ));
        assert!(SvgStyleValue::parse(SvgStyleProperty::FontSize, "32px;fill:red").is_none());
    }
}

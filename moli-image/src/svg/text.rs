use std::ops::Range;

use moli_svg::{SvgMatrixComponents, SvgTextFragment, SvgTextLayout};
use unicode_segmentation::UnicodeSegmentation;

/// Normalized addressable UTF-16 units originating in one XML element.
#[derive(Clone, Debug)]
pub struct SvgTextSourceRange {
    pub source_node_id: u32,
    pub code_units: Range<usize>,
}

/// Text geometry for one original SVG `text` element. Source IDs are XML node
/// indices, never authored IDs, pointers or DOM handles.
#[derive(Debug)]
pub struct SvgTextResource {
    pub source_node_id: u32,
    pub sources: Vec<SvgTextSourceRange>,
    pub layout: SvgTextLayout,
}

pub(super) fn tree_text_resources(tree: &usvg::Tree) -> Vec<SvgTextResource> {
    let mut result = Vec::new();
    let mut nodes = tree.root().children().iter().rev().collect::<Vec<_>>();
    while let Some(node) = nodes.pop() {
        match node {
            usvg::Node::Group(group) => nodes.extend(group.children().iter().rev()),
            usvg::Node::Text(text) => {
                if let Some(resource) = text_resource(text) {
                    result.push(resource);
                }
            }
            usvg::Node::Path(_) | usvg::Node::Image(_) => {}
        }
    }
    result
}

struct TextChunkMap {
    /// UTF-8 boundary -> addressable UTF-16 offset in the entire text element.
    boundaries: Vec<(usize, usize)>,
    graphemes: Vec<Range<usize>>,
}

impl TextChunkMap {
    fn new(text: &str, offset: usize) -> Self {
        let mut code_units = offset;
        let mut boundaries = Vec::new();
        for (byte, character) in text.char_indices() {
            boundaries.push((byte, code_units));
            code_units += character.len_utf16();
        }
        boundaries.push((text.len(), code_units));
        Self {
            boundaries,
            graphemes: text
                .grapheme_indices(true)
                .map(|(start, text)| start..start + text.len())
                .collect(),
        }
    }

    fn code_units(&self, byte: usize) -> usize {
        let index = self
            .boundaries
            .binary_search_by_key(&byte, |(byte, _)| *byte)
            .expect("usvg text offsets are UTF-8 character boundaries");
        self.boundaries[index].1
    }
}

fn text_resource(text: &usvg::Text) -> Option<SvgTextResource> {
    let source_node_id = text.source_node_id()?;
    let mut sources = Vec::new();
    let mut offset = 0;
    let chunks = text
        .chunks()
        .iter()
        .map(|chunk| {
            let map = TextChunkMap::new(chunk.text(), offset);
            offset = map.code_units(chunk.text().len());
            for span in chunk.spans() {
                if let Some(source_node_id) = span.source_node_id() {
                    sources.push(SvgTextSourceRange {
                        source_node_id,
                        code_units: map.code_units(span.start())..map.code_units(span.end()),
                    });
                }
            }
            map
        })
        .collect::<Vec<_>>();

    let mut clusters = text.layouted_clusters().iter().collect::<Vec<_>>();
    clusters.sort_by_key(|cluster| (cluster.chunk_index, cluster.byte_range.start));
    let mut fragments = Vec::new();
    for cluster in clusters {
        let map = &chunks[cluster.chunk_index];
        let first = map
            .graphemes
            .partition_point(|range| range.end <= cluster.byte_range.start);
        let last = map
            .graphemes
            .partition_point(|range| range.start < cluster.byte_range.end);
        let graphemes = &map.graphemes[first..last];
        if graphemes.is_empty() {
            continue;
        }
        let advance = f64::from(cluster.advance) / graphemes.len() as f64;
        let matrix = SvgMatrixComponents {
            a: f64::from(cluster.transform.sx),
            b: f64::from(cluster.transform.ky),
            c: f64::from(cluster.transform.kx),
            d: f64::from(cluster.transform.sy),
            e: f64::from(cluster.transform.tx),
            f: f64::from(cluster.transform.ty),
        };
        for (index, range) in graphemes.iter().enumerate() {
            // A ligature without explicit caret data is split between its
            // graphemes, not UTF-16 units. Combining sequences and surrogate
            // pairs continue to address the same complete typographic cell.
            let visual_index = if cluster.is_rtl {
                graphemes.len() - index - 1
            } else {
                index
            };
            fragments.push(SvgTextFragment {
                code_units: map.code_units(range.start)..map.code_units(range.end),
                transform: matrix.then_translate(advance * visual_index as f64, 0.0),
                advance,
                ascent: f64::from(cluster.ascent),
                descent: f64::from(cluster.descent),
                right_to_left: cluster.is_rtl,
                visible: cluster.visible,
            });
        }
    }
    Some(SvgTextResource {
        source_node_id,
        sources,
        layout: SvgTextLayout { fragments },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_font_tree(body: &str) -> usvg::Tree {
        let mut database = usvg::fontdb::Database::new();
        database.load_font_data(
            include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.ttf").to_vec(),
        );
        let family = database.faces().next().unwrap().families[0].0.clone();
        let options = usvg::Options {
            font_family: family,
            fontdb: std::sync::Arc::new(database),
            ..Default::default()
        };
        usvg::Tree::from_str(
            &format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='500' height='200'>{body}</svg>"
            ),
            &options,
        )
        .unwrap()
    }

    #[test]
    fn text_metrics_retain_real_font_advances_and_source_provenance() {
        let tree =
            fixed_font_tree("<text x='10' y='30' font-size='20'>AB<tspan>CD</tspan>EF</text>");
        let text = tree_text_resources(&tree).pop().unwrap();
        let range = 0..6;
        let query = text.layout.query(std::slice::from_ref(&range));
        // This fixture's hmtx entries are 600 units at 1000 units/em (see
        // generate-layout-test-font.py), so each shaped advance is 12px.
        assert!(
            (query.computed_text_length() - 72.0).abs() < 0.001,
            "actual {}",
            query.computed_text_length()
        );
        assert_eq!(query.number_of_chars(), 6);
        assert_eq!(text.sources.len(), 3);
        assert_ne!(
            text.sources[0].source_node_id,
            text.sources[1].source_node_id
        );
        assert_eq!(
            text.sources[0].source_node_id,
            text.sources[2].source_node_id
        );
        let child = text
            .layout
            .query(std::slice::from_ref(&text.sources[1].code_units));
        assert_eq!(child.number_of_chars(), 2);
        assert!((child.computed_text_length() - 24.0).abs() < 0.001);
        assert!((child.character(0).unwrap().start.x - 34.0).abs() < 0.001);
    }

    #[test]
    fn text_positions_are_not_typographic_advance() {
        let tree = fixed_font_tree(
            "<text x='10' y='30' font-size='20' dx='0 40 20' rotate='0 30 60'>ABC</text>",
        );
        let text = tree_text_resources(&tree).pop().unwrap();
        let range = 0..3;
        let query = text.layout.query(std::slice::from_ref(&range));
        assert!(
            (query.computed_text_length() - 36.0).abs() < 0.001,
            "actual {}",
            query.computed_text_length()
        );
        assert!((query.character(1).unwrap().start.x - 62.0).abs() < 0.001);
        assert!((query.character(1).unwrap().rotation - 30.0).abs() < 0.001);
        assert!(query.bounding_box().unwrap().width > query.computed_text_length());
    }

    #[test]
    fn text_length_spacing_and_glyph_scaling_are_distinct() {
        for (adjust, expected) in [("spacing", 36.0), ("spacingAndGlyphs", 200.0)] {
            let tree = fixed_font_tree(&format!(
                "<text x='10' y='30' font-size='20' textLength='200' lengthAdjust='{adjust}'>ABC</text>"
            ));
            let text = tree_text_resources(&tree).pop().unwrap();
            let range = 0..3;
            let query = text.layout.query(std::slice::from_ref(&range));
            assert!(
                (query.computed_text_length() - expected).abs() < 0.001,
                "{adjust}: actual {} from {:?}",
                query.computed_text_length(),
                text.layout.fragments
            );
            assert!(
                (query.bounding_box().unwrap().width - 200.0).abs() < 0.001,
                "{adjust}"
            );
        }
    }

    #[test]
    fn text_map_preserves_surrogate_and_combining_graphemes() {
        let map = TextChunkMap::new("e\u{301}\u{1f600}X", 10);
        assert_eq!(map.graphemes, [0..3, 3..7, 7..8]);
        assert_eq!(map.code_units(3), 12);
        assert_eq!(map.code_units(7), 14);
        assert_eq!(map.code_units(8), 15);
    }
}

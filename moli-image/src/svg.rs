// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The immutable `usvg::Tree` resource boundary follows DioxusLabs/blitz
// commit d788124ab881f9bb537cb452ec1d837604a374a8. Natural sizing and the
// 300x150 default concrete-object-size algorithm follow Blink's
// LayoutSVGRoot::UnscaledNaturalSizingInfo and ConcreteObjectSize.

use std::sync::{Arc, LazyLock};

/// Maximum encoded SVG body admitted to the static image decoder.
///
/// SVG parsing expands XML, paths, text, and embedded data into a substantially
/// larger tree. Keeping this below the raster input limit bounds that expansion
/// before `usvg` receives attacker-controlled bytes.
pub const MAX_ENCODED_SVG_BYTES: usize = 16 * 1024 * 1024;

/// Maximum DOM-neutral vector work units retained by one decoded SVG image.
///
/// A node costs one unit and every path segment costs another. The paint
/// executor charges the same count against its per-capture operation budget.
pub const MAX_SVG_PAINT_WORK_UNITS: usize = 250_000;

const MAX_SVG_TREE_DEPTH: usize = 1_024;
const DEFAULT_OBJECT_WIDTH: f32 = 300.0;
const DEFAULT_OBJECT_HEIGHT: f32 = 150.0;

static SVG_FONT_DATABASE: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut database = usvg::fontdb::Database::new();
    database.load_system_fonts();
    Arc::new(database)
});

/// Natural SVG sizing data and its CSS default concrete object size.
///
/// Width and height stay optional because a `viewBox` supplies an intrinsic
/// ratio, not intrinsic pixel dimensions. `concrete_*` applies the CSS Images
/// default-object-size algorithm against 300x150, matching Blink's
/// `ConcreteObjectSize` boundary for SVG image resources.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgImageMetadata {
    pub intrinsic_width: Option<f32>,
    pub intrinsic_height: Option<f32>,
    pub intrinsic_ratio: Option<f32>,
    pub concrete_width: f32,
    pub concrete_height: f32,
}

impl SvgImageMetadata {
    /// Resolve an image's missing natural dimensions against the consumer's
    /// default object size (for example, the destination Canvas bitmap).
    pub fn with_default_object_size(self, width: f32, height: f32) -> Self {
        let (concrete_width, concrete_height) = concrete_object_size_with_default(
            self.intrinsic_width,
            self.intrinsic_height,
            self.intrinsic_ratio,
            width,
            height,
        );
        Self {
            concrete_width,
            concrete_height,
            ..self
        }
    }

    pub fn concrete_dimensions(self) -> Option<(u32, u32)> {
        Some((
            rounded_dimension(self.concrete_width)?,
            rounded_dimension(self.concrete_height)?,
        ))
    }

    /// Conservative retained-memory charge for the tree and one immutable
    /// encoded body used for consumer-specific viewport resolution.
    pub fn retained_byte_estimate(self, encoded_len: usize) -> Option<usize> {
        let _ = self;
        encoded_len.checked_mul(9)
    }
}

/// Parsed, immutable SVG image shared by the resource owner and paint snapshots.
pub struct SvgImage {
    tree: usvg::Tree,
    encoded: Arc<[u8]>,
    metadata: SvgImageMetadata,
    paint_work_units: usize,
}

impl SvgImage {
    /// Parse an external image at its resolved viewport. SVG absolute units
    /// remain absolute; scaling a previously resolved tree would distort them
    /// when the original resource has percentage or missing dimensions.
    pub fn with_viewport(&self, width: f32, height: f32) -> Result<Self, SvgDecodeError> {
        let metadata = SvgImageMetadata {
            concrete_width: width,
            concrete_height: height,
            ..self.metadata
        };
        parse_svg_image(Arc::clone(&self.encoded), metadata, true)
    }

    pub fn tree(&self) -> &usvg::Tree {
        &self.tree
    }

    pub const fn metadata(&self) -> SvgImageMetadata {
        self.metadata
    }

    pub const fn paint_work_units(&self) -> usize {
        self.paint_work_units
    }
}

impl std::fmt::Debug for SvgImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SvgImage")
            .field("metadata", &self.metadata)
            .field("tree_size", &self.tree.size())
            .field("paint_work_units", &self.paint_work_units)
            .finish()
    }
}

// Equality is resource identity. A cloned `Arc<SvgImage>` remains equal while
// independently parsed trees do not require retaining their encoded XML solely
// to support structural comparisons in paint snapshots.
impl PartialEq for SvgImage {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for SvgImage {}

#[derive(Debug, thiserror::Error)]
pub enum SvgDecodeError {
    #[error("encoded SVG contains {actual} bytes, exceeding the {max}-byte input budget")]
    EncodedBufferBudgetExceeded { actual: usize, max: usize },
    #[error("SVG source is not valid UTF-8")]
    InvalidUtf8,
    #[error("SVG source is not well-formed XML: {0}")]
    MalformedXml(String),
    #[error("SVG source has no root <svg> start tag")]
    MissingRoot,
    #[error("SVG concrete object size must be finite and positive")]
    InvalidConcreteSize,
    #[error("SVG tree could not be parsed: {0}")]
    Parse(#[from] usvg::Error),
    #[error("SVG tree depth {actual} exceeds the {max}-level limit")]
    TreeDepthExceeded { actual: usize, max: usize },
    #[error("SVG contains {actual} vector work units, exceeding the {max}-unit limit")]
    PaintWorkBudgetExceeded { actual: usize, max: usize },
}

/// Reads SVG natural sizing without constructing a vector tree.
pub fn probe_svg_image(bytes: &[u8]) -> Result<SvgImageMetadata, SvgDecodeError> {
    check_encoded_budget(bytes)?;
    let source = std::str::from_utf8(bytes).map_err(|_| SvgDecodeError::InvalidUtf8)?;
    let attributes = svg_root_attributes(source)?;
    Ok(svg_image_metadata_from_root_attributes(
        attributes.width,
        attributes.height,
        attributes.view_box,
    ))
}

/// Resolves natural SVG sizing directly from one live root element's textual
/// attributes without serializing or constructing a vector tree.
///
/// Inline-SVG layout adapters use this metadata-only boundary so geometry
/// demands can honor `width`, `height`, and `viewBox` while the expensive
/// `usvg::Tree` remains exclusive to a paint demand.
pub fn svg_image_metadata_from_root_attributes(
    width: Option<&str>,
    height: Option<&str>,
    view_box: Option<&str>,
) -> SvgImageMetadata {
    let intrinsic_width = width.and_then(svg_absolute_length);
    let intrinsic_height = height.and_then(svg_absolute_length);
    let view_box_ratio = view_box
        .and_then(svg_view_box)
        .map(|(width, height)| width / height);
    let intrinsic_ratio = intrinsic_width
        .zip(intrinsic_height)
        .filter(|(_, height)| *height > 0.0)
        .map(|(width, height)| width / height)
        .or(view_box_ratio)
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
    let (concrete_width, concrete_height) =
        concrete_object_size(intrinsic_width, intrinsic_height, intrinsic_ratio);
    SvgImageMetadata {
        intrinsic_width,
        intrinsic_height,
        intrinsic_ratio,
        concrete_width,
        concrete_height,
    }
}

pub fn decode_svg_image(bytes: &[u8]) -> Result<SvgImage, SvgDecodeError> {
    let metadata = probe_svg_image(bytes)?;
    decode_svg_image_with_metadata(bytes, metadata)
}

/// Parses SVG after the caller has already admitted the immutable byte buffer.
pub fn decode_svg_image_with_metadata(
    bytes: &[u8],
    metadata: SvgImageMetadata,
) -> Result<SvgImage, SvgDecodeError> {
    check_encoded_budget(bytes)?;
    parse_svg_image(Arc::from(bytes), metadata, false)
}

fn parse_svg_image(
    encoded: Arc<[u8]>,
    metadata: SvgImageMetadata,
    explicit_viewport: bool,
) -> Result<SvgImage, SvgDecodeError> {
    let default_size = usvg::Size::from_wh(metadata.concrete_width, metadata.concrete_height)
        .ok_or(SvgDecodeError::InvalidConcreteSize)?;
    let options = usvg::Options {
        fontdb: SVG_FONT_DATABASE.clone(),
        default_size,
        // The default usvg resolver accepts data URLs and local filesystem
        // paths. Image resources must stay behind Moli's fetch and
        // decode owner, so Phase 10 deliberately omits nested raster/SVG
        // images instead of reading or decoding them inside this parser.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let viewport_source = if explicit_viewport {
        Some(svg_with_viewport(
            &encoded,
            metadata.concrete_width,
            metadata.concrete_height,
        )?)
    } else {
        None
    };
    let source = match &viewport_source {
        Some(source) => source.as_bytes(),
        None => &encoded,
    };
    let tree = usvg::Tree::from_data(source, &options)?;
    let paint_work_units = tree_work_units(&tree)?;
    Ok(SvgImage {
        tree,
        encoded,
        metadata,
        paint_work_units,
    })
}

/// usvg resolves a standalone root from its XML width/height attributes and
/// can shrink percentage roots to their content bounds. The embedding viewport
/// is supplied on a parsing copy, using XML token spans rather than text search.
fn svg_with_viewport(encoded: &[u8], width: f32, height: f32) -> Result<String, SvgDecodeError> {
    let source = std::str::from_utf8(encoded).map_err(|_| SvgDecodeError::InvalidUtf8)?;
    let mut root = false;
    let mut cursor = 0;
    let mut output = String::with_capacity(source.len() + 80);
    for token in xmlparser::Tokenizer::from(source) {
        match token.map_err(|error| SvgDecodeError::MalformedXml(error.to_string()))? {
            xmlparser::Token::ElementStart { local, .. } => {
                if root || local.as_str() != "svg" {
                    return Err(SvgDecodeError::MissingRoot);
                }
                root = true;
            }
            xmlparser::Token::Attribute {
                prefix,
                local,
                span,
                ..
            } if root
                && prefix.as_str().is_empty()
                && matches!(local.as_str(), "width" | "height") =>
            {
                output.push_str(&source[cursor..span.start()]);
                cursor = span.end();
            }
            xmlparser::Token::ElementEnd { span, .. } if root => {
                output.push_str(&source[cursor..span.start()]);
                output.push_str(&format!(" width=\"{width}\" height=\"{height}\""));
                output.push_str(&source[span.start()..]);
                return Ok(output);
            }
            _ => {}
        }
    }
    Err(SvgDecodeError::MissingRoot)
}

fn tree_work_units(tree: &usvg::Tree) -> Result<usize, SvgDecodeError> {
    let mut units = group_work_units(tree.root(), 1)?;
    for gradient in tree.linear_gradients() {
        add_work_units(&mut units, 1usize.saturating_add(gradient.stops().len()))?;
    }
    for gradient in tree.radial_gradients() {
        add_work_units(&mut units, 1usize.saturating_add(gradient.stops().len()))?;
    }
    for pattern in tree.patterns() {
        add_work_units(
            &mut units,
            1usize.saturating_add(group_work_units(pattern.root(), 1)?),
        )?;
    }
    for clip_path in tree.clip_paths() {
        add_work_units(
            &mut units,
            1usize.saturating_add(group_work_units(clip_path.root(), 1)?),
        )?;
    }
    for mask in tree.masks() {
        add_work_units(
            &mut units,
            1usize.saturating_add(group_work_units(mask.root(), 1)?),
        )?;
    }
    for filter in tree.filters() {
        add_work_units(&mut units, 1usize.saturating_add(filter.primitives().len()))?;
        for primitive in filter.primitives() {
            if let usvg::filter::Kind::Image(image) = primitive.kind() {
                add_work_units(&mut units, group_work_units(image.root(), 1)?)?;
            }
        }
    }
    Ok(units)
}

fn group_work_units(group: &usvg::Group, depth: usize) -> Result<usize, SvgDecodeError> {
    if depth > MAX_SVG_TREE_DEPTH {
        return Err(SvgDecodeError::TreeDepthExceeded {
            actual: depth,
            max: MAX_SVG_TREE_DEPTH,
        });
    }
    let mut units = 0usize;
    for node in group.children() {
        add_work_units(&mut units, 1)?;
        add_work_units(
            &mut units,
            match node {
                usvg::Node::Group(group) => group_work_units(group, depth + 1)?,
                usvg::Node::Path(path) => path.data().segments().count(),
                usvg::Node::Image(image) => match image.kind() {
                    usvg::ImageKind::SVG(tree) => group_work_units(tree.root(), depth + 1)?,
                    usvg::ImageKind::JPEG(_)
                    | usvg::ImageKind::PNG(_)
                    | usvg::ImageKind::GIF(_)
                    | usvg::ImageKind::WEBP(_) => 0,
                },
                usvg::Node::Text(text) => group_work_units(text.flattened(), depth + 1)?,
            },
        )?;
    }
    Ok(units)
}

fn add_work_units(total: &mut usize, additional: usize) -> Result<(), SvgDecodeError> {
    *total = total.saturating_add(additional);
    if *total > MAX_SVG_PAINT_WORK_UNITS {
        return Err(SvgDecodeError::PaintWorkBudgetExceeded {
            actual: *total,
            max: MAX_SVG_PAINT_WORK_UNITS,
        });
    }
    Ok(())
}

fn concrete_object_size(width: Option<f32>, height: Option<f32>, ratio: Option<f32>) -> (f32, f32) {
    concrete_object_size_with_default(
        width,
        height,
        ratio,
        DEFAULT_OBJECT_WIDTH,
        DEFAULT_OBJECT_HEIGHT,
    )
}

fn concrete_object_size_with_default(
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    default_width: f32,
    default_height: f32,
) -> (f32, f32) {
    match (width, height, ratio) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(ratio)) => (width, width / ratio),
        (Some(width), None, None) => (width, default_height),
        (None, Some(height), Some(ratio)) => (height * ratio, height),
        (None, Some(height), None) => (default_width, height),
        (None, None, Some(ratio)) => {
            let width_at_default_height = default_height * ratio;
            if width_at_default_height <= default_width {
                (width_at_default_height, default_height)
            } else {
                (default_width, default_width / ratio)
            }
        }
        (None, None, None) => (default_width, default_height),
    }
}

fn check_encoded_budget(bytes: &[u8]) -> Result<(), SvgDecodeError> {
    if bytes.len() > MAX_ENCODED_SVG_BYTES {
        return Err(SvgDecodeError::EncodedBufferBudgetExceeded {
            actual: bytes.len(),
            max: MAX_ENCODED_SVG_BYTES,
        });
    }
    Ok(())
}

#[derive(Default)]
struct SvgRootAttributes<'a> {
    width: Option<&'a str>,
    height: Option<&'a str>,
    view_box: Option<&'a str>,
}

/// Streams only the XML root start tag. This deliberately avoids constructing
/// either an XML DOM or a `usvg::Tree` on the network completion path; the
/// bounded worker performs the full parse exactly once after admission.
fn svg_root_attributes(source: &str) -> Result<SvgRootAttributes<'_>, SvgDecodeError> {
    let mut root_is_svg = false;
    let mut attributes = SvgRootAttributes::default();
    for token in xmlparser::Tokenizer::from(source) {
        match token.map_err(|error| SvgDecodeError::MalformedXml(error.to_string()))? {
            xmlparser::Token::ElementStart { local, .. } => {
                if root_is_svg || local.as_str() != "svg" {
                    return Err(SvgDecodeError::MissingRoot);
                }
                root_is_svg = true;
            }
            xmlparser::Token::Attribute {
                prefix,
                local,
                value,
                ..
            } if root_is_svg && prefix.as_str().is_empty() => match local.as_str() {
                "width" => attributes.width = Some(value.as_str()),
                "height" => attributes.height = Some(value.as_str()),
                "viewBox" => attributes.view_box = Some(value.as_str()),
                _ => {}
            },
            xmlparser::Token::ElementEnd { .. } if root_is_svg => return Ok(attributes),
            _ => {}
        }
    }
    Err(SvgDecodeError::MissingRoot)
}

fn svg_absolute_length(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() || value.ends_with('%') {
        return None;
    }
    let units = [
        ("px", 1.0_f32),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("q", 96.0 / 101.6),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
    ];
    let (number, scale) = units
        .iter()
        .find_map(|(unit, scale)| value.strip_suffix(unit).map(|number| (number, *scale)))
        .unwrap_or((value.as_str(), 1.0));
    let number = number.trim().parse::<f32>().ok()?;
    let resolved = number * scale;
    (resolved.is_finite() && resolved >= 0.0).then_some(resolved)
}

fn svg_view_box(value: &str) -> Option<(f32, f32)> {
    let mut parts = value
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>);
    let _min_x = parts.next()?.ok()?;
    let _min_y = parts.next()?.ok()?;
    let width = parts.next()?.ok()?;
    let height = parts.next()?.ok()?;
    if parts.next().is_some() || !width.is_finite() || !height.is_finite() {
        return None;
    }
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn rounded_dimension(value: f32) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > u32::MAX as f32 {
        return None;
    }
    Some(value.round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_viewports_resolve_percentage_roots_without_scaling_absolute_units() {
        let svg = decode_svg_image(br#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%"><rect width="1" height="1" fill="red"/></svg>"#).unwrap();
        let resized = svg.with_viewport(12.0, 8.0).unwrap();
        assert_eq!(
            (
                resized.tree().size().width(),
                resized.tree().size().height()
            ),
            (12.0, 8.0)
        );
        assert_eq!(resized.tree().root().abs_bounding_box().width(), 1.0);
        assert!(Arc::ptr_eq(&svg.encoded, &resized.encoded));
        assert_eq!(resized.metadata().intrinsic_width, None);
        assert_eq!(resized.metadata().intrinsic_height, None);
        let ratio =
            probe_svg_image(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"/>"#)
                .unwrap()
                .with_default_object_size(12.0, 8.0);
        assert_eq!((ratio.concrete_width, ratio.concrete_height), (12.0, 6.0));
        let ratio_image = decode_svg_image(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"><rect width="1" height="1" fill="red"/></svg>"#).unwrap();
        let resized = ratio_image.with_viewport(12.0, 8.0).unwrap();
        assert_eq!(
            (
                resized.tree().size().width(),
                resized.tree().size().height()
            ),
            (12.0, 8.0)
        );
        assert_eq!(resized.tree().root().abs_bounding_box().width(), 3.0);
    }

    #[test]
    fn metadata_preserves_natural_dimensions_and_applies_default_object_size() {
        let explicit = probe_svg_image(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="25.4mm"/>"#,
        )
        .unwrap();
        assert_eq!(explicit.intrinsic_width, Some(192.0));
        assert!((explicit.intrinsic_height.unwrap() - 96.0).abs() < 0.001);
        assert_eq!(explicit.concrete_dimensions(), Some((192, 96)));

        let ratio_only =
            probe_svg_image(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"#)
                .unwrap();
        assert_eq!(ratio_only.intrinsic_width, None);
        assert_eq!(ratio_only.intrinsic_height, None);
        assert_eq!(ratio_only.intrinsic_ratio, Some(1.0));
        assert_eq!(ratio_only.concrete_dimensions(), Some((150, 150)));

        let no_natural_size =
            probe_svg_image(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#).unwrap();
        assert_eq!(no_natural_size.concrete_dimensions(), Some((300, 150)));

        let width_and_ratio = decode_svg_image(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" viewBox="0 0 100 100"/>"#,
        )
        .unwrap();
        assert_eq!(
            (
                width_and_ratio.tree().size().width(),
                width_and_ratio.tree().size().height(),
            ),
            (200.0, 200.0)
        );
    }

    #[test]
    fn parsed_tree_retains_vector_work_without_a_raster_surface() {
        let svg = decode_svg_image(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><path d="M0 0h10v10z" fill="red"/></svg>"#,
        )
        .unwrap();
        assert_eq!(svg.metadata().concrete_dimensions(), Some((20, 10)));
        assert!(svg.paint_work_units() >= 2);
        assert_eq!(svg.tree().size().width(), 20.0);
        assert_eq!(svg.tree().size().height(), 10.0);
    }

    #[test]
    fn css_color4_gradient_stops_preserve_style_precedence_and_alpha() {
        let svg = decode_svg_image(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                <defs><linearGradient id="paint">
                    <stop offset="0" stop-color="#00ff00" style="stop-color:#ff0000;stop-color:color(display-p3 .5 0 0 / .5);stop-opacity:.5"/>
                    <stop offset="1" stop-color="color(display-p3 0 0 1)"/>
                </linearGradient></defs>
                <rect width="20" height="10" fill="url(#paint)"/>
            </svg>"##,
        )
        .unwrap();
        let stops = svg.tree().linear_gradients()[0].stops();
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].color(), usvg::Color::new_rgb(140, 0, 0));
        assert!((stops[0].opacity().get() - 128.0 / 255.0 * 0.5).abs() < 1e-6);
        assert_eq!(stops[1].color(), usvg::Color::new_rgb(0, 0, 255));
    }

    #[test]
    fn css_color4_stylesheet_colors_preserve_current_color_inheritance() {
        let rules = "stop { stop-color: color(display-p3 .5 0 0); } .endpoint { stop-color: currentColor; color: color(display-p3 0 0 1); }";
        for stylesheet in [rules.to_owned(), format!("<![CDATA[{rules}]]>")] {
            let source = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                    <style>{stylesheet}</style>
                    <defs><linearGradient id="paint">
                        <stop offset="0"/><stop class="endpoint" offset="1"/>
                    </linearGradient></defs>
                    <rect width="20" height="10" fill="url(#paint)"/>
                </svg>"##
            );
            let svg = decode_svg_image(source.as_bytes()).unwrap();
            let stops = svg.tree().linear_gradients()[0].stops();
            assert_eq!(stops[0].color(), usvg::Color::new_rgb(140, 0, 0));
            assert_eq!(stops[1].color(), usvg::Color::new_rgb(0, 0, 255));
        }
    }

    #[test]
    fn vector_work_budget_includes_definition_subtrees() {
        let svg = decode_svg_image(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                <defs>
                    <clipPath id="clip"><path d="M0 0h10v10z"/></clipPath>
                    <linearGradient id="paint"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient>
                </defs>
                <rect width="20" height="10" clip-path="url(#clip)" fill="url(#paint)"/>
            </svg>"##,
        )
        .unwrap();

        assert!(svg.paint_work_units() >= 10);
    }

    #[test]
    fn encoded_svg_budget_is_checked_before_tree_parsing() {
        let oversized = vec![b' '; MAX_ENCODED_SVG_BYTES + 1];
        assert!(matches!(
            probe_svg_image(&oversized),
            Err(SvgDecodeError::EncodedBufferBudgetExceeded { .. })
        ));
    }

    #[test]
    fn metadata_probe_uses_the_xml_root_instead_of_matching_comment_text() {
        let metadata = probe_svg_image(
            br#"<?xml version="1.0"?><!-- <svg width="999" height="999"> --><svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"/>"#,
        )
        .unwrap();
        assert_eq!(metadata.concrete_dimensions(), Some((20, 10)));
        assert!(matches!(
            probe_svg_image(br#"<html><svg/></html>"#),
            Err(SvgDecodeError::MissingRoot)
        ));
    }

    #[test]
    fn parser_does_not_retain_nested_image_resources() {
        let svg = decode_svg_image(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <image width="10" height="10" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lw2K7wAAAABJRU5ErkJggg=="/>
                <image width="10" height="10" href="/tmp/moli-must-not-read.png"/>
            </svg>"#,
        )
        .expect("unsupported nested images should be omitted, not fail the outer SVG");

        assert!(!group_contains_image(svg.tree().root()));
    }

    fn group_contains_image(group: &usvg::Group) -> bool {
        group.children().iter().any(|node| match node {
            usvg::Node::Group(group) => group_contains_image(group),
            usvg::Node::Image(_) => true,
            usvg::Node::Path(_) | usvg::Node::Text(_) => false,
        })
    }
}

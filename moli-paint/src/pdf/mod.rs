//! Native PDF output from owned paint snapshots. Layout and shaping remain in Moli.
//!
//! PDF pages are slices of the existing continuous print layout. Unsupported
//! effects are rasterized at their own clipped bounds; ordinary text stays text.

mod image;
mod links;
mod primitives;
mod text;
mod validation;

use std::{collections::BTreeSet, fmt, ops::RangeInclusive};

use krilla::{Document, geom::Size, page::PageSettings};
use moli_layout::{
    PaintDiagnostic, PaintDiagnosticSeverity, PaintFragment, PaintRect, PaintSnapshot,
};
use peniko::kurbo::{Affine, Rect};

use primitives::PdfPainter;

/// Page geometry in PDF points (72 per inch). Ranges are zero-based and inclusive.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOptions {
    pub width: f64,
    pub height: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub scale: f64,
    pub page_ranges: Vec<RangeInclusive<usize>>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            width: 612.0,
            height: 792.0,
            margin_top: 72.0 / 2.54,
            margin_bottom: 72.0 / 2.54,
            margin_left: 72.0 / 2.54,
            margin_right: 72.0 / 2.54,
            scale: 1.0,
            page_ranges: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum PdfError {
    InvalidParameters,
    PageRangeExceeded,
    ResourceLimit,
    Paint(crate::PaintError),
    Encoding(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameters => f.write_str("invalid print parameters"),
            Self::PageRangeExceeded => f.write_str("Page range exceeds page count"),
            Self::ResourceLimit => f.write_str("Printing failed: PDF work budget exceeded"),
            Self::Paint(error) => error.fmt(f),
            Self::Encoding(message) => write!(f, "Printing failed: {message}"),
        }
    }
}
impl std::error::Error for PdfError {}
impl From<crate::PaintError> for PdfError {
    fn from(error: crate::PaintError) -> Self {
        Self::Paint(error)
    }
}

/// Effects in `snapshot` that [`pdf_snapshot`] omits, in addition to
/// `PaintSnapshot::diagnostics`. Callers report these alongside the PDF.
pub fn pdf_snapshot_diagnostics(snapshot: &PaintSnapshot) -> Vec<PaintDiagnostic> {
    let backdrops = snapshot
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment, PaintFragment::BackdropFilter { .. }))
        .count();
    if backdrops == 0 {
        return Vec::new();
    }
    vec![PaintDiagnostic::new(
        "backdrop-filter-unsupported",
        format!("PDF export omits {backdrops} backdrop-filter effect(s)"),
        PaintDiagnosticSeverity::Warning,
    )]
}

/// Writes embedded, subsetted fonts and native drawing commands. Device pixel
/// ratio does not affect page geometry. This does not perform CSS fragmentation.
pub fn pdf_snapshot(snapshot: &PaintSnapshot, options: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    let metrics = crate::validation::validate_fragment_stream(snapshot)?;
    validation::validate(snapshot)?;
    let width = f64::from(snapshot.surface.css_width);
    let height = f64::from(snapshot.surface.css_height);
    let printable_width = options.width - options.margin_left - options.margin_right;
    let printable_height = options.height - options.margin_top - options.margin_bottom;
    if [
        width,
        height,
        options.width,
        options.height,
        printable_width,
        printable_height,
        options.scale,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value <= 0.0)
        || [
            options.margin_top,
            options.margin_bottom,
            options.margin_left,
            options.margin_right,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || options.width > 14_400.0
        || options.height > 14_400.0
        || !(0.1..=2.0).contains(&options.scale)
    {
        return Err(PdfError::InvalidParameters);
    }
    // Preserve the existing shrink-to-printable-width contract.
    let scale = printable_width * options.scale / width;
    let page_height = printable_height / scale;
    let page_count = (height / page_height).ceil().max(1.0);
    if !page_count.is_finite() || page_count > 100_000.0 {
        return Err(PdfError::ResourceLimit);
    }
    let page_count = page_count as usize;
    let selected: BTreeSet<usize> = if options.page_ranges.is_empty() {
        (0..page_count).collect()
    } else {
        options
            .page_ranges
            .iter()
            .flat_map(|range| *range.start()..=(*range.end()).min(page_count - 1))
            .collect()
    };
    if selected.is_empty() {
        return Err(PdfError::PageRangeExceeded);
    }
    if selected
        .len()
        .saturating_mul(metrics.operation_count.saturating_add(snapshot.links.len()))
        > 20_000_000
    {
        return Err(PdfError::ResourceLimit);
    }
    let mut document = Document::new();
    let mut fonts = text::FontCache::default();
    let mut images = std::collections::HashMap::new();
    let scopes = primitives::scope_ends(&snapshot.fragments)?;
    let regions = links::collect(snapshot)?;
    if selected.len().saturating_mul(regions.len()) > 1_000_000 {
        return Err(PdfError::ResourceLimit);
    }
    let selected = selected
        .into_iter()
        .enumerate()
        .map(|(output, original)| (original, output))
        .collect();
    let page_links = links::PageLinks {
        regions: &regions,
        selected: &selected,
        page_height,
        scale,
        options,
    };
    for &page_index in selected.keys() {
        let top = page_index as f64 * page_height;
        let source = Rect::new(0.0, top, printable_width / scale, top + page_height);
        let mut page = document.start_page_with(
            PageSettings::from_wh(options.width as f32, options.height as f32)
                .ok_or(PdfError::InvalidParameters)?,
        );
        let mut surface = page.surface();
        let target = Rect::new(
            options.margin_left,
            options.margin_top,
            options.width - options.margin_right,
            options.height - options.margin_bottom,
        );
        surface.push_clip_path(
            &primitives::path(&target).ok_or(PdfError::InvalidParameters)?,
            &Default::default(),
        );
        surface.push_transform(&primitives::transform(
            Affine::translate((options.margin_left, options.margin_top - top * scale))
                * Affine::scale(scale),
        ));
        let mut painter = PdfPainter::new(
            &mut surface,
            snapshot,
            source,
            page_height,
            page_index,
            &mut fonts,
            &mut images,
        );
        painter.fill_canvas();
        let result = painter.paint(&snapshot.fragments, &scopes);
        drop(painter);
        surface.pop();
        surface.pop();
        surface.finish();
        result?;
        page_links.append(&mut page, source)?;
        page.finish();
    }
    let mut bytes = document
        .finish()
        .map_err(|error| PdfError::Encoding(format!("{error:?}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn rect(rect: PaintRect) -> Rect {
    Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x + rect.width),
        f64::from(rect.y + rect.height),
    )
}

fn image_size(width: u32, height: u32) -> Result<Size, PdfError> {
    Size::from_wh(width as f32, height as f32).ok_or(PdfError::InvalidParameters)
}

#[cfg(test)]
mod links_tests;
#[cfg(test)]
mod tests;

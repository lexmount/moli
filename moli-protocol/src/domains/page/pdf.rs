use std::{collections::BTreeSet, io::Write as _};

use crate::automation::DevToolsPrintToPdfCommand;

const POINTS_PER_INCH: f64 = 72.0;
const DEFAULT_MARGIN_INCHES: f64 = 1.0 / 2.54;
const DEFAULT_PAGE_WIDTH_INCHES: f64 = 8.5;
const DEFAULT_PAGE_HEIGHT_INCHES: f64 = 11.0;
const MAX_PAGE_DIMENSION_INCHES: f64 = 200.0;
const MAX_PAGE_COUNT: usize = 100_000;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RasterPdfOptions {
    page_width_points: f64,
    page_height_points: f64,
    margin_top_points: f64,
    margin_bottom_points: f64,
    margin_left_points: f64,
    margin_right_points: f64,
    scale: f64,
    page_ranges: Vec<PageRange>,
}

impl Default for RasterPdfOptions {
    fn default() -> Self {
        Self {
            page_width_points: DEFAULT_PAGE_WIDTH_INCHES * POINTS_PER_INCH,
            page_height_points: DEFAULT_PAGE_HEIGHT_INCHES * POINTS_PER_INCH,
            margin_top_points: DEFAULT_MARGIN_INCHES * POINTS_PER_INCH,
            margin_bottom_points: DEFAULT_MARGIN_INCHES * POINTS_PER_INCH,
            margin_left_points: DEFAULT_MARGIN_INCHES * POINTS_PER_INCH,
            margin_right_points: DEFAULT_MARGIN_INCHES * POINTS_PER_INCH,
            scale: 1.0,
            page_ranges: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PageRange {
    first: usize,
    last: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PdfError {
    code: i32,
    message: String,
}

impl PdfError {
    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
        }
    }

    fn server(message: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
        }
    }

    pub(super) fn code(&self) -> i32 {
        self.code
    }

    pub(super) fn message(&self) -> &str {
        &self.message
    }
}

impl RasterPdfOptions {
    pub(super) fn from_command(command: &DevToolsPrintToPdfCommand) -> Result<Self, PdfError> {
        let scale = command.scale.unwrap_or(1.0);
        if !scale.is_finite() || !(0.1..=2.0).contains(&scale) {
            return Err(PdfError::invalid_params(
                "scale is outside of [0.1 - 2] range",
            ));
        }

        let margin_left = finite_margin(command.margin_left, "left margin is negative")?;
        let margin_right = finite_margin(command.margin_right, "right margin is negative")?;
        let margin_top = finite_margin(command.margin_top, "top margin is negative")?;
        let margin_bottom = finite_margin(command.margin_bottom, "bottom margin is negative")?;
        let mut paper_width = finite_paper_dimension(
            command.paper_width,
            DEFAULT_PAGE_WIDTH_INCHES,
            "paper width is zero or negative",
        )?;
        let mut paper_height = finite_paper_dimension(
            command.paper_height,
            DEFAULT_PAGE_HEIGHT_INCHES,
            "paper height is zero or negative",
        )?;
        if command.landscape.unwrap_or(false) {
            std::mem::swap(&mut paper_width, &mut paper_height);
        }
        if paper_width > MAX_PAGE_DIMENSION_INCHES || paper_height > MAX_PAGE_DIMENSION_INCHES {
            return Err(PdfError::invalid_params("invalid print parameters"));
        }
        if paper_width <= margin_left + margin_right || paper_height <= margin_top + margin_bottom {
            return Err(PdfError::invalid_params(
                "invalid print parameters: printable area is empty",
            ));
        }

        Ok(Self {
            page_width_points: paper_width * POINTS_PER_INCH,
            page_height_points: paper_height * POINTS_PER_INCH,
            margin_top_points: margin_top * POINTS_PER_INCH,
            margin_bottom_points: margin_bottom * POINTS_PER_INCH,
            margin_left_points: margin_left * POINTS_PER_INCH,
            margin_right_points: margin_right * POINTS_PER_INCH,
            scale,
            page_ranges: parse_page_ranges(command.page_ranges.as_deref().unwrap_or(""))?,
        })
    }

    fn printable_width(&self) -> f64 {
        self.page_width_points - self.margin_left_points - self.margin_right_points
    }

    fn printable_height(&self) -> f64 {
        self.page_height_points - self.margin_top_points - self.margin_bottom_points
    }

    fn selected_pages(&self, page_count: usize) -> Result<Vec<usize>, PdfError> {
        if self.page_ranges.is_empty() {
            return Ok((0..page_count).collect());
        }
        let mut pages = BTreeSet::new();
        for range in &self.page_ranges {
            if range.first >= page_count {
                continue;
            }
            let last = range.last.min(page_count.saturating_sub(1));
            pages.extend(range.first..=last);
        }
        if pages.is_empty() {
            return Err(PdfError::server("Page range exceeds page count"));
        }
        Ok(pages.into_iter().collect())
    }
}

fn finite_margin(value: Option<f64>, negative_message: &'static str) -> Result<f64, PdfError> {
    let value = value.unwrap_or(DEFAULT_MARGIN_INCHES);
    if !value.is_finite() {
        return Err(PdfError::invalid_params("invalid print parameters"));
    }
    if value < 0.0 {
        return Err(PdfError::invalid_params(negative_message));
    }
    Ok(value)
}

fn finite_paper_dimension(
    value: Option<f64>,
    default: f64,
    non_positive_message: &'static str,
) -> Result<f64, PdfError> {
    let value = value.unwrap_or(default);
    if !value.is_finite() {
        return Err(PdfError::invalid_params("invalid print parameters"));
    }
    if value <= 0.0 {
        return Err(PdfError::invalid_params(non_positive_message));
    }
    Ok(value)
}

fn parse_page_ranges(value: &str) -> Result<Vec<PageRange>, PdfError> {
    let mut ranges = Vec::new();
    for range in value
        .split(',')
        .map(str::trim)
        .filter(|range| !range.is_empty())
    {
        let (first, last) = if range == "-" {
            (1, usize::MAX)
        } else if let Some(last) = range.strip_prefix('-') {
            (1, parse_page_number(last)?)
        } else if let Some(first) = range.strip_suffix('-') {
            (parse_page_number(first)?, usize::MAX)
        } else if range.contains('-') {
            let mut parts = range.split('-').map(str::trim);
            let first = parts.next().ok_or_else(page_range_syntax_error)?;
            let last = parts.next().ok_or_else(page_range_syntax_error)?;
            if parts.next().is_some() {
                return Err(page_range_syntax_error());
            }
            (parse_page_number(first)?, parse_page_number(last)?)
        } else {
            let page = parse_page_number(range)?;
            (page, page)
        };
        if first == 0 || first > last {
            return Err(PdfError::server("Page range is invalid (start > end)"));
        }
        ranges.push(PageRange {
            first: first - 1,
            last: last.saturating_sub(1),
        });
    }
    Ok(ranges)
}

fn parse_page_number(value: &str) -> Result<usize, PdfError> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(page_range_syntax_error());
    }
    value.parse().map_err(|_| page_range_syntax_error())
}

fn page_range_syntax_error() -> PdfError {
    PdfError::server("Page range syntax error")
}

/// Wraps one full-document JPEG in a paginated PDF.
///
/// The raster is kept as one shared image XObject. Each output page clips and
/// translates that image to expose the corresponding document slice, avoiding
/// another decode/encode pass and keeping the result deterministic.
pub(super) fn build_raster_pdf(
    jpeg: &[u8],
    image_width: u32,
    image_height: u32,
    options: &RasterPdfOptions,
) -> Result<Vec<u8>, PdfError> {
    if jpeg.is_empty() || image_width == 0 || image_height == 0 {
        return Err(PdfError::server("Printing failed"));
    }

    let printable_width = options.printable_width();
    let printable_height = options.printable_height();
    let display_width = printable_width * options.scale;
    let display_height = display_width * f64::from(image_height) / f64::from(image_width);
    if !display_height.is_finite() || display_height <= 0.0 {
        return Err(PdfError::server("Printing failed"));
    }
    let page_count = (display_height / printable_height).ceil().max(1.0) as usize;
    if page_count > MAX_PAGE_COUNT {
        return Err(PdfError::server(format!(
            "Printing failed: document exceeds {MAX_PAGE_COUNT} pages"
        )));
    }
    let selected_pages = options.selected_pages(page_count)?;

    let page_object_ids = (0..selected_pages.len())
        .map(|index| 4 + index * 2)
        .collect::<Vec<_>>();
    let object_count = 3 + selected_pages.len() * 2;
    let mut objects = vec![Vec::new(); object_count + 1];
    objects[1] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();

    let kids = page_object_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    objects[2] = format!(
        "<< /Type /Pages /Kids [{kids}] /Count {} >>",
        selected_pages.len()
    )
    .into_bytes();

    let image_dictionary = format!(
        "<< /Type /XObject /Subtype /Image /Width {image_width} /Height {image_height} \
         /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
        jpeg.len()
    );
    let mut image_object = image_dictionary.into_bytes();
    image_object.extend_from_slice(jpeg);
    image_object.extend_from_slice(b"\nendstream");
    objects[3] = image_object;

    for (output_index, source_page) in selected_pages.iter().copied().enumerate() {
        let page_id = page_object_ids[output_index];
        let content_id = page_id + 1;
        objects[page_id] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] \
             /Resources << /XObject << /Im0 3 0 R >> >> /Contents {content_id} 0 R >>",
            pdf_number(options.page_width_points),
            pdf_number(options.page_height_points),
        )
        .into_bytes();

        let clip_bottom = options.margin_bottom_points;
        let image_bottom = options.page_height_points - options.margin_top_points - display_height
            + source_page as f64 * printable_height;
        let content = format!(
            "q\n{} {} {} {} re W n\n{} 0 0 {} {} {} cm\n/Im0 Do\nQ\n",
            pdf_number(options.margin_left_points),
            pdf_number(clip_bottom),
            pdf_number(printable_width),
            pdf_number(printable_height),
            pdf_number(display_width),
            pdf_number(display_height),
            pdf_number(options.margin_left_points),
            pdf_number(image_bottom),
        );
        objects[content_id] = stream_object(content.as_bytes());
    }

    serialize_pdf(objects)
}

fn stream_object(bytes: &[u8]) -> Vec<u8> {
    let mut object = format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes();
    object.extend_from_slice(bytes);
    object.extend_from_slice(b"endstream");
    object
}

fn serialize_pdf(objects: Vec<Vec<u8>>) -> Result<Vec<u8>, PdfError> {
    let mut output = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize; objects.len()];
    for (id, object) in objects.iter().enumerate().skip(1) {
        if object.is_empty() {
            return Err(PdfError::server("Printing failed"));
        }
        offsets[id] = output.len();
        let _ = writeln!(output, "{id} 0 obj");
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
    }

    let xref_offset = output.len();
    let _ = write!(output, "xref\n0 {}\n", objects.len());
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.into_iter().skip(1) {
        let _ = writeln!(output, "{offset:010} 00000 n ");
    }
    let _ = write!(
        output,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        objects.len()
    );
    Ok(output)
}

fn pdf_number(value: f64) -> String {
    let mut value = format!("{value:.4}");
    while value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    if value == "-0" {
        value = "0".to_owned();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::{AutomationContext, DevToolsPrintToPdfTransferMode, FrontendProtocol};

    fn command() -> DevToolsPrintToPdfCommand {
        DevToolsPrintToPdfCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::Cdp,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            landscape: None,
            print_background: None,
            scale: None,
            paper_width: None,
            paper_height: None,
            margin_top: None,
            margin_bottom: None,
            margin_left: None,
            margin_right: None,
            page_ranges: None,
            shrink_to_fit: None,
            transfer_mode: Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64),
        }
    }

    #[test]
    fn writes_cross_referenced_paginated_pdf() {
        let mut command = command();
        command.paper_width = Some(4.0);
        command.paper_height = Some(4.0);
        command.margin_top = Some(0.0);
        command.margin_bottom = Some(0.0);
        command.margin_left = Some(0.0);
        command.margin_right = Some(0.0);
        let options = RasterPdfOptions::from_command(&command).unwrap();
        let pdf = build_raster_pdf(&[0xff, 0xd8, 0xff, 0xd9], 400, 900, &options).unwrap();
        let text = String::from_utf8_lossy(&pdf);

        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        assert!(text.contains("/Count 3"));
        assert_eq!(text.matches("/Type /Page ").count(), 3);
        assert_eq!(text.matches("/Subtype /Image").count(), 1);

        let xref_offset = text
            .rsplit_once("startxref\n")
            .and_then(|(_, tail)| tail.lines().next())
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(&pdf[xref_offset..xref_offset + 4], b"xref");
    }

    #[test]
    fn landscape_swaps_media_box_and_ranges_are_normalized() {
        let mut command = command();
        command.landscape = Some(true);
        command.paper_width = Some(8.0);
        command.paper_height = Some(10.0);
        command.margin_top = Some(0.0);
        command.margin_bottom = Some(0.0);
        command.margin_left = Some(0.0);
        command.margin_right = Some(0.0);
        command.page_ranges = Some("3, 1-2,2".to_owned());
        let options = RasterPdfOptions::from_command(&command).unwrap();
        let pdf = build_raster_pdf(&[0xff, 0xd8, 0xff, 0xd9], 800, 2400, &options).unwrap();
        let text = String::from_utf8_lossy(&pdf);

        assert!(text.contains("/MediaBox [0 0 720 576]"));
        assert!(text.contains("/Count 3"));
    }

    #[test]
    fn validates_chromium_scale_and_page_range_errors() {
        let mut invalid_scale = command();
        invalid_scale.scale = Some(2.01);
        let error = RasterPdfOptions::from_command(&invalid_scale).unwrap_err();
        assert_eq!(error.code(), -32602);
        assert_eq!(error.message(), "scale is outside of [0.1 - 2] range");

        let mut invalid_range = command();
        invalid_range.page_ranges = Some("4-2".to_owned());
        let error = RasterPdfOptions::from_command(&invalid_range).unwrap_err();
        assert_eq!(error.code(), -32000);
        assert_eq!(error.message(), "Page range is invalid (start > end)");

        let mut syntax_error = command();
        syntax_error.page_ranges = Some("1-a".to_owned());
        let error = RasterPdfOptions::from_command(&syntax_error).unwrap_err();
        assert_eq!(error.message(), "Page range syntax error");
    }

    #[test]
    fn rejects_ranges_wholly_beyond_document() {
        let mut command = command();
        command.page_ranges = Some("998-999".to_owned());
        command.margin_top = Some(0.0);
        command.margin_bottom = Some(0.0);
        command.margin_left = Some(0.0);
        command.margin_right = Some(0.0);
        let options = RasterPdfOptions::from_command(&command).unwrap();
        let error = build_raster_pdf(&[0xff, 0xd8, 0xff, 0xd9], 800, 600, &options).unwrap_err();
        assert_eq!(error.message(), "Page range exceeds page count");
    }
}

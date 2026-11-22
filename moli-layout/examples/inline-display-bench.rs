//! Reproducible inline projection benchmark, without a renderer or V8.
//! cargo run --release -p moli-layout --example inline-display-bench -- paragraphs paint 15 1000
//! Cases: paragraphs, styled, ellipsis, forced-lines, atomic. Modes: layout, paint, print.

use std::{collections::HashMap, hint::black_box, time::Instant};

use moli_layout::{
    DocumentLayoutServices, LayoutElementCategory, LayoutElementSemantics, LayoutError,
    LayoutFlushReason, LayoutNamespace, LayoutPassRequest, LayoutSource, LayoutSourceKind,
    LayoutStyleResolver, LayoutViewport, ResolvedLayoutElementStyles, ResolvedLayoutStyle,
    SystemFontPolicy, WebFontFace, WebFontRegistration, build_layout_pass,
};
use style::values::{
    computed::{
        CSSPixelLength, LengthPercentage,
        font::{FamilyName, FontFamily, FontFamilyList, FontFamilyNameSyntax, SingleFontFamily},
        length::Size,
    },
    generics::NonNegative,
    specified::{
        box_::Display,
        text::{TextOverflow, TextOverflowSide},
    },
};
use style::{Atom, properties::ComputedValues, servo_arc::Arc};

struct Node {
    name: &'static str,
    text: Option<String>,
    children: Vec<usize>,
    parent: Option<usize>,
}
struct Source(Vec<Node>);
impl Source {
    fn push(&mut self, parent: usize, name: &'static str, text: Option<String>) -> usize {
        let id = self.0.len();
        self.0.push(Node {
            name,
            text,
            children: Vec::new(),
            parent: Some(parent),
        });
        self.0[parent].children.push(id);
        id
    }
}
impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;
    fn root(&self) -> usize {
        0
    }
    fn label(&self, node: usize) -> String {
        self.0[node].name.into()
    }
    fn flat_parent(&self, node: usize) -> Option<usize> {
        self.0[node].parent
    }
    fn flat_children(&self, node: usize) -> Self::ChildIter<'_> {
        self.0[node].children.iter().copied()
    }
    fn node_kind(&self, node: usize) -> LayoutSourceKind {
        if self.0[node].text.is_some() {
            LayoutSourceKind::Text
        } else {
            LayoutSourceKind::Element
        }
    }
    fn text(&self, node: usize) -> Option<&str> {
        self.0[node].text.as_deref()
    }
    fn element_semantics(&self, node: usize) -> Option<LayoutElementSemantics> {
        self.0[node].text.is_none().then(|| {
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                self.0[node].name,
                LayoutElementCategory::Generic,
                None,
            )
        })
    }
}
struct Styles(HashMap<usize, ResolvedLayoutStyle>);
impl LayoutStyleResolver<usize> for Styles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok(self
            .0
            .get(&node)
            .cloned()
            .map(ResolvedLayoutElementStyles::from_primary))
    }
}

fn style(width: f32, display: Display, ellipsis: bool, preserve: bool) -> ResolvedLayoutStyle {
    let mut font = style::properties::style_structs::Font::initial_values();
    font.set_font_family(FontFamily {
        families: FontFamilyList {
            list: style::ArcSlice::from_iter(std::iter::once(SingleFontFamily::FamilyName(
                FamilyName {
                    name: Atom::from("BenchAhem"),
                    syntax: FontFamilyNameSyntax::Quoted,
                },
            ))),
        },
        is_system_font: false,
        is_initial: false,
    });
    let mut values = (*ComputedValues::initial_values_with_font_override(font)).clone();
    values.mutate_box().set_display(display);
    values
        .mutate_position()
        .set_width(Size::LengthPercentage(NonNegative(
            LengthPercentage::new_length(CSSPixelLength::new(width)),
        )));
    if ellipsis {
        values
            .mutate_box()
            .set_overflow_x(style::values::computed::Overflow::Hidden);
        values
            .mutate_box()
            .set_overflow_y(style::values::computed::Overflow::Hidden);
        values.mutate_inherited_text().set_text_wrap_mode(
            style::properties::longhands::text_wrap_mode::computed_value::T::Nowrap,
        );
        values.mutate_text().set_text_overflow(TextOverflow {
            first: TextOverflowSide::Clip,
            second: TextOverflowSide::Ellipsis,
            sides_are_logical: true,
        });
    }
    if preserve {
        values.mutate_inherited_text().set_white_space_collapse(
            style::properties::longhands::white_space_collapse::computed_value::T::Preserve,
        );
    }
    ResolvedLayoutStyle::from_stylo(Arc::new(values)).with_text_metrics(16.0, 20.0)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let case = args.get(1).map_or("paragraphs", String::as_str);
    let mode = args.get(2).map_or("paint", String::as_str);
    let iterations = args
        .get(3)
        .map_or(15, |value| value.parse::<usize>().unwrap())
        .max(1);
    let count = args
        .get(4)
        .map_or(1000, |value| value.parse::<usize>().unwrap())
        .max(1);
    assert!(["paragraphs", "styled", "ellipsis", "forced-lines", "atomic"].contains(&case));
    assert!(["layout", "paint", "print"].contains(&mode));
    let mut source = Source(vec![Node {
        name: "div",
        text: None,
        children: Vec::new(),
        parent: None,
    }]);
    let root = style(800.0, Display::Block, false, false);
    let block = style(
        280.0,
        Display::Block,
        matches!(case, "ellipsis" | "atomic"),
        false,
    );
    let inline = style(0.0, Display::Inline, false, false);
    let atomic = style(80.0, Display::InlineBlock, false, false);
    let mut styles = Styles(HashMap::from([(0, root)]));
    let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 ABCDEFGHIJKLMNOP";
    if case == "forced-lines" {
        styles.0.insert(0, style(280.0, Display::Block, true, true));
        source.push(
            0,
            "#text",
            Some(
                std::iter::repeat_n(text, count)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        );
    } else if case == "styled" {
        for _ in 0..count {
            let span = source.push(0, "span", None);
            styles.0.insert(span, inline.clone());
            source.push(span, "#text", Some("ABCD ".into()));
        }
    } else {
        for _ in 0..count {
            let paragraph = source.push(0, "p", None);
            styles.0.insert(paragraph, block.clone());
            if case == "atomic" {
                source.push(paragraph, "#text", Some("ABC ".into()));
                let item = source.push(paragraph, "span", None);
                styles.0.insert(item, atomic.clone());
                source.push(item, "#text", Some("INNER".into()));
            }
            source.push(paragraph, "#text", Some(text.into()));
        }
    }
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(WebFontRegistration::new(
            "bench",
            WebFontFace::new("BenchAhem"),
            include_bytes!("../tests/fixtures/moli-ahem.ttf").to_vec(),
        ))
        .unwrap();
    let viewport = LayoutViewport::new(800, 600, 1.0);
    let request = match mode {
        "layout" => LayoutPassRequest::new(viewport, LayoutFlushReason::SynchronousGeometry),
        "print" => LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Print),
        _ => LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
    };
    let mut elapsed = Vec::new();
    let mut projection = Vec::new();
    let mut numeric = Vec::new();
    let mut box_tree = Vec::new();
    let mut inline = Vec::new();
    let mut finish = Vec::new();
    let mut retained = 0;
    let mut fragments = 0;
    let mut checksum = 0;
    for iteration in 0..iterations + 3 {
        let started = Instant::now();
        let output = build_layout_pass(&source, &mut styles, &mut services, request).unwrap();
        let duration = started.elapsed().as_secs_f64() * 1000.0;
        if iteration >= 3 {
            elapsed.push(duration);
            projection.push(output.metrics.projection_elapsed.as_secs_f64() * 1000.0);
            numeric.push(output.metrics.numeric_layout_elapsed.as_secs_f64() * 1000.0);
            box_tree.push(output.metrics.box_tree_elapsed.as_secs_f64() * 1000.0);
            inline.push(output.metrics.inline_preparation_elapsed.as_secs_f64() * 1000.0);
            finish.push(duration - output.metrics.elapsed.as_secs_f64() * 1000.0);
        }
        let memory = output.retention_metrics();
        retained = memory.estimated_geometry_bytes;
        fragments = memory.fragment_count;
        checksum = black_box(checksum ^ fragments ^ output.metrics.paint_operation_count);
    }
    let peak_kib = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:").map(|value| {
                value
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()
            })
        })
        .unwrap();
    println!(
        "{{\"case\":\"{case}\",\"mode\":\"{mode}\",\"count\":{count},\"iterations\":{iterations},\"elapsed_ms\":{},\"projection_ms\":{},\"numeric_ms\":{},\"box_tree_ms\":{},\"inline_ms\":{},\"finish_ms\":{},\"peak_rss_kib\":{peak_kib},\"retained_bytes\":{retained},\"fragments\":{fragments},\"checksum\":{checksum}}}",
        median(&mut elapsed),
        median(&mut projection),
        median(&mut numeric),
        median(&mut box_tree),
        median(&mut inline),
        median(&mut finish)
    );
}

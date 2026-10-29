//! Linux layout-backend cost probe. Compare release builds with identical inputs.
//! This measures layout and immutable geometry, not raster/encoding or protocol.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{self, Write},
    time::Instant,
};

use moli_layout::{
    DocumentLayoutServices, LayoutElementCategory, LayoutElementSemantics, LayoutError,
    LayoutFlushReason, LayoutNamespace, LayoutPassRequest, LayoutPassResult, LayoutQuery,
    LayoutQueryBatch, LayoutSource, LayoutSourceKind, LayoutStyleResolver, LayoutTableRole,
    LayoutViewport, PaintColor, ResolvedLayoutElementStyles, ResolvedLayoutStyle, SystemFontPolicy,
    WebFontFace, WebFontRegistration, build_layout_pass,
};
use style::{
    Atom,
    properties::ComputedValues,
    values::{
        computed::{
            CSSPixelLength, LengthPercentage, NonNegativeLengthPercentage,
            font::{
                FamilyName, FontFamily, FontFamilyList, FontFamilyNameSyntax, SingleFontFamily,
            },
            length::Size as ComputedSize,
        },
        generics::NonNegative,
        specified::box_::Display,
    },
};
use taffy::{Dimension, Size, Style, style_helpers::length};

struct Node {
    parent: Option<usize>,
    children: Vec<usize>,
    text: Option<String>,
    tag: &'static str,
    role: LayoutElementCategory,
}

struct Source(Vec<Node>);

impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> usize {
        0
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
    fn label(&self, node: usize) -> String {
        format!("{}-{node}", self.0[node].tag)
    }
    fn element_semantics(&self, node: usize) -> Option<LayoutElementSemantics> {
        self.0[node].text.is_none().then(|| {
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                self.0[node].tag,
                self.0[node].role,
                None,
            )
        })
    }
}

struct Styles(Vec<Option<ResolvedLayoutStyle>>);

impl LayoutStyleResolver<usize> for Styles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok(self.0[node]
            .clone()
            .map(ResolvedLayoutElementStyles::from_primary))
    }
}

fn css_length(value: f32) -> NonNegativeLengthPercentage {
    NonNegative(LengthPercentage::new_length(CSSPixelLength::new(value)))
}

fn text_style(base: &ComputedValues, display: Display, width: f32) -> ResolvedLayoutStyle {
    let mut computed = base.clone();
    computed.mutate_box().set_display(display);
    computed
        .mutate_position()
        .set_width(ComputedSize::LengthPercentage(css_length(width)));
    computed
        .mutate_position()
        .set_height(ComputedSize::LengthPercentage(css_length(32.0)));
    ResolvedLayoutStyle::from_stylo(style::servo_arc::Arc::new(computed))
}

fn append_node(
    source: &mut Source,
    styles: &mut Styles,
    parent: Option<usize>,
    tag: &'static str,
    role: LayoutElementCategory,
    style: Option<ResolvedLayoutStyle>,
    text: Option<String>,
) -> usize {
    let id = source.0.len();
    source.0.push(Node {
        parent,
        children: Vec::new(),
        text,
        tag,
        role,
    });
    styles.0.push(style);
    if let Some(parent) = parent {
        source.0[parent].children.push(id);
    }
    id
}

fn fixture(
    container: &str,
    unique: bool,
    count: usize,
    font: &ComputedValues,
) -> (Source, Styles, Vec<usize>) {
    use moli_layout::LayoutDisplay;
    let mut source = Source(Vec::new());
    let mut styles = Styles(Vec::new());
    let display = match container {
        "blocks" => LayoutDisplay::Block,
        "grid" => LayoutDisplay::Grid,
        "table" => LayoutDisplay::Table,
        _ => unreachable!(),
    };
    let root_style = ResolvedLayoutStyle::synthetic(
        display,
        Style {
            size: Size {
                width: Dimension::length(640.0),
                height: Dimension::auto(),
            },
            grid_template_columns: vec![length(160.0); 4],
            ..Style::default()
        },
        PaintColor::TRANSPARENT,
    );
    let role = if container == "table" {
        LayoutElementCategory::Table(LayoutTableRole::Table)
    } else {
        LayoutElementCategory::Generic
    };
    let root = append_node(
        &mut source,
        &mut styles,
        None,
        if container == "table" { "table" } else { "div" },
        role,
        Some(root_style),
        None,
    );
    let mut row = root;
    let mut cells = Vec::new();
    for index in 0..count {
        if container == "table" && index % 4 == 0 {
            row = append_node(
                &mut source,
                &mut styles,
                Some(root),
                "tr",
                LayoutElementCategory::Table(LayoutTableRole::Row),
                Some(ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::TableRow,
                    Style::default(),
                    PaintColor::TRANSPARENT,
                )),
                None,
            );
        }
        let cell = append_node(
            &mut source,
            &mut styles,
            Some(row),
            if container == "table" { "td" } else { "div" },
            if container == "table" {
                LayoutElementCategory::Table(LayoutTableRole::Cell)
            } else {
                LayoutElementCategory::Generic
            },
            Some(text_style(
                font,
                if container == "table" {
                    Display::TableCell
                } else {
                    Display::Block
                },
                100.0,
            )),
            None,
        );
        append_node(
            &mut source,
            &mut styles,
            Some(cell),
            "#text",
            LayoutElementCategory::Generic,
            None,
            Some(if unique {
                format!("Item {index:04} text")
            } else {
                "Alpha Beta Gamma".to_owned()
            }),
        );
        cells.push(cell);
    }
    (source, styles, cells)
}

fn cpu_ns() -> Result<u64, Box<dyn std::error::Error>> {
    Ok(std::fs::read_to_string("/proc/thread-self/schedstat")?
        .split_whitespace()
        .next()
        .ok_or("missing thread CPU counter")?
        .parse()?)
}

fn fingerprint(pass: &LayoutPassResult<usize>, cells: &[usize]) -> u64 {
    let mut hash = DefaultHasher::new();
    for &source in cells {
        let batch = LayoutQueryBatch::new(vec![LayoutQuery::ElementMetrics { source }]);
        format!("{:?}", pass.answer_queries(&batch).answers).hash(&mut hash);
        let text_rects = pass.text_range_rects(source + 1, 0..usize::MAX);
        assert!(
            !text_rects.is_empty(),
            "text must have actual font geometry"
        );
        format!("{text_rects:?}").hash(&mut hash);
    }
    hash.finish()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 4 {
        return Err(
            "usage: layout-cost <blocks|grid|table> <repeated|unique> <nodes> <iterations>".into(),
        );
    }
    let container = args[0].as_str();
    if !["blocks", "grid", "table"].contains(&container)
        || !["repeated", "unique"].contains(&args[1].as_str())
    {
        return Err("unknown container or text workload".into());
    }
    let count: usize = args[2].parse()?;
    let iterations: usize = args[3].parse()?;
    if count == 0 || iterations == 0 {
        return Err("nodes and iterations must be positive".into());
    }
    let mut font = style::properties::style_structs::Font::initial_values();
    font.set_font_family(FontFamily {
        families: FontFamilyList {
            list: style::ArcSlice::from_iter(std::iter::once(SingleFontFamily::FamilyName(
                FamilyName {
                    name: Atom::from("Moli Ahem"),
                    syntax: FontFamilyNameSyntax::Quoted,
                },
            ))),
        },
        is_system_font: false,
        is_initial: false,
    });
    let font = ComputedValues::initial_values_with_font_override(font);
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services.register_web_font(WebFontRegistration::new(
        "fixed",
        WebFontFace::new("Moli Ahem"),
        include_bytes!("../tests/fixtures/moli-ahem.ttf").to_vec(),
    ))?;
    let (source, mut styles, cells) = fixture(container, args[1] == "unique", count, &font);
    let request = LayoutPassRequest::new(
        LayoutViewport::new(1280, 720, 1.0),
        LayoutFlushReason::Explicit,
    );
    let mut last = build_layout_pass(&source, &mut styles, &mut services, request)?;
    println!(
        "{{\"phase\":\"ready\",\"nodes\":{count},\"iterations\":{iterations},\"fingerprint\":{},\"box_slot_bytes\":{},\"taffy_cache_bytes\":{}}}",
        fingerprint(&last, &cells),
        std::mem::size_of::<moli_layout::LayoutBox<usize>>(),
        std::mem::size_of::<taffy::Cache>()
    );
    io::stdout().flush()?;
    let mut control = String::new();
    io::stdin().read_line(&mut control)?;
    if control.trim() != "go" {
        return Err("expected go on stdin".into());
    }
    for step in 0..iterations {
        styles.0[cells[0]] = Some(text_style(
            &font,
            if container == "table" {
                Display::TableCell
            } else {
                Display::Block
            },
            120.0 + (step % 7) as f32 * 5.0,
        ));
        let before_cpu = cpu_ns()?;
        let started = Instant::now();
        let pass = build_layout_pass(&source, &mut styles, &mut services, request)?;
        let wall_ns = started.elapsed().as_nanos();
        let after_cpu = cpu_ns()?;
        last = pass;
        println!(
            "{{\"phase\":\"pass\",\"step\":{step},\"wall_ns\":{wall_ns},\"cpu_before_ns\":{before_cpu},\"cpu_after_ns\":{after_cpu},\"boxes\":{},\"fragments\":{}}}",
            last.metrics.box_count, last.metrics.fragment_count
        );
    }
    println!(
        "{{\"phase\":\"done\",\"fingerprint\":{},\"font_count\":{}}}",
        fingerprint(&last, &cells),
        services.web_font_count()
    );
    io::stdout().flush()?;
    control.clear();
    io::stdin().read_line(&mut control)?;
    if control.trim() != "exit" {
        return Err("expected exit on stdin".into());
    }
    Ok(())
}

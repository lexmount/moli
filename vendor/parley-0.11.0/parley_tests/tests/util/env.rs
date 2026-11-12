// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    BoundingBox, FontContext, FontFamily, FontFamilyName, Layout, LayoutContext, LineHeight,
    PlainEditor, PlainEditorDriver, PositionedLayoutItem, RangedBuilder, StyleProperty, TextStyle,
    TreeBuilder,
};
use peniko::{Color, kurbo::Size};
use vello_cpu::Pixmap;

use super::renderer::{
    ColorBrush, RenderingConfig, draw_layout, draw_layout_with_clusters, render_to_pixmap,
};

fn current_imgs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("current")
}

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots")
}

pub(crate) const FONT_FAMILY_LIST: &[FontFamilyName<'_>] = &[
    FontFamilyName::Named(Cow::Borrowed("Roboto")),
    FontFamilyName::Named(Cow::Borrowed("Noto Kufi Arabic")),
];

pub(crate) const CLUSTER_INFO_COLOR: Color = Color::from_rgba8(100, 100, 100, 255);

/// Color used to render the baseline of inline boxes. Chosen to contrast against the (black)
/// inline box fill.
pub(crate) const INLINE_BOX_BASELINE_COLOR: Color = Color::from_rgba8(255, 0, 255, 255);

pub(crate) struct TestEnv {
    test_name: String,
    check_counter: u32,
    font_cx: FontContext,
    layout_cx: LayoutContext<ColorBrush>,
    text_color: Color,
    rendering_config: RenderingConfig,
    cursor_size: f32,
    // TODO: Add core::panic::Location for case.
    errors: Vec<(PathBuf, String)>,
    next_test_case_name: String,
    /// Max size a screenshot can have, in bytes.
    max_screenshot_size: Option<usize>,
}

fn is_accept_mode() -> bool {
    std::env::var("PARLEY_TEST")
        .map(|x| x.eq_ignore_ascii_case("accept"))
        .unwrap_or(false)
}

fn is_generate_all_mode() -> bool {
    std::env::var("PARLEY_TEST")
        .map(|x| x.eq_ignore_ascii_case("generate-all"))
        .unwrap_or(false)
}

pub(crate) fn load_fonts(
    collection: &mut Collection,
    font_dirs: impl Iterator<Item = PathBuf>,
) -> std::io::Result<()> {
    for dir in font_dirs {
        let paths = std::fs::read_dir(dir)?;
        for entry in paths {
            let entry = entry?;
            if !entry.metadata()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_none_or(|ext| !["ttf", "otf", "ttc", "otc"].contains(&ext))
            {
                continue;
            }
            let font_data = std::fs::read(&path)?;
            collection.register_fonts(Blob::new(Arc::new(font_data)), None);
        }
    }
    Ok(())
}

pub(crate) fn create_font_context() -> FontContext {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    load_fonts(&mut collection, parley_dev::font_dirs()).unwrap();
    for font in FONT_FAMILY_LIST {
        if let FontFamilyName::Named(font_name) = font {
            collection
                .family_id(font_name)
                .unwrap_or_else(|| panic!("{font_name} font not found"));
        }
    }
    FontContext {
        collection,
        source_cache: SourceCache::default(),
    }
}

impl TestEnv {
    pub(crate) fn new<S: Into<Option<Size>>>(test_name: &str, size: S) -> Self {
        let file_prefix = format!("{test_name}-");
        let entries = std::fs::read_dir(current_imgs_dir()).unwrap();
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with(&file_prefix) && name.ends_with(".png"))
                .unwrap_or(false)
            {
                std::fs::remove_file(&path).unwrap();
            }
        }
        Self {
            test_name: test_name.to_string(),
            check_counter: 0,
            font_cx: create_font_context(),
            layout_cx: LayoutContext::new(),
            text_color: Color::BLACK,
            rendering_config: RenderingConfig {
                background_color: Color::WHITE,
                padding_color: Color::from_rgba8(166, 200, 255, 255),
                cursor_color: Color::from_rgba8(0, 0, 255, 255),
                selection_colors: [
                    Color::from_rgba8(0, 255, 0, 255),
                    Color::from_rgba8(255, 0, 0, 255),
                ],
                inline_box_color: Color::BLACK,
                size: size.into(),
                scale: 1.0,
                hint: false,
                glyph_transform: None,
            },
            cursor_size: 2.0,
            errors: Vec::new(),
            next_test_case_name: String::new(),
            max_screenshot_size: Some(32 * 1024),
        }
    }

    #[cfg(all(target_os = "macos", feature = "system"))]
    pub(crate) fn font_context(&mut self) -> &mut FontContext {
        &mut self.font_cx
    }

    pub(crate) fn collection(&mut self) -> &mut Collection {
        &mut self.font_cx.collection
    }

    pub(crate) fn rendering_config(&mut self) -> &mut RenderingConfig {
        &mut self.rendering_config
    }

    pub(crate) fn max_screenshot_size(&mut self) -> &mut Option<usize> {
        &mut self.max_screenshot_size
    }

    fn default_style(&self) -> [StyleProperty<'static, ColorBrush>; 3] {
        [
            StyleProperty::Brush(ColorBrush {
                color: self.text_color,
            }),
            StyleProperty::FontFamily(FontFamily::List(FONT_FAMILY_LIST.into())),
            // When our tests were originally generated, they used this style value, as it was the default.
            // To avoid regenerating all tests, as the default has now changed, set this explicitly.
            StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.0)),
        ]
    }

    pub(crate) fn ranged_builder<'a>(&'a mut self, text: &'a str) -> RangedBuilder<'a, ColorBrush> {
        let default_style = self.default_style();
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        for style in default_style {
            builder.push_default(style);
        }
        builder
    }

    pub(crate) fn tree_builder(&mut self) -> TreeBuilder<'_, ColorBrush> {
        let default_style = self.default_style();
        let mut builder =
            self.layout_cx
                .tree_builder(&mut self.font_cx, 1.0, true, &TextStyle::default());
        builder.push_style_modification_span(&default_style);
        builder
    }

    pub(crate) fn driver<'a>(
        &'a mut self,
        editor: &'a mut PlainEditor<ColorBrush>,
    ) -> PlainEditorDriver<'a, ColorBrush> {
        editor.driver(&mut self.font_cx, &mut self.layout_cx)
    }

    pub(crate) fn editor(&mut self, text: &str) -> PlainEditor<ColorBrush> {
        let mut editor = PlainEditor::new(16.);
        for style in self.default_style() {
            editor.edit_styles().insert(style);
        }
        editor.set_text(text);
        editor
    }

    fn image_name(&mut self, test_case_name: &str) -> String {
        if test_case_name.is_empty() {
            let name = format!("{}-{}.png", self.test_name, self.check_counter);
            self.check_counter += 1;
            name
        } else {
            assert!(
                test_case_name
                    .chars()
                    .all(|c| c == '_' || char::is_alphanumeric(c)),
                "snapshot filename should not contain special characters"
            );
            format!("{}-{}.png", self.test_name, test_case_name)
        }
    }

    fn check_images(&self, current_png: &[u8], snapshot_path: &Path) -> Result<(), String> {
        if !snapshot_path.is_file() {
            return Err(format!("Cannot find snapshot {}", snapshot_path.display()));
        }
        let snapshot_file = File::open(snapshot_path).unwrap();
        let snapshot_img = Pixmap::from_png(BufReader::new(snapshot_file)).unwrap();
        // `Pixmap::into_png` unpremultiplies with rounding but `Pixmap::from_png`
        // re-premultiplies with truncation, so a decoded snapshot is not bit-identical
        // to the pixmap it was written from. Compare the decoded form of the PNG we
        // would save, so both sides carry the same (deterministic) error.
        let current_img = Pixmap::from_png(Cursor::new(current_png)).unwrap();
        if snapshot_img.width() != current_img.width()
            || snapshot_img.height() != current_img.height()
        {
            return Err(format!(
                "Snapshot has different size: snapshot {}x{}; generated image: {}x{}",
                snapshot_img.width(),
                snapshot_img.height(),
                current_img.width(),
                current_img.height()
            ));
        }

        let mut n_different_pixels = 0;
        let mut color_cumulative_difference = 0.0;
        for (pixel1, pixel2) in snapshot_img.data().iter().zip(current_img.data()) {
            if pixel1 != pixel2 {
                n_different_pixels += 1;
            }
            let diff_r = (pixel1.r as f32 - pixel2.r as f32).abs();
            let diff_g = (pixel1.g as f32 - pixel2.g as f32).abs();
            let diff_b = (pixel1.b as f32 - pixel2.b as f32).abs();
            color_cumulative_difference += diff_r.max(diff_g).max(diff_b);
        }
        if n_different_pixels > 0 {
            return Err(format!(
                "Testing image differs in {n_different_pixels} pixels (color difference = {color_cumulative_difference})",
            ));
        }
        Ok(())
    }

    pub(crate) fn with_name(&mut self, test_case_name: &str) -> &mut Self {
        self.next_test_case_name = test_case_name.to_string();
        self
    }

    pub(crate) fn check_editor_snapshot(&mut self, editor: &mut PlainEditor<ColorBrush>) {
        editor.refresh_layout(&mut self.font_cx, &mut self.layout_cx);
        self.render_and_check_snapshot(
            editor.try_layout().unwrap(),
            editor.cursor_geometry(self.cursor_size),
            &editor.selection_geometry(),
        );
    }

    pub(crate) fn check_layout_snapshot(&mut self, layout: &Layout<ColorBrush>) {
        self.render_and_check_snapshot(layout, None, &[]);
    }

    pub(crate) fn render_and_check_snapshot(
        &mut self,
        layout: &Layout<ColorBrush>,
        cursor_rect: Option<BoundingBox>,
        selection_rects: &[(BoundingBox, usize)],
    ) {
        let renderer = draw_layout(&self.rendering_config, layout, cursor_rect, selection_rects);
        let current_img = render_to_pixmap(renderer);
        self.check_image(&current_img);
    }

    pub(crate) fn check_cluster_snapshot(
        &mut self,
        layout: &Layout<ColorBrush>,
        text: &str,
        char_info_font_size: f32,
    ) {
        // Clusters span graphemes, which can be multiple characters of source text (e.g. a base
        // letter plus combining marks). Build a reference layout per unique grapheme.
        let mut grapheme_layouts = HashMap::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                for cluster in glyph_run.run().clusters() {
                    let grapheme = &text[cluster.text_range()];
                    if grapheme_layouts.contains_key(grapheme) {
                        continue;
                    }
                    let mut builder = self.ranged_builder(grapheme);
                    builder.push_default(StyleProperty::FontSize(char_info_font_size));
                    builder.push_default(StyleProperty::Brush(ColorBrush::new(CLUSTER_INFO_COLOR)));
                    let mut grapheme_layout = builder.build(grapheme);
                    grapheme_layout.break_all_lines(Some(400.0));
                    grapheme_layouts.insert(grapheme.to_string(), grapheme_layout);
                }
            }
        }

        let renderer =
            draw_layout_with_clusters(&self.rendering_config, layout, text, &grapheme_layouts);
        let current_img = render_to_pixmap(renderer);
        self.check_image(&current_img);
    }

    pub(crate) fn check_image(&mut self, img: &Pixmap) {
        let test_case_name = std::mem::take(&mut self.next_test_case_name);
        let image_name = self.image_name(&test_case_name);

        let snapshot_path = snapshot_dir().join(&image_name);
        let comparison_path = current_imgs_dir().join(&image_name);

        #[cfg(not(target_os = "android"))]
        #[track_caller]
        fn save_image(image_data: &[u8], path: &PathBuf, max_size: Option<usize>) {
            use oxipng::{Options, optimize_from_memory};

            let data = optimize_from_memory(image_data, &Options::from_preset(5)).unwrap();
            let saved_len = data.len();

            // Whenever we save a file, we optimise it just in case.
            // In some cases this is redundant with what kompari already does,
            // but it's simpler to cover all cases.
            std::fs::write(path, data).unwrap();

            if let Some(max_size) = max_size
                && saved_len > max_size
            {
                panic!(
                    "New screenshot file size ({saved_len}b) was larger than the supported file size ({max_size}b). See TestEnv::max_screenshot_size to change the maximum.",
                );
            }
        }

        // We special-case android targets because oxipng doesn't build in Android CI.
        #[cfg(target_os = "android")]
        fn save_image(_image_data: &[u8], _path: &PathBuf, _max_size: Option<usize>) {
            panic!("Saving screenshots is not supported on Android targets");
        }

        // Max size a screenshot can have, in bytes.
        let max_size = self.max_screenshot_size;
        let png = img.clone().into_png().unwrap();
        if let Err(e) = self.check_images(&png, &snapshot_path) {
            if is_accept_mode() {
                save_image(&png, &snapshot_path, max_size);
            } else {
                save_image(&png, &comparison_path, max_size);
                self.errors.push((comparison_path, e));
            }
        } else if is_generate_all_mode() {
            save_image(&png, &comparison_path, max_size);
        } else {
            let reference_size = snapshot_path.metadata().unwrap().len() as usize;
            if let Some(max_size) = max_size
                && reference_size > max_size
            {
                panic!(
                    "Existing file size ({reference_size}b) was larger than the supported file size ({max_size}b). See TestEnv::max_screenshot_size to change the maximum."
                );
            }
        }
    }
}

impl Drop for TestEnv {
    // Dropping of TestEnv cause panic (if there is not already one)
    // We do not panic immediately when error is detected because we want to
    // generate all images in the test and do visual confirmation of the whole
    // set and not stop at the first error.
    fn drop(&mut self) {
        if !self.errors.is_empty() && !std::thread::panicking() {
            use std::fmt::Write;
            let mut panic_msg = String::new();
            for (path, msg) in &self.errors {
                write!(
                    &mut panic_msg,
                    "{}\nImage written into: {}\n",
                    msg,
                    path.display()
                )
                .unwrap();
            }
            panic!("{}", &panic_msg);
        }
    }
}
